//! Expressions: literals, names, operators, assignment, casts, conditionals,
//! array and object literals, `?`, indexing, closures, switch expressions.
//! Calls and member access live in `calls.rs`.

use crate::flow::Flow;
use crate::ty::*;
use crate::*;

fn is_lit_expr(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Lit(l) => matches!(l.kind, LitKind::Int { suffix: None, .. } | LitKind::Float { suffix: None, .. }),
        ExprKind::Paren(i) => is_lit_expr(i),
        ExprKind::Unary { op: UnOp::Neg, expr } => is_lit_expr(expr),
        _ => false,
    }
}

pub(crate) fn strip_paren(mut e: &Expr) -> &Expr {
    while let ExprKind::Paren(i) = &e.kind {
        e = i;
    }
    e
}

impl<'a> Checker<'a> {
    // ---- subtyping ----------------------------------------------------------

    /// Strip one `| null`.
    pub fn non_null(&self, t: Ty) -> Ty {
        match self.tys.kind(t) {
            TyKind::Nullable(i) => *i,
            _ => t,
        }
    }

    pub fn is_managed(&self, t: Ty) -> bool {
        matches!(
            self.tys.kind(t),
            TyKind::Str | TyKind::Array(_) | TyKind::Fn(..) | TyKind::Variants(..)
        ) || matches!(self.tys.kind(t), TyKind::Adt(d, _) if !matches!(self.kind_of(*d), DefKind::Enum))
    }

    /// Representation-preserving subtyping (used inside function types).
    pub fn sub_norepr(&mut self, a: Ty, b: Ty) -> bool {
        if a == b || a == ERROR || b == ERROR || a == NEVER {
            return true;
        }
        match (self.tys.kind(a).clone(), self.tys.kind(b).clone()) {
            (TyKind::Nullable(x), TyKind::Nullable(y)) => self.sub_norepr(x, y),
            (_, TyKind::Nullable(y)) if self.is_managed(a) => self.sub_norepr(a, y),
            _ => {
                if let (Some(ua), Some(ub)) = (self.union_view(a), self.union_view(b)) {
                    return ua.0 == ub.0 && ua.1 == ub.1 && ua.2 & !ub.2 == 0;
                }
                false
            }
        }
    }

    pub fn assignable(&mut self, from: Ty, to: Ty) -> bool {
        if from == to || from == ERROR || to == ERROR || from == NEVER {
            return true;
        }
        let (fk, tk) = (self.tys.kind(from).clone(), self.tys.kind(to).clone());
        match (&fk, &tk) {
            (TyKind::Null, TyKind::Nullable(_)) => true,
            (TyKind::Nullable(a), TyKind::Nullable(b)) => self.assignable(*a, *b),
            (_, TyKind::Nullable(b)) if !matches!(fk, TyKind::Null | TyKind::Nullable(_)) => self.assignable(from, *b),
            (TyKind::Fn(pa, ra), TyKind::Fn(pb, rb)) => {
                pa.len() == pb.len()
                    && pa.iter().zip(pb.iter()).all(|(x, y)| self.sub_norepr(*y, *x))
                    && self.sub_norepr(*ra, *rb)
            }
            _ => {
                if let (Some(ua), Some(ub)) = (self.union_view(from), self.union_view(to)) {
                    return ua.0 == ub.0 && ua.1 == ub.1 && ua.2 & !ub.2 == 0;
                }
                false
            }
        }
    }

    pub fn record_coercion(&mut self, id: NodeId, from: Ty, to: Ty) {
        if from != to && from != ERROR && to != ERROR {
            self.mods[self.cm].out.coercions.insert(id, Coercion { from, to });
        }
    }

    /// Check an already typed expression against `target`, recording the coercion.
    pub fn coerce_to(&mut self, e: &'a Expr, from: Ty, to: Ty) -> bool {
        if self.assignable(from, to) {
            self.record_coercion(e.id, from, to);
            true
        } else {
            self.mismatch_fix(e, to, from);
            false
        }
    }

    /// Is the constant `e` (a possibly negated numeric literal) changed by converting it to `to`?
    /// Such a conversion is never offered as a fix: `1.5 as i32` and `300 as u8` hide a bug.
    fn lossy_const(e: &Expr, to: NumTy) -> bool {
        let (e, neg) = match &strip_paren(e).kind {
            ExprKind::Unary { op: UnOp::Neg, expr } => (strip_paren(expr), true),
            _ => (strip_paren(e), false),
        };
        let ExprKind::Lit(l) = &e.kind else { return false };
        match &l.kind {
            LitKind::Int { value, .. } => {
                if is_int(to) {
                    if neg && !is_signed(to) { *value > 0 } else { *value > int_max(to, neg) }
                } else {
                    !Self::float_fits(*value, to)
                }
            }
            LitKind::Float { text, .. } => {
                let v: f64 = text.parse().unwrap_or(f64::INFINITY);
                let v = if neg { -v } else { v };
                if is_int(to) {
                    let (lo, hi) = (-(int_max(to, true) as f64), int_max(to, false) as f64);
                    !(v.fract() == 0.0 && v >= lo && v <= hi)
                } else {
                    to == NumTy::F32 && !(v as f32).is_finite()
                }
            }
            _ => false,
        }
    }

    /// The `(e as T)` fix for a numeric mismatch, unless the conversion is statically lossy.
    fn as_fix(&self, e: &Expr, to: Ty, from: Ty, title: String) -> Option<Fix> {
        let (TyKind::Num(tn), TyKind::Num(_)) = (self.tys.kind(to), self.tys.kind(from)) else { return None };
        if Self::lossy_const(e, *tn) {
            return None;
        }
        let st = self.show(to);
        Some(Fix {
            title,
            edits: vec![
                TextEdit { span: Span::new(e.span.file, e.span.lo, e.span.lo), replacement: "(".into() },
                TextEdit { span: Span::new(e.span.file, e.span.hi, e.span.hi), replacement: format!(" as {st})") },
            ],
        })
    }

    /// `OXS-E0100` with a mechanical fix for numeric conversions.
    pub fn mismatch_fix(&mut self, e: &Expr, expected: Ty, found: Ty) {
        if self.tys.has_error(expected) || self.tys.has_error(found) {
            return;
        }
        let (se, sf) = (self.show(expected), self.show(found));
        let fix = self.as_fix(e, expected, found, format!("Convert with `as {se}`"));
        self.err_fix("OXS-E0100", e.span, &[("expected", &se), ("found", &sf)], fix);
    }

    /// Check `e` against `target` (checking mode).
    pub fn expr_to(&mut self, e: &'a Expr, target: Ty) -> Ty {
        let t = self.expr(e, Some(target));
        self.coerce_to(e, t, target);
        t
    }

    // ---- entry ---------------------------------------------------------------

    /// The receiver of a member/call/index/paren node (the next link of a chain).
    fn receiver(e: &'a Expr) -> Option<&'a Expr> {
        match &e.kind {
            ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => Some(obj),
            ExprKind::Call { callee, .. } => match &strip_paren(callee).kind {
                // a method call evaluates the receiver of the callee member, not the member itself
                ExprKind::Member { obj, .. } => Some(obj),
                _ => Some(callee),
            },
            ExprKind::Paren(i) => Some(i),
            _ => None,
        }
    }

    /// Long `a.b().c()...` chains are evaluated innermost first, so the native stack does
    /// not grow with the chain length: each link finds its receiver already typed in `memo`.
    fn precompute_chain(&mut self, e: &'a Expr) {
        // only chains of at least 12 links are worth it
        let mut probe = e;
        let mut n = 0;
        while let Some(r) = Self::receiver(probe) {
            n += 1;
            probe = r;
            if n >= 12 {
                break;
            }
        }
        if n < 12 || self.memo.contains_key(&Self::receiver(e).map_or(e.id, |r| r.id)) {
            return;
        }
        let mut spine: Vec<&'a Expr> = Vec::new();
        let mut cur = e;
        while let Some(r) = Self::receiver(cur) {
            spine.push(r);
            cur = r;
        }
        // `spine` runs from e's receiver to the base; evaluate the base first
        for node in spine.into_iter().rev() {
            let t = self.expr(node, None);
            self.memo.insert(node.id, t);
        }
    }

    pub fn expr(&mut self, e: &'a Expr, exp: Option<Ty>) -> Ty {
        if let Some(t) = self.memo.remove(&e.id) {
            return t;
        }
        if matches!(e.kind, ExprKind::Member { .. } | ExprKind::Call { .. } | ExprKind::Index { .. }) {
            self.precompute_chain(e);
        }
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            self.depth -= 1;
            self.err("OXS-E0113", e.span, &[("detail", "expression is nested too deeply")]);
            self.set_ty(e.id, ERROR);
            return ERROR;
        }
        let t = self.expr_inner(e, exp);
        self.depth -= 1;
        self.set_ty(e.id, t);
        if matches!(self.tys.kind(t), TyKind::Adt(d, _) if self.kind_of(*d) == DefKind::Host) {
            self.note_effect(1);
            if self.in_law() {
                self.err("OXS-E0125", e.span, &[("detail", "a law cannot use host capabilities")]);
            }
        }
        t
    }

    pub fn note_effect(&mut self, bit: u8) {
        if let Some(d) = self.fnctx.last().and_then(|c| c.def) {
            self.eff.entry(d).or_default().mask |= bit;
        }
    }

    pub fn note_call(&mut self, callee: DefId, span: Span) {
        if let Some(d) = self.fnctx.last().and_then(|c| c.def) {
            self.eff.entry(d).or_default().edges.push((callee, span));
        }
    }

