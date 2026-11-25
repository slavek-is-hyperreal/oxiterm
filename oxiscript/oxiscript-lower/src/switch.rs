//! `switch` statements and expressions: constant tables (union tags, enum members, integer
//! literals) become IR `switch` terminators, everything else an if-chain.

use crate::fb::tgt;
use crate::fx::{strip, Frame, Fx, Key, Opnd, Slot};
use oxiscript_check::{CaseLabel, DefKind, Res, SwitchInfo, SwitchKind, TyKind};
use oxiscript_ir::{CmpOp, Const, Op, Terminator, Ty as It, ValueId};
use oxiscript_syntax::visit::Visitor;
use oxiscript_syntax::{Expr, ExprKind, LitKind, Stmt, SwitchExpr, SwitchStmt, UnOp};

enum Body<'e> {
    Stmts(&'e [Stmt]),
    Expr(&'e Expr),
}

struct Clause<'e> {
    labels: &'e [Expr],
    is_default: bool,
    body: Body<'e>,
}

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    pub fn switch_stmt(&mut self, sw: &SwitchStmt) {
        let clauses: Vec<Clause> = sw
            .cases
            .iter()
            .map(|c| Clause {
                labels: &c.labels,
                is_default: c.is_default,
                body: Body::Stmts(&c.body),
            })
            .collect();
        self.lower_switch(sw.scrutinee.id, &sw.scrutinee, clauses, None);
    }

    pub fn switch_expr(&mut self, e: &Expr, sw: &SwitchExpr) -> Opnd {
        let rty = self.ty_of(e.id);
        let rr = self.repr(rty);
        let clauses: Vec<Clause> = sw
            .arms
            .iter()
            .map(|a| Clause {
                labels: &a.labels,
                is_default: a.is_default,
                body: Body::Expr(&a.body),
            })
            .collect();
        match self.lower_switch(e.id, &sw.scrutinee, clauses, Some(rr.clone())) {
            Some(v) => Opnd::owned(v),
            None => Opnd::void(),
        }
    }

    /// Constant integer value of a case label, if it is one.
    fn const_label(&self, e: &Expr) -> Option<i128> {
        let e = strip(e);
        match &e.kind {
            ExprKind::Lit(l) => match &l.kind {
                LitKind::Int { value, .. } => Some(*value as i128),
                _ => None,
            },
            ExprKind::Unary { op: UnOp::Neg, expr } => match &strip(expr).kind {
                ExprKind::Lit(l) => match &l.kind {
                    LitKind::Int { value, .. } => Some(-(*value as i128)),
                    _ => None,
                },
                _ => None,
            },
            ExprKind::Member { .. } => match self.res_of(e.id) {
                Some(Res::EnumMember { def, index }) => Some(self.lw.ck.enums[&def].members[index as usize].1),
                _ => None,
            },
            _ => None,
        }
    }

    /// Returns the result value (switch expressions: `result` is `Some(repr)`).
    fn lower_switch(&mut self, key: oxiscript_syntax::NodeId, scrut: &Expr, clauses: Vec<Clause>, result: Option<Option<It>>) -> Option<ValueId> {
        let info: SwitchInfo = match self.mi().switches.get(&key).cloned() {
            Some(i) => i,
            None => {
                self.unsupported("this switch", scrut.span);
                return None;
            }
        };
        self.push_scope();
        // the scrutinee lives in a hidden local until the switch ends
        let skey = self.tmp_key();
        let s_expr = strip(scrut);
        let is_union = info.kind == SwitchKind::Union;
        let sv: ValueId = if is_union {
            let ExprKind::Member { obj, .. } = &s_expr.kind else {
                self.unsupported("a union switch on this expression", scrut.span);
                return None;
            };
            let o = self.expr(obj);
            let Some(v) = self.own_of(o) else { return None };
            v
        } else {
            let o = self.expr(s_expr);
            let Some(v) = self.own_of(o) else { return None };
            v
        };
        self.declare(skey, Slot::Ssa { v: sv, owned: true });
        let tag = if is_union {
            Some(self.fb.emit1(Op::Tag { obj: sv }, It::U32))
        } else {
            None
        };
        let result_ty: Option<It> = result.clone().flatten();
        let set = self.assigned_in(|sc| {
            for c in &clauses {
                match &c.body {
                    Body::Stmts(ss) => ss.iter().for_each(|s| sc.visit_stmt(s)),
                    Body::Expr(x) => sc.visit_expr(x),
                }
            }
        });
        let mut exit = self.new_join(result_ty.clone(), set.clone());
        let blocks: Vec<_> = clauses.iter().map(|_| self.new_block("case")).collect();
        let default_idx = clauses.iter().position(|c| c.is_default);
        let none_blk = self.new_block("nomatch");
        let default_blk = default_idx.map_or(none_blk, |i| blocks[i]);
        // dispatch
        let constant_table: Option<Vec<(i64, usize)>> = {
            let mut tab = Vec::new();
            let mut ok = true;
            if is_union {
                for (i, labs) in info.labels.iter().enumerate() {
                    for l in labs {
                        if let CaseLabel::Variant(v) = l {
                            tab.push((i64::from(*v), i));
                        }
                    }
                }
            } else if matches!(info.kind, SwitchKind::Enum | SwitchKind::Int) {
                for (i, c) in clauses.iter().enumerate() {
                    for l in c.labels {
                        match self.const_label(l) {
                            Some(n) if i64::try_from(n).is_ok() => tab.push((n as i64, i)),
                            _ => ok = false,
                        }
                    }
                }
            } else {
                ok = false;
            }
            if ok {
                Some(tab)
            } else {
                None
            }
        };
        let snap_dispatch = self.env.clone();
        if let Some(tab) = constant_table {
            let scrut_v = tag.unwrap_or(sv);
            let cases: Vec<(i64, _)> = tab.iter().map(|(k, i)| (*k, tgt(blocks[*i], &[]))).collect();
            self.terminate(Terminator::Switch {
                value: scrut_v,
                cases,
                default: tgt(default_blk, &[]),
            });
        } else {
            // if-chain over the labels in clause order
            for (i, c) in clauses.iter().enumerate() {
                for (j, l) in c.labels.iter().enumerate() {
                    let cond = self.label_test(&info, i, j, l, sv);
                    let next = self.new_block("test");
                    self.terminate(Terminator::BrIf {
                        cond,
                        then: tgt(blocks[i], &[]),
                        els: tgt(next, &[]),
                    });
                    self.start_block(next);
                }
            }
            self.terminate(Terminator::Br(tgt(default_blk, &[])));
        }
        // clause bodies
        self.loops.push(Frame {
            label: None,
            brk: exit.blk,
            cont: None,
            scope_depth: self.scopes.len(),
            brk_set: set.clone(),
            cont_set: Vec::new(),
            brk_reached: false,
            cont_hit: false,
            is_switch: true,
        });
        let mut res_val: Option<ValueId> = None;
        for (i, c) in clauses.iter().enumerate() {
            self.env = snap_dispatch.clone();
            self.start_block(blocks[i]);
            match &c.body {
                Body::Stmts(ss) => {
                    self.push_scope();
                    for s in *ss {
                        self.stmt(s);
                    }
                    self.pop_scope();
                    self.jump_join(&mut exit, None);
                }
                Body::Expr(x) => {
                    let rv = match &result_ty {
                        Some(t) => {
                            let o = self.expr_val(x, t);
                            self.own_of(o)
                        }
                        None => {
                            let o = self.expr(x);
                            self.drop_o(o);
                            None
                        }
                    };
                    self.jump_join(&mut exit, rv);
                }
            }
        }
        let f = self.loops.pop().expect("frame");
        exit.reached |= f.brk_reached;
        // no clause matched
        self.env = snap_dispatch;
        self.start_block(none_blk);
        if info.exhaustive || result.is_some() {
            self.terminate(Terminator::Trap(oxiscript_ir::TrapKind::Unreachable));
        } else {
            self.jump_join(&mut exit, None);
        }
        let r = self.enter_join(&exit);
        res_val = r.or(res_val);
        self.pop_scope();
        res_val
    }

    /// Condition "the scrutinee matches label `j` of clause `i`".
    fn label_test(&mut self, info: &SwitchInfo, i: usize, j: usize, l: &Expr, sv: ValueId) -> ValueId {
        match info.labels.get(i).and_then(|v| v.get(j)) {
            Some(CaseLabel::Null) => self.fb.emit1(Op::IsNull { val: sv }, It::Bool),
            Some(CaseLabel::Variant(v)) => {
                let tag = self.fb.emit1(Op::Tag { obj: sv }, It::U32);
                let ix = self.fb.emit1(Op::Const(Const::Int(i128::from(*v))), It::U32);
                self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: tag, b: ix }, It::Bool)
            }
            _ => {
                // a value label: compare for equality
                let o = self.expr(l);
                let Some(lv) = o.v else { return self.const_bool(false) };
                let st = self.fb.ty(sv).clone();
                let r = if st == It::Str {
                    self.fb.emit1(Op::StrEq { a: sv, b: lv }, It::Bool)
                } else {
                    self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: sv, b: lv }, It::Bool)
                };
                self.drop_o(o);
                r
            }
        }
    }

    #[allow(dead_code)]
    fn unused(&self, _k: Key, _d: DefKind, _t: TyKind) {}
}
