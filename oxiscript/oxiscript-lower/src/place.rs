//! Assignable places: variables, globals, fields (with copy-on-write through struct hops)
//! and array elements. One generic implementation serves `=`, `op=`, `++` and mutating
//! method calls.

use crate::ctx::GKey;
use crate::fb::tgt;
use crate::fx::{strip, Fx, Key, Opnd, Slot, THIS_FIELD};
use oxiscript_check::{DefKind, Res, Ty, TyKind};
use oxiscript_ir::{BinOp as IB, Const, FieldRef, Op, Ovf, Terminator, Ty as It, ValueId};
use oxiscript_syntax::{BinOp, Expr, ExprKind, IncDec};

pub(crate) enum Parent {
    /// A reference object (class instance): mutated in place.
    Obj(ValueId),
    /// A struct value that lives in another place: made unique before it is written.
    Val(Box<Place>),
}

pub(crate) enum Place {
    Var(Key),
    Glob(GKey, Ty),
    Field {
        parent: Parent,
        ir_ty: String,
        variant: Option<String>,
        field: String,
        fty: It,
    },
    Elem {
        arr: ValueId,
        idx: ValueId,
        ety: It,
    },
}

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    /// Evaluate the sub-expressions of an assignment target (owner objects, indexes) once.
    /// Owned intermediates are put on the held stack; `finish_place` releases them.
    pub fn prepare_place(&mut self, target: &Expr) -> Option<Place> {
        let t = strip(target);
        match &t.kind {
            ExprKind::Ident(_) | ExprKind::This => match self.res_of(t.id)? {
                Res::Local(l) | Res::This(l) => Some(Place::Var(l.0)),
                Res::Def(d) if self.def_kind(d) == DefKind::Global => {
                    let ty = self.lw.ck.globals[&d].ty;
                    let ty = self.cty(ty);
                    Some(Place::Glob(GKey::Global(d), ty))
                }
                _ => None,
            },
            ExprKind::Member { obj, .. } => match self.res_of(t.id)? {
                Res::Field { owner, index } => {
                    let declared = self.field_decl_ty(obj, owner, index);
                    if self.is_ctor_this(obj) {
                        return Some(Place::Var(THIS_FIELD | index));
                    }
                    let fty = self.repr(declared)?;
                    let (ir_ty, variant, field) = self.field_ref(obj, owner, index);
                    let parent = if self.def_kind(owner) == DefKind::Struct {
                        match self.prepare_place(obj) {
                            Some(p) => Parent::Val(Box::new(p)),
                            None => {
                                let o = self.expr(obj);
                                self.hold(o);
                                Parent::Obj(o.v?)
                            }
                        }
                    } else {
                        let o = self.expr(obj);
                        self.hold(o);
                        Parent::Obj(o.v?)
                    };
                    Some(Place::Field {
                        parent,
                        ir_ty,
                        variant,
                        field,
                        fty,
                    })
                }
                Res::StaticField { owner, index } => {
                    let fty = self.lw.ck.records[&owner].fields[index as usize].ty;
                    let fty = self.cty(fty);
                    Some(Place::Glob(GKey::Static(owner, index), fty))
                }
                Res::Def(d) if self.def_kind(d) == DefKind::Global => {
                    let ty = self.lw.ck.globals[&d].ty;
                    let ty = self.cty(ty);
                    Some(Place::Glob(GKey::Global(d), ty))
                }
                _ => None,
            },
            ExprKind::Index { obj, index, .. } => {
                let a = self.expr(obj);
                self.hold(a);
                let i = self.expr(index);
                let (av, iv) = (a.v?, i.v?);
                let ety = match self.fb.ty(av).clone() {
                    It::Array(e) => *e,
                    _ => It::I32,
                };
                Some(Place::Elem { arr: av, idx: iv, ety })
            }
            _ => None,
        }
    }

    pub fn finish_place(&mut self, base: usize) {
        while self.held.len() > base {
            let v = self.held.pop().expect("held");
            if !self.dead {
                self.release(v);
            }
        }
    }

    // ------------------------------------------------------------------ reads

    fn parent_read(&mut self, p: &Parent) -> Option<ValueId> {
        match p {
            Parent::Obj(v) => Some(*v),
            Parent::Val(inner) => self.place_borrow(inner),
        }
    }

    fn parent_write(&mut self, p: &Parent) -> Option<ValueId> {
        match p {
            Parent::Obj(v) => Some(*v),
            Parent::Val(inner) => self.place_unique(inner),
        }
    }

    /// Load the current value without retaining it (valid until the next store or call).
    pub fn place_borrow(&mut self, p: &Place) -> Option<ValueId> {
        match p {
            Place::Var(key) => {
                let slot = self.env.get(key).cloned()?;
                match slot {
                    Slot::Ssa { v, .. } => Some(v),
                    Slot::Cell { cell, ty, .. } => {
                        let ft = self.cell_field_ty(&ty);
                        Some(self.fb.emit1(
                            Op::GetField {
                                field: FieldRef {
                                    ty,
                                    variant: None,
                                    field: "v".into(),
                                },
                                obj: cell,
                            },
                            ft,
                        ))
                    }
                    Slot::Cap { env, env_ty, field, cell } => {
                        let fty = self.env_field_ty(&env_ty, &field);
                        let c = self.fb.emit1(
                            Op::GetField {
                                field: FieldRef {
                                    ty: env_ty,
                                    variant: None,
                                    field,
                                },
                                obj: env,
                            },
                            fty,
                        );
                        match cell {
                            Some(cty) => {
                                let ft = self.cell_field_ty(&cty);
                                Some(self.fb.emit1(
                                    Op::GetField {
                                        field: FieldRef {
                                            ty: cty,
                                            variant: None,
                                            field: "v".into(),
                                        },
                                        obj: c,
                                    },
                                    ft,
                                ))
                            }
                            None => Some(c),
                        }
                    }
                }
            }
            Place::Glob(key, ty) => {
                let g = self.lw.global_ir(*key, *ty);
                let sty = if g.wrapped { It::Opt(Box::new(g.vty.clone())) } else { g.vty.clone() };
                let v = self.fb.emit1(Op::GlobalGet { name: g.sym.clone() }, sty);
                Some(if g.wrapped {
                    self.fb.emit1(Op::Unopt { val: v }, g.vty.clone())
                } else {
                    v
                })
            }
            Place::Field { parent, ir_ty, variant, field, fty } => {
                let pv = self.parent_read(parent)?;
                Some(self.fb.emit1(
                    Op::GetField {
                        field: FieldRef {
                            ty: ir_ty.clone(),
                            variant: variant.clone(),
                            field: field.clone(),
                        },
                        obj: pv,
                    },
                    fty.clone(),
                ))
            }
            Place::Elem { arr, idx, ety } => Some(self.fb.emit1(Op::ArrayGet { arr: *arr, idx: *idx }, ety.clone())),
        }
    }

    pub fn place_read_owned(&mut self, p: &Place) -> Opnd {
        match self.place_borrow(p) {
            Some(v) => {
                if self.is_mg(v) {
                    self.retain(v);
                }
                Opnd::owned(v)
            }
            None => Opnd::void(),
        }
    }

    // ------------------------------------------------------------------ writes

    /// Store an owned value (consumed) into the place.
    pub fn place_write(&mut self, p: &Place, v: ValueId) {
        match p {
            Place::Var(key) => {
                let slot = self.env.get(key).cloned();
                match slot {
                    Some(Slot::Ssa { v: old, owned }) => {
                        if owned && self.is_mg(old) {
                            self.release(old);
                        }
                        self.env.insert(*key, Slot::Ssa { v, owned: true });
                    }
                    None => {
                        self.env.insert(*key, Slot::Ssa { v, owned: true });
                    }
                    Some(Slot::Cell { cell, ty, .. }) => {
                        self.fb.emit0(Op::SetField {
                            field: FieldRef {
                                ty,
                                variant: None,
                                field: "v".into(),
                            },
                            obj: cell,
                            val: v,
                            init: false,
                        });
                    }
                    Some(Slot::Cap { env, env_ty, field, cell }) => {
                        let Some(cty) = cell else {
                            self.lw.unsupported("assignment to a by-value capture", None);
                            return;
                        };
                        let fty = self.env_field_ty(&env_ty, &field);
                        let c = self.fb.emit1(
                            Op::GetField {
                                field: FieldRef {
                                    ty: env_ty,
                                    variant: None,
                                    field,
                                },
                                obj: env,
                            },
                            fty,
                        );
                        self.fb.emit0(Op::SetField {
                            field: FieldRef {
                                ty: cty,
                                variant: None,
                                field: "v".into(),
                            },
                            obj: c,
                            val: v,
                            init: false,
                        });
                    }
                }
            }
            Place::Glob(key, ty) => {
                let g = self.lw.global_ir(*key, *ty);
                let val = if g.wrapped {
                    self.fb.emit1(Op::Opt { val: v }, It::Opt(Box::new(g.vty.clone())))
                } else {
                    v
                };
                self.fb.emit0(Op::GlobalSet { name: g.sym.clone(), val });
            }
            Place::Field { parent, ir_ty, variant, field, .. } => {
                let Some(pv) = self.parent_write(parent) else { return };
                self.fb.emit0(Op::SetField {
                    field: FieldRef {
                        ty: ir_ty.clone(),
                        variant: variant.clone(),
                        field: field.clone(),
                    },
                    obj: pv,
                    val: v,
                    init: false,
                });
            }
            Place::Elem { arr, idx, .. } => {
                self.fb.emit0(Op::ArraySet { arr: *arr, idx: *idx, val: v });
            }
        }
    }

    /// Make the struct value stored in the place uniquely referenced (copy-on-write) and
    /// return it (borrowed from the place; valid until the next store or call).
    pub fn place_unique(&mut self, p: &Place) -> Option<ValueId> {
        match p {
            Place::Var(key) => {
                let slot = self.env.get(key).cloned()?;
                if let Slot::Ssa { v, owned } = slot {
                    if !owned {
                        self.retain(v);
                    }
                    let ty = self.fb.ty(v).clone();
                    let u = self.fb.emit1(Op::IsUnique { obj: v }, It::Bool);
                    let copy = self.new_block("cow_copy");
                    let jb = self.new_block("cow_join");
                    let q = self.fb.block_param(jb, "q", ty);
                    self.fb.term(Terminator::BrIf {
                        cond: u,
                        then: tgt(jb, &[v]),
                        els: tgt(copy, &[]),
                    });
                    self.start_block(copy);
                    let c = self.fb.emit1(Op::Clone { obj: v }, self.fb.ty(v).clone());
                    self.release(v);
                    self.fb.term(Terminator::Br(tgt(jb, &[c])));
                    self.start_block(jb);
                    self.env.insert(*key, Slot::Ssa { v: q, owned: true });
                    return Some(q);
                }
                // cell or capture: clone through the cell
                let cur = self.place_borrow(p)?;
                self.cow_with(cur, |fx| fx.place_borrow(p), |fx, c| fx.place_store_raw(p, c))
            }
            Place::Glob(..) => {
                let cur = self.place_borrow(p)?;
                self.cow_with(cur, |fx| fx.place_borrow(p), |fx, c| fx.place_store_raw(p, c))
            }
            Place::Field { parent, ir_ty, variant, field, fty } => {
                let pv = self.parent_write(parent)?;
                let fr = FieldRef {
                    ty: ir_ty.clone(),
                    variant: variant.clone(),
                    field: field.clone(),
                };
                let cur = self.fb.emit1(Op::GetField { field: fr.clone(), obj: pv }, fty.clone());
                let (fr2, fr3, ft2) = (fr.clone(), fr, fty.clone());
                self.cow_with(
                    cur,
                    move |fx| Some(fx.fb.emit1(Op::GetField { field: fr2.clone(), obj: pv }, ft2.clone())),
                    move |fx, c| {
                        fx.fb.emit0(Op::SetField {
                            field: fr3.clone(),
                            obj: pv,
                            val: c,
                            init: false,
                        });
                        Some(())
                    },
                )
            }
            Place::Elem { arr, idx, ety } => {
                let cur = self.fb.emit1(Op::ArrayGet { arr: *arr, idx: *idx }, ety.clone());
                let (a, i, e) = (*arr, *idx, ety.clone());
                self.cow_with(
                    cur,
                    move |fx| Some(fx.fb.emit1(Op::ArrayGet { arr: a, idx: i }, e.clone())),
                    move |fx, c| {
                        fx.fb.emit0(Op::ArraySet { arr: a, idx: i, val: c });
                        Some(())
                    },
                )
            }
        }
    }

    /// Store without any uniqueness handling (used by the copy-on-write sequence itself).
    fn place_store_raw(&mut self, p: &Place, c: ValueId) -> Option<()> {
        match p {
            Place::Var(key) => {
                match self.env.get(key).cloned()? {
                    Slot::Cell { cell, ty, .. } => {
                        self.fb.emit0(Op::SetField {
                            field: FieldRef {
                                ty,
                                variant: None,
                                field: "v".into(),
                            },
                            obj: cell,
                            val: c,
                            init: false,
                        });
                    }
                    Slot::Cap { env, env_ty, field, cell: Some(cty) } => {
                        let fty = self.env_field_ty(&env_ty, &field);
                        let cc = self.fb.emit1(
                            Op::GetField {
                                field: FieldRef {
                                    ty: env_ty,
                                    variant: None,
                                    field,
                                },
                                obj: env,
                            },
                            fty,
                        );
                        self.fb.emit0(Op::SetField {
                            field: FieldRef {
                                ty: cty,
                                variant: None,
                                field: "v".into(),
                            },
                            obj: cc,
                            val: c,
                            init: false,
                        });
                    }
                    _ => return None,
                }
                Some(())
            }
            Place::Glob(key, ty) => {
                let g = self.lw.global_ir(*key, *ty);
                let val = if g.wrapped {
                    self.fb.emit1(Op::Opt { val: c }, It::Opt(Box::new(g.vty.clone())))
                } else {
                    c
                };
                self.fb.emit0(Op::GlobalSet { name: g.sym.clone(), val });
                Some(())
            }
            _ => None,
        }
    }

    /// `if !is_unique(cur) { store(clone(cur)) }; reload`. `cur` is the borrowed current value.
    fn cow_with(
        &mut self,
        cur: ValueId,
        reload: impl Fn(&mut Self) -> Option<ValueId>,
        store: impl Fn(&mut Self, ValueId) -> Option<()>,
    ) -> Option<ValueId> {
        let ty = self.fb.ty(cur).clone();
        let u = self.fb.emit1(Op::IsUnique { obj: cur }, It::Bool);
        let copy = self.new_block("cow_copy");
        let ok = self.new_block("cow_ok");
        self.fb.term(Terminator::BrIf {
            cond: u,
            then: tgt(ok, &[]),
            els: tgt(copy, &[]),
        });
        self.start_block(copy);
        let c = self.fb.emit1(Op::Clone { obj: cur }, ty);
        store(self, c)?;
        self.fb.term(Terminator::Br(tgt(ok, &[])));
        self.start_block(ok);
        reload(self)
    }

    // ------------------------------------------------------------------ assignment forms

    pub fn assign(&mut self, e: &Expr, op: Option<BinOp>, target: &Expr, value: &Expr, want: bool) -> Opnd {
        let base = self.held.len();
        let Some(place) = self.prepare_place(target) else {
            self.unsupported("this assignment target", target.span);
            return Opnd::void();
        };
        let nv: Option<ValueId> = match op {
            None => {
                let v = self.expr(value);
                self.own_of(v)
            }
            Some(bop) => {
                let cur = self.place_read_owned(&place);
                self.hold(cur);
                let rhs = self.expr(value);
                self.unhold(cur);
                match (cur.v, rhs.v) {
                    (Some(cv), Some(rv)) => {
                        let r = self.binop_values(e, bop, cv, rv);
                        self.drop_o(cur);
                        self.drop_o(rhs);
                        self.own_of(r)
                    }
                    _ => None,
                }
            }
        };
        if let Some(nv) = nv {
            self.place_write(&place, nv);
        }
        let res = if want && nv.is_some() {
            self.place_read_owned(&place)
        } else {
            Opnd::void()
        };
        self.finish_place(base);
        res
    }

    pub fn update_expr(&mut self, e: &Expr, op: IncDec, target: &Expr, _prefix: bool) -> Opnd {
        let base = self.held.len();
        let Some(place) = self.prepare_place(target) else {
            self.unsupported("this `++`/`--` target", target.span);
            return Opnd::void();
        };
        let cur = self.place_read_owned(&place);
        if let Some(cv) = cur.v {
            let t = self.fb.ty(cv).clone();
            let one = match &t {
                It::F32 => self.fb.emit1(Op::Const(Const::F32(1.0f32.to_bits())), It::F32),
                It::F64 => self.fb.emit1(Op::Const(Const::F64(1.0f64.to_bits())), It::F64),
                other => self.fb.emit1(Op::Const(Const::Int(1)), other.clone()),
            };
            let bo = if op == IncDec::Inc { IB::Add } else { IB::Sub };
            let r = self.fb.emit1(Op::Bin { op: bo, ovf: Ovf::Trap, a: cv, b: one }, t);
            self.place_write(&place, r);
        }
        let _ = e;
        self.finish_place(base);
        Opnd::void()
    }

    /// Does the type behave as a struct value (copy-on-write)?
    #[allow(dead_code)]
    pub fn is_struct_value(&self, t: Ty) -> bool {
        matches!(self.lw.ck.tys.kind(t), TyKind::Adt(d, _) if matches!(self.lw.ck.defs[d.0 as usize].kind, DefKind::Struct))
    }
}