    /// Propagate effects from callees to callers (linear worklist) and check law purity.
    pub fn compute_effects(&mut self) {
        let mut mask: HashMap<DefId, u8> = HashMap::new();
        let mut rev: HashMap<DefId, Vec<DefId>> = HashMap::new();
        for (d, r) in &self.eff {
            mask.insert(*d, r.mask);
            for (c, _) in &r.edges {
                rev.entry(*c).or_default().push(*d);
            }
        }
        let mut work: Vec<DefId> = mask.iter().filter(|(_, m)| **m != 0).map(|(d, _)| *d).collect();
        while let Some(d) = work.pop() {
            let m = mask.get(&d).copied().unwrap_or(0);
            if let Some(callers) = rev.get(&d) {
                for c in callers {
                    let cm = mask.entry(*c).or_insert(0);
                    if *cm | m != *cm {
                        *cm |= m;
                        work.push(*c);
                    }
                }
            }
        }
        // callees without a record (host-free leaf functions) are pure
        for (d, m) in &mask {
            self.effects.insert(
                *d,
                EffectSummary { uses_host: m & 1 != 0, reads_globals: m & 2 != 0, writes_globals: m & 4 != 0 },
            );
        }
        let laws: Vec<DefId> = (0..self.defs.len()).map(|i| DefId(i as u32)).filter(|d| self.kind_of(*d) == DefKind::Law).collect();
        for l in laws {
            let edges = self.eff.get(&l).map(|r| r.edges.clone()).unwrap_or_default();
            for (c, span) in edges {
                if mask.get(&c).copied().unwrap_or(0) != 0 {
                    let n = self.def_name(c).to_string();
                    self.err(
                        "OXS-E0125",
                        span,
                        &[("detail", &format!("a law may only call pure functions; `{n}` uses host capabilities or module-level state"))],
                    );
                }
            }
        }
    }

    pub fn in_law(&self) -> bool {
        self.fnctx.last().is_some_and(|c| c.is_law)
    }

    fn expr_inner(&mut self, e: &'a Expr, exp: Option<Ty>) -> Ty {
        match &e.kind {
            ExprKind::Lit(l) => self.lit(l, exp, false),
            ExprKind::Paren(i) => self.expr(i, exp),
            ExprKind::Template(parts) => self.template(parts),
            ExprKind::Ident(id) => self.ident(e, id),
            ExprKind::This => self.this_expr(e),
            ExprKind::Array(els) => self.array_lit(e, els, exp),
            ExprKind::Object(fields) => self.object_lit(e, fields, exp),
            ExprKind::Unary { op, expr } => self.unary(e, *op, expr, exp),
            ExprKind::Binary { .. } => self.binary_chain(e, exp),
            ExprKind::Assign { op, target, value } => self.assign(e, *op, target, value),
            ExprKind::Update { target, .. } => self.update(e, target),
            ExprKind::Call { callee, type_args, args, optional } => self.call(e, callee, type_args, args, *optional, exp),
            ExprKind::New { ty, args } => self.new_expr(e, ty, args, exp),
            ExprKind::Member { obj, name, optional } => self.member_expr(e, obj, name, *optional),
            ExprKind::Index { obj, index, optional } => self.index_expr(e, obj, index, *optional),
            ExprKind::Try(inner) => self.try_expr(e, inner),
            ExprKind::Cast { expr, ty } => self.cast_expr(e, expr, ty),
            ExprKind::Cond { .. } => self.cond_expr(e, exp),
            ExprKind::Arrow(a) => self.arrow(e, a, exp),
            ExprKind::Switch(s) => self.switch_expr(e, s, exp),
        }
    }

    // ---- literals --------------------------------------------------------------

    fn float_fits(v: u128, n: NumTy) -> bool {
        let bits = if n == NumTy::F32 { 24 } else { 53 };
        v <= (1u128 << bits) || {
            // representable when the value has at most `bits` significant bits
            let tz = v.trailing_zeros();
            128 - v.leading_zeros() - tz <= bits
        }
    }

    pub fn lit(&mut self, l: &'a Lit, exp: Option<Ty>, neg: bool) -> Ty {
        let hint = exp.map(|t| self.non_null(t)).map(|t| self.tys.kind(t).clone());
        let t = match &l.kind {
            LitKind::Int { value, suffix } => {
                let ty = if let Some(s) = suffix {
                    *s
                } else {
                    match hint {
                        Some(TyKind::Num(n)) => n,
                        _ => NumTy::I32,
                    }
                };
                if is_int(ty) {
                    if !(neg && !is_signed(ty)) && *value > int_max(ty, neg) {
                        let shown = if neg { format!("-{value}") } else { value.to_string() };
                        self.err("OXS-E0102", l.span, &[("value", &shown), ("ty", ty.name())]);
                    }
                } else if !Self::float_fits(*value, ty) {
                    self.err("OXS-E0102", l.span, &[("value", &value.to_string()), ("ty", ty.name())]);
                }
                num(ty)
            }
            LitKind::Float { text, suffix } => {
                let ty = suffix.unwrap_or(match hint {
                    Some(TyKind::Num(n)) if n.is_float() => n,
                    _ => NumTy::F64,
                });
                let v: f64 = text.parse().unwrap_or(f64::INFINITY);
                let bad = !v.is_finite() || (ty == NumTy::F32 && !(v as f32).is_finite());
                if bad {
                    self.err("OXS-E0102", l.span, &[("value", text), ("ty", ty.name())]);
                }
                num(ty)
            }
            LitKind::Str(_) => STR,
            LitKind::Bool(_) => BOOL,
            LitKind::Null => NULL,
        };
        self.set_ty(l.id, t);
        t
    }

    fn template(&mut self, parts: &'a [TemplatePart]) -> Ty {
        for p in parts {
            if let TemplatePart::Expr(x) = p {
                let t = self.expr(x, None);
                let ok = matches!(self.tys.kind(t), TyKind::Num(_) | TyKind::Bool | TyKind::Str | TyKind::Error | TyKind::Never);
                if !ok {
                    let s = self.show(t);
                    self.err(
                        "OXS-E0100",
                        x.span,
                        &[("expected", "string | number | bool"), ("found", &s)],
                    );
                }
            }
        }
        STR
    }

    // ---- names -------------------------------------------------------------------

    pub fn narrowable(&self, l: LocalId) -> bool {
        let Some(ctx) = self.fnctx.last() else { return false };
        let name = &self.mods[self.cm].out.locals[l.0 as usize].name;
        if ctx.closure_assigned.contains(name) || ctx.closure_assigned.contains("*") {
            return false;
        }
        let owner = self.mods[self.cm].local_fn[l.0 as usize] as usize;
        if owner + 1 < self.fnctx.len() && (ctx.assigned.contains(name) || ctx.assigned.contains("*")) {
            return false;
        }
        true
    }

    /// Use of local `l` at `node`: capture bookkeeping, definite assignment, narrowing.
    pub fn use_local(&mut self, node: NodeId, l: LocalId, span: Span) -> Ty {
        let owner = self.mods[self.cm].local_fn[l.0 as usize] as usize;
        let cur = self.fnctx.len().saturating_sub(1);
        if owner < cur {
            for i in owner + 1..=cur {
                if self.fnctx[i].is_closure {
                    if !self.fnctx[i].captures.contains(&l) {
                        self.fnctx[i].captures.push(l);
                    }
                } else {
                    let n = self.mods[self.cm].out.locals[l.0 as usize].name.clone();
                    self.err(
                        "OXS-E0126",
                        span,
                        &[("detail", &format!("nested function cannot capture `{n}` from the enclosing function; pass it as a parameter or use an arrow function"))],
                    );
                    return ERROR;
                }
            }
            self.mods[self.cm].out.locals[l.0 as usize].captured = true;
        }
        self.mods[self.cm].out.locals[l.0 as usize].used = true;
        let declared = self.mods[self.cm].out.locals[l.0 as usize].ty;
        let skip = std::mem::take(&mut self.skip_unassigned);
        if !skip && self.flow.is_unassigned(l.0) {
            let n = self.mods[self.cm].out.locals[l.0 as usize].name.clone();
            self.err("OXS-E0118", span, &[("name", &n)]);
            return declared;
        }
        let mut t = declared;
        if self.narrowable(l) {
            let p = self.paths.root(l.0);
            if let Some(f) = self.flow.get(p) {
                t = f;
            }
        }
        let kind = self.mods[self.cm].out.locals[l.0 as usize].kind;
        self.set_res(node, if kind == LocalKind::This { Res::This(l) } else { Res::Local(l) });
        t
    }

    fn ident(&mut self, e: &'a Expr, id: &'a Ident) -> Ty {
        match self.lookup(&id.name) {
            None => {
                self.err("OXS-E0300", id.span, &[("name", &id.name)]);
                ERROR
            }
            Some(Binding::Local(l)) => self.use_local(e.id, l, id.span),
            Some(Binding::Def(d)) => self.value_of_def(e, d),
            Some(Binding::BuiltinFn(_)) => {
                self.err("OXS-E0115", id.span, &[("detail", "`range` can only be called: `range(a, b)`")]);
                ERROR
            }
            Some(Binding::TParam(_) | Binding::Prim(_) | Binding::ArrayCtor) => {
                self.err("OXS-E0303", id.span, &[("name", &id.name), ("found", "a type"), ("expected", "a value")]);
                ERROR
            }
        }
    }

    pub fn value_of_def(&mut self, e: &'a Expr, d: DefId) -> Ty {
        let kind = self.kind_of(d);
        let name = self.def_name(d).to_string();
        match kind {
            DefKind::Fn => {
                self.ensure_fn(d);
                let Some(sig) = self.fns.get(&d).cloned() else { return ERROR };
                if !sig.tparams.is_empty() {
                    self.err(
                        "OXS-E0117",
                        e.span,
                        &[("detail", &format!("generic function `{name}` cannot be used as a value; call it or write an arrow function"))],
                    );
                    return ERROR;
                }
                if sig.ret == ERROR && self.defs[d.0 as usize].sig == Sig::InProgress {
                    self.err("OXS-E0103", e.span, &[("what", &format!("return type of recursive function `{name}`; add a return type annotation"))]);
                    return ERROR;
                }
                self.set_res(e.id, Res::Def(d));
                let ps: Vec<Ty> = sig.params.iter().map(|p| p.ty).collect();
                self.tys.func(&ps, sig.ret)
            }
            DefKind::Global => {
                self.ensure_global(d);
                self.set_res(e.id, Res::Def(d));
                if self.globals.get(&d).is_some_and(|g| g.mutable) {
                    self.note_effect(2);
                    if self.in_law() {
                        self.err("OXS-E0125", e.span, &[("detail", "a law cannot read module-level `let` state")]);
                    }
                }
                self.globals.get(&d).map_or(ERROR, |g| g.ty)
            }
            DefKind::Struct | DefKind::Class | DefKind::Enum | DefKind::Union | DefKind::Variant { .. } | DefKind::Interface | DefKind::Alias | DefKind::Host | DefKind::Handle => {
                self.err("OXS-E0303", e.span, &[("name", &name), ("found", "a type"), ("expected", "a value")]);
                ERROR
            }
            DefKind::Namespace => {
                self.err("OXS-E0303", e.span, &[("name", &name), ("found", "a namespace"), ("expected", "a value")]);
                ERROR
            }
            DefKind::Law => {
                self.err("OXS-E0303", e.span, &[("name", &name), ("found", "a law"), ("expected", "a value")]);
                ERROR
            }
            DefKind::Method | DefKind::Ctor => ERROR,
        }
    }

