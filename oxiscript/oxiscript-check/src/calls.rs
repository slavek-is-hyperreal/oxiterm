//! Member access, calls, `new`, generic argument inference.

use crate::expr::strip_paren;
use crate::ty::*;
use crate::*;

/// What a call needs to know about its target.
#[derive(Clone, Debug)]
pub(crate) struct Callee {
    /// Own type parameters still to be solved (explicit or inferred).
    pub tparams: Vec<ParamId>,
    pub params: Vec<Ty>,
    pub defaults: Vec<bool>,
    pub ret: Ty,
    pub def: Option<DefId>,
    /// Owner type arguments already fixed by the receiver.
    pub owner_args: Vec<Ty>,
    pub mutating: bool,
    pub builtin: Option<Builtin>,
}

pub(crate) enum MemberOut {
    Value(Ty, Res),
    Callable(Callee, Res),
    Err,
}

impl<'a> Checker<'a> {
    fn static_target(&mut self, obj: &'a Expr) -> Option<DefId> {
        let o = strip_paren(obj);
        let ExprKind::Ident(id) = &o.kind else { return None };
        match self.lookup(&id.name) {
            Some(Binding::Def(d)) if matches!(self.kind_of(d), DefKind::Enum | DefKind::Class | DefKind::Struct | DefKind::Namespace) => Some(d),
            _ => None,
        }
    }

    pub fn member_expr(&mut self, e: &'a Expr, obj: &'a Expr, name: &'a Ident, optional: bool) -> Ty {
        match self.resolve_member(e, obj, name, optional) {
            MemberOut::Value(t, r) => {
                self.set_res(e.id, r);
                if matches!(r, Res::Field { .. }) {
                    if let Some(p) = self.path_of(e) {
                        if let Some(f) = self.flow.get(p) {
                            return f;
                        }
                    }
                }
                t
            }
            MemberOut::Callable(c, r) => {
                self.set_res(e.id, r);
                // a method used as a value
                let _ = c;
                self.err(
                    "OXS-E0115",
                    e.span,
                    &[("detail", &format!("method `{}` cannot be used as a value; call it or wrap it in an arrow function", name.name))],
                );
                ERROR
            }
            MemberOut::Err => ERROR,
        }
    }

    pub(crate) fn private_ok(&self, owner: DefId) -> bool {
        self.fnctx.iter().any(|c| c.owner == Some(owner))
    }

    pub(crate) fn resolve_member(&mut self, e: &'a Expr, obj: &'a Expr, name: &'a Ident, optional: bool) -> MemberOut {
        // static access: Enum.Member, Class.field, Class.method, ns.item
        if let Some(d) = self.static_target(obj) {
            return self.static_member(e, obj, d, name);
        }
        let is_this = matches!(strip_paren(obj).kind, ExprKind::This);
        if is_this {
            self.this_member_ok = true;
        }
        let ot = self.expr(obj, None);
        self.this_member_ok = false;
        if ot == ERROR {
            return MemberOut::Err;
        }
        let base = match self.tys.kind(ot).clone() {
            TyKind::Nullable(i) => {
                if optional {
                    i
                } else {
                    self.possibly_null(obj, ot);
                    return MemberOut::Err;
                }
            }
            TyKind::Null => {
                self.possibly_null(obj, ot);
                return MemberOut::Err;
            }
            _ => {
                if optional {
                    if self.result_parts(ot).is_some() {
                        let txt = self.span_text(obj.span).to_string();
                        self.err("OXS-E0106", e.span, &[("expr", &txt)]);
                    } else {
                        let s = self.show(ot);
                        self.err("OXS-E0127", obj.span, &[("expr", self.span_text(obj.span)), ("ty", &s)]);
                    }
                    return MemberOut::Err;
                }
                ot
            }
        };
        let out = self.lookup_member(e, obj, base, name, is_this);
        if optional {
            // the chain is nullable
            match out {
                MemberOut::Value(t, r) => {
                    let n = self.tys.nullable(t);
                    return MemberOut::Value(n, r);
                }
                MemberOut::Callable(mut c, r) => {
                    c.ret = self.tys.nullable(c.ret);
                    return MemberOut::Callable(c, r);
                }
                MemberOut::Err => return MemberOut::Err,
            }
        }
        out
    }

