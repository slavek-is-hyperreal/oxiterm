//! Name lookup and lowering of syntactic types to [`Ty`].

use crate::ty::*;
use crate::*;

impl<'a> Checker<'a> {
    // ---- scopes -----------------------------------------------------------

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }
    pub fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub fn builtin_type(name: &str) -> Option<Binding> {
        Some(match name {
            "bool" => Binding::Prim(BOOL),
            "string" => Binding::Prim(STR),
            "void" => Binding::Prim(VOID),
            "never" => Binding::Prim(NEVER),
            "Array" => Binding::ArrayCtor,
            n => Binding::Prim(num(NumTy::from_name(n)?)),
        })
    }

    /// Lookup of a value-position name.
    pub fn lookup(&self, name: &str) -> Option<Binding> {
        for s in self.scopes.iter().rev() {
            if let Some(b) = s.get(name) {
                return Some(*b);
            }
        }
        if let Some(b) = self.mods[self.cm].top.get(name) {
            return Some(*b);
        }
        if let Some(b) = self.mods[self.prelude].top.get(name) {
            return Some(*b);
        }
        match name {
            "range" => Some(Binding::BuiltinFn(Builtin::Range)),
            _ => None,
        }
    }

    /// Lookup of a type-position name (locals are skipped).
    pub fn lookup_type(&self, name: &str) -> Option<Binding> {
        for s in self.scopes.iter().rev() {
            if let Some(b) = s.get(name) {
                if !matches!(b, Binding::Local(_)) {
                    return Some(*b);
                }
            }
        }
        if let Some(b) = self.mods[self.cm].top.get(name) {
            return Some(*b);
        }
        if let Some(b) = self.mods[self.prelude].top.get(name) {
            return Some(*b);
        }
        Self::builtin_type(name)
    }

    /// Bind a name in the innermost block scope.
    pub fn bind_scope(&mut self, name: &str, b: Binding, span: Span) -> bool {
        if name == "_" {
            return true;
        }
        let dup = self.scopes.last().is_some_and(|s| s.contains_key(name));
        if dup {
            self.err("OXS-E0301", span, &[("name", name)]);
            return false;
        }
        if let Some(s) = self.scopes.last_mut() {
            s.insert(name.to_string(), b);
        }
        true
    }

    // ---- types ------------------------------------------------------------

    pub fn lower_type(&mut self, t: &'a TypeExpr) -> Ty {
        let r = self.lower_type_inner(t, false);
        self.check_overflow(t.span);
        r
    }

    /// A type that may name an interface (bounds, `implements`, `extends`).
    pub fn lower_bound_type(&mut self, t: &'a TypeExpr) -> Ty {
        self.lower_type_inner(t, true)
    }

    pub fn check_overflow(&mut self, span: Span) {
        if self.tys.overflowed {
            self.tys.overflowed = false;
            self.err(
                "OXS-E0113",
                span,
                &[("detail", "type is nested too deeply or too many types were created")],
            );
        }
    }

    fn lower_type_inner(&mut self, t: &'a TypeExpr, iface_ok: bool) -> Ty {
        match &t.kind {
            TypeExprKind::Paren(i) => self.lower_type_inner(i, iface_ok),
            TypeExprKind::Null => NULL,
            TypeExprKind::Array(e) => {
                let et = self.lower_type_inner(e, false);
                if et == VOID || et == NEVER {
                    self.err("OXS-E0115", t.span, &[("detail", "arrays of `void` or `never` are not allowed")]);
                    return ERROR;
                }
                self.tys.array(et)
            }
            TypeExprKind::Fn { params, ret } => {
                let ps: Vec<Ty> = params.iter().map(|p| self.lower_type_inner(&p.ty, false)).collect();
                let r = self.lower_type_inner(ret, false);
                self.tys.func(&ps, r)
            }
            TypeExprKind::Lit(_) => {
                self.err(
                    "OXS-E0115",
                    t.span,
                    &[("detail", "literal types are only allowed as the `kind` field of a union member")],
                );
                ERROR
            }
            TypeExprKind::Object(_) => {
                self.err(
                    "OXS-E0115",
                    t.span,
                    &[("detail", "anonymous object types are only allowed as members of a union or as the right side of a `type` alias; declare a `struct`")],
                );
                ERROR
            }
            TypeExprKind::Union(ms) => {
                let mut nonnull: Vec<Ty> = Vec::new();
                let mut has_null = false;
                for m in ms {
                    let mt = self.lower_type_inner(m, false);
                    if mt == NULL {
                        has_null = true;
                    } else if mt != ERROR {
                        nonnull.push(mt);
                    } else {
                        return ERROR;
                    }
                }
                nonnull.dedup();
                if nonnull.len() == 1 && has_null {
                    let n = nonnull[0];
                    if n == VOID || n == NEVER {
                        self.err("OXS-E0115", t.span, &[("detail", "`void | null` is not a type")]);
                        return ERROR;
                    }
                    return self.tys.nullable(n);
                }
                if nonnull.len() == 1 {
                    return nonnull[0];
                }
                self.err(
                    "OXS-E0115",
                    t.span,
                    &[("detail", "unions other than `T | null` must be declared as a discriminated union (`type Shape = { kind: \"a\", ... } | { kind: \"b\", ... }`); `string | number` style unions are not supported")],
                );
                ERROR
            }
            TypeExprKind::Path { segments, args } => self.lower_path_type(t, segments, args, iface_ok),
        }
    }

    fn lower_path_type(
        &mut self,
        t: &'a TypeExpr,
        segments: &'a [Ident],
        args: &'a [TypeExpr],
        iface_ok: bool,
    ) -> Ty {
        // resolve to a binding
        let first = &segments[0];
        let b = if segments.len() == 1 {
            self.lookup_type(&first.name)
        } else {
            // namespace-qualified
            match self.lookup_type(&first.name) {
                Some(Binding::Def(d)) if self.kind_of(d) == DefKind::Namespace => {
                    let m = self.defs[d.0 as usize].info.target_module.unwrap_or(0) as usize;
                    let name = &segments[1].name;
                    if segments.len() > 2 {
                        self.err("OXS-E0300", t.span, &[("name", &self.span_text(t.span).to_string())]);
                        return ERROR;
                    }
                    match self.mods[m].exports.get(name) {
                        Some(d) => Some(Binding::Def(*d)),
                        None => {
                            self.err("OXS-E0300", segments[1].span, &[("name", name)]);
                            return ERROR;
                        }
                    }
                }
                Some(_) => {
                    self.err("OXS-E0303", first.span, &[("name", &first.name), ("found", "not a namespace"), ("expected", "a namespace")]);
                    return ERROR;
                }
                None => None,
            }
        };
        let Some(b) = b else {
            self.err("OXS-E0300", first.span, &[("name", &first.name)]);
            return ERROR;
        };
        let argtys: Vec<Ty> = args.iter().map(|a| self.lower_type_inner(a, false)).collect();
        if argtys.contains(&ERROR) {
            return ERROR;
        }
        let name = &segments[segments.len() - 1].name;
        match b {
            Binding::Prim(p) => {
                if !args.is_empty() {
                    self.err("OXS-E0117", t.span, &[("detail", &format!("type `{name}` takes no type arguments"))]);
                    return ERROR;
                }
                p
            }
            Binding::ArrayCtor => {
                if argtys.len() != 1 {
                    self.err("OXS-E0117", t.span, &[("detail", "`Array` takes exactly one type argument")]);
                    return ERROR;
                }
                if argtys[0] == VOID || argtys[0] == NEVER {
                    self.err("OXS-E0115", t.span, &[("detail", "arrays of `void` or `never` are not allowed")]);
                    return ERROR;
                }
                self.tys.array(argtys[0])
            }
            Binding::TParam(p) => {
                if !args.is_empty() {
                    self.err("OXS-E0117", t.span, &[("detail", &format!("type parameter `{name}` takes no type arguments"))]);
                    return ERROR;
                }
                self.tys.mk(TyKind::Param(p))
            }
            Binding::Local(_) | Binding::BuiltinFn(_) => ERROR,
            Binding::Def(d) => self.type_of_def(d, argtys, t.span, iface_ok),
        }
    }

    /// The type named by definition `d` applied to `args`.
    pub fn type_of_def(&mut self, d: DefId, args: Vec<Ty>, span: Span, iface_ok: bool) -> Ty {
        let info = &self.defs[d.0 as usize].info;
        let kind = info.kind;
        let name = info.name.clone();
        match kind {
            DefKind::Struct
            | DefKind::Class
            | DefKind::Union
            | DefKind::Variant { .. }
            | DefKind::Enum
            | DefKind::Interface
            | DefKind::Host
            | DefKind::Handle => {
                let want = self.defs[d.0 as usize].info.tparams.len();
                if args.len() != want {
                    self.err(
                        "OXS-E0117",
                        span,
                        &[("detail", &format!("Generic type '{name}' requires {want} type argument(s)"))],
                    );
                    return ERROR;
                }
                if kind == DefKind::Interface && !iface_ok {
                    self.err(
                        "OXS-E0115",
                        span,
                        &[("detail", &format!("interface `{name}` cannot be used as the type of a value yet: interfaces are generic bounds only (`<T extends {name}>`)"))],
                    );
                    return ERROR;
                }
                if want > 0 {
                    self.defer_bounds(d, &args, span);
                }
                self.tys.adt(d, &args)
            }
            DefKind::Alias => {
                self.ensure_alias(d);
                let Some(&target) = self.aliases.get(&d) else {
                    return ERROR;
                };
                let tps = self.defs[d.0 as usize].info.tparams.clone();
                if tps.len() != args.len() {
                    self.err(
                        "OXS-E0117",
                        span,
                        &[("detail", &format!("Generic type '{name}' requires {} type argument(s)", tps.len()))],
                    );
                    return ERROR;
                }
                let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(args).collect();
                self.tys.subst(target, &map)
            }
            _ => {
                self.err(
                    "OXS-E0303",
                    span,
                    &[("name", &name), ("found", "a value"), ("expected", "a type")],
                );
                ERROR
            }
        }
    }

    // ---- bounds -----------------------------------------------------------

    fn defer_bounds(&mut self, d: DefId, args: &[Ty], span: Span) {
        let any = self.defs[d.0 as usize]
            .info
            .tparams
            .iter()
            .any(|p| self.params[p.0 as usize].bound.is_some() || true);
        if any {
            self.pending_bounds.push((d, args.to_vec(), span, self.cm));
        }
    }

    /// Verify queued `T extends I` constraints of generic type applications.
    pub fn flush_bounds(&mut self) {
        let pend = std::mem::take(&mut self.pending_bounds);
        for (d, args, span, m) in pend {
            let saved = self.cm;
            self.cm = m;
            let tps = self.defs[d.0 as usize].info.tparams.clone();
            self.check_bounds(&tps, &args, span);
            self.cm = saved;
        }
    }

    /// Check `args` against the bounds of `tps`; reports `OXS-E0117`.
    pub fn check_bounds(&mut self, tps: &[ParamId], args: &[Ty], span: Span) -> bool {
        let map: Vec<(ParamId, Ty)> = tps.iter().copied().zip(args.iter().copied()).collect();
        let mut ok = true;
        for (i, p) in tps.iter().enumerate() {
            let Some(b) = self.params[p.0 as usize].bound else { continue };
            let bt = self.tys.subst(b, &map);
            let a = args[i];
            if a == ERROR || bt == ERROR {
                continue;
            }
            if let Err(why) = self.satisfies(a, bt) {
                let (sa, sb) = (self.show(a), self.show(bt));
                self.err(
                    "OXS-E0117",
                    span,
                    &[("detail", &format!("Type '{sa}' does not satisfy the constraint '{sb}': {why}"))],
                );
                ok = false;
            }
        }
        ok
    }

    /// Does `t` structurally satisfy interface type `iface`?
    pub fn satisfies(&mut self, t: Ty, iface: Ty) -> Result<(), String> {
        let TyKind::Adt(idef, iargs) = self.tys.kind(iface).clone() else {
            return Err("not an interface".into());
        };
        if self.kind_of(idef) != DefKind::Interface {
            return Err("not an interface".into());
        }
        // a type parameter satisfies its own bound (and bounds that extend it)
        if let TyKind::Param(p) = self.tys.kind(t).clone() {
            if let Some(b) = self.params[p.0 as usize].bound {
                if b == iface {
                    return Ok(());
                }
                if let TyKind::Adt(bd, bargs) = self.tys.kind(b).clone() {
                    if bd == idef && bargs == iargs {
                        return Ok(());
                    }
                }
                // bound's methods cover the interface's methods
                self.ensure_iface(idef);
                let need = self.iface_methods_inst(idef, &iargs);
                let have = {
                    if let TyKind::Adt(bd, bargs) = self.tys.kind(b).clone() {
                        self.ensure_iface(bd);
                        self.iface_methods_inst(bd, &bargs)
                    } else {
                        Vec::new()
                    }
                };
                for (n, ps, r) in need {
                    if !have.iter().any(|(hn, hp, hr)| *hn == n && *hp == ps && *hr == r) {
                        return Err(format!("missing method `{n}`"));
                    }
                }
                return Ok(());
            }
            return Err("type parameter has no bound".into());
        }
        self.ensure_iface(idef);
        let need = self.iface_methods_inst(idef, &iargs);
        for (n, ps, r) in need {
            match self.find_method_sig(t, &n) {
                Some((mps, mr, mutating)) => {
                    if mutating || mps != ps || mr != r {
                        let (a, b) = (
                            format!("({})", ps.iter().map(|x| self.show(*x)).collect::<Vec<_>>().join(", ")),
                            format!("({})", mps.iter().map(|x| self.show(*x)).collect::<Vec<_>>().join(", ")),
                        );
                        return Err(format!(
                            "method `{n}` has signature {b}: {}, expected {a}: {}{}",
                            self.show(mr),
                            self.show(r),
                            if mutating { " (and is `mutating`)" } else { "" }
                        ));
                    }
                }
                None => return Err(format!("property `{n}` is missing")),
            }
        }
        Ok(())
    }

    /// Methods of interface `d` applied to `args`, as `(name, param types, ret)`.
    pub fn iface_methods_inst(&mut self, d: DefId, args: &[Ty]) -> Vec<(String, Vec<Ty>, Ty)> {
        let Some(sig) = self.ifaces.get(&d).cloned() else { return Vec::new() };
        let tps = self.defs[d.0 as usize].info.tparams.clone();
        let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(args.iter().copied()).collect();
        sig.methods
            .iter()
            .map(|m| {
                let ps: Vec<Ty> = m.params.iter().map(|p| self.tys.subst(*p, &map)).collect();
                let r = self.tys.subst(m.ret, &map);
                (m.name.clone(), ps, r)
            })
            .collect()
    }

    /// Signature `(params, ret, mutating)` of a method of a struct/class type, instantiated.
    pub fn find_method_sig(&mut self, t: Ty, name: &str) -> Option<(Vec<Ty>, Ty, bool)> {
        let TyKind::Adt(d, args) = self.tys.kind(t).clone() else { return None };
        if !matches!(self.kind_of(d), DefKind::Struct | DefKind::Class) {
            return None;
        }
        self.ensure_record(d);
        let rec = self.records.get(&d)?;
        let (_, m) = rec.methods.iter().find(|(n, _)| n == name).cloned()?;
        self.ensure_fn(m);
        let sig = self.fns.get(&m)?.clone();
        if sig.is_static || !sig.tparams.is_empty() && sig.tparams.len() > args.len() {
            return None;
        }
        let tps = self.defs[d.0 as usize].info.tparams.clone();
        let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(args.iter().copied()).collect();
        let ps = sig.params.iter().map(|p| self.tys.subst(p.ty, &map)).collect();
        let r = self.tys.subst(sig.ret, &map);
        Some((ps, r, sig.mutating))
    }
}