    fn this_expr(&mut self, e: &'a Expr) -> Ty {
        if let Some(Binding::Local(l)) = self.lookup("this") {
            // use of `this` inside a constructor before all fields are assigned
            if let Some(c) = self.fnctx.iter().rev().find(|c| !c.is_closure) {
                if c.is_ctor && self.flow.has_unassigned_fields() && !self.this_member_ok {
                    self.err(
                        "OXS-E0119",
                        e.span,
                        &[("detail", "`this` cannot be used before every field is assigned in the constructor")],
                    );
                }
            }
            self.this_member_ok = false;
            self.use_local(e.id, l, e.span)
        } else {
            self.err("OXS-E0119", e.span, &[("detail", "`this` is only valid inside a method or constructor")]);
            ERROR
        }
    }

    // ---- arrays and objects -------------------------------------------------------

    /// Widen a lone union variant to its union (for inferred element/variable types).
    pub fn widen(&mut self, t: Ty) -> Ty {
        if let TyKind::Adt(d, a) = self.tys.kind(t).clone() {
            if let DefKind::Variant { union: Some(u), .. } = self.kind_of(d) {
                let args: Vec<Ty> = if self.defs[u.0 as usize].info.tparams.is_empty() { vec![] } else { a.to_vec() };
                return self.tys.adt(u, &args);
            }
        }
        if let TyKind::Variants(u, a, _) = self.tys.kind(t).clone() {
            return self.tys.adt(u, &a);
        }
        t
    }

    fn array_lit(&mut self, e: &'a Expr, els: &'a [Expr], exp: Option<Ty>) -> Ty {
        let elem_hint = exp.map(|t| self.non_null(t)).and_then(|t| match self.tys.kind(t) {
            TyKind::Array(x) => Some(*x),
            _ => None,
        });
        if let Some(h) = elem_hint {
            for x in els {
                self.expr_to(x, h);
            }
            return self.tys.array(h);
        }
        if els.is_empty() {
            self.err("OXS-E0103", e.span, &[("what", "empty array literal")]);
            return ERROR;
        }
        let mut tys: Vec<Ty> = Vec::with_capacity(els.len());
        let first = self.expr(&els[0], None);
        tys.push(first);
        let mut elem = self.widen(first);
        for x in &els[1..] {
            let t = self.expr(x, Some(elem));
            if self.assignable(t, elem) {
                self.record_coercion(x.id, t, elem);
            } else if self.assignable(elem, t) {
                elem = t;
            } else {
                self.mismatch_fix(x, elem, t);
            }
            tys.push(t);
        }
        if elem == NULL {
            self.err("OXS-E0103", e.span, &[("what", "array of `null`")]);
            return ERROR;
        }
        if elem == NEVER || elem == VOID {
            return ERROR;
        }
        for (x, t) in els.iter().zip(tys) {
            self.record_coercion(x.id, t, elem);
        }
        self.tys.array(elem)
    }

    fn object_lit(&mut self, e: &'a Expr, fields: &'a [ObjectField], exp: Option<Ty>) -> Ty {
        let Some(exp) = exp else {
            self.err("OXS-E0103", e.span, &[("what", "object literal")]);
            for f in fields {
                if let Some(v) = &f.value {
                    self.expr(v, None);
                }
            }
            return ERROR;
        };
        let target = self.non_null(exp);
        // find the record def and its type arguments
        let (def, args): (DefId, Vec<Ty>) = if let Some((u, uargs, mask)) = self.union_view(target) {
            // choose the variant by the discriminant literal
            let Some(usig) = self.unions.get(&u).cloned() else { return ERROR };
            let mut chosen = None;
            for f in fields {
                if f.name.name == usig.discr_field {
                    let lit = f.value.as_ref().map(strip_paren).and_then(|v| match &v.kind {
                        ExprKind::Lit(l) => match &l.kind {
                            LitKind::Str(s) => Some(Discr::Str(s.clone())),
                            LitKind::Int { value, .. } => i128::try_from(*value).ok().map(Discr::Int),
                            LitKind::Bool(b) => Some(Discr::Bool(*b)),
                            _ => None,
                        },
                        _ => None,
                    });
                    if let Some(v) = lit {
                        chosen = usig.variants.iter().position(|x| x.value == v).map(|i| (i, f));
                        if chosen.is_none() {
                            let uname = self.show(target);
                            self.err(
                                "OXS-E0120",
                                f.span,
                                &[("detail", &format!("Type '{}' is not assignable to type '{uname}'; no member has {}: {}", self.span_text(f.value.as_ref().map_or(f.span, |v| v.span)), usig.discr_field, self.span_text(f.value.as_ref().map_or(f.span, |v| v.span))))],
                            );
                            return ERROR;
                        }
                    } else {
                        self.err("OXS-E0120", f.span, &[("detail", &format!("the `{}` field of a union value must be a literal", usig.discr_field))]);
                        return ERROR;
                    }
                }
            }
            let Some((i, df)) = chosen else {
                let uname = self.show(target);
                self.err("OXS-E0120", e.span, &[("detail", &format!("Property '{}' is missing in object literal of type '{uname}'", usig.discr_field))]);
                return ERROR;
            };
            if mask & (1u64 << i) == 0 {
                let uname = self.show(target);
                self.err("OXS-E0120", df.span, &[("detail", &format!("variant `{}` is not possible here: expected `{uname}`", usig.variants[i].value.display()))]);
                return ERROR;
            }
            let v = usig.variants[i].clone();
            let tp = self.defs[u.0 as usize].info.tparams.clone();
            let map: Vec<(ParamId, Ty)> = tp.into_iter().zip(uargs.iter().copied()).collect();
            let vargs: Vec<Ty> = v.args.iter().map(|x| self.tys.subst(*x, &map)).collect();
            (v.def, vargs)
        } else if let TyKind::Adt(d, a) = self.tys.kind(target).clone() {
            match self.kind_of(d) {
                DefKind::Struct => (d, a.to_vec()),
                DefKind::Variant { union: None, .. } => (d, a.to_vec()),
                DefKind::Class => {
                    self.err("OXS-E0120", e.span, &[("detail", "classes are created with `new`, not with an object literal")]);
                    return ERROR;
                }
                _ => {
                    let s = self.show(target);
                    self.err("OXS-E0120", e.span, &[("detail", &format!("Type '{{...}}' is not assignable to type '{s}'"))]);
                    return ERROR;
                }
            }
        } else {
            if target != ERROR {
                let s = self.show(target);
                self.err("OXS-E0120", e.span, &[("detail", &format!("an object literal cannot have type '{s}'"))]);
            }
            return ERROR;
        };
        self.ensure_record(def);
        let Some(rec) = self.records.get(&def).cloned() else { return ERROR };
        let tps = self.defs[def.0 as usize].info.tparams.clone();
        let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(args.iter().copied()).collect();
        let disc_name = rec.discr.as_ref().map(|(n, _)| n.clone());
        let inst: Vec<&FieldInfo> = rec.fields.iter().filter(|f| !f.is_static).collect();
        let mut seen: Vec<&str> = Vec::new();
        for f in fields {
            let name = f.name.name.as_str();
            if disc_name.as_deref() == Some(name) {
                // discriminant literal: typed as its base type
                if let Some(v) = &f.value {
                    let b = self.unions_discr_ty(def);
                    self.expr(v, Some(b));
                } else {
                    self.err("OXS-E0120", f.span, &[("detail", "the discriminant must be written as a literal, e.g. `kind: \"ok\"`")]);
                }
                seen.push(name);
                continue;
            }
            if seen.contains(&name) {
                self.err("OXS-E0120", f.span, &[("detail", &format!("Duplicate identifier '{name}' in object literal"))]);
                continue;
            }
            seen.push(name);
            let Some(fs) = inst.iter().find(|x| x.name == name) else {
                let s = self.show(target);
                self.err("OXS-E0120", f.name.span, &[("detail", &format!("Object literal may only specify known properties, and '{name}' does not exist in type '{s}'"))]);
                if let Some(v) = &f.value {
                    self.expr(v, None);
                }
                continue;
            };
            let fty = self.tys.subst(fs.ty, &map);
            if let Some(v) = &f.value {
                self.expr_to(v, fty);
            } else {
                // shorthand `{ x }`
                match self.lookup(name) {
                    Some(Binding::Local(l)) => {
                        let t = self.use_local(f.id, l, f.name.span);
                        if self.assignable(t, fty) {
                            self.record_coercion(f.id, t, fty);
                        } else {
                            self.mismatch(f.name.span, fty, t);
                        }
                    }
                    Some(Binding::Def(d)) if self.kind_of(d) == DefKind::Global => {
                        self.ensure_global(d);
                        let t = self.globals.get(&d).map_or(ERROR, |g| g.ty);
                        self.set_res(f.id, Res::Def(d));
                        if !self.assignable(t, fty) {
                            self.mismatch(f.name.span, fty, t);
                        }
                        self.record_coercion(f.id, t, fty);
                    }
                    _ => {
                        self.err("OXS-E0300", f.name.span, &[("name", name)]);
                    }
                }
                self.set_ty(f.id, fty);
            }
        }
        let missing: Vec<&str> = inst.iter().filter(|f| !f.has_init && !seen.contains(&f.name.as_str())).map(|f| f.name.as_str()).collect();
        if !missing.is_empty() {
            let s = self.show(target);
            self.err(
                "OXS-E0120",
                e.span,
                &[("detail", &format!("Property '{}' is missing in object literal but required in type '{s}'", missing.join("', '")))],
            );
        }
        if disc_name.is_some() && !seen.contains(&disc_name.as_deref().unwrap_or("")) {
            self.err("OXS-E0120", e.span, &[("detail", &format!("Property '{}' is missing in object literal", disc_name.unwrap_or_default()))]);
        }
        self.mods[self.cm].out.object_lits.insert(e.id, def);
        self.tys.adt(def, &args)
    }