    fn static_member(&mut self, e: &'a Expr, obj: &'a Expr, d: DefId, name: &'a Ident) -> MemberOut {
        let kind = self.kind_of(d);
        self.set_res(strip_paren(obj).id, Res::Def(d));
        match kind {
            DefKind::Enum => {
                self.ensure_enum(d);
                let ty = self.tys.adt(d, &[]);
                self.set_ty(obj.id, ty);
                self.set_ty(strip_paren(obj).id, ty);
                let Some(sig) = self.enums.get(&d) else { return MemberOut::Err };
                if let Some(i) = sig.members.iter().position(|(n, _)| *n == name.name) { MemberOut::Value(ty, Res::EnumMember { def: d, index: i as u32 }) } else {
                    let n = self.def_name(d).to_string();
                    self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &format!("typeof {n}"))]);
                    MemberOut::Err
                }
            }
            DefKind::Class | DefKind::Struct => {
                self.ensure_record(d);
                let targs: Vec<Ty> = self.defs[d.0 as usize].info.tparams.iter().map(|p| self.tys.mk(TyKind::Param(*p))).collect();
                let ty = self.tys.adt(d, &targs);
                self.set_ty(obj.id, ty);
                self.set_ty(strip_paren(obj).id, ty);
                let Some(rec) = self.records.get(&d).cloned() else { return MemberOut::Err };
                if let Some(i) = rec.fields.iter().position(|f| f.is_static && f.name == name.name) {
                    let f = &rec.fields[i];
                    if f.private && !self.private_ok(d) {
                        self.err("OXS-E0119", name.span, &[("detail", &format!("Property '{}' is private and only accessible within class '{}'", name.name, self.def_name(d)))]);
                    }
                    self.note_effect(2);
                    self.note_global_read(crate::inst::GKey::Static(d, i as u32), name.span);
                    if self.in_law() {
                        self.err("OXS-E0125", name.span, &[("detail", "a law cannot read static state")]);
                    }
                    return MemberOut::Value(f.ty, Res::StaticField { owner: d, index: i as u32 });
                }
                if let Some((_, m)) = rec.methods.iter().find(|(n, _)| *n == name.name) {
                    self.ensure_fn(*m);
                    let Some(sig) = self.fns.get(m).cloned() else { return MemberOut::Err };
                    if !sig.is_static {
                        self.err("OXS-E0119", name.span, &[("detail", &format!("'{}' is an instance method; call it on a value, not on the type", name.name))]);
                        return MemberOut::Err;
                    }
                    if sig.private && !self.private_ok(d) {
                        self.err("OXS-E0119", name.span, &[("detail", &format!("Method '{}' is private and only accessible within class '{}'", name.name, self.def_name(d)))]);
                    }
                    let callee = Callee {
                        tparams: sig.tparams.clone(),
                        params: sig.params.iter().map(|p| p.ty).collect(),
                        defaults: sig.params.iter().map(|p| p.has_default).collect(),
                        ret: sig.ret,
                        def: Some(*m),
                        owner_args: vec![],
                        mutating: false,
                        builtin: None,
                    };
                    return MemberOut::Callable(callee, Res::Method(*m));
                }
                let n = self.def_name(d).to_string();
                self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &format!("typeof {n}"))]);
                MemberOut::Err
            }
            DefKind::Namespace => {
                self.set_ty(obj.id, VOID);
                self.set_ty(strip_paren(obj).id, VOID);
                let Some(m) = self.defs[d.0 as usize].info.target_module else { return MemberOut::Err };
                let Some(&t) = self.mods[m as usize].exports.get(&name.name) else {
                    self.err("OXS-E0300", name.span, &[("name", &name.name)]);
                    return MemberOut::Err;
                };
                match self.kind_of(t) {
                    DefKind::Fn => {
                        self.ensure_fn(t);
                        let Some(sig) = self.fns.get(&t).cloned() else { return MemberOut::Err };
                        let callee = Callee {
                            tparams: sig.tparams.clone(),
                            params: sig.params.iter().map(|p| p.ty).collect(),
                            defaults: sig.params.iter().map(|p| p.has_default).collect(),
                            ret: sig.ret,
                            def: Some(t),
                            owner_args: vec![],
                            mutating: false,
                            builtin: None,
                        };
                        MemberOut::Callable(callee, Res::Def(t))
                    }
                    DefKind::Global => {
                        self.ensure_global(t);
                        MemberOut::Value(self.globals.get(&t).map_or(ERROR, |g| g.ty), Res::Def(t))
                    }
                    _ => {
                        let n = name.name.clone();
                        self.err("OXS-E0303", e.span, &[("name", &n), ("found", "a type"), ("expected", "a value")]);
                        MemberOut::Err
                    }
                }
            }
            _ => MemberOut::Err,
        }
    }

    fn lookup_member(&mut self, e: &'a Expr, obj: &'a Expr, base: Ty, name: &'a Ident, is_this: bool) -> MemberOut {
        let kind = self.tys.kind(base).clone();
        match kind {
            TyKind::Adt(d, args) => {
                let dk = self.kind_of(d);
                match dk {
                    DefKind::Struct | DefKind::Class | DefKind::Variant { .. } => {
                        self.ensure_record(d);
                        let Some(rec) = self.records.get(&d).cloned() else { return MemberOut::Err };
                        let tps = self.defs[d.0 as usize].info.tparams.clone();
                        let map: Vec<(ParamId, Ty)> = tps.iter().copied().zip(args.iter().copied()).collect();
                        if let Some(i) = rec.fields.iter().position(|f| !f.is_static && f.name == name.name) {
                            let f = &rec.fields[i];
                            if f.private && !self.private_ok(d) {
                                self.err("OXS-E0119", name.span, &[("detail", &format!("Property '{}' is private and only accessible within class '{}'", name.name, self.def_name(d)))]);
                            }
                            if is_this && !self.assign_target && self.flow.is_unassigned(0x8000_0000 | i as u32) {
                                self.err("OXS-E0118", name.span, &[("name", &format!("this.{}", name.name))]);
                            }
                            let ty = self.tys.subst(f.ty, &map);
                            return MemberOut::Value(ty, Res::Field { owner: d, index: i as u32 });
                        }
                        if let Some((disc, _)) = rec.discr.as_ref().map(|(n, v)| (n.clone(), v.clone())) {
                            if disc == name.name {
                                let ty = self.discr_ty_of_variant(d);
                                if let DefKind::Variant { union: Some(u), .. } = dk {
                                    return MemberOut::Value(ty, Res::Discriminant { union: u });
                                }
                                return MemberOut::Value(ty, Res::Discriminant { union: d });
                            }
                        }
                        if let Some((_, m)) = rec.methods.iter().find(|(n, _)| *n == name.name) {
                            self.ensure_fn(*m);
                            let Some(sig) = self.fns.get(m).cloned() else { return MemberOut::Err };
                            if sig.is_static {
                                self.err("OXS-E0119", name.span, &[("detail", &format!("'{}' is a static method; call it on the type", name.name))]);
                                return MemberOut::Err;
                            }
                            if sig.private && !self.private_ok(d) {
                                self.err("OXS-E0119", name.span, &[("detail", &format!("Method '{}' is private and only accessible within class '{}'", name.name, self.def_name(d)))]);
                            }
                            if is_this && self.flow.has_unassigned_fields() {
                                self.err("OXS-E0119", name.span, &[("detail", "a method cannot be called before every field is assigned in the constructor")]);
                            }
                            let own: Vec<ParamId> = sig.tparams.iter().copied().skip(tps.len()).collect();
                            let params: Vec<Ty> = sig.params.iter().map(|p| self.tys.subst(p.ty, &map)).collect();
                            let ret = self.tys.subst(sig.ret, &map);
                            let callee = Callee {
                                tparams: own,
                                params,
                                defaults: sig.params.iter().map(|p| p.has_default).collect(),
                                ret,
                                def: Some(*m),
                                owner_args: args.to_vec(),
                                mutating: sig.mutating,
                                builtin: None,
                            };
                            return MemberOut::Callable(callee, Res::Method(*m));
                        }
                        let s = self.show(base);
                        self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
                        MemberOut::Err
                    }
                    DefKind::Union => {
                        self.ensure_union(d);
                        if let Some(sig) = self.unions.get(&d) {
                            if sig.discr_field == name.name {
                                return MemberOut::Value(sig.discr_ty, Res::Discriminant { union: d });
                            }
                        }
                        if let Some(r) = self.builtin_member(base, &name.name) {
                            return r;
                        }
                        self.union_member_err(base, name);
                        MemberOut::Err
                    }
                    DefKind::Host => {
                        let Some(h) = self.hosts.get(&d).cloned() else { return MemberOut::Err };
                        if let Some(i) = h.fns.iter().position(|(n, _)| *n == name.name) {
                            let TyKind::Fn(ps, r) = self.tys.kind(h.fns[i].1).clone() else { return MemberOut::Err };
                            let callee = Callee {
                                tparams: vec![],
                                params: ps.to_vec(),
                                defaults: vec![false; ps.len()],
                                ret: r,
                                def: None,
                                owner_args: vec![],
                                mutating: false,
                                builtin: None,
                            };
                            MemberOut::Callable(callee, Res::HostFn { host: d, index: i as u32 })
                        } else {
                            let s = self.show(base);
                            self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
                            MemberOut::Err
                        }
                    }
                    _ => {
                        if let Some(r) = self.builtin_member(base, &name.name) {
                            return r;
                        }
                        let s = self.show(base);
                        self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
                        MemberOut::Err
                    }
                }
            }
            TyKind::Variants(u, _, _) => {
                if let Some(sig) = self.unions.get(&u) {
                    if sig.discr_field == name.name {
                        return MemberOut::Value(sig.discr_ty, Res::Discriminant { union: u });
                    }
                }
                if let Some(r) = self.builtin_member(base, &name.name) {
                    return r;
                }
                self.union_member_err(base, name);
                MemberOut::Err
            }
            TyKind::Param(p) => {
                let Some(b) = self.params[p.0 as usize].bound else {
                    let s = self.show(base);
                    self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
                    return MemberOut::Err;
                };
                let TyKind::Adt(idef, iargs) = self.tys.kind(b).clone() else { return MemberOut::Err };
                self.ensure_iface(idef);
                let methods = self.iface_methods_inst(idef, &iargs);
                if let Some(i) = methods.iter().position(|(n, _, _)| *n == name.name) {
                    let (_, ps, r) = methods[i].clone();
                    self.note_effect(16);
                    let own = self.ifaces.get(&idef).map(|s| s.methods[i].tparams.clone()).unwrap_or_default();
                    let callee = Callee {
                        tparams: own,
                        defaults: vec![false; ps.len()],
                        params: ps,
                        ret: r,
                        def: None,
                        owner_args: vec![],
                        mutating: false,
                        builtin: None,
                    };
                    MemberOut::Callable(callee, Res::IfaceMethod { iface: idef, index: i as u32 })
                } else {
                    let s = self.show(base);
                    self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
                    MemberOut::Err
                }
            }
            _ => {
                if let Some(r) = self.builtin_member(base, &name.name) {
                    return r;
                }
                let s = self.show(base);
                let _ = (e, obj);
                if base == STR && name.name == "length" {
                    let idx = self.err_fix(
                        "OXS-E0108",
                        name.span,
                        &[("name", &name.name), ("ty", &s)],
                        Some(Fix::replace(
                            "A string has a byte length in UTF-8: use `.byteLength`",
                            name.span,
                            "byteLength",
                        )),
                    );
                    if let Some(d) = self.diags.get_mut(idx) {
                        d.fixes.push(Fix {
                            title: "For the number of characters use `s.chars().length`.".into(),
                            edits: Vec::new(),
                        });
                    }
                    return MemberOut::Err;
                }
                self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
                MemberOut::Err
            }
        }
    }

    fn union_member_err(&mut self, base: Ty, name: &'a Ident) {
        let s = self.show(base);
        self.err("OXS-E0108", name.span, &[("name", &name.name), ("ty", &s)]);
    }

    fn discr_ty_of_variant(&self, v: DefId) -> Ty {
        match self.records.get(&v).and_then(|r| r.discr.as_ref()) {
            Some((_, Discr::Str(_))) => STR,
            Some((_, Discr::Int(_))) => I32,
            Some((_, Discr::Bool(_))) => BOOL,
            None => ERROR,
        }
    }

    // ---- calls --------------------------------------------------------------------------------

    fn is_sensitive(x: &Expr) -> bool {
        match &strip_paren(x).kind {
            ExprKind::Arrow(a) => a.params.iter().any(|p| p.ty.is_none()),
            _ => false,
        }
    }

    pub fn call(
        &mut self,
        e: &'a Expr,
        callee_e: &'a Expr,
        targs: &'a [TypeExpr],
        args: &'a [Expr],
        optional: bool,
        exp: Option<Ty>,
    ) -> Ty {
        let c = strip_paren(callee_e);
        let mut recv_mutating: Option<&'a Expr> = None;
        let mut callee: Option<Callee> = None;
        let mut chain_nullable = false;
        match &c.kind {
            ExprKind::Member { obj, name, optional: mopt } => match self.resolve_member(c, obj, name, *mopt) {
                MemberOut::Callable(cal, res) => {
                    self.set_res(c.id, res);
                    if cal.mutating {
                        recv_mutating = Some(obj);
                    }
                    chain_nullable = *mopt;
                    callee = Some(cal);
                }
                MemberOut::Value(t, res) => {
                    self.set_res(c.id, res);
                    self.set_ty(c.id, t);
                    if *mopt {
                        chain_nullable = true;
                    }
                    return self.closure_call(e, callee_e, t, args, optional, chain_nullable, exp);
                }
                MemberOut::Err => {
                    for a in args {
                        self.expr(a, None);
                    }
                    return ERROR;
                }
            },
            ExprKind::Ident(id) => match self.lookup(&id.name) {
                Some(Binding::BuiltinFn(Builtin::Range)) => {
                    self.set_res(c.id, Res::Builtin(Builtin::Range));
                    callee = Some(Callee {
                        tparams: vec![],
                        params: vec![I32, I32],
                        defaults: vec![false, false],
                        ret: self.tys.array(I32),
                        def: None,
                        owner_args: vec![],
                        mutating: false,
                        builtin: Some(Builtin::Range),
                    });
                }
                Some(Binding::Def(d)) if self.kind_of(d) == DefKind::Fn => {
                    self.ensure_fn(d);
                    let Some(sig) = self.fns.get(&d).cloned() else {
                        for a in args {
                            self.expr(a, None);
                        }
                        return ERROR;
                    };
                    if sig.ret == ERROR && self.defs[d.0 as usize].sig == Sig::InProgress {
                        let n = self.def_name(d).to_string();
                        self.err("OXS-E0103", e.span, &[("what", &format!("return type of recursive function `{n}`; add a return type annotation"))]);
                        for a in args {
                            self.expr(a, None);
                        }
                        return ERROR;
                    }
                    self.set_res(c.id, Res::Def(d));
                    callee = Some(Callee {
                        tparams: sig.tparams.clone(),
                        params: sig.params.iter().map(|p| p.ty).collect(),
                        defaults: sig.params.iter().map(|p| p.has_default).collect(),
                        ret: sig.ret,
                        def: Some(d),
                        owner_args: vec![],
                        mutating: false,
                        builtin: None,
                    });
                }
                Some(Binding::Def(d)) if matches!(self.kind_of(d), DefKind::Class | DefKind::Struct) => {
                    let n = self.def_name(d).to_string();
                    self.err(
                        "OXS-E0112",
                        c.span,
                        &[("ty", &format!("typeof {n}"))],
                    );
                    for a in args {
                        self.expr(a, None);
                    }
                    return ERROR;
                }
                _ => {}
            },
            _ => {}
        }
        let Some(cal) = callee else {
            // any other callee: an expression of function type
            let t = self.expr(callee_e, None);
            return self.closure_call(e, callee_e, t, args, optional, false, exp);
        };
        if optional {
            self.err("OXS-E0127", c.span, &[("expr", self.span_text(c.span)), ("ty", "function")]);
        }
        self.set_ty(c.id, ERROR_PLACEHOLDER_FN);
        let (ret, solved) = self.apply_call(e, cal.clone(), targs, args, exp);
        if let Some(obj) = recv_mutating {
            self.after_mutating_call(obj);
        }
        if let (Some(b), ExprKind::Member { obj, .. }) = (cal.builtin, &c.kind) {
            if matches!(b, Builtin::NumWrapping | Builtin::NumSaturating | Builtin::Downcast) {
                let ot = self.mods[self.cm].out.expr_ty.get(obj.id.0 as usize).copied().unwrap_or(ERROR);
                let ot = self.non_null(ot);
                if let Some(t) = solved.first().copied() {
                    if t != ERROR {
                        self.check_builtin_targs(b, ot, t, e.span);
                    }
                }
            }
        }
        // function-typed callee node gets a type for the tables
        let mut ps = cal.params.clone();
        ps.truncate(args.len().max(ps.len()));
        let ft = self.tys.func(&cal.params, ret);
        self.set_ty(c.id, ft);
        if !std::ptr::eq(c, callee_e) {
            self.set_ty(callee_e.id, ft);
        }
        if chain_nullable {
            self.tys.nullable(ret)
        } else {
            ret
        }
    }

    fn after_mutating_call(&mut self, obj: &'a Expr) {
        // the receiver must be a mutable place; its narrowed facts die
        let o = strip_paren(obj);
        let ot = self.mods[self.cm].out.expr_ty.get(o.id.0 as usize).copied().unwrap_or(ERROR);
        if self.is_class_ty(ot) {
            return;
        }
        match &o.kind {
            ExprKind::Ident(_) | ExprKind::This | ExprKind::Member { .. } | ExprKind::Index { .. } => {
                if self.check_receiver_mutable(o).is_none() {
                    return;
                }
                if let Some(p) = self.path_of(o) {
                    self.kill_children(p);
                }
            }
            _ => {
                self.err("OXS-E0109", obj.span, &[("what", "the receiver of a `mutating` method"), ("detail", "it is a temporary value")]);
            }
        }
    }

    fn closure_call(&mut self, e: &'a Expr, callee_e: &'a Expr, t: Ty, args: &'a [Expr], optional: bool, chain_nullable: bool, exp: Option<Ty>) -> Ty {
        let _ = exp;
        let (ft, nullable) = match self.tys.kind(t).clone() {
            TyKind::Nullable(i) if optional => (i, true),
            TyKind::Nullable(_) => {
                self.possibly_null(callee_e, t);
                for a in args {
                    self.expr(a, None);
                }
                return ERROR;
            }
            _ => (t, false),
        };
        if optional && !nullable && t != ERROR {
            let s = self.show(t);
            self.err("OXS-E0127", callee_e.span, &[("expr", self.span_text(callee_e.span)), ("ty", &s)]);
        }
        match self.tys.kind(ft).clone() {
            TyKind::Fn(ps, r) => {
                let cal = Callee {
                    tparams: vec![],
                    params: ps.to_vec(),
                    defaults: vec![false; ps.len()],
                    ret: r,
                    def: None,
                    owner_args: vec![],
                    mutating: false,
                    builtin: None,
                };
                let (ret, _) = self.apply_call(e, cal, &[], args, None);
                if nullable || chain_nullable {
                    self.tys.nullable(ret)
                } else {
                    ret
                }
            }
            TyKind::Error => {
                for a in args {
                    self.expr(a, None);
                }
                ERROR
            }
            _ => {
                let s = self.show(t);
                self.err("OXS-E0112", callee_e.span, &[("ty", &s)]);
                for a in args {
                    self.expr(a, None);
                }
                ERROR
            }
        }
    }

    /// One-way structural matching of a parameter type against an argument type,
    /// binding the type parameters in `solve`.
    pub(crate) fn match_ty(&mut self, p: Ty, a: Ty, solve: &[ParamId], map: &mut [Option<Ty>]) {
        if a == ERROR || a == NEVER {
            return;
        }
        match self.tys.kind(p).clone() {
            TyKind::Param(q) => {
                if let Some(i) = solve.iter().position(|x| *x == q) {
                    if a == NULL {
                        return;
                    }
                    match map[i] {
                        None => map[i] = Some(a),
                        Some(prev) => {
                            if prev != a {
                                if self.assignable(a, prev) {
                                    // keep the wider
                                } else if self.assignable(prev, a) {
                                    map[i] = Some(a);
                                } else if let Some(j) = self.join_ty(prev, a) {
                                    map[i] = Some(j);
                                }
                            }
                        }
                    }
                }
            }
            TyKind::Array(pe) => {
                if let TyKind::Array(ae) = self.tys.kind(a).clone() {
                    self.match_ty(pe, ae, solve, map);
                }
            }
            TyKind::Nullable(pi) => match self.tys.kind(a).clone() {
                TyKind::Nullable(ai) => self.match_ty(pi, ai, solve, map),
                TyKind::Null => {}
                _ => self.match_ty(pi, a, solve, map),
            },
            TyKind::Adt(pd, pargs) => {
                if pargs.is_empty() {
                    return;
                }
                let aview = self.union_view(a);
                if self.kind_of(pd) == DefKind::Union {
                    if let Some((u, uargs, _)) = aview {
                        if u == pd {
                            for (x, y) in pargs.iter().zip(uargs.iter()) {
                                self.match_ty(*x, *y, solve, map);
                            }
                        }
                    }
                } else if let TyKind::Adt(ad, aargs) = self.tys.kind(a).clone() {
                    if ad == pd {
                        for (x, y) in pargs.iter().zip(aargs.iter()) {
                            self.match_ty(*x, *y, solve, map);
                        }
                    }
                }
            }
            TyKind::Fn(pps, pr) => {
                if let TyKind::Fn(aps, ar) = self.tys.kind(a).clone() {
                    for (x, y) in pps.iter().zip(aps.iter()) {
                        self.match_ty(*x, *y, solve, map);
                    }
                    self.match_ty(pr, ar, solve, map);
                }
            }
            _ => {}
        }
    }

    /// Check arguments against a callee, inferring its own type parameters. Returns
    /// the result type and the solved type arguments (owner's first).
    pub(crate) fn apply_call(
        &mut self,
        e: &'a Expr,
        cal: Callee,
        targs: &'a [TypeExpr],
        args: &'a [Expr],
        exp: Option<Ty>,
    ) -> (Ty, Vec<Ty>) {
        let n = cal.params.len();
        let mut arity_ok = true;
        let solve = cal.tparams.clone();
        let mut map: Vec<Option<Ty>> = vec![None; solve.len()];
        // explicit type arguments
        if !targs.is_empty() {
            if targs.len() != solve.len() {
                self.err(
                    "OXS-E0117",
                    e.span,
                    &[("detail", &format!("Expected {} type arguments, but got {}", solve.len(), targs.len()))],
                );
            } else {
                for (i, t) in targs.iter().enumerate() {
                    let ty = self.lower_type(t);
                    map[i] = Some(ty);
                }
            }
        } else if !solve.is_empty() {
            if let Some(x) = exp {
                // return-type driven hint; arguments override conflicting hints
                let mut hint: Vec<Option<Ty>> = vec![None; solve.len()];
                self.match_ty(cal.ret, x, &solve, &mut hint);
                for (m, h) in map.iter_mut().zip(hint) {
                    if m.is_none() {
                        *m = h;
                    }
                }
            }
        }
        // required argument count: trailing defaults and trailing `void` parameters are
        // optional (after substituting explicit and return-type-hinted type arguments)
        {
            let m0: Vec<(ParamId, Ty)> = solve.iter().copied().zip(map.iter()).filter_map(|(p, t)| t.map(|t| (p, t))).collect();
            let mut required = n;
            while required > 0 {
                let pt = self.tys.subst(cal.params[required - 1], &m0);
                if cal.defaults[required - 1] || pt == VOID {
                    required -= 1;
                } else {
                    break;
                }
            }
            if args.len() > n || args.len() < required {
                arity_ok = false;
                let want = if required == n || args.len() > n { n.to_string() } else { format!("{required}") };
                self.err("OXS-E0107", e.span, &[("expected", &want), ("found", &args.len().to_string())]);
            }
        }
        let hinted: Vec<bool> = map.iter().map(Option::is_some).collect();
        let sub = |ck: &mut Self, t: Ty, map: &[Option<Ty>]| -> Ty {
            let m: Vec<(ParamId, Ty)> = solve.iter().copied().zip(map.iter()).filter_map(|(p, t)| t.map(|t| (p, t))).collect();
            ck.tys.subst(t, &m)
        };
        let has_unsolved = |ck: &Self, t: Ty, map: &[Option<Ty>]| -> bool {
            solve.iter().zip(map.iter()).any(|(p, m)| m.is_none() && ck.tys.contains_param(t, *p))
        };
        let mut arg_tys: Vec<Ty> = vec![ERROR; args.len()];
        // phase 1: arguments that do not depend on the expected type of a lambda
        for (i, a) in args.iter().enumerate() {
            if i >= n || Self::is_sensitive(a) {
                continue;
            }
            let pt = cal.params[i];
            let hint = if has_unsolved(self, pt, &map) { None } else { Some(sub(self, pt, &map)) };
            let t = self.expr(a, hint);
            arg_tys[i] = t;
            self.match_ty(pt, t, &solve, &mut map);
        }
        // phase 2: context-sensitive lambdas
        for (i, a) in args.iter().enumerate() {
            if i >= n || !Self::is_sensitive(a) {
                continue;
            }
            let pt = cal.params[i];
            // unsolved parameters become poison in the expected type; the lambda reports E0103 if it needs them
            let m: Vec<(ParamId, Ty)> = solve
                .iter()
                .copied()
                .zip(map.iter())
                .map(|(p, t)| (p, t.unwrap_or(ERROR)))
                .collect();
            let hint = self.tys.subst(pt, &m);
            let t = self.expr(a, Some(hint));
            arg_tys[i] = t;
            self.match_ty(pt, t, &solve, &mut map);
        }
        // extra arguments
        for (i, a) in args.iter().enumerate() {
            if i >= n {
                self.expr(a, None);
            }
        }
        // unsolved parameters
        let mut solved: Vec<Ty> = Vec::new();
        let mut failed = false;
        for (i, p) in solve.iter().enumerate() {
            if let Some(t) = map[i] { solved.push(t) } else {
                if !failed {
                    let pn = self.params[p.0 as usize].name.clone();
                    self.err("OXS-E0103", e.span, &[("what", &format!("type argument `{pn}`; provide it explicitly, e.g. `f<i32>(...)`"))]);
                }
                failed = true;
                solved.push(ERROR);
            }
        }
        let _ = hinted;
        let m: Vec<(ParamId, Ty)> = solve.iter().copied().zip(solved.iter().copied()).collect();
        // final assignability of every argument
        if arity_ok || args.len() <= n {
            for (i, a) in args.iter().enumerate() {
                if i >= n {
                    break;
                }
                let target = self.tys.subst(cal.params[i], &m);
                let t = arg_tys[i];
                if !failed {
                    self.coerce_to(a, t, target);
                }
            }
        }
        if !failed && !solve.is_empty() {
            let span = e.span;
            self.check_bounds(&solve, &solved, span);
        }
        let ret = self.tys.subst(cal.ret, &m);
        if !failed && !solve.is_empty() {
            self.check_wf(e.span, ret);
            for p in &cal.params {
                let pt = self.tys.subst(*p, &m);
                self.check_wf(e.span, pt);
            }
        }
        self.check_overflow(e.span);
        // record the call
        let mut all_args = cal.owner_args.clone();
        all_args.extend(solved.iter().copied());
        let mut info = CallInfo { type_args: all_args.clone(), ..CallInfo::default() };
        if args.len() < n {
            for k in args.len()..n {
                if cal.defaults[k] {
                    info.defaults_used += 1;
                } else if cal.params[k] == VOID {
                    info.void_elided += 1;
                }
            }
        }
        if let Some(d) = cal.def {
            self.note_call(d, e.span);
            self.note_inst(d, &all_args, e.span);
            if !all_args.is_empty() && !all_args.contains(&ERROR) {
                let inst = Instance { def: d, args: all_args };
                let idx = if let Some(i) = self.inst_map.get(&inst) { *i } else {
                    let i = self.instances.len() as u32;
                    self.instances.push(inst.clone());
                    self.inst_map.insert(inst, i);
                    i
                };
                info.instance = Some(idx);
            }
        }
        if let Some(b) = cal.builtin {
            self.set_res(e.id, Res::Builtin(b));
        }
        self.mods[self.cm].out.calls.insert(e.id, info);
        (ret, solved)
    }

    // ---- new --------------------------------------------------------------------------------

    pub fn new_expr(&mut self, e: &'a Expr, ty: &'a TypeExpr, args: &'a [Expr], exp: Option<Ty>) -> Ty {
        // `new Box(5)` with inferred type arguments
        let mut t = ty;
        while let TypeExprKind::Paren(i) = &t.kind {
            t = i;
        }
        let mut class_def: Option<DefId> = None;
        let mut infer_targs = false;
        if let TypeExprKind::Path { segments, args: targs } = &t.kind {
            if segments.len() == 1 && targs.is_empty() {
                if let Some(Binding::Def(d)) = self.lookup_type(&segments[0].name) {
                    if self.kind_of(d) == DefKind::Class && !self.defs[d.0 as usize].info.tparams.is_empty() {
                        class_def = Some(d);
                        infer_targs = true;
                    }
                }
            }
        }
        let cty = if infer_targs {
            let d = class_def.unwrap_or(DefId(0));
            let params: Vec<Ty> = self.defs[d.0 as usize].info.tparams.iter().map(|p| self.tys.mk(TyKind::Param(*p))).collect();
            self.tys.adt(d, &params)
        } else {
            self.lower_type(ty)
        };
        let TyKind::Adt(d, cargs) = self.tys.kind(cty).clone() else {
            if cty != ERROR {
                let s = self.show(cty);
                self.err("OXS-E0119", ty.span, &[("detail", &format!("`new` needs a class; '{s}' cannot be constructed this way"))]);
            }
            for a in args {
                self.expr(a, None);
            }
            return ERROR;
        };
        match self.kind_of(d) {
            DefKind::Class => {}
            DefKind::Struct | DefKind::Variant { .. } => {
                let n = self.def_name(d).to_string();
                self.err(
                    "OXS-E0119",
                    ty.span,
                    &[("detail", &format!("`{n}` is a struct; create it with an object literal `{{ ... }}`, not `new` (only classes are created with `new`)"))],
                );
                for a in args {
                    self.expr(a, None);
                }
                return ERROR;
            }
            DefKind::Host | DefKind::Handle => {
                self.err("OXS-E0119", ty.span, &[("detail", "capability values cannot be created by the program; they are passed to `main`")]);
                return ERROR;
            }
            _ => {
                let s = self.show(cty);
                self.err("OXS-E0119", ty.span, &[("detail", &format!("`new` needs a class; '{s}' is not a class"))]);
                return ERROR;
            }
        }
        self.ensure_record(d);
        let tps = self.defs[d.0 as usize].info.tparams.clone();
        let rec = self.records.get(&d).cloned();
        let ctor = rec.as_ref().and_then(|r| r.ctor);
        let solve: Vec<ParamId> = if infer_targs { tps.clone() } else { vec![] };
        let map: Vec<(ParamId, Ty)> = if infer_targs { vec![] } else { tps.iter().copied().zip(cargs.iter().copied()).collect() };
        let (params, defaults): (Vec<Ty>, Vec<bool>) = match ctor {
            Some(c) => {
                self.ensure_fn(c);
                match self.fns.get(&c).cloned() {
                    Some(sig) => (
                        sig.params.iter().map(|p| self.tys.subst(p.ty, &map)).collect(),
                        sig.params.iter().map(|p| p.has_default).collect(),
                    ),
                    None => (vec![], vec![]),
                }
            }
            None => (vec![], vec![]),
        };
        let cal = Callee {
            tparams: solve,
            params,
            defaults,
            ret: cty,
            def: ctor,
            owner_args: if infer_targs { vec![] } else { cargs.to_vec() },
            mutating: false,
            builtin: None,
        };
        let (ret, solved) = self.apply_call(e, cal, &[], args, exp);
        if infer_targs && !solved.is_empty() {
            // type arguments are now known: the class type is the result
        }
        if let Some(c) = ctor {
            self.set_res(e.id, Res::Method(c));
        }
        ret
    }
}

// a harmless placeholder used while the callee node's real function type is being computed
const ERROR_PLACEHOLDER_FN: Ty = ERROR;
