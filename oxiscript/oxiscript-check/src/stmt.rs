//! Statements, patterns, switch, function bodies and module-level checking.

use crate::exhaust::{self, ColSig, Pat};
use crate::expr::strip_paren;
use crate::flow::{full_mask, Flow};
use crate::ty::*;
use crate::*;

struct AssignedNames {
    all: HashSet<String>,
    in_closure: HashSet<String>,
    /// receivers of method calls (a `mutating` method can change fields, never the variable itself)
    recv: HashSet<String>,
    recv_in_closure: HashSet<String>,
    closure_depth: u32,
    depth: u32,
}

impl AssignedNames {
    fn new() -> AssignedNames {
        AssignedNames { all: HashSet::new(), in_closure: HashSet::new(), recv: HashSet::new(), recv_in_closure: HashSet::new(), closure_depth: 0, depth: 0 }
    }
}

fn root_name(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Ident(i) => Some(i.name.clone()),
        ExprKind::This => Some("this".into()),
        ExprKind::Member { obj, .. } | ExprKind::Index { obj, .. } => root_name(obj),
        ExprKind::Paren(i) => root_name(i),
        _ => None,
    }
}

/// Expression depth beyond which the prepass stops walking and conservatively treats every
/// name as assigned (marker `*`), so a very long chain cannot exhaust the native stack.
const PREPASS_DEPTH: u32 = 120;

impl<'a> Visitor<'a> for AssignedNames {
    fn visit_expr(&mut self, e: &'a Expr) {
        if self.depth > PREPASS_DEPTH {
            self.all.insert("*".into());
            self.in_closure.insert("*".into());
            self.recv.insert("*".into());
            self.recv_in_closure.insert("*".into());
            return;
        }
        self.depth += 1;
        self.visit_expr_inner(e);
        self.depth -= 1;
    }
}

impl AssignedNames {
    fn visit_expr_inner(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Assign { target, .. } | ExprKind::Update { target, .. } => {
                if let Some(n) = root_name(target) {
                    if self.closure_depth > 0 {
                        self.in_closure.insert(n.clone());
                    }
                    self.all.insert(n);
                }
            }
            ExprKind::Call { callee, .. } => {
                // a call on a place may run a `mutating` method: count the receiver as assigned
                if let ExprKind::Member { obj, .. } = &strip_paren(callee).kind {
                    if let Some(n) = root_name(obj) {
                        if self.closure_depth > 0 {
                            self.recv_in_closure.insert(n.clone());
                        }
                        self.recv.insert(n);
                    }
                }
            }
            _ => {}
        }
        if let ExprKind::Arrow(_) = &e.kind {
            self.closure_depth += 1;
            walk_expr(self, e);
            self.closure_depth -= 1;
        } else {
            walk_expr(self, e);
        }
    }
}

pub(crate) struct SwitchScrut {
    pub kind: SwitchKind,
    pub ty: Ty,
    pub path: Option<u32>,
    pub union: Option<(DefId, Box<[Ty]>, u64)>,
    pub enum_def: Option<DefId>,
    pub inner: Option<Ty>,
}

pub(crate) struct SwitchPlan {
    pub labels: Vec<Vec<CaseLabel>>,
    pub rows: Vec<Pat>,
    pub narrow: Vec<Option<(u32, Ty)>>,
    pub has_default: bool,
}

impl<'a> Checker<'a> {
    pub(crate) fn new_fnctx(
        &self,
        is_closure: bool,
        ret: Option<Ty>,
        name: String,
        assigned: Rc<HashSet<String>>,
        closure_assigned: Rc<HashSet<String>>,
        tparams: Vec<ParamId>,
    ) -> FnCtx {
        FnCtx {
            ret,
            inferred: None,
            is_closure,
            is_ctor: false,
            this: None,
            this_is_value: false,
            mutating: false,
            owner: None,
            loops: Vec::new(),
            captures: Vec::new(),
            name,
            name_span: None,
            is_law: false,
            tparams,
            assigned,
            closure_assigned,
            closure_recv: Rc::default(),
            has_ret_value: false,
            exported: false,
            def: None,
        }
    }

    pub(crate) fn check_receiver_mutable(&mut self, t: &'a Expr) -> Option<()> {
        self.check_place_mutable_pub(t)
    }

    // ---- blocks ---------------------------------------------------------------------

    pub fn block(&mut self, b: &'a Block) {
        self.push_scope();
        self.block_in_scope(b);
        self.pop_scope();
    }

    /// Statements of `b` in the current scope; nested items are hoisted first.
    pub fn block_in_scope(&mut self, b: &'a Block) {
        self.hoist(&b.stmts);
        for s in &b.stmts {
            self.stmt(s);
        }
        if let Some(t) = &b.tail {
            // only `law` bodies have a tail; checked by the caller, other blocks never get here
            self.expr(t, None);
        }
    }

    fn hoist(&mut self, stmts: &'a [Stmt]) {
        let mut local_defs = Vec::new();
        for s in stmts {
            if let StmtKind::Item(item) = &s.kind {
                if let Some(d) = self.declare_item(item) {
                    local_defs.push(d);
                }
            }
        }
        for d in local_defs {
            self.ensure_def(d);
        }
    }

    // ---- statements -------------------------------------------------------------------