    fn unions_discr_ty(&self, variant: DefId) -> Ty {
        if let DefKind::Variant { union: Some(u), .. } = self.kind_of(variant) {
            if let Some(s) = self.unions.get(&u) {
                return s.discr_ty;
            }
        }
        match self.records.get(&variant).and_then(|r| r.discr.as_ref()) {
            Some((_, Discr::Str(_))) => STR,
            Some((_, Discr::Int(_))) => I32,
            Some((_, Discr::Bool(_))) => BOOL,
            None => ERROR,
        }
    }

    // ---- operators ---------------------------------------------------------------

    fn unary(&mut self, e: &'a Expr, op: UnOp, inner: &'a Expr, exp: Option<Ty>) -> Ty {
        match op {
            UnOp::Not => {
                let t = self.expr(inner, Some(BOOL));
                if t != BOOL && t != ERROR && t != NEVER {
                    let s = self.show(t);
                    self.err("OXS-E0128", e.span, &[("op", "!"), ("ty", &s)]);
                }
                BOOL
            }
            UnOp::Neg => {
                let inner_s = strip_paren(inner);
                if let ExprKind::Lit(l) = &inner_s.kind {
                    if matches!(l.kind, LitKind::Int { .. }) && std::ptr::eq(inner_s, inner) {
                        let t = self.lit(l, exp, true);
                        self.set_ty(inner.id, t);
                        if let TyKind::Num(n) = self.tys.kind(t) {
                            if !is_signed(*n) && !n.is_float() {
                                let s = n.name();
                                self.err("OXS-E0128", e.span, &[("op", "-"), ("ty", s)]);
                            }
                        }
                        return t;
                    }
                }
                let t = self.expr(inner, exp);
                match self.tys.kind(t) {
                    TyKind::Num(n) if is_signed(*n) || n.is_float() => t,
                    TyKind::Error | TyKind::Never => t,
                    _ => {
                        let s = self.show(t);
                        self.err("OXS-E0128", e.span, &[("op", "-"), ("ty", &s)]);
                        ERROR
                    }
                }
            }
            UnOp::BitNot => {
                let t = self.expr(inner, exp);
                match self.tys.kind(t) {
                    TyKind::Num(n) if is_int(*n) => t,
                    TyKind::Error | TyKind::Never => t,
                    _ => {
                        let s = self.show(t);
                        self.err("OXS-E0128", e.span, &[("op", "~"), ("ty", &s)]);
                        ERROR
                    }
                }
            }
        }
    }

