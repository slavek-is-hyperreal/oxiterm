//! Statements, blocks, loops, patterns, returns.

use crate::fb::tgt;
use crate::fx::{strip, AssignScan, Frame, Fx, FnKind, Join, Key, Opnd, Slot};
use oxiscript_check::{DefKind, LocalId, Res, Ty, TyKind};
use oxiscript_ir::{CmpOp, Const, FieldRef, Op, Ovf, Terminator, Ty as It, ValueId};
use oxiscript_syntax::visit::Visitor;
use oxiscript_syntax::{Block, Expr, ExprKind, Ident, Pattern, PatternKind, Stmt, StmtKind, VarDecl};

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    /// Locals assigned inside a fragment that are owned SSA slots now.
    pub fn assigned_in(&mut self, f: impl FnOnce(&mut AssignScan<'_, '_>)) -> Vec<Key> {
        let mut sc = AssignScan {
            mi: &self.lw.ck.modules[self.m],
            tys: &self.lw.ck.tys,
            defs: &self.lw.ck.defs,
            fns: &self.lw.ck.fns,
            keys: Vec::new(),
        };
        f(&mut sc);
        let keys = sc.keys;
        self.ssa_owned(&keys)
    }

    // ------------------------------------------------------------------ function frame

    /// Bind an incoming (borrowed) parameter value to its local.
    pub fn bind_param(&mut self, l: LocalId, v: ValueId) {
        let key = l.0;
        if self.is_cell(l) {
            if self.is_mg(v) {
                self.retain(v);
            }
            self.declare_local(l, Some(v));
        } else if self.assigned_fn.contains(&key) || self.mi().locals[l.0 as usize].mutated {
            if self.is_mg(v) {
                self.retain(v);
            }
            self.declare(key, Slot::Ssa { v, owned: true });
        } else {
            self.declare(key, Slot::Ssa { v, owned: false });
        }
    }

    /// Return from the function: release everything the function owns, then `ret`.
    pub fn emit_return(&mut self, v: Option<ValueId>) {
        if self.dead {
            return;
        }
        let mut rets: Vec<ValueId> = Vec::new();
        let mut restore: Option<(Key, Slot)> = None;
        if self.kind == FnKind::Mutating {
            if let Some(k) = self.this_key {
                if let Some(Slot::Ssa { v: tv, owned }) = self.env.get(&k).cloned() {
                    rets.push(tv);
                    restore = Some((k, Slot::Ssa { v: tv, owned }));
                    // the returned receiver is not released with the scopes
                    self.env.insert(k, Slot::Ssa { v: tv, owned: false });
                }
            }
        }
        if let Some(v) = v {
            rets.push(v);
        }
        self.release_held();
        self.release_scopes_from(0);
        if let Some((k, s)) = restore {
            self.env.insert(k, s);
        }
        self.terminate(Terminator::Ret(rets));
    }

    /// Finish the function: implicit return, terminator hygiene, canonical numbering.
    pub fn finish(mut self) -> oxiscript_ir::Func {
        if !self.dead {
            if self.fb.f.rets.is_empty() || self.kind == FnKind::Mutating && self.fb.f.rets.len() == 1 {
                self.emit_return(None);
            } else {
                self.terminate(Terminator::Trap(oxiscript_ir::TrapKind::Unreachable));
            }
        }
        self.fb.finish()
    }

    // ------------------------------------------------------------------ blocks and statements

    pub fn block_stmts(&mut self, b: &Block) {
        self.push_scope();
        for s in &b.stmts {
            self.stmt(s);
        }
        self.pop_scope();
    }

    pub fn stmt(&mut self, s: &Stmt) {
        self.loc_of(s.span);
        match &s.kind {
            StmtKind::Var(v) => self.var_decl(v),
            StmtKind::Expr(e) => self.expr_stmt(e),
            StmtKind::If { cond, then, els } => self.if_stmt(cond, then, els.as_deref()),
            StmtKind::While { cond, body } => self.loop_stmt(None, None, Some(cond), None, body, false),
            StmtKind::DoWhile { body, cond } => self.loop_stmt(None, None, Some(cond), None, body, true),
            StmtKind::For { init, cond, update, body, .. } => {
                self.loop_stmt(None, init.as_deref(), cond.as_ref(), update.as_ref(), body, false);
            }
            StmtKind::ForOf { binding, iter, body, .. } => self.for_of(None, binding, iter, body),
            StmtKind::Switch(sw) => self.switch_stmt(sw),
            StmtKind::Return(e) => self.return_stmt(e.as_ref()),
            StmtKind::Break(l) => self.break_stmt(l.as_ref()),
            StmtKind::Continue(l) => self.continue_stmt(l.as_ref()),
            StmtKind::Block(b) => self.block_stmts(b),
            StmtKind::Labeled { label, body } => self.labeled(label, body),
            StmtKind::Item(_) | StmtKind::Empty => {}
        }
    }

    fn expr_stmt(&mut self, e: &Expr) {
        let e = strip(e);
        match &e.kind {
            ExprKind::Assign { op, target, value } => {
                let o = self.assign(e, *op, target, value, false);
                self.drop_o(o);
            }
            ExprKind::Update { op, target, prefix } => {
                self.update_expr(e, *op, target, *prefix);
            }
            _ => {
                let o = self.expr(e);
                self.drop_o(o);
            }
        }
    }

    fn return_stmt(&mut self, e: Option<&Expr>) {
        if self.ctor.is_some() {
            self.ctor_finish();
            return;
        }
        let v = match e {
            Some(x) => {
                let o = self.expr(x);
                if self.fb.f.rets.is_empty() || (self.kind == FnKind::Mutating && self.fb.f.rets.len() == 1) {
                    self.drop_o(o);
                    None
                } else {
                    self.own_of(o)
                }
            }
            None => None,
        };
        self.emit_return(v);
    }

    // ------------------------------------------------------------------ variables and patterns

    fn var_decl(&mut self, v: &VarDecl) {
        match &v.pattern.kind {
            PatternKind::Ident(id) => {
                let Some(&l) = self.mi().binding_local.get(&id.id) else {
                    self.unsupported("this declaration", v.pattern.span);
                    return;
                };
                let info = self.local_info(l);
                let lty = self.cty(info.ty);
                let irt = self.lw.repr(lty);
                let init = match (&v.init, irt) {
                    (Some(x), Some(t)) => {
                        let o = self.expr_val(x, &t);
                        self.own_of(o)
                    }
                    (Some(x), None) => {
                        let o = self.expr(x);
                        self.drop_o(o);
                        None
                    }
                    (None, Some(t)) => self.placeholder(&t),
                    (None, None) => None,
                };
                self.declare_local(l, init);
            }
            PatternKind::Wildcard => {
                if let Some(x) = &v.init {
                    let o = self.expr(x);
                    self.drop_o(o);
                }
            }
            PatternKind::Object(_) => {
                let Some(x) = &v.init else { return };
                let ty = self.ty_of(x.id);
                let ty = match self.mi().coercions.get(&x.id).copied() {
                    Some(c) => self.cty(c.to),
                    None => ty,
                };
                let o = self.expr(x);
                self.hold(o);
                self.bind_pattern(&v.pattern, o, ty);
                self.unhold(o);
                self.drop_o(o);
            }
        }
    }

    /// A value for a declaration without initialiser (assigned before any read).
    fn placeholder(&mut self, t: &It) -> Option<ValueId> {
        match t {
            It::Str => {
                let v = self.str_const("");
                self.retain(v);
                Some(v)
            }
            It::Array(_) => Some(self.fb.emit1(Op::ArrayEmpty, t.clone())),
            It::Opt(_) => Some(self.fb.emit1(Op::Null, t.clone())),
            x if x.is_managed() => None,
            x => Some(self.dummy(x)),
        }
    }

    pub fn bind_pattern(&mut self, p: &Pattern, value: Opnd, ty: Ty) {
        match &p.kind {
            PatternKind::Ident(id) => {
                let Some(&l) = self.mi().binding_local.get(&id.id) else { return };
                let v = self.own_of(value);
                self.declare_local(l, v);
            }
            PatternKind::Wildcard => {}
            PatternKind::Object(fields) => {
                let Some(obj) = value.v else { return };
                let (owner, args): (oxiscript_check::DefId, Vec<Ty>) = match self.kind(ty) {
                    TyKind::Adt(d, a) => (d, a.to_vec()),
                    _ => {
                        self.unsupported("destructuring this type", p.span);
                        return;
                    }
                };
                let rec = self.lw.ck.records[&owner].clone();
                let map = self.lw.tparam_map(owner, &args);
                let ir_ty = self.lw.struct_name(owner, &args);
                for f in fields {
                    let Some(fi) = rec.fields.iter().position(|x| !x.is_static && x.name == f.key.name) else {
                        continue;
                    };
                    let fty = self.lw.subst(rec.fields[fi].ty, &map);
                    let Some(fr) = self.lw.repr(fty) else { continue };
                    let fv = self.fb.emit1(
                        Op::GetField {
                            field: FieldRef {
                                ty: ir_ty.clone(),
                                variant: None,
                                field: f.key.name.clone(),
                            },
                            obj,
                        },
                        fr,
                    );
                    if self.is_mg(fv) {
                        self.retain(fv);
                    }
                    match &f.pattern {
                        None => {
                            if let Some(&l) = self.mi().binding_local.get(&f.id) {
                                self.declare_local(l, Some(fv));
                            }
                        }
                        Some(sub) => {
                            let o = Opnd::owned(fv);
                            self.bind_pattern(sub, o, fty);
                            if matches!(sub.kind, PatternKind::Wildcard) {
                                self.drop_o(o);
                            } else if matches!(sub.kind, PatternKind::Object(_)) {
                                self.drop_o(o);
                            }
                        }
                    }
                }
            }
        }
    }

    // ------------------------------------------------------------------ if

    fn if_stmt(&mut self, cond: &Expr, then: &Stmt, els: Option<&Stmt>) {
        let c = self.expr(cond);
        let Some(cv) = c.v else { return };
        let set = self.assigned_in(|sc| {
            sc.visit_stmt(then);
            if let Some(e) = els {
                sc.visit_stmt(e);
            }
        });
        let snap = self.env.clone();
        let mut j = self.new_join(None, set);
        let tb = self.new_block("then");
        let eb = self.new_block("else");
        self.fb.term(Terminator::BrIf {
            cond: cv,
            then: tgt(tb, &[]),
            els: tgt(eb, &[]),
        });
        self.start_block(tb);
        self.push_scope();
        self.stmt(then);
        self.pop_scope();
        self.jump_join(&mut j, None);
        self.env = snap.clone();
        self.start_block(eb);
        if let Some(e) = els {
            self.push_scope();
            self.stmt(e);
            self.pop_scope();
        }
        self.jump_join(&mut j, None);
        self.env = snap;
        self.enter_join(&j);
    }

    // ------------------------------------------------------------------ loops

    fn labeled(&mut self, label: &Ident, body: &Stmt) {
        match &body.kind {
            StmtKind::While { cond, body: b } => {
                self.loop_stmt(Some(label.name.clone()), None, Some(cond), None, b, false);
            }
            StmtKind::DoWhile { body: b, cond } => {
                self.loop_stmt(Some(label.name.clone()), None, Some(cond), None, b, true);
            }
            StmtKind::For { init, cond, update, body: b, .. } => {
                self.loop_stmt(Some(label.name.clone()), init.as_deref(), cond.as_ref(), update.as_ref(), b, false);
            }
            StmtKind::ForOf { binding, iter, body: b, .. } => {
                self.for_of(Some(label.name.clone()), binding, iter, b);
            }
            _ => {
                // a labelled block: `break label` leaves it
                let set = self.assigned_in(|sc| sc.visit_stmt(body));
                let mut exit = self.new_join(None, set.clone());
                self.loops.push(Frame {
                    label: Some(label.name.clone()),
                    brk: exit.blk,
                    cont: None,
                    scope_depth: self.scopes.len(),
                    brk_set: set,
                    cont_set: Vec::new(),
                    brk_reached: false,
                    cont_hit: false,
                    is_switch: true,
                });
                self.stmt(body);
                let f = self.loops.pop().expect("frame");
                self.jump_join(&mut exit, None);
                exit.reached |= f.brk_reached;
                self.enter_join(&exit);
            }
        }
    }

    /// `while`, `do-while` and `for`. `init` runs in a fresh scope around the loop.
    fn loop_stmt(&mut self, label: Option<String>, init: Option<&Stmt>, cond: Option<&Expr>, update: Option<&Expr>, body: &Stmt, do_while: bool) {
        self.push_scope();
        if let Some(i) = init {
            self.stmt(i);
        }
        let set = self.assigned_in(|sc| {
            if let Some(c) = cond {
                sc.visit_expr(c);
            }
            if let Some(u) = update {
                sc.visit_expr(u);
            }
            sc.visit_stmt(body);
        });
        let mut head = self.new_join(None, set.clone());
        let mut exit = self.new_join(None, set.clone());
        let mut upd = self.new_join(None, set.clone());
        self.jump_join(&mut head, None);
        self.enter_join(&head);
        if !do_while {
            if let Some(c) = cond {
                let co = self.expr(c);
                let cv = co.val();
                let bodyb = self.new_block("body");
                let exit_args = self.join_args(&set);
                self.terminate_to_body(cv, bodyb, exit.blk, &exit_args);
                exit.reached = true;
                self.start_block(bodyb);
            }
        }
        self.loops.push(Frame {
            label,
            brk: exit.blk,
            cont: Some(upd.blk),
            scope_depth: self.scopes.len(),
            brk_set: set.clone(),
            cont_set: set.clone(),
            brk_reached: false,
            cont_hit: false,
            is_switch: false,
        });
        self.push_scope();
        self.stmt(body);
        self.pop_scope();
        let f = self.loops.pop().expect("frame");
        self.jump_join(&mut upd, None);
        upd.reached |= f.cont_hit;
        exit.reached |= f.brk_reached;
        self.enter_join(&upd);
        if let Some(u) = update {
            self.expr_stmt(u);
        }
        if do_while {
            if let Some(c) = cond {
                let co = self.expr(c);
                let cv = co.val();
                let then_args = self.join_args(&set);
                let else_args = self.join_args(&set);
                self.terminate(Terminator::BrIf {
                    cond: cv,
                    then: tgt(head.blk, &then_args),
                    els: tgt(exit.blk, &else_args),
                });
                exit.reached = true;
            } else {
                self.jump_join(&mut head, None);
            }
        } else {
            self.jump_join(&mut head, None);
        }
        self.enter_join(&exit);
        self.pop_scope();
    }

    /// `br_if cond, body(), exit(args)`; the code after it is dead until a block is entered.
    fn terminate_to_body(&mut self, cond: ValueId, body: oxiscript_ir::BlockId, exit: oxiscript_ir::BlockId, exit_args: &[ValueId]) {
        self.terminate(Terminator::BrIf {
            cond,
            then: tgt(body, &[]),
            els: tgt(exit, exit_args),
        });
    }

    fn for_of(&mut self, label: Option<String>, binding: &Pattern, iter: &Expr, body: &Stmt) {
        self.push_scope();
        let it = strip(iter);
        let is_range = matches!(&it.kind, ExprKind::Call { .. }) && self.builtin_of(it.id) == Some(oxiscript_check::Builtin::Range);
        let elem_ty = if is_range {
            oxiscript_check::ty::I32
        } else {
            let t = self.ty_of(it.id);
            match self.kind(t) {
                TyKind::Array(e) => e,
                _ => t,
            }
        };
        let idx_key = self.tmp_key();
        let arr_key = self.tmp_key();
        let end_key = self.tmp_key();
        if is_range {
            let ExprKind::Call { args, .. } = &it.kind else { return };
            let a = self.expr(&args[0]);
            let b = self.expr(&args[1]);
            let (Some(av), Some(bv)) = (a.v, b.v) else { return };
            self.declare(idx_key, Slot::Ssa { v: av, owned: true });
            self.declare(end_key, Slot::Ssa { v: bv, owned: true });
        } else {
            let o = self.expr(it);
            let Some(av) = self.own_of(o) else { return };
            self.declare(arr_key, Slot::Ssa { v: av, owned: true });
            let z = self.const_i32(0);
            self.declare(idx_key, Slot::Ssa { v: z, owned: true });
        }
        let mut set = self.assigned_in(|sc| sc.visit_stmt(body));
        set.push(idx_key);
        let mut head = self.new_join(None, set.clone());
        let mut exit = self.new_join(None, set.clone());
        let mut upd = self.new_join(None, set.clone());
        self.jump_join(&mut head, None);
        self.enter_join(&head);
        // condition: idx < len (or < end)
        let iv = match self.env.get(&idx_key) {
            Some(Slot::Ssa { v, .. }) => *v,
            _ => return,
        };
        let limit = if is_range {
            match self.env.get(&end_key) {
                Some(Slot::Ssa { v, .. }) => *v,
                _ => return,
            }
        } else {
            let a = match self.env.get(&arr_key) {
                Some(Slot::Ssa { v, .. }) => *v,
                _ => return,
            };
            self.fb.emit1(Op::ArrayLen { arr: a }, It::I32)
        };
        let more = self.fb.emit1(Op::Cmp { op: CmpOp::Lt, a: iv, b: limit }, It::Bool);
        let bodyb = self.new_block("body");
        let exit_args = self.join_args(&set);
        self.terminate(Terminator::BrIf {
            cond: more,
            then: tgt(bodyb, &[]),
            els: tgt(exit.blk, &exit_args),
        });
        exit.reached = true;
        self.start_block(bodyb);
        self.loops.push(Frame {
            label,
            brk: exit.blk,
            cont: Some(upd.blk),
            scope_depth: self.scopes.len(),
            brk_set: set.clone(),
            cont_set: set.clone(),
            brk_reached: false,
            cont_hit: false,
            is_switch: false,
        });
        self.push_scope();
        // the loop variable(s)
        let cur = match self.env.get(&idx_key) {
            Some(Slot::Ssa { v, .. }) => *v,
            _ => return,
        };
        let elem = if is_range {
            Opnd::borrowed(cur)
        } else {
            let a = match self.env.get(&arr_key) {
                Some(Slot::Ssa { v, .. }) => *v,
                _ => return,
            };
            let et = match self.fb.ty(a).clone() {
                It::Array(e) => *e,
                _ => It::I32,
            };
            let ev = self.fb.emit1(Op::ArrayGet { arr: a, idx: cur }, et);
            if self.is_mg(ev) {
                self.retain(ev);
            }
            Opnd::owned(ev)
        };
        self.bind_pattern(binding, elem, elem_ty);
        if matches!(binding.kind, PatternKind::Wildcard | PatternKind::Object(_)) {
            self.drop_o(elem);
        }
        self.stmt(body);
        self.pop_scope();
        let f = self.loops.pop().expect("frame");
        self.jump_join(&mut upd, None);
        upd.reached |= f.cont_hit;
        exit.reached |= f.brk_reached;
        self.enter_join(&upd);
        // idx += 1
        let cur = match self.env.get(&idx_key) {
            Some(Slot::Ssa { v, .. }) => *v,
            _ => return,
        };
        let one = self.const_i32(1);
        let nx = self.fb.emit1(Op::Bin { op: oxiscript_ir::BinOp::Add, ovf: Ovf::Trap, a: cur, b: one }, It::I32);
        self.env.insert(idx_key, Slot::Ssa { v: nx, owned: true });
        self.jump_join(&mut head, None);
        self.enter_join(&exit);
        self.pop_scope();
    }

    // ------------------------------------------------------------------ break / continue

    fn find_frame(&self, label: Option<&Ident>, for_continue: bool) -> Option<usize> {
        for (i, f) in self.loops.iter().enumerate().rev() {
            match label {
                Some(l) => {
                    if f.label.as_deref() == Some(&l.name) && (!for_continue || f.cont.is_some()) {
                        return Some(i);
                    }
                }
                None => {
                    if for_continue {
                        if f.cont.is_some() {
                            return Some(i);
                        }
                    } else {
                        return Some(i);
                    }
                }
            }
        }
        None
    }

    fn break_stmt(&mut self, label: Option<&Ident>) {
        let Some(i) = self.find_frame(label, false) else {
            self.lw.unsupported("`break` outside a loop", None);
            return;
        };
        if self.dead {
            return;
        }
        let f = self.loops[i].clone();
        self.release_held();
        self.release_scopes_from(f.scope_depth);
        let args = self.join_args(&f.brk_set);
        self.loops[i].brk_reached = true;
        self.terminate(Terminator::Br(tgt(f.brk, &args)));
    }

    fn continue_stmt(&mut self, label: Option<&Ident>) {
        let Some(i) = self.find_frame(label, true) else {
            self.lw.unsupported("`continue` outside a loop", None);
            return;
        };
        if self.dead {
            return;
        }
        let f = self.loops[i].clone();
        self.release_scopes_from(f.scope_depth);
        let args = self.join_args(&f.cont_set);
        self.loops[i].cont_hit = true;
        self.terminate(Terminator::Br(tgt(f.cont.expect("loop"), &args)));
    }
}