    pub fn stmt(&mut self, s: &'a Stmt) {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            self.depth -= 1;
            let snip = self.clip_snippet(s.span);
            self.err(
                "OXS-E0113",
                s.span,
                &[("detail", &format!("statement is nested too deeply at `{snip}`"))],
            );
            return;
        }
        self.check_attrs(&s.attrs, "stmt", s.id);
        self.stmt_inner(s);
        self.depth -= 1;
    }

    fn stmt_body(&mut self, s: &'a Stmt) {
        // a single-statement body gets its own scope
        self.push_scope();
        self.stmt(s);
        self.pop_scope();
    }

    fn stmt_inner(&mut self, s: &'a Stmt) {
        match &s.kind {
            StmtKind::Empty => {}
            StmtKind::Var(v) => self.var_decl(v, false),
            StmtKind::Expr(e) => {
                let t = self.expr(e, None);
                let _ = t;
                let effect = matches!(
                    strip_paren(e).kind,
                    ExprKind::Assign { .. } | ExprKind::Update { .. } | ExprKind::Call { .. } | ExprKind::New { .. } | ExprKind::Try(_) | ExprKind::Switch(_)
                );
                if !effect {
                    self.err("OXS-W0004", e.span, &[]);
                }
                if t == NEVER {
                    self.flow.dead = true;
                }
            }
            StmtKind::Block(b) => self.block(b),
            StmtKind::If { .. } => self.if_chain(s),
            StmtKind::While { cond, body } => {
                let label = self.pending_label.take();
                self.invalidate_assigned_in(s);
                let entry = self.flow.clone();
                let (ft, mut ff) = self.cond_bool(cond);
                if matches!(&strip_paren(cond).kind, ExprKind::Lit(l) if matches!(l.kind, LitKind::Bool(true))) {
                    ff.dead = true;
                }
                self.push_loop(label, true);
                self.flow = ft;
                self.stmt_body(body);
                let lc = self.pop_loop();
                let _ = entry;
                let mut fs = vec![ff];
                fs.extend(lc.breaks);
                self.flow = self.join_all(fs);
            }
            StmtKind::DoWhile { body, cond } => {
                let label = self.pending_label.take();
                self.invalidate_assigned_in(s);
                self.push_loop(label, true);
                self.stmt_body(body);
                let lc = self.pop_loop();
                if self.flow.dead {
                    // the condition is reached through `continue` only; check it on a live copy
                    self.flow.dead = false;
                }
                let (_ft, ff) = self.cond_bool(cond);
                let mut fs = vec![ff];
                fs.extend(lc.breaks);
                self.flow = self.join_all(fs);
            }
            StmtKind::For { init, cond, update, body, .. } => {
                let label = self.pending_label.take();
                self.push_scope();
                if let Some(i) = init {
                    self.stmt(i);
                }
                self.invalidate_assigned_in(s);
                let (ft, ff) = if let Some(c) = cond { self.cond_bool(c) } else {
                    let mut f = self.flow.clone();
                    f.dead = true;
                    (self.flow.clone(), f)
                };
                self.push_loop(label, true);
                self.flow = ft.clone();
                self.stmt_body(body);
                if let Some(u) = update {
                    let saved = self.flow.clone();
                    self.flow = ft;
                    self.expr(u, None);
                    self.flow = saved;
                }
                let lc = self.pop_loop();
                let mut fs = vec![ff];
                fs.extend(lc.breaks);
                self.flow = self.join_all(fs);
                self.pop_scope();
            }
            StmtKind::ForOf { kind, binding, iter, body, .. } => {
                let label = self.pending_label.take();
                let it = self.expr(iter, None);
                let elem = match self.tys.kind(it).clone() {
                    TyKind::Array(e) => e,
                    TyKind::Error => ERROR,
                    _ => {
                        let s2 = self.show(it);
                        self.err("OXS-E0128", iter.span, &[("op", "of"), ("ty", &s2)]);
                        ERROR
                    }
                };
                self.invalidate_assigned_in(s);
                let entry = self.flow.clone();
                self.push_scope();
                let lk = if *kind == VarKind::Const { LocalKind::ForOf } else { LocalKind::Let };
                self.bind_pattern(binding, elem, lk);
                self.push_loop(label, true);
                self.stmt_body(body);
                let lc = self.pop_loop();
                self.pop_scope();
                let mut fs = vec![entry];
                fs.extend(lc.breaks);
                self.flow = self.join_all(fs);
            }
            StmtKind::Switch(sw) => self.switch_stmt(s, sw),
            StmtKind::Return(v) => self.return_stmt(s, v.as_ref()),
            StmtKind::Break(l) => self.jump(s, l.as_ref(), true),
            StmtKind::Continue(l) => self.jump(s, l.as_ref(), false),
            StmtKind::Labeled { label, body } => {
                let is_loop = matches!(body.kind, StmtKind::While { .. } | StmtKind::DoWhile { .. } | StmtKind::For { .. } | StmtKind::ForOf { .. });
                if is_loop {
                    self.pending_label = Some(label.name.clone());
                    self.stmt(body);
                    self.pending_label = None;
                } else {
                    self.push_loop(Some(label.name.clone()), false);
                    self.stmt(body);
                    let lc = self.pop_loop();
                    let mut fs = vec![self.flow.clone()];
                    fs.extend(lc.breaks);
                    self.flow = self.join_all(fs);
                }
            }
            StmtKind::Item(item) => {
                let Some(&d) = self.mods[self.cm].out.node_def.get(&item.id) else { return };
                match self.kind_of(d) {
                    DefKind::Fn => self.check_fn_body(d),
                    DefKind::Struct | DefKind::Class => self.check_record_bodies(d),
                    DefKind::Global => {}
                    _ => {}
                }
            }
        }
    }

    fn push_loop(&mut self, label: Option<String>, is_loop: bool) {
        if let Some(c) = self.fnctx.last_mut() {
            c.loops.push(LoopCtx { label, is_switch: false, is_loop, breaks: Vec::new(), brk: false });
        }
    }

    fn pop_loop(&mut self) -> LoopCtx {
        self.fnctx
            .last_mut()
            .and_then(|c| c.loops.pop())
            .unwrap_or(LoopCtx { label: None, is_switch: false, is_loop: true, breaks: vec![], brk: false })
    }

    fn jump(&mut self, s: &'a Stmt, label: Option<&'a Ident>, is_break: bool) {
        let kw = if is_break { "break" } else { "continue" };
        let flow = self.flow.clone();
        let Some(ctx) = self.fnctx.last_mut() else { return };
        let target = match label {
            Some(l) => ctx.loops.iter().rposition(|c| c.label.as_deref() == Some(l.name.as_str())),
            None => ctx.loops.iter().rposition(|c| if is_break { c.is_loop || c.is_switch } else { c.is_loop }),
        };
        if let Some(i) = target {
            if !is_break && !ctx.loops[i].is_loop {
                self.err("OXS-E0122", s.span, &[("detail", &format!("`continue` target `{}` is not a loop", label.map_or("", |l| l.name.as_str())))]);
            } else if is_break {
                ctx.loops[i].breaks.push(flow);
                ctx.loops[i].brk = true;
            }
        } else {
            let d = match label {
                Some(l) => format!("Jump target `{}` cannot be found", l.name),
                None => format!("A `{kw}` statement can only be used within a loop{}", if is_break { " or switch" } else { "" }),
            };
            self.err("OXS-E0122", s.span, &[("detail", &d)]);
        }
        self.flow.dead = true;
    }

    fn return_stmt(&mut self, s: &'a Stmt, v: Option<&'a Expr>) {
        let Some(ctx) = self.fnctx.last() else { return };
        let ret = ctx.ret;
        let inferred = ctx.inferred;
        match (ret, v) {
            (Some(r), Some(x)) => {
                if r == VOID {
                    let t = self.expr(x, None);
                    if t != ERROR {
                        self.mismatch_fix(x, VOID, t);
                    }
                } else {
                    self.expr_to(x, r);
                }
            }
            (Some(r), None) => {
                if r != VOID && r != ERROR && r != NEVER {
                    self.mismatch(s.span, r, VOID);
                }
            }
            (None, Some(x)) => {
                let raw_t = self.expr(x, None);
                let t = self.widen_expr(x, raw_t);
                match inferred {
                    None => {
                        if t != raw_t {
                            self.record_coercion(x.id, raw_t, t);
                        }
                        if let Some(c) = self.fnctx.last_mut() {
                            c.inferred = Some(t);
                        }
                    }
                    Some(i) => {
                        if self.assignable(t, i) {
                            self.record_coercion(x.id, raw_t, i);
                        } else if self.assignable(i, t) {
                            if t != raw_t {
                                self.record_coercion(x.id, raw_t, t);
                            }
                            if let Some(c) = self.fnctx.last_mut() {
                                c.inferred = Some(t);
                            }
                        } else if t != ERROR && i != ERROR {
                            self.mismatch_fix(x, i, t);
                        }
                    }
                }
                if let Some(c) = self.fnctx.last_mut() {
                    c.has_ret_value = true;
                }
            }
            (None, None) => {
                if let Some(i) = inferred {
                    if i != VOID && i != ERROR {
                        self.mismatch(s.span, i, VOID);
                    }
                } else if let Some(c) = self.fnctx.last_mut() {
                    c.inferred = Some(VOID);
                }
            }
        }
        self.flow.dead = true;
    }

    fn if_chain(&mut self, s: &'a Stmt) {
        let mut ends: Vec<Flow> = Vec::new();
        let mut cur = s;
        loop {
            let StmtKind::If { cond, then, els } = &cur.kind else { break };
            let (ft, ff) = self.cond_bool(cond);
            self.flow = ft;
            self.stmt_body(then);
            ends.push(self.flow.clone());
            self.flow = ff;
            match els {
                Some(n) if matches!(n.kind, StmtKind::If { .. }) && n.attrs.is_empty() => {
                    cur = n;
                }
                Some(n) => {
                    self.stmt_body(n);
                    ends.push(self.flow.clone());
                    break;
                }
                None => {
                    ends.push(self.flow.clone());
                    break;
                }
            }
        }
        self.flow = self.join_all(ends);
    }

    /// Forget narrowing of variables assigned anywhere inside `s` (loop heads).
    fn invalidate_assigned_in(&mut self, s: &'a Stmt) {
        let mut v = AssignedNames::new();
        v.visit_stmt(s);
        if v.all.is_empty() && v.recv.is_empty() {
            return;
        }
        let locals = &self.mods[self.cm].out.locals;
        let paths = &self.paths;
        let all = v.all.contains("*");
        self.flow.facts.retain(|(p, _)| {
            let n = paths.nodes[*p as usize];
            let name = &locals[n.root as usize].name;
            !(all || v.all.contains(name) || (n.depth > 0 && v.recv.contains(name)))
        });
    }

    // ---- declarations ---------------------------------------------------------------------

    fn var_decl(&mut self, v: &'a VarDecl, _global: bool) {
        let decl = v.ty.as_ref().map(|t| self.lower_type(t));
        let lk = if v.kind == VarKind::Const { LocalKind::Const } else { LocalKind::Let };
        match &v.pattern.kind {
            PatternKind::Ident(id) => {
                let (ty, init_ty) = match (&v.init, decl) {
                    (Some(x), Some(d)) => {
                        let t = self.expr_to(x, d);
                        (d, Some(t))
                    }
                    (Some(x), None) => {
                        let t = self.expr(x, None);
                        let t2 = if lk == LocalKind::Let { self.widen(t) } else { t };
                        if t == NULL || t == VOID || t == NEVER {
                            let what = format!("variable `{}`", id.name);
                            self.err("OXS-E0103", id.span, &[("what", &what)]);
                            (ERROR, None)
                        } else {
                            if t2 != t {
                                self.record_coercion(x.id, t, t2);
                            }
                            (t2, Some(t))
                        }
                    }
                    (None, Some(d)) => {
                        if v.kind == VarKind::Const {
                            self.err("OXS-E0123", id.span, &[("detail", &format!("'const' declarations must be initialized: `{}`", id.name))]);
                        }
                        (d, None)
                    }
                    (None, None) => {
                        let what = format!("variable `{}`", id.name);
                        self.err("OXS-E0103", id.span, &[("what", &what)]);
                        (ERROR, None)
                    }
                };
                let l = self.new_local(&id.name, ty, lk, id.span, id.id);
                self.bind_scope(&id.name, Binding::Local(l), id.span);
                if v.init.is_none() {
                    self.flow.mark_unassigned(l.0);
                } else if let Some(it) = init_ty {
                    // the initialiser may be narrower than the declared type
                    if decl.is_some() && it != ty && it != ERROR && it != NEVER && self.narrowable(l) {
                        let p = self.paths.root(l.0);
                        self.flow.set(p, it);
                    }
                }
            }
            PatternKind::Wildcard => {
                if let Some(x) = &v.init {
                    match decl {
                        Some(d) => {
                            self.expr_to(x, d);
                        }
                        None => {
                            self.expr(x, None);
                        }
                    }
                }
            }
            PatternKind::Object(_) => {
                let Some(x) = &v.init else {
                    self.err("OXS-E0123", v.pattern.span, &[("detail", "a destructuring declaration must have an initializer")]);
                    return;
                };
                let t = match decl {
                    Some(d) => {
                        self.expr_to(x, d);
                        d
                    }
                    None => self.expr(x, None),
                };
                self.bind_pattern(&v.pattern, t, lk);
            }
        }
    }

    /// Bind the names of `p` (matched against a value of type `ty`) in the current scope.
    fn bind_pattern(&mut self, p: &'a Pattern, ty: Ty, lk: LocalKind) {
        match &p.kind {
            PatternKind::Wildcard => {}
            PatternKind::Ident(id) => {
                let l = self.new_local(&id.name, ty, lk, id.span, id.id);
                self.bind_scope(&id.name, Binding::Local(l), id.span);
            }
            PatternKind::Object(fields) => {
                let base = match self.tys.kind(ty).clone() {
                    TyKind::Nullable(_) | TyKind::Null => {
                        self.err("OXS-E0105", p.span, &[("expr", self.span_text(p.span))]);
                        ERROR
                    }
                    _ => ty,
                };
                let (def, args) = match self.tys.kind(base).clone() {
                    TyKind::Adt(d, a) if matches!(self.kind_of(d), DefKind::Struct | DefKind::Class | DefKind::Variant { .. }) => (Some(d), a.to_vec()),
                    TyKind::Error => (None, vec![]),
                    _ => {
                        if base != ERROR {
                            let s = self.show(base);
                            self.err("OXS-E0115", p.span, &[("detail", &format!("cannot destructure a value of type '{s}'; only structs and classes can be destructured"))]);
                        }
                        (None, vec![])
                    }
                };
                for f in fields {
                    let fty = match def {
                        Some(d) => {
                            self.ensure_record(d);
                            let tps = self.defs[d.0 as usize].info.tparams.clone();
                            let map: Vec<(ParamId, Ty)> = tps.into_iter().zip(args.iter().copied()).collect();
                            if let Some(fs) = self.records.get(&d).and_then(|r| r.fields.iter().find(|x| !x.is_static && x.name == f.key.name).cloned()) {
                                if fs.private && !self.fnctx.iter().any(|c| c.owner == Some(d)) {
                                    self.err("OXS-E0119", f.key.span, &[("detail", &format!("Property '{}' is private", f.key.name))]);
                                }
                                self.tys.subst(fs.ty, &map)
                            } else {
                                let s = self.show(base);
                                self.err("OXS-E0108", f.key.span, &[("name", &f.key.name), ("ty", &s)]);
                                ERROR
                            }
                        }
                        None => ERROR,
                    };
                    if let Some(sub) = &f.pattern { self.bind_pattern(sub, fty, lk) } else {
                        let l = self.new_local(&f.key.name, fty, lk, f.key.span, f.id);
                        self.bind_scope(&f.key.name, Binding::Local(l), f.key.span);
                    }
                }
            }
        }
    }

    // ---- switch ------------------------------------------------------------------------------

    pub(crate) fn switch_scrutinee(&mut self, e: &'a Expr) -> SwitchScrut {
        let t = self.expr(e, None);
        let inner = strip_paren(e);
        if let (ExprKind::Member { obj, .. }, Some(Res::Discriminant { union })) = (&inner.kind, self.mods[self.cm].out.res.get(&inner.id).copied()) {
            let ot = self.mods[self.cm].out.expr_ty.get(obj.id.0 as usize).copied().unwrap_or(ERROR);
            if let Some(view) = self.union_view(ot) {
                let path = self.path_of(strip_paren(obj));
                let _ = union;
                return SwitchScrut { kind: SwitchKind::Union, ty: t, path, union: Some(view), enum_def: None, inner: None };
            }
        }
        let path = self.path_of(inner);
        match self.tys.kind(t).clone() {
            TyKind::Adt(d, _) if self.kind_of(d) == DefKind::Enum => SwitchScrut { kind: SwitchKind::Enum, ty: t, path: None, union: None, enum_def: Some(d), inner: None },
            TyKind::Nullable(i) => SwitchScrut { kind: SwitchKind::Nullable, ty: t, path, union: None, enum_def: None, inner: Some(i) },
            TyKind::Bool => SwitchScrut { kind: SwitchKind::Bool, ty: t, path: None, union: None, enum_def: None, inner: None },
            TyKind::Num(n) if is_int(n) => SwitchScrut { kind: SwitchKind::Int, ty: t, path: None, union: None, enum_def: None, inner: None },
            TyKind::Str => SwitchScrut { kind: SwitchKind::Str, ty: t, path: None, union: None, enum_def: None, inner: None },
            TyKind::Error | TyKind::Never => SwitchScrut { kind: SwitchKind::Int, ty: ERROR, path: None, union: None, enum_def: None, inner: None },
            _ => {
                let s = self.show(t);
                self.err(
                    "OXS-E0123",
                    e.span,
                    &[("detail", &format!("cannot switch on a value of type '{s}'; switch on an integer, string, bool, enum, `x.kind` of a union, or a nullable"))],
                );
                SwitchScrut { kind: SwitchKind::Int, ty: ERROR, path: None, union: None, enum_def: None, inner: None }
            }
        }
    }

    fn lit_key(l: &Expr) -> Option<String> {
        match &strip_paren(l).kind {
            ExprKind::Unary { op: UnOp::Neg, expr } => match &strip_paren(expr).kind {
                ExprKind::Lit(x) => match &x.kind {
                    LitKind::Int { value, .. } => Some(format!("i:-{value}")),
                    LitKind::Float { .. } => Some("f".into()),
                    _ => None,
                },
                _ => None,
            },
            ExprKind::Lit(x) => match &x.kind {
                LitKind::Str(s) => Some(format!("s:{s}")),
                LitKind::Int { value, .. } => Some(format!("i:{value}")),
                LitKind::Bool(b) => Some(format!("b:{b}")),
                LitKind::Null => Some("null".into()),
                LitKind::Float { .. } => None,
            },
            _ => None,
        }
    }

    pub(crate) fn switch_plan(&mut self, sc: &SwitchScrut, clauses: &[(&'a [Expr], bool)]) -> SwitchPlan {
        let mut labels: Vec<Vec<CaseLabel>> = Vec::new();
        let mut rows: Vec<Pat> = Vec::new();
        let mut seen: HashSet<u32> = HashSet::new();
        let mut seen_keys: HashSet<String> = HashSet::new();
        let mut has_default = false;
        for (exprs, is_default) in clauses {
            let mut ls = Vec::new();
            for l in *exprs {
                match sc.kind {
                    SwitchKind::Union => {
                        let Some((u, uargs, mask)) = &sc.union else { continue };
                        let dty = self.unions.get(u).map_or(ERROR, |s| s.discr_ty);
                        let t = self.expr(l, Some(dty));
                        let want = match &strip_paren(l).kind {
                            ExprKind::Lit(x) => match &x.kind {
                                LitKind::Str(s) => Some(Discr::Str(s.clone())),
                                LitKind::Int { value, .. } => i128::try_from(*value).ok().map(Discr::Int),
                                LitKind::Bool(b) => Some(Discr::Bool(*b)),
                                _ => None,
                            },
                            _ => None,
                        };
                        let Some(w) = want else {
                            if t != ERROR {
                                self.err("OXS-E0116", l.span, &[("detail", "a case label for a union switch must be a literal discriminant value")]);
                            }
                            continue;
                        };
                        let sig = self.unions.get(u).cloned();
                        if let Some(i) = sig.and_then(|s| s.variants.iter().position(|v| v.value == w)) {
                            if !seen.insert(i as u32) {
                                self.err("OXS-E0116", l.span, &[("detail", &format!("Duplicate case label {}", w.display()))]);
                            } else if (*mask & (1u64 << i)) == 0 {
                                self.err("OXS-W0005", l.span, &[("label", &w.display())]);
                            }
                            ls.push(CaseLabel::Variant(i as u32));
                            rows.push(Pat::Ctor(i as u32, vec![]));
                        } else {
                            let s = self.show(sc.ty);
                            let uty = self.tys.adt(*u, uargs);
                            let us = self.show(uty);
                            self.err(
                                "OXS-E0116",
                                l.span,
                                &[("detail", &format!("Type '{}' is not comparable to type '{s}': no variant of '{us}' has this discriminant", w.display()))],
                            );
                        }
                    }
                    SwitchKind::Enum => {
                        let Some(ed) = sc.enum_def else { continue };
                        let t = self.expr(l, Some(sc.ty));
                        if t != sc.ty && t != ERROR {
                            self.mismatch_fix(l, sc.ty, t);
                            continue;
                        }
                        match self.mods[self.cm].out.res.get(&strip_paren(l).id).copied() {
                            Some(Res::EnumMember { def, index }) if def == ed => {
                                if !seen.insert(index) {
                                    self.err("OXS-E0116", l.span, &[("detail", "Duplicate case label")]);
                                }
                                ls.push(CaseLabel::Member(index));
                                rows.push(Pat::Ctor(index, vec![]));
                            }
                            _ => {
                                if t != ERROR {
                                    self.err("OXS-E0116", l.span, &[("detail", "a case label for an enum switch must be an enum member such as `Color.Red`")]);
                                }
                            }
                        }
                    }
                    SwitchKind::Bool => {
                        let t = self.expr(l, Some(BOOL));
                        self.coerce_to(l, t, BOOL);
                        if let ExprKind::Lit(x) = &strip_paren(l).kind {
                            if let LitKind::Bool(b) = x.kind {
                                if !seen_keys.insert(format!("b:{b}")) {
                                    self.err("OXS-E0116", l.span, &[("detail", "Duplicate case label")]);
                                }
                                rows.push(Pat::Ctor(u32::from(b), vec![]));
                                ls.push(CaseLabel::Value);
                                continue;
                            }
                        }
                        self.err("OXS-E0116", l.span, &[("detail", "a case label for a bool switch must be `true` or `false`")]);
                    }
                    SwitchKind::Nullable => {
                        let inner = sc.inner.unwrap_or(ERROR);
                        if matches!(&strip_paren(l).kind, ExprKind::Lit(x) if matches!(x.kind, LitKind::Null)) {
                            self.expr(l, None);
                            rows.push(Pat::Ctor(0, vec![]));
                            ls.push(CaseLabel::Null);
                        } else {
                            let t = self.expr(l, Some(inner));
                            self.coerce_to(l, t, inner);
                            if let Some(k) = Self::lit_key(l) {
                                if !seen_keys.insert(k) {
                                    self.err("OXS-E0116", l.span, &[("detail", "Duplicate case label")]);
                                }
                            }
                            ls.push(CaseLabel::Value);
                        }
                    }
                    SwitchKind::Int | SwitchKind::Str => {
                        if sc.ty == ERROR {
                            self.expr(l, None);
                            continue;
                        }
                        let t = self.expr(l, Some(sc.ty));
                        self.coerce_to(l, t, sc.ty);
                        if let Some(k) = Self::lit_key(l) {
                            if !seen_keys.insert(k) {
                                self.err("OXS-E0116", l.span, &[("detail", "Duplicate case label")]);
                            }
                        }
                        ls.push(CaseLabel::Value);
                    }
                }
            }
            if *is_default {
                if has_default {
                    self.err("OXS-E0116", exprs.first().map_or(Span::DUMMY, |e| e.span), &[("detail", "A switch can have only one `default` clause")]);
                }
                has_default = true;
                ls.push(CaseLabel::Default);
                rows.push(Pat::Wild);
            }
            labels.push(ls);
        }
        // narrowing per clause
        let mut narrow = Vec::new();
        let explicit_union: u64 = labels
            .iter()
            .flatten()
            .filter_map(|l| if let CaseLabel::Variant(i) = l { Some(1u64 << i) } else { None })
            .fold(0, |a, b| a | b);
        let any_null = labels.iter().flatten().any(|l| *l == CaseLabel::Null);
        for ls in &labels {
            let n = match (sc.kind, sc.path) {
                (SwitchKind::Union, Some(p)) => {
                    let Some((u, uargs, mask)) = &sc.union else { narrow.push(None); continue };
                    let mut m: u64 = ls
                        .iter()
                        .filter_map(|l| if let CaseLabel::Variant(i) = l { Some(1u64 << i) } else { None })
                        .fold(0, |a, b| a | b);
                    if ls.contains(&CaseLabel::Default) {
                        m |= mask & !explicit_union;
                    }
                    m &= *mask;
                    let t = self.mk_variants(*u, uargs, m);
                    Some((p, t))
                }
                (SwitchKind::Nullable, Some(p)) => {
                    let inner = sc.inner.unwrap_or(ERROR);
                    let has_null = ls.contains(&CaseLabel::Null);
                    let has_other = ls.iter().any(|l| matches!(l, CaseLabel::Value)) || (ls.contains(&CaseLabel::Default) && any_null);
                    if has_null && !has_other && !ls.contains(&CaseLabel::Default) {
                        Some((p, NULL))
                    } else if !has_null && has_other {
                        Some((p, inner))
                    } else {
                        None
                    }
                }
                _ => None,
            };
            narrow.push(n);
        }
        SwitchPlan { labels, rows, narrow, has_default }
    }

    pub(crate) fn switch_exhaustive(&mut self, sc: &SwitchScrut, plan: &SwitchPlan, span: Span, required: bool, stmt_close: Option<Span>) -> bool {
        let (sig, names): (ColSig, Vec<String>) = match sc.kind {
            SwitchKind::Union => {
                let Some((u, _, mask)) = &sc.union else { return true };
                let sg = self.unions.get(u).cloned();
                let n = sg.as_ref().map_or(0, |s| s.variants.len());
                let names: Vec<String> = sg.map(|s| s.variants.iter().map(|v| v.value.display()).collect()).unwrap_or_default();
                // variants that cannot occur are covered
                let mut rows = plan.rows.clone();
                for i in 0..n {
                    if mask & (1u64 << i) == 0 {
                        rows.push(Pat::Ctor(i as u32, vec![]));
                    }
                }
                return self.finish_exhaust(&rows, ColSig::Finite(n as u32), &names, sc, span, required, stmt_close);
            }
            SwitchKind::Enum => {
                let Some(ed) = sc.enum_def else { return true };
                let names: Vec<String> = self.enums.get(&ed).map(|s| s.members.iter().map(|(n, _)| format!("{}.{n}", self.defs[ed.0 as usize].info.name)).collect()).unwrap_or_default();
                (ColSig::Finite(names.len() as u32), names)
            }
            SwitchKind::Bool => (ColSig::Finite(2), vec!["false".into(), "true".into()]),
            SwitchKind::Nullable => (ColSig::Finite(2), vec!["null".into(), "non-null values".into()]),
            SwitchKind::Int | SwitchKind::Str => (ColSig::Infinite, vec![]),
        };
        self.finish_exhaust(&plan.rows, sig, &names, sc, span, required, stmt_close)
    }

    fn finish_exhaust(&mut self, rows: &[Pat], sig: ColSig, names: &[String], sc: &SwitchScrut, span: Span, required: bool, stmt_close: Option<Span>) -> bool {
        let ok = exhaust::is_exhaustive(rows, sig);
        let must = required || matches!(sc.kind, SwitchKind::Union | SwitchKind::Enum | SwitchKind::Bool | SwitchKind::Nullable);
        if !ok && must && sc.ty != ERROR {
            let missing = match sig {
                ColSig::Finite(_) => exhaust::missing(rows, sig).into_iter().filter_map(|i| names.get(i as usize).cloned()).collect::<Vec<_>>(),
                ColSig::Infinite => vec!["`default`".to_string()],
            };
            let tys = self.show(sc.ty);
            let list = missing.join(", ");
            let fix = match (stmt_close, sc.kind) {
                (Some(close), SwitchKind::Union | SwitchKind::Enum | SwitchKind::Bool) => {
                    let text: String = missing.iter().map(|m| format!("case {m}: {{ }}\n")).collect();
                    Some(Fix::insert("Add the missing cases", close, text))
                }
                _ => None,
            };
            self.err_fix("OXS-E0101", span, &[("ty", &tys), ("missing", &list)], fix);
        }
        ok
    }

    pub(crate) fn record_switch(&mut self, id: NodeId, sc: &SwitchScrut, labels: Vec<Vec<CaseLabel>>, exhaustive: bool, has_default: bool) {
        let union = sc.union.as_ref().map(|(u, a, _)| self.tys.adt(*u, a));
        self.mods[self.cm].out.switches.insert(id, SwitchInfo { kind: sc.kind, union, labels, exhaustive, has_default });
    }

    fn switch_stmt(&mut self, s: &'a Stmt, sw: &'a SwitchStmt) {
        let sc = self.switch_scrutinee(&sw.scrutinee);
        let entry = self.flow.clone();
        let clauses: Vec<(&'a [Expr], bool)> = sw.cases.iter().map(|c| (c.labels.as_slice(), c.is_default)).collect();
        let plan = self.switch_plan(&sc, &clauses);
        let close = if s.span.hi > 0 && self.span_text(Span::new(s.span.file, s.span.hi - 1, s.span.hi)) == "}" {
            Some(Span::new(s.span.file, s.span.hi - 1, s.span.hi - 1))
        } else {
            None
        };
        let exhaustive = self.switch_exhaustive(&sc, &plan, sw.scrutinee.span, false, close);
        // the clause bodies
        if let Some(c) = self.fnctx.last_mut() {
            c.loops.push(LoopCtx { label: None, is_switch: true, is_loop: false, breaks: Vec::new(), brk: false });
        }
        let mut ends: Vec<Flow> = Vec::new();
        for (i, c) in sw.cases.iter().enumerate() {
            self.flow = entry.clone();
            if let Some(Some((p, t))) = plan.narrow.get(i) {
                self.flow.set(*p, *t);
            }
            self.push_scope();
            self.hoist(&c.body);
            for st in &c.body {
                self.stmt(st);
            }
            self.pop_scope();
            ends.push(self.flow.clone());
        }
        let lc = self.pop_loop();
        ends.extend(lc.breaks);
        if !exhaustive {
            ends.push(entry);
        }
        if sw.cases.is_empty() {
            ends.push(self.flow.clone());
        }
        self.flow = self.join_all(ends);
        self.record_switch(sw.scrutinee.id, &sc, plan.labels, exhaustive, plan.has_default);
        // switch info is keyed by the statement node so lowering can find it
        if let Some(info) = self.mods[self.cm].out.switches.get(&sw.scrutinee.id).cloned() {
            self.mods[self.cm].out.switches.insert(s.id, info);
        }
    }

    pub(crate) fn switch_expr(&mut self, e: &'a Expr, s: &'a SwitchExpr, exp: Option<Ty>) -> Ty {
        let sc = self.switch_scrutinee(&s.scrutinee);
        let entry = self.flow.clone();
        let clauses: Vec<(&'a [Expr], bool)> = s.arms.iter().map(|a| (a.labels.as_slice(), a.is_default)).collect();
        let plan = self.switch_plan(&sc, &clauses);
        let exhaustive = self.switch_exhaustive(&sc, &plan, e.span, true, None);
        let mut arms: Vec<(&'a Expr, Ty)> = Vec::new();
        let mut ends: Vec<Flow> = Vec::new();
        let mut hint = exp;
        for (i, arm) in s.arms.iter().enumerate() {
            self.flow = entry.clone();
            if let Some(Some((p, t))) = plan.narrow.get(i) {
                self.flow.set(*p, *t);
            }
            let t = self.expr(&arm.body, hint);
            if hint.is_none() && t != ERROR && t != NULL && t != NEVER {
                hint = Some(t);
            }
            self.set_ty(arm.id, t);
            ends.push(self.flow.clone());
            arms.push((&arm.body, t));
        }
        self.flow = if ends.is_empty() { entry } else { self.join_all(ends) };
        self.record_switch(e.id, &sc, plan.labels, exhaustive, plan.has_default);
        if arms.is_empty() {
            return ERROR;
        }
        self.unify_arms(&arms, exp)
    }

    // ---- functions -----------------------------------------------------------------------------

    fn assigned_names(body: &'a Block) -> (Rc<HashSet<String>>, Rc<HashSet<String>>, Rc<HashSet<String>>) {
        let mut v = AssignedNames::new();
        v.visit_block(body);
        (Rc::new(v.all), Rc::new(v.in_closure), Rc::new(v.recv_in_closure))
    }

    pub fn check_fn_body(&mut self, d: DefId) {
        if self.defs[d.0 as usize].body_done {
            return;
        }
        let (decl, member, record): (&'a FnDecl, Option<&'a Member>, Option<&'a RecordDecl>) = match self.defs[d.0 as usize].ast {
            Ast::Item(item) => match &item.kind {
                ItemKind::Fn(f) => (f, None, None),
                _ => return,
            },
            Ast::Member(m, r) => match &m.kind {
                MemberKind::Method(f) | MemberKind::Constructor(f) => (f, Some(m), Some(r)),
                MemberKind::Field(_) => return,
            },
            _ => return,
        };
        let Some(body) = &decl.body else { return };
        self.defs[d.0 as usize].body_done = true;
        if !self.fns.contains_key(&d) {
            self.ensure_fn(d);
        }
        let Some(sig) = self.fns.get(&d).cloned() else { return };
        let _ = member;
        self.with_def_env(d, |s| s.fn_body_inner(d, decl, body, record, &sig));
        self.flush_bounds();
    }

    fn fn_body_inner(&mut self, d: DefId, decl: &'a FnDecl, body: &'a Block, record: Option<&'a RecordDecl>, sig: &FnSig) {
        let kind = self.kind_of(d);
        let is_ctor = kind == DefKind::Ctor;
        let infer = !is_ctor && decl.ret.is_none();
        let (assigned, cassigned, crecv) = Self::assigned_names(body);
        let info = self.defs[d.0 as usize].info.clone();
        let mut ctx = self.new_fnctx(false, if infer { None } else { Some(sig.ret) }, info.name.clone(), assigned, cassigned, info.tparams.clone());
        ctx.is_ctor = is_ctor;
        ctx.def = Some(d);
        ctx.closure_recv = crecv;
        ctx.owner = info.owner;
        ctx.mutating = sig.mutating;
        ctx.exported = info.exported;
        ctx.name_span = Some(decl.name.span);
        let owner = info.owner;
        let this_is_value = owner.is_some_and(|o| self.kind_of(o) == DefKind::Struct);
        ctx.this_is_value = this_is_value;
        // constructors: constructor-assigned fields are tracked through the flow state
        self.fnctx.push(ctx);
        let saved_flow = std::mem::take(&mut self.flow);
        self.flow = Flow::default();
        let saved_depth = self.depth;
        self.push_scope();
        // defaults are checked in the scope that has no parameters
        for (p, ps) in decl.params.iter().zip(&sig.params) {
            if let Some(dv) = &p.default {
                self.default_expr(dv, ps.ty);
            }
        }
        // `this`
        if let (Some(o), false) = (owner, sig.is_static) {
            let targs: Vec<Ty> = self.defs[o.0 as usize].info.tparams.iter().map(|p| self.tys.mk(TyKind::Param(*p))).collect();
            let tt = self.tys.adt(o, &targs);
            let l = self.new_local("this", tt, LocalKind::This, decl.name.span, NodeId(u32::MAX - 1));
            self.bind_scope("this", Binding::Local(l), decl.name.span);
            if let Some(c) = self.fnctx.last_mut() {
                c.this = Some((l, tt));
            }
            self.mods[self.cm].out.this_locals.insert(d, l);
            if is_ctor {
                if let Some(rec) = self.records.get(&o).cloned() {
                    for (i, f) in rec.fields.iter().enumerate() {
                        if !f.is_static && !f.has_init {
                            self.flow.mark_unassigned(0x8000_0000 | i as u32);
                        }
                    }
                }
            }
        }
        for (p, ps) in decl.params.iter().zip(&sig.params) {
            let l = self.new_local(&p.name.name, ps.ty, LocalKind::Param, p.name.span, p.name.id);
            self.bind_scope(&p.name.name, Binding::Local(l), p.name.span);
        }
        let _ = record;
        self.block_in_scope(body);
        // end of body
        let (ret, inferred) = {
            let c = self.fnctx.last();
            (c.and_then(|c| c.ret), c.and_then(|c| c.inferred))
        };
        let fin = ret.or(inferred).unwrap_or(VOID);
        if !self.flow.dead {
            if is_ctor {
                let o = owner.unwrap_or(d);
                if let Some(rec) = self.records.get(&o).cloned() {
                    let missing: Vec<String> = self
                        .flow
                        .unassigned
                        .iter()
                        .filter(|u| **u & 0x8000_0000 != 0)
                        .filter_map(|u| rec.fields.get((*u & 0x7fff_ffff) as usize).map(|f| f.name.clone()))
                        .collect();
                    if !missing.is_empty() {
                        self.err(
                            "OXS-E0123",
                            decl.name.span,
                            &[("detail", &format!("Property '{}' has no initializer and is not definitely assigned in the constructor", missing.join("', '")))],
                        );
                    }
                }
            } else if fin != VOID && fin != ERROR {
                let n = self.show(fin);
                self.err("OXS-E0114", decl.name.span, &[("name", &info.name), ("ty", &n)]);
            }
        }
        self.pop_scope();
        self.fnctx.pop();
        self.flow = saved_flow;
        self.depth = saved_depth;
        if infer {
            let r = inferred.unwrap_or(VOID);
            if let Some(s) = self.fns.get_mut(&d) {
                s.ret = r;
            }
        }
    }

    pub fn check_law(&mut self, d: DefId) {
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::Law(l) = &item.kind else { return };
        self.with_def_env(d, |s| {
            s.resolve_bounds(d, &l.type_params);
            let names = (Rc::new(HashSet::new()), Rc::new(HashSet::new()));
            let info = s.defs[d.0 as usize].info.clone();
            let mut ctx = s.new_fnctx(false, Some(BOOL), info.name.clone(), names.0, names.1, info.tparams.clone());
            ctx.is_law = true;
            ctx.def = Some(d);
            s.fnctx.push(ctx);
            let saved = std::mem::take(&mut s.flow);
            s.push_scope();
            for p in &l.params {
                let ty = if let Some(t) = &p.ty { s.lower_type(t) } else {
                    s.err("OXS-E0103", p.span, &[("what", "parameter")]);
                    ERROR
                };
                if matches!(s.tys.kind(ty), TyKind::Adt(h, _) if s.kind_of(*h) == DefKind::Host) {
                    s.err("OXS-E0125", p.span, &[("detail", "a law cannot take a host capability as a parameter")]);
                }
                let loc = s.new_local(&p.name.name, ty, LocalKind::Param, p.name.span, p.name.id);
                s.bind_scope(&p.name.name, Binding::Local(loc), p.name.span);
            }
            if let Some(w) = &l.when {
                let (ft, _) = s.cond_bool(w);
                s.flow = ft;
            }
            s.hoist(&l.body.stmts);
            for st in &l.body.stmts {
                s.stmt(st);
            }
            match &l.body.tail {
                Some(t) => {
                    let ty = s.expr(t, Some(BOOL));
                    if ty != BOOL && ty != ERROR && ty != NEVER {
                        let sh = s.show(ty);
                        s.err("OXS-E0125", t.span, &[("detail", &format!("a law body must be a `bool` expression, found '{sh}'"))]);
                    }
                }
                None => {
                    s.err("OXS-E0125", l.body.span, &[("detail", "a law body must end in a `bool` expression without `;`")]);
                }
            }
            s.pop_scope();
            s.fnctx.pop();
            s.flow = saved;
        });
        self.flush_bounds();
    }

    pub fn check_global(&mut self, d: DefId) {
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::Var(v) = &item.kind else { return };
        let is_local = self.defs[d.0 as usize].info.local;
        let _ = is_local;
        let name = self.def_name(d).to_string();
        let mutable = v.kind == VarKind::Let;
        let outer_init = self.cur_init.replace(crate::inst::GKey::Global(d));
        self.with_def_env(d, |s| {
            let ctx = s.new_fnctx(false, None, name.clone(), Rc::default(), Rc::default(), vec![]);
            s.fnctx.push(ctx);
            let saved = std::mem::take(&mut s.flow);
            s.push_scope();
            let decl = v.ty.as_ref().map(|t| s.lower_type(t));
            let ty = match (&v.init, decl) {
                (Some(x), Some(t)) => {
                    // pre-register so the initialiser may mention the declared type only
                    s.expr_to(x, t);
                    t
                }
                (Some(x), None) => {
                    let t = s.expr(x, None);
                    let t = if mutable { s.widen(t) } else { t };
                    if t == NULL || t == VOID || t == NEVER {
                        s.err("OXS-E0103", v.pattern.span, &[("what", &format!("variable `{name}`"))]);
                        ERROR
                    } else {
                        t
                    }
                }
                (None, d2) => {
                    s.err(
                        "OXS-E0123",
                        v.pattern.span,
                        &[("detail", &format!("module-level `{}` `{name}` needs an initializer", if mutable { "let" } else { "const" }))],
                    );
                    d2.unwrap_or(ERROR)
                }
            };
            s.pop_scope();
            s.fnctx.pop();
            s.flow = saved;
            s.globals.insert(d, GlobalSig { ty, mutable });
        });
        self.cur_init = outer_init;
    }

    pub fn check_record_bodies(&mut self, d: DefId) {
        if self.defs[d.0 as usize].body_done {
            return;
        }
        self.defs[d.0 as usize].body_done = true;
        self.ensure_record(d);
        let Ast::Item(item) = self.defs[d.0 as usize].ast else { return };
        let ItemKind::Record(r) = &item.kind else { return };
        let Some(rec) = self.records.get(&d).cloned() else { return };
        // field initialisers
        let mut fi = 0usize;
        for m in &r.members {
            match &m.kind {
                MemberKind::Field(f) => {
                    let fty = rec.fields.iter().find(|x| x.name == f.name.name).map(|x| x.ty);
                    let fidx = rec.fields.iter().position(|x| x.name == f.name.name);
                    let _ = fi;
                    fi += 1;
                    if let (Some(init), Some(fty)) = (&f.init, fty) {
                        let outer_init = std::mem::replace(
                            &mut self.cur_init,
                            match fidx {
                                Some(i) if rec.fields[i].is_static => Some(crate::inst::GKey::Static(d, i as u32)),
                                _ => None,
                            },
                        );
                        self.with_def_env(d, |s| {
                            let ctx = s.new_fnctx(false, None, "<field>".into(), Rc::default(), Rc::default(), vec![]);
                            s.fnctx.push(ctx);
                            let saved = std::mem::take(&mut s.flow);
                            s.push_scope();
                            s.expr_to(init, fty);
                            s.pop_scope();
                            s.fnctx.pop();
                            s.flow = saved;
                        });
                        self.cur_init = outer_init;
                    }
                }
                MemberKind::Method(_) | MemberKind::Constructor(_) => {
                    if let Some(&md) = self.mods[self.cm].out.node_def.get(&m.id) {
                        self.check_fn_body(md);
                    }
                }
            }
        }
        // a class without a constructor needs every field initialised
        if rec.is_class && rec.ctor.is_none() {
            let missing: Vec<&str> = rec.fields.iter().filter(|f| !f.is_static && !f.has_init).map(|f| f.name.as_str()).collect();
            if !missing.is_empty() {
                self.err(
                    "OXS-E0123",
                    r.name.span,
                    &[("detail", &format!("Property '{}' has no initializer and the class has no constructor", missing.join("', '")))],
                );
            }
        }
        // a static has no constructor to assign it: like a module-level binding it needs an initialiser
        for f in rec.fields.iter().filter(|f| f.is_static && !f.has_init) {
            self.err("OXS-E0123", f.span, &[("detail", &format!("static field '{}' needs an initializer", f.name))]);
        }
        // `implements` is checked structurally
        let targs: Vec<Ty> = self.defs[d.0 as usize].info.tparams.iter().map(|p| self.tys.mk(TyKind::Param(*p))).collect();
        let me = self.tys.adt(d, &targs);
        for (i, iface) in rec.implements.iter().enumerate() {
            if let Err(why) = self.satisfies(me, *iface) {
                let (a, b) = (self.show(me), self.show(*iface));
                let sp = r.implements.get(i).map_or(r.name.span, |t| t.span);
                self.err("OXS-E0117", sp, &[("detail", &format!("Class '{a}' incorrectly implements interface '{b}': {why}"))]);
            }
        }
        self.flush_bounds();
    }

    pub fn check_module_bodies(&mut self, k: usize) {
        self.cm = k;
        self.flush_bounds();
        let items = self.mods[k].out.items.clone();
        for d in items {
            match self.kind_of(d) {
                DefKind::Fn => {
                    self.check_fn_body(d);
                    self.check_export_sig(d);
                }
                DefKind::Struct | DefKind::Class => self.check_record_bodies(d),
                DefKind::Global => self.ensure_global(d),
                DefKind::Law => self.check_law(d),
                _ => {}
            }
        }
        // exported non-function items must not expose unresolved types (nothing to do: signatures are explicit)
        let _ = full_mask(0);
        self.flush_bounds();
    }
}