    pub fn op_str(op: BinOp) -> &'static str {
        match op {
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Rem => "%",
            BinOp::AddWrap => "+%",
            BinOp::SubWrap => "-%",
            BinOp::MulWrap => "*%",
            BinOp::AddSat => "+|",
            BinOp::SubSat => "-|",
            BinOp::MulSat => "*|",
            BinOp::BitAnd => "&",
            BinOp::BitOr => "|",
            BinOp::BitXor => "^",
            BinOp::Shl => "<<",
            BinOp::Shr => ">>",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::Eq => "===",
            BinOp::Ne => "!==",
            BinOp::And => "&&",
            BinOp::Or => "||",
            BinOp::Coalesce => "??",
        }
    }

    /// Result type of an arithmetic or comparison operator on operand types, or `None`.
    pub fn binary_result(&mut self, op: BinOp, l: Ty, r: Ty) -> Option<Ty> {
        let (lk, rk) = (self.tys.kind(l).clone(), self.tys.kind(r).clone());
        match op {
            BinOp::Add => match (&lk, &rk) {
                (TyKind::Str, TyKind::Str) => Some(STR),
                (TyKind::Num(_), TyKind::Num(_)) if l == r => Some(l),
                _ => None,
            },
            BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Rem => match (&lk, &rk) {
                (TyKind::Num(_), TyKind::Num(_)) if l == r => Some(l),
                _ => None,
            },
            BinOp::AddWrap | BinOp::SubWrap | BinOp::MulWrap | BinOp::AddSat | BinOp::SubSat | BinOp::MulSat | BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor => match (&lk, &rk) {
                (TyKind::Num(n), TyKind::Num(_)) if l == r && is_int(*n) => Some(l),
                _ => None,
            },
            BinOp::Shl | BinOp::Shr => match (&lk, &rk) {
                (TyKind::Num(a), TyKind::Num(_)) if is_int(*a) && l == r => Some(l),
                _ => None,
            },
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => match (&lk, &rk) {
                (TyKind::Num(_), TyKind::Num(_)) if l == r => Some(BOOL),
                (TyKind::Str, TyKind::Str) => Some(BOOL),
                _ => None,
            },
            BinOp::And | BinOp::Or => {
                if l == BOOL && r == BOOL {
                    Some(BOOL)
                } else {
                    None
                }
            }
            BinOp::Eq | BinOp::Ne | BinOp::Coalesce => None,
        }
    }

    /// Can `===` compare these types?
    fn eq_ok(&mut self, l: Ty, r: Ty) -> bool {
        let (ln, rn) = (self.non_null(l), self.non_null(r));
        let nullish = |t: Ty, k: &TyKind| t == NULL || matches!(k, TyKind::Nullable(_));
        let (lk, rk) = (self.tys.kind(l).clone(), self.tys.kind(r).clone());
        if l == NULL || r == NULL {
            return nullish(l, &lk) && nullish(r, &rk) || (l == NULL && nullish(r, &rk)) || (r == NULL && nullish(l, &lk));
        }
        if ln != rn {
            return false;
        }
        match self.tys.kind(ln) {
            TyKind::Num(_) | TyKind::Bool | TyKind::Str => true,
            TyKind::Adt(d, _) => matches!(self.kind_of(*d), DefKind::Enum | DefKind::Class),
            _ => false,
        }
    }

    fn lhs_leaf_chain(e: &'a Expr) -> Vec<&'a Expr> {
        // spine of left-nested binary operators, outermost first
        let mut v = vec![e];
        let mut cur = e;
        while let ExprKind::Binary { lhs, .. } = &cur.kind {
            if let ExprKind::Binary { .. } = &lhs.kind {
                v.push(lhs);
                cur = lhs;
            } else {
                break;
            }
        }
        v
    }

    /// Binary operators; left-deep chains are folded iteratively.
    pub fn binary_chain(&mut self, e: &'a Expr, exp: Option<Ty>) -> Ty {
        let ExprKind::Binary { op: top_op, .. } = &e.kind else { return ERROR };
        // logical and equality forms are handled through the condition machinery
        if matches!(top_op, BinOp::And | BinOp::Or | BinOp::Eq | BinOp::Ne) {
            let (t, f) = self.cond(e);
            self.flow = self.join_flows(t, f);
            return BOOL;
        }
        if *top_op == BinOp::Coalesce {
            return self.coalesce(e, exp);
        }
        let spine = Self::lhs_leaf_chain(e);
        // only fold chains of the same operator class iteratively; a different class ends the spine
        let mut chain: Vec<&'a Expr> = Vec::new();
        for n in &spine {
            let ExprKind::Binary { op, .. } = &n.kind else { break };
            if matches!(op, BinOp::And | BinOp::Or | BinOp::Eq | BinOp::Ne | BinOp::Coalesce) && !std::ptr::eq(*n, e) {
                break;
            }
            chain.push(n);
        }
        // innermost node's lhs is evaluated first
        let innermost = chain[chain.len() - 1];
        let ExprKind::Binary { lhs: base, .. } = &innermost.kind else { return ERROR };
        let arithmetic = |op: BinOp| !matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge);
        let ExprKind::Binary { op: iop, .. } = &innermost.kind else { return ERROR };
        let base_hint = if arithmetic(*top_op) { exp } else { None };
        let mut acc: Ty;
        // literal on the left with a non-literal right operand: evaluate the right side first
        let mut pre_rhs: Option<(Ty, usize)> = None;
        if is_lit_expr(base) && !chain.is_empty() {
            let ExprKind::Binary { rhs, .. } = &innermost.kind else { return ERROR };
            if !is_lit_expr(rhs) && !matches!(iop, BinOp::Shl | BinOp::Shr) {
                let rt = self.expr(rhs, None);
                pre_rhs = Some((rt, chain.len() - 1));
                let hint = if matches!(self.tys.kind(rt), TyKind::Num(_)) { Some(rt) } else { base_hint };
                acc = self.expr(base, hint);
            } else {
                acc = self.expr(base, base_hint);
            }
        } else {
            acc = self.expr(base, base_hint);
        }
        for (k, node) in chain.iter().rev().enumerate() {
            let idx = chain.len() - 1 - k;
            let ExprKind::Binary { op, rhs, .. } = &node.kind else { return ERROR };
            let rt = match pre_rhs {
                Some((rt, i)) if i == idx => rt,
                _ => {
                    let hint = match op {
                        BinOp::Shl | BinOp::Shr => Some(acc),
                        _ if matches!(self.tys.kind(acc), TyKind::Num(_)) => Some(acc),
                        _ => None,
                    };
                    self.expr(rhs, hint)
                }
            };
            let lhs_e = match &node.kind {
                ExprKind::Binary { lhs, .. } => Some(&**lhs),
                _ => None,
            };
            let res = if acc == ERROR || rt == ERROR {
                if matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) { BOOL } else { ERROR }
            } else if acc == NEVER || rt == NEVER {
                if matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) { BOOL } else { NEVER }
            } else if let Some(t) = self.binary_result(*op, acc, rt) {
                if matches!(op, BinOp::Shl | BinOp::Shr) {
                    self.check_shift_amount(rhs, acc);
                }
                t
            } else if let (TyKind::Nullable(i), Some(l)) = (self.tys.kind(acc).clone(), lhs_e) {
                if self.binary_result(*op, i, rt).is_some() {
                    self.possibly_null(l, acc);
                    ERROR
                } else {
                    self.operand_mismatch(node, *op, acc, rt, rhs);
                    ERROR
                }
            } else if let TyKind::Nullable(i) = self.tys.kind(rt).clone() {
                if self.binary_result(*op, acc, i).is_some() {
                    self.possibly_null(rhs, rt);
                    ERROR
                } else {
                    self.operand_mismatch(node, *op, acc, rt, rhs);
                    ERROR
                }
            } else {
                self.operand_mismatch(node, *op, acc, rt, rhs);
                ERROR
            };
            self.set_ty(node.id, res);
            acc = res;
        }
        acc
    }

    /// A constant shift amount must be in `0..bits` of the shifted type (ir.md `shl`/`shr`).
    fn check_shift_amount(&mut self, rhs: &Expr, lt: Ty) {
        let TyKind::Num(n) = self.tys.kind(lt).clone() else { return };
        let bits = u128::from(int_bits(n));
        let (e, neg) = match &strip_paren(rhs).kind {
            ExprKind::Unary { op: UnOp::Neg, expr } => (strip_paren(expr), true),
            _ => (strip_paren(rhs), false),
        };
        if let ExprKind::Lit(Lit { kind: LitKind::Int { value, .. }, .. }) = &e.kind {
            if neg && *value > 0 || *value >= bits {
                let shown = if neg { format!("-{value}") } else { value.to_string() };
                let what = format!("a shift amount of `{}` (0..{})", n.name(), bits - 1);
                self.err("OXS-E0102", rhs.span, &[("value", &shown), ("ty", &what)]);
            }
        }
    }

    fn operand_mismatch(&mut self, node: &'a Expr, op: BinOp, l: Ty, r: Ty, rhs: &'a Expr) {
        let (sl, sr) = (self.show(l), self.show(r));
        let fix = if matches!(op, BinOp::Shl | BinOp::Shr) { None } else { self.as_fix(rhs, l, r, format!("Convert the right operand with `as {sl}`")) };
        self.err_fix("OXS-E0110", node.span, &[("op", Self::op_str(op)), ("lhs", &sl), ("rhs", &sr)], fix);
    }

    fn coalesce(&mut self, e: &'a Expr, exp: Option<Ty>) -> Ty {
        // a ?? b ?? c is left-nested; recursion depth is bounded by the expression depth cap
        let ExprKind::Binary { lhs, rhs, .. } = &e.kind else { return ERROR };
        let lhint = exp.map(|t| self.tys.nullable(t));
        let lt = self.expr(lhs, lhint);
        let inner = match self.tys.kind(lt).clone() {
            TyKind::Nullable(i) => i,
            TyKind::Error => ERROR,
            TyKind::Null => ERROR,
            _ => {
                let s = self.show(lt);
                self.err("OXS-E0127", lhs.span, &[("expr", self.span_text(lhs.span)), ("ty", &s)]);
                lt
            }
        };
        let hint = if inner == ERROR { exp } else { Some(inner) };
        let rt = self.expr(rhs, hint);
        if inner == ERROR {
            return rt;
        }
        if self.assignable(rt, inner) {
            self.record_coercion(rhs.id, rt, inner);
            inner
        } else if let TyKind::Nullable(ri) = self.tys.kind(rt).clone() {
            if self.assignable(ri, inner) {
                return rt;
            }
            self.mismatch_fix(rhs, inner, rt);
            inner
        } else {
            self.mismatch_fix(rhs, inner, rt);
            inner
        }
    }

    // ---- conditions and narrowing --------------------------------------------------

    /// Check boolean expression `e`; returns the flows when it is true and when it is false.
    pub fn cond(&mut self, e: &'a Expr) -> (Flow, Flow) {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            self.depth -= 1;
            self.err("OXS-E0113", e.span, &[("detail", "expression is nested too deeply")]);
            return (self.flow.clone(), self.flow.clone());
        }
        let r = self.cond_inner(e);
        self.depth -= 1;
        r
    }

    fn cond_inner(&mut self, e: &'a Expr) -> (Flow, Flow) {
        match &e.kind {
            ExprKind::Paren(i) => {
                let r = self.cond(i);
                let t = self.mods[self.cm].out.expr_ty.get(i.id.0 as usize).copied().unwrap_or(ERROR);
                self.set_ty(e.id, t);
                r
            }
            ExprKind::Unary { op: UnOp::Not, expr } => {
                let (t, f) = self.cond(expr);
                let it = self.mods[self.cm].out.expr_ty.get(expr.id.0 as usize).copied().unwrap_or(ERROR);
                if it != BOOL && it != ERROR && it != NEVER {
                    let s = self.show(it);
                    self.err("OXS-E0128", e.span, &[("op", "!"), ("ty", &s)]);
                }
                self.set_ty(e.id, BOOL);
                (f, t)
            }
            ExprKind::Binary { op: BinOp::And, lhs, rhs } => {
                let (ta, fa) = self.cond(lhs);
                self.flow = ta;
                let (tb, fb) = self.cond(rhs);
                self.check_bool_operand(lhs, BinOp::And, e);
                self.check_bool_operand(rhs, BinOp::And, e);
                let f = self.join_flows(fa, fb);
                self.set_ty(e.id, BOOL);
                (tb, f)
            }
            ExprKind::Binary { op: BinOp::Or, lhs, rhs } => {
                let (ta, fa) = self.cond(lhs);
                self.flow = fa;
                let (tb, fb) = self.cond(rhs);
                self.check_bool_operand(lhs, BinOp::Or, e);
                self.check_bool_operand(rhs, BinOp::Or, e);
                let t = self.join_flows(ta, tb);
                self.set_ty(e.id, BOOL);
                (t, fb)
            }
            ExprKind::Binary { op: op @ (BinOp::Eq | BinOp::Ne), lhs, rhs } => {
                let r = self.equality(e, *op, lhs, rhs);
                self.set_ty(e.id, BOOL);
                r
            }
            _ => {
                let t = self.expr(e, Some(BOOL));
                let _ = t;
                (self.flow.clone(), self.flow.clone())
            }
        }
    }

    fn check_bool_operand(&mut self, x: &'a Expr, op: BinOp, whole: &'a Expr) {
        let t = self.mods[self.cm].out.expr_ty.get(x.id.0 as usize).copied().unwrap_or(ERROR);
        if t != BOOL && t != ERROR && t != NEVER {
            let s = self.show(t);
            let _ = whole;
            self.err("OXS-E0110", x.span, &[("op", Self::op_str(op)), ("lhs", &s), ("rhs", "bool")]);
        }
    }

    /// Type check a condition used by `if`/`while`/`?:`; returns the two flows.
    pub fn cond_bool(&mut self, e: &'a Expr) -> (Flow, Flow) {
        let r = self.cond(e);
        let t = self.mods[self.cm].out.expr_ty.get(e.id.0 as usize).copied().unwrap_or(ERROR);
        if t != BOOL && t != ERROR && t != NEVER {
            self.mismatch_fix(e, BOOL, t);
        }
        r
    }

    fn equality(&mut self, e: &'a Expr, op: BinOp, lhs: &'a Expr, rhs: &'a Expr) -> (Flow, Flow) {
        // literals adapt to the other side
        let (lt, rt);
        if is_lit_expr(lhs) && !is_lit_expr(rhs) {
            rt = self.expr(rhs, None);
            lt = self.expr(lhs, Some(rt));
        } else {
            lt = self.expr(lhs, None);
            rt = self.expr(rhs, Some(lt));
        }
        let unconditional = (self.flow.clone(), self.flow.clone());
        if lt == ERROR || rt == ERROR || lt == NEVER || rt == NEVER {
            return unconditional;
        }
        if !self.eq_ok(lt, rt) {
            let (sl, sr) = (self.show(lt), self.show(rt));
            self.err("OXS-E0110", e.span, &[("op", Self::op_str(op)), ("lhs", &sl), ("rhs", &sr)]);
            return unconditional;
        }
        // narrowing: `x === null` and `p.kind === "lit"`
        let mut tf = self.flow.clone();
        let mut ff = self.flow.clone();
        let (subject, other) = if rt == NULL || matches!(strip_paren(rhs).kind, ExprKind::Lit(_)) { (lhs, rhs) } else { (rhs, lhs) };
        let ot = self.mods[self.cm].out.expr_ty.get(other.id.0 as usize).copied().unwrap_or(ERROR);
        let st = self.mods[self.cm].out.expr_ty.get(subject.id.0 as usize).copied().unwrap_or(ERROR);
        let subj = strip_paren(subject);
        if ot == NULL {
            if let Some(p) = self.path_of(subj) {
                if let TyKind::Nullable(inner) = self.tys.kind(st).clone() {
                    tf.set(p, NULL);
                    ff.set(p, inner);
                } else if st == NULL {
                    ff.set(p, NEVER);
                }
            }
        } else if let ExprKind::Member { obj, .. } = &subj.kind {
            if let Some(Res::Discriminant { union }) = self.mods[self.cm].out.res.get(&subj.id).copied() {
                let lit_e = strip_paren(other);
                if let (Some(p), ExprKind::Lit(l)) = (self.path_of(strip_paren(obj)), &lit_e.kind) {
                    let ot_obj = self.mods[self.cm].out.expr_ty.get(obj.id.0 as usize).copied().unwrap_or(ERROR);
                    if let Some((u, uargs, mask)) = self.union_view(ot_obj) {
                        if u != union {
                            return (tf, ff);
                        }
                        let want = match &l.kind {
                            LitKind::Str(s) => Some(Discr::Str(s.clone())),
                            LitKind::Int { value, .. } => i128::try_from(*value).ok().map(Discr::Int),
                            LitKind::Bool(b) => Some(Discr::Bool(*b)),
                            _ => None,
                        };
                        if let (Some(w), Some(sig)) = (want, self.unions.get(&u).cloned()) {
                            if let Some(i) = sig.variants.iter().position(|v| v.value == w) {
                                let bit = 1u64 << i;
                                let tt = self.mk_variants(u, &uargs, mask & bit);
                                let ft = self.mk_variants(u, &uargs, mask & !bit);
                                tf.set(p, tt);
                                ff.set(p, ft);
                            } else {
                                let s = self.show(ot_obj);
                                self.err(
                                    "OXS-E0116",
                                    other.span,
                                    &[("detail", &format!("This comparison appears to be unintentional because the types '{s}' and '{}' have no overlap: no variant has {}: {}", self.span_text(other.span), sig.discr_field, self.span_text(other.span)))],
                                );
                            }
                        }
                    }
                }
            }
        }
        if op == BinOp::Eq { (tf, ff) } else { (ff, tf) }
    }

    /// The narrowing path of an already checked expression, if it has one.
    pub fn path_of(&mut self, e: &'a Expr) -> Option<u32> {
        match &e.kind {
            ExprKind::Paren(i) => self.path_of(i),
            ExprKind::Ident(_) | ExprKind::This => {
                let r = self.mods[self.cm].out.res.get(&e.id).copied()?;
                match r {
                    Res::Local(l) => self.narrowable(l).then(|| self.paths.root(l.0)),
                    Res::This(l) => {
                        let value = self.fnctx.iter().rev().find(|c| !c.is_closure).is_some_and(|c| c.this_is_value);
                        (value && self.narrowable(l)).then(|| self.paths.root(l.0))
                    }
                    _ => None,
                }
            }
            ExprKind::Member { obj, name, optional: false } => {
                let Some(Res::Field { .. }) = self.mods[self.cm].out.res.get(&e.id).copied() else { return None };
                let ot = self.mods[self.cm].out.expr_ty.get(obj.id.0 as usize).copied()?;
                // only value types (struct, union variants) can be narrowed through
                let value_ty = match self.tys.kind(ot) {
                    TyKind::Adt(d, _) => matches!(self.kind_of(*d), DefKind::Struct | DefKind::Variant { .. }),
                    TyKind::Variants(..) => true,
                    _ => false,
                };
                if !value_ty {
                    return None;
                }
                let p = self.path_of(obj)?;
                let root = self.paths.nodes[p as usize].root as usize;
                let rn = &self.mods[self.cm].out.locals[root].name;
                if self.fnctx.last().is_some_and(|c| c.closure_recv.contains(rn) || c.closure_recv.contains("*")) {
                    return None;
                }
                self.paths.field(p, &name.name)
            }
            _ => None,
        }
    }

    // ---- conditional expression ------------------------------------------------------------

    fn cond_expr(&mut self, e: &'a Expr, exp: Option<Ty>) -> Ty {
        // right-nested chains `a ? b : c ? d : e` are walked iteratively
        let mut chain: Vec<&'a Expr> = vec![e];
        let mut cur = e;
        while let ExprKind::Cond { els, .. } = &cur.kind {
            if let ExprKind::Cond { .. } = &els.kind {
                chain.push(els);
                cur = els;
            } else {
                break;
            }
        }
        let mut arms: Vec<(&'a Expr, Ty)> = Vec::new();
        let mut false_flow = self.flow.clone();
        let mut end_flows: Vec<Flow> = Vec::new();
        let mut hint = exp;
        for node in &chain {
            let ExprKind::Cond { cond, then, .. } = &node.kind else { return ERROR };
            self.flow = false_flow.clone();
            let (tf, ff) = self.cond_bool(cond);
            self.flow = tf;
            let tt = self.expr(then, hint);
            if hint.is_none() && tt != ERROR && tt != NULL {
                hint = Some(tt);
            }
            end_flows.push(self.flow.clone());
            arms.push((then, tt));
            false_flow = ff;
        }
        let last = chain[chain.len() - 1];
        let ExprKind::Cond { els, .. } = &last.kind else { return ERROR };
        self.flow = false_flow;
        let et = self.expr(els, hint);
        end_flows.push(self.flow.clone());
        arms.push((els, et));
        self.flow = self.join_all(end_flows);
        let res = self.unify_arms(&arms, exp);
        for n in chain.iter().skip(1) {
            self.set_ty(n.id, res);
        }
        res
    }

    /// Common type of several branch results.
    pub fn unify_arms(&mut self, arms: &[(&'a Expr, Ty)], exp: Option<Ty>) -> Ty {
        if let Some(x) = exp {
            if x != ERROR {
                let mut ok = true;
                for (ex, t) in arms {
                    if !self.assignable(*t, x) {
                        ok = false;
                    }
                    let _ = ex;
                }
                if ok {
                    for (ex, t) in arms {
                        self.record_coercion(ex.id, *t, x);
                    }
                    return x;
                }
            }
        }
        // pick a type all arms are assignable to
        let cands: Vec<Ty> = arms.iter().map(|(_, t)| *t).filter(|t| *t != ERROR && *t != NEVER).collect();
        if cands.is_empty() {
            return arms.first().map_or(ERROR, |a| if arms.iter().all(|x| x.1 == NEVER) { NEVER } else { a.1 });
        }
        let mut pick = None;
        for c in &cands {
            let mut all = true;
            for t in &cands {
                if !self.assignable(*t, *c) {
                    all = false;
                    break;
                }
            }
            if all {
                pick = Some(*c);
                break;
            }
        }
        if pick.is_none() {
            // widen variants of one union
            let mut acc = cands[0];
            let mut ok = true;
            for t in &cands[1..] {
                match self.join_ty(acc, *t) {
                    Some(j) => acc = j,
                    None => ok = false,
                }
            }
            if ok {
                pick = Some(acc);
            }
        }
        if let Some(p) = pick {
            for (ex, t) in arms {
                if *t != ERROR {
                    self.record_coercion(ex.id, *t, p);
                }
            }
            p
        } else {
            let (a, b) = (cands[0], cands.iter().copied().find(|t| *t != cands[0]).unwrap_or(cands[0]));
            if let Some((ex, _)) = arms.iter().find(|(_, t)| *t == b) {
                self.mismatch_fix(ex, a, b);
            }
            ERROR
        }
    }

    // ---- assignment -------------------------------------------------------------------------

    /// Type of an assignment target and whether it can be assigned; reports errors.
    fn place(&mut self, target: &'a Expr, plain: bool) -> Option<Ty> {
        let t = strip_paren(target);
        match &t.kind {
            ExprKind::Ident(_) => {
                self.skip_unassigned = plain;
                let ty = self.expr(t, None);
                self.skip_unassigned = false;
                if !std::ptr::eq(t, target) {
                    self.set_ty(target.id, ty);
                }
                match self.mods[self.cm].out.res.get(&t.id).copied() {
                    Some(Res::Local(l)) => {
                        let info = &self.mods[self.cm].out.locals[l.0 as usize];
                        if matches!(info.kind, LocalKind::Const | LocalKind::ForOf) {
                            let n = info.name.clone();
                            let is_for = info.kind == LocalKind::ForOf;
                            let fix = if is_for { None } else { self.const_to_let_fix(l) };
                            let detail = if is_for { "it is a loop variable declared with `const`" } else { "it is a constant" };
                            self.err_fix("OXS-E0109", target.span, &[("what", &format!("`{n}`")), ("detail", detail)], fix);
                            return None;
                        }
                        self.mods[self.cm].out.locals[l.0 as usize].mutated = true;
                        Some(self.mods[self.cm].out.locals[l.0 as usize].ty)
                    }
                    Some(Res::Def(d)) if self.kind_of(d) == DefKind::Global => {
                        let g = self.globals.get(&d).cloned()?;
                        if self.defs[d.0 as usize].info.module != self.cm as u32 {
                            let n = self.def_name(d).to_string();
                            self.err("OXS-E0109", target.span, &[("what", &format!("`{n}`")), ("detail", "it is an imported binding; module-level state belongs to the module that declares it")]);
                            return None;
                        }
                        if !g.mutable {
                            let n = self.def_name(d).to_string();
                            self.err("OXS-E0109", target.span, &[("what", &format!("`{n}`")), ("detail", "it is a constant")]);
                            return None;
                        }
                        self.note_effect(4);
                        Some(g.ty)
                    }
                    Some(_) => {
                        self.err("OXS-E0109", target.span, &[("what", "this name"), ("detail", "it is not a variable")]);
                        None
                    }
                    None => None,
                }
            }
            ExprKind::Member { .. } | ExprKind::Index { .. } => {
                self.assign_target = plain;
                let ty = self.expr(t, None);
                self.assign_target = false;
                if ty == ERROR {
                    return None;
                }
                self.check_place_mutable(t)?;
                Some(ty)
            }
            ExprKind::This => {
                self.err("OXS-E0109", target.span, &[("what", "`this`"), ("detail", "it is not a variable")]);
                None
            }
            _ => {
                self.expr(t, None);
                self.err("OXS-E0109", target.span, &[("what", "this expression"), ("detail", "it is not a variable, field or element")]);
                None
            }
        }
    }

    fn const_to_let_fix(&self, l: LocalId) -> Option<Fix> {
        let span = self.mods[self.cm].out.locals[l.0 as usize].span;
        // the declaration keyword precedes the binding name on the same statement
        let src = self.srcs.get(span.file.0 as usize).copied().unwrap_or("");
        let before = src.get(..span.lo as usize)?;
        let at = before.rfind("const")?;
        if span.lo as usize - at > 8 {
            return None;
        }
        Some(Fix::replace(
            "Declare it with `let`",
            Span::new(span.file, at as u32, at as u32 + 5),
            "let",
        ))
    }

    /// Walk a place expression and check that its root is mutable (`Some(())` when it is).
    pub(crate) fn check_place_mutable_pub(&mut self, t: &'a Expr) -> Option<()> {
        self.check_place_mutable(t)
    }

    fn check_place_mutable(&mut self, t: &'a Expr) -> Option<()> {
        match &strip_paren(t).kind {
            ExprKind::Member { obj, name, optional } => {
                if *optional {
                    self.err("OXS-E0109", t.span, &[("what", "an optional chain"), ("detail", "the left side of an assignment cannot use `?.`")]);
                    return None;
                }
                let res = self.mods[self.cm].out.res.get(&strip_paren(t).id).copied();
                let ot = self.mods[self.cm].out.expr_ty.get(obj.id.0 as usize).copied().unwrap_or(ERROR);
                match res {
                    Some(Res::Discriminant { .. }) => {
                        self.err(
                            "OXS-E0109",
                            t.span,
                            &[("what", &format!("`{}`", name.name)), ("detail", "the discriminant is derived from the variant and is read-only")],
                        );
                        None
                    }
                    Some(Res::Field { owner, index } | Res::StaticField { owner, index }) => {
                        let Some(f) = self.records.get(&owner).and_then(|r| r.fields.get(index as usize)).cloned() else { return Some(()) };
                        if matches!(self.kind_of(owner), DefKind::Variant { union: Some(_), .. }) {
                            self.err(
                                "OXS-E0109",
                                t.span,
                                &[("what", &format!("`{}`", name.name)), ("detail", "the fields of a union member are read-only; build a new value instead")],
                            );
                            return None;
                        }
                        if f.readonly {
                            // allowed only inside the constructor of the owning class on `this`
                            let in_ctor = self.fnctx.iter().rev().find(|c| !c.is_closure).is_some_and(|c| c.is_ctor && c.owner == Some(owner));
                            let on_this = matches!(strip_paren(obj).kind, ExprKind::This);
                            if !(in_ctor && on_this) {
                                self.err(
                                    "OXS-E0109",
                                    t.span,
                                    &[("what", &format!("`{}`", name.name)), ("detail", "it is a read-only property")],
                                );
                                return None;
                            }
                        }
                        // reference types are always mutable through any binding
                        if matches!(res, Some(Res::StaticField { .. })) || self.is_class_ty(ot) {
                            return Some(());
                        }
                        self.check_place_mutable(obj)
                    }
                    Some(Res::Builtin(_)) => {
                        self.err("OXS-E0109", t.span, &[("what", &format!("`{}`", name.name)), ("detail", "it is a read-only built-in property")]);
                        None
                    }
                    Some(Res::EnumMember { .. }) => {
                        self.err("OXS-E0109", t.span, &[("what", &format!("`{}`", name.name)), ("detail", "an enum member is a constant")]);
                        None
                    }
                    Some(Res::Def(d)) if self.kind_of(d) == DefKind::Global => {
                        self.err("OXS-E0109", t.span, &[("what", &format!("`{}`", name.name)), ("detail", "it is an imported binding; module-level state belongs to the module that declares it")]);
                        None
                    }
                    _ => Some(()),
                }
            }
            ExprKind::Index { obj, .. } => {
                let ot = self.mods[self.cm].out.expr_ty.get(obj.id.0 as usize).copied().unwrap_or(ERROR);
                if let TyKind::Array(_) = self.tys.kind(ot) {
                    Some(())
                } else {
                    self.err("OXS-E0109", t.span, &[("what", "this element"), ("detail", "only array elements can be assigned")]);
                    None
                }
            }
            ExprKind::Ident(_) => {
                match self.mods[self.cm].out.res.get(&strip_paren(t).id).copied() {
                    Some(Res::Local(l)) => {
                        let info = &self.mods[self.cm].out.locals[l.0 as usize];
                        if matches!(info.kind, LocalKind::Const | LocalKind::ForOf) {
                            let n = info.name.clone();
                            if info.kind == LocalKind::ForOf {
                                self.err(
                                    "OXS-E0109",
                                    t.span,
                                    &[("what", "a field of a loop variable"), ("detail", &format!("`{n}` is a copy of an array element, so the change would be lost; write to the element instead, e.g. `xs[i].field = ...`"))],
                                );
                                return None;
                            }
                            let fix = self.const_to_let_fix(l);
                            self.err_fix(
                                "OXS-E0109",
                                t.span,
                                &[("what", "a field of a `const` value"), ("detail", &format!("`{n}` is a constant and a struct is a value, so changing its field would change a copy; declare `{n}` with `let`"))],
                                fix,
                            );
                            return None;
                        }
                        self.mods[self.cm].out.locals[l.0 as usize].mutated = true;
                        Some(())
                    }
                    Some(Res::Def(d)) if self.kind_of(d) == DefKind::Global => {
                        if self.globals.get(&d).is_some_and(|g| !g.mutable) {
                            self.err("OXS-E0109", t.span, &[("what", "a field of a `const` value"), ("detail", "the module-level constant is a value; declare it with `let`")]);
                            return None;
                        }
                        Some(())
                    }
                    _ => Some(()),
                }
            }
            ExprKind::This => {
                let (value, mutating) = self.fnctx.iter().rev().find(|c| !c.is_closure).map_or((false, true), |c| (c.this_is_value, c.mutating));
                if value && !mutating {
                    let fix = self.mutating_fix();
                    self.err_fix(
                        "OXS-E0119",
                        t.span,
                        &[("detail", "this method assigns to `this` so it must be declared `mutating`")],
                        fix,
                    );
                    return None;
                }
                Some(())
            }
            _ => {
                self.err("OXS-E0109", t.span, &[("what", "this expression"), ("detail", "it is a temporary value, not a variable")]);
                None
            }
        }
    }

    pub fn mutating_fix(&self) -> Option<Fix> {
        let c = self.fnctx.iter().rev().find(|c| !c.is_closure)?;
        let sp = c.name_span?;
        Some(Fix::insert("Declare the method `mutating`", sp, "mutating "))
    }

    pub fn is_class_ty(&self, t: Ty) -> bool {
        matches!(self.tys.kind(t), TyKind::Adt(d, _) if matches!(self.kind_of(*d), DefKind::Class | DefKind::Host))
    }

    fn invalidate_target(&mut self, target: &'a Expr) {
        // kill narrowing of the assigned path (and everything below) and mark ctor fields assigned
        let t = strip_paren(target);
        if let Some(p) = self.path_of(t) {
            self.kill_path(p);
        }
        if let ExprKind::Ident(_) = &t.kind {
            if let Some(Res::Local(l)) = self.mods[self.cm].out.res.get(&t.id).copied() {
                self.flow.assign(l.0);
            }
        }
        if let ExprKind::Member { obj, .. } = &t.kind {
            if matches!(strip_paren(obj).kind, ExprKind::This) {
                if let Some(Res::Field { index, .. }) = self.mods[self.cm].out.res.get(&t.id).copied() {
                    self.flow.assign(0x8000_0000 | index);
                }
            }
            // assigning through a value-type prefix invalidates narrowing of the prefix's children
            if let Some(p) = self.path_of(strip_paren(obj)) {
                self.kill_children(p);
            }
        }
        if let ExprKind::Index { obj, .. } = &t.kind {
            // an element write cannot change a path, but a struct element's fields may alias nothing
            let _ = obj;
        }
    }

    pub(crate) fn kill_children(&mut self, p: u32) {
        let paths = &self.paths;
        self.flow.facts.retain(|(q, _)| *q == p || !paths.is_prefix(p, *q));
    }

    fn assign(&mut self, e: &'a Expr, op: Option<BinOp>, target: &'a Expr, value: &'a Expr) -> Ty {
        // the value is evaluated first for definite-assignment purposes only when it does not mention the target
        let tt = self.place(target, op.is_none());
        let Some(tt) = tt else {
            self.expr(value, None);
            return ERROR;
        };
        self.set_ty(target.id, tt);
        match op {
            None => {
                let vt = self.expr(value, Some(tt));
                let ok = self.coerce_to(value, vt, tt);
                self.invalidate_target(target);
                // assignment narrowing for locals declared with a wider type
                if ok && vt != tt {
                    if let Some(p) = self.path_of(strip_paren(target)) {
                        if vt != ERROR && self.mods[self.cm].out.res.get(&strip_paren(target).id).is_some_and(|r| matches!(r, Res::Local(_))) {
                            self.flow.set(p, vt);
                        }
                    }
                }
            }
            Some(bop) => {
                let vt = self.expr(value, Some(tt));
                if tt != ERROR && vt != ERROR {
                    match self.binary_result(bop, tt, vt) {
                        Some(r) if r == tt => {
                            if matches!(bop, BinOp::Shl | BinOp::Shr) {
                                self.check_shift_amount(value, tt);
                            }
                        }
                        _ => {
                            let (sl, sr) = (self.show(tt), self.show(vt));
                            let fix = if matches!(bop, BinOp::Shl | BinOp::Shr) { None } else { self.as_fix(value, tt, vt, format!("Convert the right operand with `as {sl}`")) };
                            self.err_fix("OXS-E0110", e.span, &[("op", &format!("{}=", Self::op_str(bop))), ("lhs", &sl), ("rhs", &sr)], fix);
                        }
                    }
                }
                self.invalidate_target(target);
            }
        }
        let _ = e;
        tt
    }

    fn update(&mut self, e: &'a Expr, target: &'a Expr) -> Ty {
        let Some(tt) = self.place(target, false) else { return ERROR };
        self.set_ty(target.id, tt);
        if !matches!(self.tys.kind(tt), TyKind::Num(_)) && tt != ERROR {
            let s = self.show(tt);
            self.err("OXS-E0128", e.span, &[("op", "++"), ("ty", &s)]);
        }
        self.invalidate_target(target);
        tt
    }

    // ---- casts ------------------------------------------------------------------------------

    fn cast_expr(&mut self, e: &'a Expr, inner: &'a Expr, ty: &'a TypeExpr) -> Ty {
        let target = self.lower_type(ty);
        let hint = if matches!(self.tys.kind(target), TyKind::Num(_)) { None } else { Some(target) };
        let from = self.expr(inner, hint);
        if from == ERROR || target == ERROR {
            return target;
        }
        let ok = match (self.tys.kind(from).clone(), self.tys.kind(target).clone()) {
            (TyKind::Num(_), TyKind::Num(_)) => true,
            (TyKind::Adt(a, _), TyKind::Num(n)) if self.kind_of(a) == DefKind::Enum => {
                let _ = n;
                true
            }
            _ => self.assignable(from, target),
        };
        if !ok {
            let (sf, st) = (self.show(from), self.show(target));
            self.err("OXS-E0111", e.span, &[("from", &sf), ("to", &st)]);
            return target;
        }
        if self.assignable(from, target) {
            self.record_coercion(inner.id, from, target);
        }
        self.mods[self.cm].out.casts.insert(e.id, Coercion { from, to: target });
        target
    }

    // ---- try operator -------------------------------------------------------------------------

    /// If `t` is a `Result`-shaped union, its `(T, E)` types.
    pub fn result_parts(&mut self, t: Ty) -> Option<(Ty, Ty)> {
        let (u, args, _mask) = self.union_view(t)?;
        let sig = self.unions.get(&u)?.clone();
        if sig.variants.len() != 2 || sig.discr_field != "kind" {
            return None;
        }
        if sig.variants[0].value != Discr::Str("ok".into()) || sig.variants[1].value != Discr::Str("err".into()) {
            return None;
        }
        let ok = self.records.get(&sig.variants[0].def)?.clone();
        let er = self.records.get(&sig.variants[1].def)?.clone();
        if ok.fields.len() != 1 || ok.fields[0].name != "value" || er.fields.len() != 1 || er.fields[0].name != "error" {
            return None;
        }
        let tp = self.defs[u.0 as usize].info.tparams.clone();
        let map: Vec<(ParamId, Ty)> = tp.into_iter().zip(args.iter().copied()).collect();
        let vt = self.tys.subst(ok.fields[0].ty, &map);
        let et = self.tys.subst(er.fields[0].ty, &map);
        Some((vt, et))
    }

    /// A default-parameter expression runs in the caller's frame: `?` has no function to return from.
    pub fn default_expr(&mut self, dv: &'a Expr, t: Ty) {
        let saved = self.default_depth.replace(self.fnctx.len());
        self.expr_to(dv, t);
        self.default_depth = saved;
    }

    fn try_expr(&mut self, e: &'a Expr, inner: &'a Expr) -> Ty {
        if self.default_depth == Some(self.fnctx.len()) {
            self.err("OXS-E0122", e.span, &[("detail", "`?` cannot be used in a default parameter value: it is evaluated at the call site, where there is no function to return from")]);
            self.expr(inner, None);
            return ERROR;
        }
        let t = self.expr(inner, None);
        if t == ERROR {
            return ERROR;
        }
        let Some((vt, et)) = self.result_parts(t) else {
            let s = self.show(t);
            self.err("OXS-E0100", inner.span, &[("expected", "Result<T, E>"), ("found", &s)]);
            return ERROR;
        };
        // the enclosing function must return a Result with the same error type
        let ret = self.fnctx.last().and_then(|c| c.ret);
        let ret_parts = match ret {
            Some(r) => self.result_parts(r),
            None => None,
        };
        match ret_parts {
            Some((_, e2)) if e2 == et || e2 == ERROR || et == ERROR => {}
            _ => {
                let s = self.show(et);
                self.err("OXS-E0104", e.span, &[("err", &s)]);
                return vt;
            }
        }
        if let Some(r) = ret {
            self.mods[self.cm].out.casts.insert(e.id, Coercion { from: t, to: r });
        }
        vt
    }

    // ---- index ----------------------------------------------------------------------------------

    fn index_expr(&mut self, e: &'a Expr, obj: &'a Expr, index: &'a Expr, optional: bool) -> Ty {
        let ot = self.expr(obj, None);
        let (base, nullable_chain) = match self.tys.kind(ot).clone() {
            TyKind::Nullable(i) if optional => (i, true),
            TyKind::Nullable(_) => {
                self.possibly_null(obj, ot);
                self.expr(index, Some(I32));
                return ERROR;
            }
            _ => (ot, false),
        };
        if optional && !nullable_chain && ot != ERROR {
            let s = self.show(ot);
            self.err("OXS-E0127", obj.span, &[("expr", self.span_text(obj.span)), ("ty", &s)]);
        }
        let it = self.expr(index, Some(I32));
        let elem = match self.tys.kind(base).clone() {
            TyKind::Array(x) => {
                if it != I32 && it != ERROR && it != NEVER {
                    self.mismatch_fix(index, I32, it);
                }
                x
            }
            TyKind::Error => return ERROR,
            TyKind::Str => {
                self.err("OXS-E0128", e.span, &[("op", "[]"), ("ty", "string")]);
                return ERROR;
            }
            _ => {
                let s = self.show(base);
                self.err("OXS-E0128", e.span, &[("op", "[]"), ("ty", &s)]);
                return ERROR;
            }
        };
        if nullable_chain {
            self.tys.nullable(elem)
        } else {
            elem
        }
    }

    pub fn possibly_null(&mut self, e: &'a Expr, t: Ty) {
        let _ = t;
        let txt = self.span_text(e.span).to_string();
        self.err("OXS-E0105", e.span, &[("expr", &txt)]);
    }

    // ---- closures --------------------------------------------------------------------------------

    fn arrow(&mut self, e: &'a Expr, a: &'a ArrowFn, exp: Option<Ty>) -> Ty {
        let exp_fn = exp.map(|t| self.non_null(t)).and_then(|t| match self.tys.kind(t).clone() {
            TyKind::Fn(p, r) => Some((p.to_vec(), r)),
            _ => None,
        });
        if let Some((ps, _)) = &exp_fn {
            if ps.len() != a.params.len() {
                // fewer parameters are allowed (callbacks may ignore arguments), more are not
                if a.params.len() > ps.len() {
                    self.err(
                        "OXS-E0107",
                        e.span,
                        &[("expected", &ps.len().to_string()), ("found", &a.params.len().to_string())],
                    );
                }
            }
        }
        let mut ptys: Vec<Ty> = Vec::new();
        for (i, p) in a.params.iter().enumerate() {
            let t = match (&p.ty, &exp_fn) {
                (Some(te), _) => {
                    let t = self.lower_type(te);
                    if let Some((ps, _)) = &exp_fn {
                        if let Some(x) = ps.get(i) {
                            if !self.sub_norepr(*x, t) && *x != ERROR && t != ERROR {
                                self.mismatch(p.span, *x, t);
                            }
                        }
                    }
                    t
                }
                (None, Some((ps, _))) if i < ps.len() && ps[i] != ERROR => ps[i],
                _ => {
                    self.err("OXS-E0103", p.span, &[("what", &format!("parameter `{}`", p.name.name))]);
                    ERROR
                }
            };
            if let Some(dv) = &p.default {
                if t != ERROR {
                    self.default_expr(dv, t);
                }
            }
            ptys.push(t);
        }
        let declared_ret = a.ret.as_ref().map(|r| self.lower_type(r));
        let exp_ret = match (&declared_ret, &exp_fn) {
            (Some(r), _) => Some(*r),
            (None, Some((_, r))) if *r != ERROR => Some(*r),
            _ => None,
        };
        // enter the closure
        let outer_flow = self.flow.clone();
        let (assigned, closure_assigned, tparams, is_law) = {
            let c = self.fnctx.last();
            (
                c.map(|c| c.assigned.clone()).unwrap_or_default(),
                c.map(|c| c.closure_assigned.clone()).unwrap_or_default(),
                c.map(|c| c.tparams.clone()).unwrap_or_default(),
                c.is_some_and(|c| c.is_law),
            )
        };
        let ctx = self.new_fnctx(true, exp_ret, "<closure>".to_string(), assigned, closure_assigned, tparams);
        let mut ctx = ctx;
        ctx.closure_recv = self.fnctx.last().map(|c| c.closure_recv.clone()).unwrap_or_default();
        ctx.is_law = is_law;
        ctx.def = self.fnctx.last().and_then(|c| c.def);
        self.fnctx.push(ctx);
        // facts about names that are never assigned stay valid inside the closure
        let keep: Vec<(u32, Ty)> = {
            let mut v = Vec::new();
            for (p, t) in &self.flow.facts {
                let root = self.paths.nodes[*p as usize].root;
                let name = &self.mods[self.cm].out.locals[root as usize].name;
                if !self.fnctx.last().is_some_and(|c| c.assigned.contains(name)) {
                    v.push((*p, *t));
                }
            }
            v
        };
        self.flow.facts = keep;
        self.push_scope();
        for (p, t) in a.params.iter().zip(&ptys) {
            let l = self.new_local(&p.name.name, *t, LocalKind::Param, p.name.span, p.name.id);
            self.bind_scope(&p.name.name, Binding::Local(l), p.name.span);
        }
        let body_ret = match &a.body {
            ArrowBody::Expr(x) => {
                let hint = exp_ret.filter(|r| *r != VOID);
                let t = self.expr(x, hint);
                match exp_ret {
                    Some(r) if r == VOID => {
                        // the value of a `() => expr` used as a void callback is dropped: make it visible
                        if t != VOID && t != ERROR {
                            self.record_coercion(x.id, t, VOID);
                        }
                        VOID
                    }
                    Some(r) => {
                        self.coerce_to(x, t, r);
                        r
                    }
                    None => {
                        let w = self.widen(t);
                        self.record_coercion(x.id, t, w);
                        w
                    }
                }
            }
            ArrowBody::Block(b) => {
                self.block(b);
                let c = self.fnctx.last().map(|c| (c.ret, c.inferred, c.has_ret_value)).unwrap_or((None, None, false));
                let r = match c {
                    (Some(r), _, _) => r,
                    (None, Some(i), _) => i,
                    (None, None, _) => VOID,
                };
                if r != VOID && r != ERROR && !self.flow.dead {
                    self.err("OXS-E0114", e.span, &[("name", "<closure>"), ("ty", &self.show(r))]);
                }
                r
            }
        };
        self.pop_scope();
        let ctx = self.fnctx.pop();
        self.flow = outer_flow;
        if let Some(c) = ctx {
            let mut caps = Vec::new();
            for l in &c.captures {
                caps.push(Capture { local: *l, mutated: false });
                // propagate to an enclosing closure (already recorded by use_local for every level)
            }
            self.mods[self.cm].out.closures.insert(e.id, ClosureInfo { captures: caps });
        }
        let ft = self.tys.func(&ptys, body_ret);
        // a block-bodied closure with an expected return of `void` may not return a value
        ft
    }
}

impl Discr {
    pub fn display(&self) -> String {
        match self {
            Discr::Str(s) => format!("\"{s}\""),
            Discr::Int(n) => n.to_string(),
            Discr::Bool(b) => b.to_string(),
        }
    }
}
