//! Calls: user functions and methods, closures, host capability calls, builtins,
//! constructors, closure creation.

use crate::ctx::DefAst;
use crate::fb::{tgt, Fb};
use crate::fx::{strip, AssignScan, Fx, FnKind, Key, Opnd, Slot};
use oxiscript_abi::idl::{IdlFn, IdlRet, IdlTy};
use oxiscript_check::{Builtin, CallInfo, DefId, DefKind, ParamId, Res, Ty, TyKind};
use oxiscript_ir::{BinOp as IrBinOp, CmpOp, Const, Field, FieldRef, Op, Ovf, Terminator, Ty as It, TypeRef, UnOp as IrUnOp, ValueId};
use oxiscript_syntax::visit::Visitor;
use oxiscript_syntax::{ArrowBody, ArrowFn, Expr, ExprKind};

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    pub fn call_expr(&mut self, e: &Expr) -> Opnd {
        let ExprKind::Call { callee, args, optional, .. } = &e.kind else {
            return Opnd::void();
        };
        let c = strip(callee);
        if let Some(b) = self.builtin_of(e.id) {
            return self.builtin_call(e, b, c, args);
        }
        match self.res_of(c.id) {
            Some(Res::Def(d)) if self.def_kind(d) == DefKind::Fn => self.user_call(e, d, None, args),
            Some(Res::Method(d)) => self.method_call(e, c, d, args),
            Some(Res::IfaceMethod { iface, index }) => self.iface_call(e, c, iface, index, args),
            Some(Res::HostFn { host, index }) => self.host_fn_call(e, c, host, index, args),
            _ => self.closure_call(e, c, args, *optional),
        }
    }

    // ------------------------------------------------------------------ user functions

    fn callee_info(&mut self, e: &Expr, d: DefId) -> (String, Vec<Ty>, Ty, Vec<(ParamId, Ty)>, bool) {
        let info: CallInfo = self.mi().calls.get(&e.id).cloned().unwrap_or_default();
        let targs: Vec<Ty> = info.type_args.iter().map(|t| self.cty(*t)).collect();
        let sig = self.lw.ck.fns[&d].clone();
        let map: Vec<(ParamId, Ty)> = sig.tparams.iter().copied().zip(targs.iter().copied()).collect();
        let ptys: Vec<Ty> = sig.params.iter().map(|p| self.lw.subst(p.ty, &map)).collect();
        let ret = self.lw.subst(sig.ret, &map);
        let sym = self.lw.fn_sym(d, targs);
        (sym, ptys, ret, map, sig.mutating)
    }

    /// Lower call arguments (held until `finish_args`), including defaults and elided `void` ones.
    fn eval_args(&mut self, e: &Expr, d: Option<DefId>, args: &[Expr], ptys: &[Ty], map: &[(ParamId, Ty)]) -> Vec<Opnd> {
        let mut ops = Vec::new();
        for (i, a) in args.iter().enumerate() {
            let o = self.expr(a);
            let void_param = ptys.get(i).is_some_and(|t| self.lw.repr(*t).is_none());
            if void_param {
                self.drop_o(o);
                continue;
            }
            let o = if o.v.is_none() {
                match ptys.get(i).and_then(|t| self.lw.repr(*t)) {
                    Some(t) => {
                        let dv = self.dummy(&t);
                        Opnd::borrowed(dv)
                    }
                    None => o,
                }
            } else {
                o
            };
            self.hold(o);
            ops.push(o);
        }
        // trailing defaults
        if let Some(d) = d {
            for k in args.len()..ptys.len() {
                if self.lw.repr(ptys[k]).is_none() {
                    continue;
                }
                let dflt = match self.lw.defast.get(&d) {
                    Some(DefAst::Fn(decl)) => decl.params.get(k).and_then(|p| p.default.as_ref()),
                    _ => None,
                };
                if let Some(x) = dflt {
                    let m = self.lw.ck.defs[d.0 as usize].module as usize;
                    let o = self.with_ctx(m, map.to_vec(), |fx| fx.expr(x));
                    self.hold(o);
                    ops.push(o);
                }
            }
        }
        let _ = e;
        ops
    }

    fn finish_args(&mut self, ops: &[Opnd]) {
        for o in ops.iter().rev() {
            self.unhold(*o);
            self.drop_o(*o);
        }
    }

    fn user_call(&mut self, e: &Expr, d: DefId, recv: Option<Opnd>, args: &[Expr]) -> Opnd {
        if recv.is_none() {
            if let Some(iname) = self.lw.ck.intrinsic_of(d).map(str::to_string) {
                let sig = self.lw.ck.fns[&d].clone();
                let ptys: Vec<Ty> = sig.params.iter().map(|p| p.ty).collect();
                let ops = self.eval_args(e, Some(d), args, &ptys, &[]);
                let argv: Vec<ValueId> = ops.iter().filter_map(|o| o.v).collect();
                let r = self.emit_intrinsic(&iname, &argv, sig.ret, Some(e.span));
                self.finish_args(&ops);
                return r;
            }
        }
        let (sym, ptys, ret, map, _) = self.callee_info(e, d);
        let ops = self.eval_args(e, Some(d), args, &ptys, &map);
        let mut argv: Vec<ValueId> = recv.iter().filter_map(|r| r.v).collect();
        argv.extend(ops.iter().filter_map(|o| o.v));
        let r = self.emit_call_result(sym, argv, ret);
        self.finish_args(&ops);
        r
    }

    /// Lower a bundled std `@intrinsic("...")` call directly to a single OxiIR instruction.
    pub fn emit_intrinsic(&mut self, name: &str, argv: &[ValueId], ret: Ty, span: Option<oxiscript_syntax::Span>) -> Opnd {
        let Some(rt) = self.lw.repr(ret) else {
            self.lw.unsupported(&format!("intrinsic `{name}` with void result"), span);
            return Opnd::void();
        };
        let op = match (name, argv) {
            ("abs", &[a]) => Op::Un { op: IrUnOp::Abs, ovf: Ovf::Trap, a },
            ("sqrt", &[a]) => Op::Un { op: IrUnOp::Sqrt, ovf: Ovf::Trap, a },
            ("floor", &[a]) => Op::Un { op: IrUnOp::Floor, ovf: Ovf::Trap, a },
            ("ceil", &[a]) => Op::Un { op: IrUnOp::Ceil, ovf: Ovf::Trap, a },
            ("trunc", &[a]) => Op::Un { op: IrUnOp::Trunc, ovf: Ovf::Trap, a },
            ("nearest", &[a]) => Op::Un { op: IrUnOp::Nearest, ovf: Ovf::Trap, a },
            ("min", &[a, b]) => Op::Bin { op: IrBinOp::Min, ovf: Ovf::Trap, a, b },
            ("max", &[a, b]) => Op::Bin { op: IrBinOp::Max, ovf: Ovf::Trap, a, b },
            ("clz", &[a]) => Op::Un { op: IrUnOp::Clz, ovf: Ovf::Trap, a },
            ("ctz", &[a]) => Op::Un { op: IrUnOp::Ctz, ovf: Ovf::Trap, a },
            ("popcnt", &[a]) => Op::Un { op: IrUnOp::Popcnt, ovf: Ovf::Trap, a },
            ("bitcast", &[a]) => Op::Bitcast { a },
            ("str_byte", &[s, idx]) => Op::StrByte { s, idx },
            ("str_cmp", &[a, b]) => Op::StrCmp { a, b },
            ("str_from_utf8", &[bytes, len]) => Op::StrFromUtf8 { bytes, len },
            _ => {
                self.lw.unsupported(&format!("compiler intrinsic `{name}`"), span);
                return Opnd::void();
            }
        };
        let owned = matches!(op, Op::StrFromUtf8 { .. });
        let r = self.fb.emit1(op, rt);
        if owned { Opnd::owned(r) } else { Opnd::borrowed(r) }
    }

    /// Emit a call and wrap its single result; a `never` result ends the block.
    pub fn emit_call_result(&mut self, sym: String, argv: Vec<ValueId>, ret: Ty) -> Opnd {
        let rr = self.lw.repr(ret);
        let rets: Vec<It> = rr.iter().cloned().collect();
        let res = self.fb.emit(Op::Call { callee: sym, args: argv }, &rets);
        if matches!(self.kind(ret), TyKind::Never) {
            self.terminate(Terminator::Trap(oxiscript_ir::TrapKind::Unreachable));
            return Opnd::void();
        }
        match res.first() {
            Some(v) => Opnd::owned(*v),
            None => Opnd::void(),
        }
    }

    fn method_call(&mut self, e: &Expr, c: &Expr, d: DefId, args: &[Expr]) -> Opnd {
        let ExprKind::Member { obj, optional, .. } = &c.kind else {
            return self.user_call(e, d, None, args);
        };
        let sig = self.lw.ck.fns[&d].clone();
        if sig.is_static {
            return self.user_call(e, d, None, args);
        }
        if sig.mutating {
            return self.mutating_call(e, d, obj, args);
        }
        if *optional {
            return self.optional_method(e, d, obj, args);
        }
        let recv = self.expr(obj);
        self.hold(recv);
        let r = self.user_call(e, d, Some(recv), args);
        self.unhold(recv);
        self.drop_o(recv);
        r
    }

    fn optional_method(&mut self, e: &Expr, d: DefId, obj: &Expr, args: &[Expr]) -> Opnd {
        let ot = self.ty_of(strip(obj).id);
        let rty = self.ty_of(e.id);
        let rr = self.repr(rty).unwrap_or(It::I32);
        let o = self.expr(obj);
        let ov = o.val();
        let isn = self.fb.emit1(Op::IsNull { val: ov }, It::Bool);
        let some = self.new_block("some");
        let none = self.new_block("none");
        let mut j = self.new_join(Some(rr.clone()), Vec::new());
        self.fb.term(Terminator::BrIf {
            cond: isn,
            then: tgt(none, &[]),
            els: tgt(some, &[]),
        });
        self.start_block(none);
        let nv = self.fb.emit1(Op::Null, rr);
        self.drop_o(o);
        self.jump_join(&mut j, Some(nv));
        self.start_block(some);
        let inner = match self.kind(ot) {
            TyKind::Nullable(i) => i,
            _ => ot,
        };
        let un = self.narrow(o, ot, inner);
        self.hold(un);
        // the callee info of this call is recorded on `e`; its result type is the nullable one
        let (sym, ptys, ret, map, _) = self.callee_info(e, d);
        let ops = self.eval_args(e, Some(d), args, &ptys, &map);
        let mut argv = vec![un.val()];
        argv.extend(ops.iter().filter_map(|o| o.v));
        let r = self.emit_call_result(sym, argv, ret);
        self.finish_args(&ops);
        self.unhold(un);
        self.drop_o(un);
        let w = self.wrap_nullable_if_needed(r, ret, rty);
        let wv = match w.v {
            Some(_) => self.own_of(w).expect("value"),
            None => self.fb.emit1(Op::Null, It::I32),
        };
        self.jump_join(&mut j, Some(wv));
        let r = self.enter_join(&j).expect("join result");
        Opnd::owned(r)
    }

    /// `p.m(args)` for a `mutating` struct method: the receiver is moved into the call and the
    /// updated receiver stored back (ir.md section 4).
    fn mutating_call(&mut self, e: &Expr, d: DefId, obj: &Expr, args: &[Expr]) -> Opnd {
        let (sym, ptys, ret, map, _) = self.callee_info(e, d);
        let base = self.held.len();
        let o = strip(obj);
        // fast path: an owned SSA local is moved
        let fast_key: Option<Key> = match (&o.kind, self.res_of(o.id)) {
            (ExprKind::Ident(_) | ExprKind::This, Some(Res::Local(l) | Res::This(l))) => {
                if matches!(self.env.get(&l.0), Some(Slot::Ssa { owned: true, .. })) {
                    Some(l.0)
                } else {
                    None
                }
            }
            _ => None,
        };
        let ops;
        let result;
        if let Some(key) = fast_key {
            ops = self.eval_args(e, Some(d), args, &ptys, &map);
            let Some(Slot::Ssa { v: rv, .. }) = self.env.get(&key).cloned() else {
                return Opnd::void();
            };
            // an argument that is the very object we move needs its own count
            let aliased = ops.iter().any(|o| o.v == Some(rv));
            if aliased {
                self.retain(rv);
            }
            let mut argv = vec![rv];
            argv.extend(ops.iter().filter_map(|o| o.v));
            let mut rets = vec![self.fb.ty(rv).clone()];
            rets.extend(self.lw.repr(ret));
            let res = self.fb.emit(Op::Call { callee: sym, args: argv }, &rets);
            if aliased {
                // the callee only borrows its arguments: give back the extra count taken above
                self.release(rv);
            }
            self.env.insert(key, Slot::Ssa { v: res[0], owned: true });
            result = res.get(1).map(|v| Opnd::owned(*v)).unwrap_or_else(Opnd::void);
        } else {
            let Some(place) = self.prepare_place(obj) else {
                self.unsupported("the receiver of this `mutating` call", obj.span);
                return Opnd::void();
            };
            let cur = self.place_read_owned(&place);
            self.hold(cur);
            ops = self.eval_args(e, Some(d), args, &ptys, &map);
            self.unhold(cur);
            let cv = cur.val();
            let mut argv = vec![cv];
            argv.extend(ops.iter().filter_map(|o| o.v));
            let mut rets = vec![self.fb.ty(cv).clone()];
            rets.extend(self.lw.repr(ret));
            let res = self.fb.emit(Op::Call { callee: sym, args: argv }, &rets);
            self.place_write(&place, res[0]);
            result = res.get(1).map(|v| Opnd::owned(*v)).unwrap_or_else(Opnd::void);
        }
        self.finish_args(&ops);
        self.finish_place(base);
        result
    }

    fn iface_call(&mut self, e: &Expr, c: &Expr, iface: DefId, index: u32, args: &[Expr]) -> Opnd {
        let ExprKind::Member { obj, .. } = &c.kind else { return Opnd::void() };
        let name = self.lw.ck.ifaces[&iface].methods[index as usize].name.clone();
        let ot = self.ty_of(strip(obj).id);
        let TyKind::Adt(owner, oargs) = self.kind(ot) else {
            if matches!(self.kind(ot), TyKind::Num(_) | TyKind::Bool | TyKind::Str) {
                if name == "hash" && args.is_empty() {
                    let recv = self.expr(obj);
                    let Some(rv) = recv.v else { return Opnd::void() };
                    self.hold(recv);
                    let r = self.emit_prim_hash(rv);
                    self.unhold(recv);
                    self.drop_o(recv);
                    return r;
                }
                if name == "equals" && args.len() == 1 {
                    let recv = self.expr(obj);
                    let Some(rv) = recv.v else { return Opnd::void() };
                    self.hold(recv);
                    let other = self.expr(&args[0]);
                    self.hold(other);
                    let r = self.emit_prim_equals(rv, other.val());
                    self.unhold(other);
                    self.drop_o(other);
                    self.unhold(recv);
                    self.drop_o(recv);
                    return r;
                }
            }
            self.unsupported("calling an interface method on this type", e.span);
            return Opnd::void();
        };
        let oargs: Vec<Ty> = oargs.to_vec();
        let Some(&(_, md)) = self.lw.ck.records.get(&owner).and_then(|r| r.methods.iter().find(|(n, _)| *n == name)) else {
            self.unsupported("a missing method of the bound's implementation", e.span);
            return Opnd::void();
        };
        let sig = self.lw.ck.fns[&md].clone();
        let map: Vec<(ParamId, Ty)> = sig.tparams.iter().copied().zip(oargs.iter().copied()).collect();
        let ptys: Vec<Ty> = sig.params.iter().map(|p| self.lw.subst(p.ty, &map)).collect();
        let ret = self.lw.subst(sig.ret, &map);
        let sym = self.lw.fn_sym(md, oargs);
        if sig.mutating {
            self.unsupported("a `mutating` method called through an interface bound", e.span);
            return Opnd::void();
        }
        let recv = self.expr(obj);
        self.hold(recv);
        let ops = self.eval_args(e, Some(md), args, &ptys, &map);
        let mut argv: Vec<ValueId> = recv.v.into_iter().collect();
        argv.extend(ops.iter().filter_map(|o| o.v));
        let r = self.emit_call_result(sym, argv, ret);
        self.finish_args(&ops);
        self.unhold(recv);
        self.drop_o(recv);
        r
    }

    // ------------------------------------------------------------------ closures

    fn closure_call(&mut self, e: &Expr, c: &Expr, args: &[Expr], optional: bool) -> Opnd {
        let ct = self.ty_of(c.id);
        let rty = self.ty_of(e.id);
        let fo = self.expr(c);
        let Some(fv) = fo.v else { return Opnd::void() };
        let (inner, nullable) = match self.kind(ct) {
            TyKind::Nullable(i) => (i, true),
            _ => (ct, false),
        };
        let (ptys, fret) = match self.kind(inner) {
            TyKind::Fn(ps, r) => (ps.to_vec(), r),
            _ => {
                self.unsupported("calling this value", e.span);
                return Opnd::void();
            }
        };
        if nullable && optional {
            let rr = self.repr(rty).unwrap_or(It::I32);
            let isn = self.fb.emit1(Op::IsNull { val: fv }, It::Bool);
            let some = self.new_block("some");
            let none = self.new_block("none");
            let mut j = self.new_join(Some(rr.clone()), Vec::new());
            self.fb.term(Terminator::BrIf {
                cond: isn,
                then: tgt(none, &[]),
                els: tgt(some, &[]),
            });
            self.start_block(none);
            let nv = self.fb.emit1(Op::Null, rr);
            self.drop_o(fo);
            self.jump_join(&mut j, Some(nv));
            self.start_block(some);
            let un = self.narrow(fo, ct, inner);
            let r = self.call_closure_value(un, &ptys, fret, args, e);
            self.drop_o(un);
            let w = self.wrap_nullable_if_needed(r, fret, rty);
            let wv = match w.v {
                Some(_) => self.own_of(w).expect("value"),
                None => self.fb.emit1(Op::Null, It::I32),
            };
            self.jump_join(&mut j, Some(wv));
            let r = self.enter_join(&j).expect("join result");
            return Opnd::owned(r);
        }
        let fo2 = if nullable { self.narrow(fo, ct, inner) } else { fo };
        self.hold(fo2);
        let r = self.call_closure_value(fo2, &ptys, fret, args, e);
        self.unhold(fo2);
        self.drop_o(fo2);
        r
    }

    fn call_closure_value(&mut self, fo: Opnd, ptys: &[Ty], fret: Ty, args: &[Expr], e: &Expr) -> Opnd {
        let ops = self.eval_args(e, None, args, ptys, &[]);
        let argv: Vec<ValueId> = ops.iter().filter_map(|o| o.v).collect();
        let rr = self.repr(fret);
        let rets: Vec<It> = rr.iter().cloned().collect();
        let res = self.fb.emit(Op::CallClosure { callee: fo.val(), args: argv }, &rets);
        self.finish_args(&ops);
        if matches!(self.kind(fret), TyKind::Never) {
            self.terminate(Terminator::Trap(oxiscript_ir::TrapKind::Unreachable));
            return Opnd::void();
        }
        match res.first() {
            Some(v) => Opnd::owned(*v),
            None => Opnd::void(),
        }
    }

    /// A function used as a value: a closure with an empty environment around a thunk.
    pub fn fn_value(&mut self, d: DefId, e: &Expr) -> Opnd {
        let fty = self.ty_of(e.id);
        let Some(It::Fn(sig)) = self.lw.repr(fty) else {
            self.unsupported("this function value", e.span);
            return Opnd::void();
        };
        let sym = self.lw.fn_sym(d, Vec::new());
        let key = format!("thunk:{sym}");
        let env = self.lw.empty_env();
        let tsym = if let Some(t) = self.lw.thunks.get(&key) {
            t.clone()
        } else {
            let id = self.lw.next_id();
            let tsym = format!("thunk.{id}");
            let mut fb = Fb::new(&tsym);
            let envp = fb.param("env", It::Ref(env.clone()), false);
            let _ = envp;
            let params: Vec<ValueId> = sig.params.iter().map(|t| fb.param("p", t.clone(), false)).collect();
            fb.f.rets = sig.rets.clone();
            let res = fb.emit(Op::Call { callee: sym, args: params }, &sig.rets);
            fb.term(Terminator::Ret(res));
            self.lw.out.funcs.push(fb.finish());
            self.lw.thunks.insert(key, tsym.clone());
            tsym
        };
        let r = self.fb.emit1(
            Op::ClosureNew {
                env,
                func: tsym,
                captures: Vec::new(),
            },
            It::Fn(sig),
        );
        Opnd::owned(r)
    }

    pub fn arrow(&mut self, e: &Expr, a: &ArrowFn) -> Opnd {
        let fty = self.ty_of(e.id);
        let (TyKind::Fn(ptys, rty), Some(irfn)) = (self.kind(fty), self.lw.repr(fty)) else {
            self.unsupported("this closure type", e.span);
            return Opnd::void();
        };
        let ptys: Vec<Ty> = ptys.to_vec();
        let caps = self.mi().closures.get(&e.id).cloned().unwrap_or_default().captures;
        let mut fields: Vec<Field> = Vec::new();
        let mut vals: Vec<ValueId> = Vec::new();
        let mut cap_slots: Vec<(Key, Option<String>)> = Vec::new();
        for cap in &caps {
            let key = cap.local.0;
            let i = fields.len();
            let name = format!("c{i}");
            match self.env.get(&key).cloned() {
                Some(Slot::Cell { cell, ty, .. }) => {
                    self.retain(cell);
                    fields.push(Field { name, ty: It::Ref(ty.clone()) });
                    vals.push(cell);
                    cap_slots.push((key, Some(ty)));
                }
                Some(Slot::Cap { env, env_ty, field, cell: Some(cty) }) => {
                    let fty0 = self.env_field_ty(&env_ty, &field);
                    let cv = self.fb.emit1(
                        Op::GetField {
                            field: FieldRef {
                                ty: env_ty,
                                variant: None,
                                field,
                            },
                            obj: env,
                        },
                        fty0,
                    );
                    self.retain(cv);
                    fields.push(Field { name, ty: It::Ref(cty.clone()) });
                    vals.push(cv);
                    cap_slots.push((key, Some(cty)));
                }
                Some(_) => {
                    let o = self.read_key(key, true);
                    let Some(v) = o.v else { continue };
                    fields.push(Field {
                        name,
                        ty: self.fb.ty(v).clone(),
                    });
                    vals.push(v);
                    cap_slots.push((key, None));
                }
                None => {
                    self.unsupported("capturing this variable", e.span);
                }
            }
        }
        let env_ty = self.lw.closure_env_type(fields);
        let id = self.lw.next_id();
        let sym = format!("lam.{id}");
        let ret_ir = self.lw.repr(rty);
        let (m, subst) = (self.m, self.subst.clone());
        let func = {
            let lw: &mut crate::ctx::Lw = &mut *self.lw;
            let mut cx = Fx::new(lw, &sym, m, subst);
            cx.kind = FnKind::Closure;
            cx.ret_ty = rty;
            let envp = cx.fb.param("env", It::Ref(env_ty.clone()), false);
            for (i, (key, cty)) in cap_slots.iter().enumerate() {
                cx.env.insert(
                    *key,
                    Slot::Cap {
                        env: envp,
                        env_ty: env_ty.clone(),
                        field: format!("c{i}"),
                        cell: cty.clone(),
                    },
                );
            }
            cx.fb.f.rets = ret_ir.iter().cloned().collect();
            cx.scan_assigned_body(|sc| match &a.body {
                ArrowBody::Expr(x) => sc.visit_expr(x),
                ArrowBody::Block(b) => sc.visit_block(b),
            });
            for (pi, p) in a.params.iter().enumerate() {
                let Some(&l) = cx.mi().binding_local.get(&p.name.id) else { continue };
                if let Some(it) = cx.repr(ptys[pi]) {
                    let v = cx.fb.param("p", it, false);
                    cx.bind_param(l, v);
                }
            }
            match &a.body {
                ArrowBody::Expr(x) => {
                    let o = cx.expr(x);
                    if ret_ir.is_some() {
                        let v = cx.own_of(o);
                        cx.emit_return(v);
                    } else {
                        cx.drop_o(o);
                        cx.emit_return(None);
                    }
                }
                ArrowBody::Block(b) => {
                    cx.block_stmts(b);
                    if !cx.dead {
                        cx.emit_return(None);
                    }
                }
            }
            cx.finish()
        };
        self.lw.out.funcs.push(func);
        let r = self.fb.emit1(
            Op::ClosureNew {
                env: env_ty,
                func: sym,
                captures: vals,
            },
            irfn,
        );
        Opnd::owned(r)
    }

    /// Compute the locals assigned in a body (outside closures) for ownership decisions.
    pub fn scan_assigned_body(&mut self, f: impl FnOnce(&mut AssignScan<'_, '_>)) {
        let mut sc = AssignScan {
            mi: &self.lw.ck.modules[self.m],
            tys: &self.lw.ck.tys,
            defs: &self.lw.ck.defs,
            fns: &self.lw.ck.fns,
            keys: Vec::new(),
        };
        f(&mut sc);
        for k in sc.keys {
            self.assigned_fn.insert(k);
        }
    }

    // ------------------------------------------------------------------ constructors

    pub fn new_expr(&mut self, e: &Expr, args: &[Expr]) -> Opnd {
        let ty = self.ty_of(e.id);
        let TyKind::Adt(class, cargs) = self.kind(ty) else {
            self.unsupported("`new` of this type", e.span);
            return Opnd::void();
        };
        let cargs: Vec<Ty> = cargs.to_vec();
        let ctor = self.lw.ck.records.get(&class).and_then(|r| r.ctor);
        match ctor {
            Some(cd) => {
                let sig = self.lw.ck.fns[&cd].clone();
                let map: Vec<(ParamId, Ty)> = self.lw.tparam_map(class, &cargs);
                let ptys: Vec<Ty> = sig.params.iter().map(|p| self.lw.subst(p.ty, &map)).collect();
                let sym = self.lw.fn_sym(cd, cargs);
                let ops = self.eval_args(e, Some(cd), args, &ptys, &map);
                let argv: Vec<ValueId> = ops.iter().filter_map(|o| o.v).collect();
                let r = self.emit_call_result(sym, argv, ty);
                self.finish_args(&ops);
                r
            }
            None => {
                let sym = self.lw.fn_sym(class, cargs);
                let r = self.emit_call_result(sym, Vec::new(), ty);
                for a in args {
                    let o = self.expr(a);
                    self.drop_o(o);
                }
                r
            }
        }
    }

    // ------------------------------------------------------------------ host calls

    fn host_fn_call(&mut self, e: &Expr, c: &Expr, host: DefId, index: u32, args: &[Expr]) -> Opnd {
        let ExprKind::Member { obj, .. } = &c.kind else { return Opnd::void() };
        let hs = self.lw.ck.hosts[&host].clone();
        let (fname, fty) = hs.fns[index as usize].clone();
        let Some(iface) = self.lw.find_interface(&hs.module, hs.version) else {
            self.lw.err(
                "OXS-E0400",
                format!("host interface `{}` @{} is not granted", hs.module, hs.version),
                Some(e.span),
            );
            return Opnd::void();
        };
        let idl: IdlFn = match self.lw.interfaces[iface].find_fn(&fname) {
            Some(f) => f.clone(),
            None => {
                self.unsupported("a host function missing from the interface", e.span);
                return Opnd::void();
            }
        };
        let TyKind::Fn(ptys, _) = self.kind(fty) else { return Opnd::void() };
        let ptys: Vec<Ty> = ptys.to_vec();
        let handle = self.expr(obj);
        let Some(hv) = handle.v else { return Opnd::void() };
        let ops = self.eval_args(e, None, args, &ptys, &[]);
        // callbacks are consumed by the host
        let mut argv = vec![hv];
        for (i, o) in ops.iter().enumerate() {
            let Some(v) = o.v else { continue };
            if matches!(idl.params.get(i).map(|p| &p.ty), Some(IdlTy::Callback(_))) {
                // pass an owned closure and stop tracking it
                self.unhold(*o);
                if !o.own {
                    self.retain(v);
                }
            }
            argv.push(v);
        }
        let callback_idx: Vec<usize> = idl
            .params
            .iter()
            .enumerate()
            .filter(|(_, p)| matches!(p.ty, IdlTy::Callback(_)))
            .map(|(i, _)| i)
            .collect();
        let sym = self.lw.host_import(iface, &idl);
        let rty = self.ty_of(e.id);
        let res = match &idl.ret {
            IdlRet::Void => {
                self.fb.emit0(Op::Call { callee: sym, args: argv });
                Opnd::void()
            }
            IdlRet::Value(t) => {
                let ity = self.idl_ty(t, iface);
                let v = self.fb.emit1(Op::Call { callee: sym, args: argv }, ity);
                Opnd::owned(v)
            }
            IdlRet::Result(None) => {
                let st = self.fb.emit1(Op::Call { callee: sym, args: argv }, It::I32);
                let r = self.build_result(rty, st, None);
                Opnd::owned(r)
            }
            IdlRet::Result(Some(IdlTy::Str)) => self.out_buffer_call(sym, argv, rty, true),
            IdlRet::Result(Some(IdlTy::Bytes)) => self.out_buffer_call(sym, argv, rty, false),
            IdlRet::Result(Some(t)) => {
                let ity = self.idl_ty(t, iface);
                let vs = self.fb.emit(Op::Call { callee: sym, args: argv }, &[It::I32, ity]);
                let r = self.build_result(rty, vs[0], Some(vs[1]));
                Opnd::owned(r)
            }
        };
        // release operands (callbacks were consumed)
        for (i, o) in ops.iter().enumerate().rev() {
            if callback_idx.contains(&i) {
                continue;
            }
            self.unhold(*o);
            self.drop_o(*o);
        }
        self.drop_o(handle);
        res
    }

    fn idl_ty(&mut self, t: &IdlTy, iface: usize) -> It {
        let itf = self.lw.interfaces[iface].clone();
        let sig = oxiscript_ir::host::import_sig(
            &IdlFn {
                name: String::new(),
                params: Vec::new(),
                ret: IdlRet::Value(t.clone()),
            },
            &itf,
        );
        sig.rets.first().cloned().unwrap_or(It::I32)
    }

    /// `Result<T, i32>` from a status (and a value): ok when the status is 0.
    fn build_result(&mut self, rty: Ty, status: ValueId, val: Option<ValueId>) -> ValueId {
        let (u, uargs) = match self.kind(rty) {
            TyKind::Adt(d, a) => (d, a.to_vec()),
            _ => {
                self.unsupported("a host `result` outside a Result type", oxiscript_syntax::Span::DUMMY);
                return status;
            }
        };
        let un = self.lw.union_name(u, &uargs);
        let ok_n = self.lw.variant_name(&un, 0);
        let er_n = self.lw.variant_name(&un, 1);
        let zero = self.const_i32(0);
        let is_ok = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: status, b: zero }, It::Bool);
        let okb = self.new_block("rok");
        let erb = self.new_block("rerr");
        let jb = self.new_block("rjoin");
        let rp = self.fb.block_param(jb, "r", It::Ref(un.clone()));
        self.fb.term(Terminator::BrIf {
            cond: is_ok,
            then: tgt(okb, &[]),
            els: tgt(erb, &[]),
        });
        self.start_block(okb);
        let o = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: un.clone(),
                variant: Some(ok_n.clone()),
            }),
            It::Ref(un.clone()),
        );
        if let Some(v) = val {
            if self.is_mg(v) {
                // a value built by the call is owned: move it into the object
            }
            self.fb.emit0(Op::SetField {
                field: FieldRef {
                    ty: un.clone(),
                    variant: Some(ok_n),
                    field: "value".into(),
                },
                obj: o,
                val: v,
                init: true,
            });
        }
        self.fb.term(Terminator::Br(tgt(jb, &[o])));
        self.start_block(erb);
        let er = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: un.clone(),
                variant: Some(er_n.clone()),
            }),
            It::Ref(un.clone()),
        );
        self.fb.emit0(Op::SetField {
            field: FieldRef {
                ty: un,
                variant: Some(er_n),
                field: "error".into(),
            },
            obj: er,
            val: status,
            init: true,
        });
        if let Some(v) = val {
            if self.is_mg(v) {
                self.release(v);
            }
        }
        self.fb.term(Terminator::Br(tgt(jb, &[er])));
        self.start_block(jb);
        rp
    }

    /// The out-buffer protocol with its retry loop (`result<str>` / `result<bytes>`).
    fn out_buffer_call(&mut self, sym: String, argv: Vec<ValueId>, rty: Ty, is_str: bool) -> Opnd {
        let (u, uargs) = match self.kind(rty) {
            TyKind::Adt(d, a) => (d, a.to_vec()),
            _ => {
                self.unsupported("a host `result` outside a Result type", oxiscript_syntax::Span::DUMMY);
                return Opnd::void();
            }
        };
        let un = self.lw.union_name(u, &uargs);
        let ok_n = self.lw.variant_name(&un, 0);
        let er_n = self.lw.variant_name(&un, 1);
        let bytes = It::Array(Box::new(It::U8));
        let cap0 = self.const_i32(64);
        let tries0 = self.const_i32(0);
        let lp = self.new_block("obuf");
        let cap = self.fb.block_param(lp, "cap", It::I32);
        let tries = self.fb.block_param(lp, "tries", It::I32);
        let jb = self.new_block("ojoin");
        let rp = self.fb.block_param(jb, "r", It::Ref(un.clone()));
        self.fb.term(Terminator::Br(tgt(lp, &[cap0, tries0])));
        self.start_block(lp);
        let buf = self.fb.emit1(Op::ArrayNew { len: cap }, bytes.clone());
        let mut a = argv;
        a.push(buf);
        let vs = self.fb.emit(Op::Call { callee: sym, args: a }, &[It::I32, It::I32]);
        let (st, len) = (vs[0], vs[1]);
        let zero = self.const_i32(0);
        let is_ok = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: st, b: zero }, It::Bool);
        let okb = self.new_block("ook");
        let chk = self.new_block("ochk");
        self.fb.term(Terminator::BrIf {
            cond: is_ok,
            then: tgt(okb, &[]),
            els: tgt(chk, &[]),
        });
        // success
        self.start_block(okb);
        let payload: ValueId = if is_str {
            self.fb.emit1(Op::StrFromUtf8 { bytes: buf, len }, It::Str)
        } else {
            // bytes: copy the first `len` bytes into a fresh array
            let h = self.lw.array_helper("slice", &It::U8, None);
            self.fb.emit1(
                Op::Call {
                    callee: h,
                    args: vec![buf, zero, len],
                },
                bytes.clone(),
            )
        };
        self.release(buf);
        let o = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: un.clone(),
                variant: Some(ok_n.clone()),
            }),
            It::Ref(un.clone()),
        );
        self.fb.emit0(Op::SetField {
            field: FieldRef {
                ty: un.clone(),
                variant: Some(ok_n),
                field: "value".into(),
            },
            obj: o,
            val: payload,
            init: true,
        });
        self.fb.term(Terminator::Br(tgt(jb, &[o])));
        // not ok: retry once the host asked for a bigger buffer (status 1), a few times at most
        self.start_block(chk);
        self.release(buf);
        let one = self.const_i32(1);
        let is_small = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: st, b: one }, It::Bool);
        let four = self.const_i32(4);
        let few = self.fb.emit1(Op::Cmp { op: CmpOp::Lt, a: tries, b: four }, It::Bool);
        let can = self.fb.emit1(Op::Bin { op: oxiscript_ir::BinOp::And, ovf: Ovf::Trap, a: is_small, b: few }, It::Bool);
        let retry = self.new_block("oretry");
        let fail = self.new_block("ofail");
        self.fb.term(Terminator::BrIf {
            cond: can,
            then: tgt(retry, &[]),
            els: tgt(fail, &[]),
        });
        self.start_block(retry);
        let bigger = self.fb.emit1(Op::Cmp { op: CmpOp::Gt, a: len, b: cap }, It::Bool);
        let two = self.const_i32(2);
        let dbl = self.fb.emit1(Op::Bin { op: oxiscript_ir::BinOp::Mul, ovf: Ovf::Wrap, a: cap, b: two }, It::I32);
        let ncap = self.fb.emit1(Op::Select { cond: bigger, a: len, b: dbl }, It::I32);
        let t1 = self.fb.emit1(Op::Bin { op: oxiscript_ir::BinOp::Add, ovf: Ovf::Trap, a: tries, b: one }, It::I32);
        self.fb.term(Terminator::Br(tgt(lp, &[ncap, t1])));
        self.start_block(fail);
        let er = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: un.clone(),
                variant: Some(er_n.clone()),
            }),
            It::Ref(un.clone()),
        );
        self.fb.emit0(Op::SetField {
            field: FieldRef {
                ty: un,
                variant: Some(er_n),
                field: "error".into(),
            },
            obj: er,
            val: st,
            init: true,
        });
        self.fb.term(Terminator::Br(tgt(jb, &[er])));
        self.start_block(jb);
        Opnd::owned(rp)
    }

    // ------------------------------------------------------------------ builtins

    fn recv_of<'x>(&self, c: &'x Expr) -> Option<&'x Expr> {
        match &strip(c).kind {
            ExprKind::Member { obj, .. } => Some(obj),
            _ => None,
        }
    }

    fn builtin_call(&mut self, e: &Expr, b: Builtin, c: &Expr, args: &[Expr]) -> Opnd {
        if b == Builtin::Range {
            return self.range_call(args);
        }
        let Some(obj) = self.recv_of(c) else {
            self.unsupported("this builtin call", e.span);
            return Opnd::void();
        };
        let rty = self.ty_of(e.id);
        let recv = self.expr(obj);
        let Some(rv) = recv.v else { return Opnd::void() };
        self.hold(recv);
        let out: Opnd = match b {
            Builtin::ArrayPush => {
                let v = self.expr(&args[0]);
                if let Some(v) = self.own_of(v) {
                    self.fb.emit0(Op::ArrayPush { arr: rv, val: v });
                }
                Opnd::void()
            }
            Builtin::ArrayPop => {
                let rr = self.repr(rty).unwrap_or(It::I32);
                let ot = self.ty_of(strip(obj).id);
                let elem = match self.kind(ot) {
                    TyKind::Array(t) => t,
                    _ => ot,
                };
                let n = self.fb.emit1(Op::ArrayLen { arr: rv }, It::I32);
                let z = self.const_i32(0);
                let empty = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: n, b: z }, It::Bool);
                let none = self.new_block("none");
                let some = self.new_block("some");
                let mut j = self.new_join(Some(rr.clone()), Vec::new());
                self.fb.term(Terminator::BrIf {
                    cond: empty,
                    then: tgt(none, &[]),
                    els: tgt(some, &[]),
                });
                self.start_block(none);
                let nv = self.fb.emit1(Op::Null, rr);
                self.jump_join(&mut j, Some(nv));
                self.start_block(some);
                let et = match self.fb.ty(rv).clone() {
                    It::Array(el) => *el,
                    _ => It::I32,
                };
                let pv = self.fb.emit1(Op::ArrayPop { arr: rv }, et);
                let w = self.wrap_nullable_if_needed(Opnd::owned(pv), elem, rty);
                let wv = self.own_of(w).expect("value");
                self.jump_join(&mut j, Some(wv));
                let r = self.enter_join(&j).expect("join result");
                Opnd::owned(r)
            }
            Builtin::ArrayForEach
            | Builtin::ArrayMap
            | Builtin::ArrayFilter
            | Builtin::ArrayReduce
            | Builtin::ArraySome
            | Builtin::ArrayEvery => {
                let t = match self.fb.ty(rv).clone() {
                    It::Array(el) => *el,
                    _ => It::I32,
                };
                let f = self.expr(&args[0]);
                self.hold(f);
                let mut argv = vec![rv, f.val()];
                let init = if b == Builtin::ArrayReduce {
                    let i = self.expr(&args[1]);
                    self.hold(i);
                    argv.push(i.val());
                    Some(i)
                } else {
                    None
                };
                let (kind, u, rets): (&str, Option<It>, Vec<It>) = match b {
                    Builtin::ArrayForEach => ("foreach", None, vec![]),
                    Builtin::ArrayMap => {
                        let ur = match self.repr(rty) {
                            Some(It::Array(el)) => *el,
                            _ => It::I32,
                        };
                        ("map", Some(ur.clone()), vec![It::Array(Box::new(ur))])
                    }
                    Builtin::ArrayFilter => ("filter", None, vec![It::Array(Box::new(t.clone()))]),
                    Builtin::ArrayReduce => {
                        let ur = self.repr(rty).unwrap_or(It::I32);
                        ("reduce", Some(ur.clone()), vec![ur])
                    }
                    Builtin::ArraySome => ("some", None, vec![It::Bool]),
                    _ => ("every", None, vec![It::Bool]),
                };
                let h = self.lw.array_helper(kind, &t, u.as_ref());
                let res = self.fb.emit(Op::Call { callee: h, args: argv }, &rets);
                if let Some(i) = init {
                    self.unhold(i);
                    self.drop_o(i);
                }
                self.unhold(f);
                self.drop_o(f);
                match res.first() {
                    Some(v) => Opnd::owned(*v),
                    None => Opnd::void(),
                }
            }
            Builtin::ArraySlice => {
                let t = match self.fb.ty(rv).clone() {
                    It::Array(el) => *el,
                    _ => It::I32,
                };
                let lo = self.expr(&args[0]);
                let hi = self.expr(&args[1]);
                let h = self.lw.array_helper("slice", &t, None);
                let r = self.fb.emit1(
                    Op::Call {
                        callee: h,
                        args: vec![rv, lo.val(), hi.val()],
                    },
                    It::Array(Box::new(t)),
                );
                Opnd::owned(r)
            }
            Builtin::ArrayConcat => {
                let t = match self.fb.ty(rv).clone() {
                    It::Array(el) => *el,
                    _ => It::I32,
                };
                let o = self.expr(&args[0]);
                self.hold(o);
                let h = self.lw.array_helper("concat", &t, None);
                let r = self.fb.emit1(
                    Op::Call {
                        callee: h,
                        args: vec![rv, o.val()],
                    },
                    It::Array(Box::new(t)),
                );
                self.unhold(o);
                self.drop_o(o);
                Opnd::owned(r)
            }
            Builtin::StrSlice => {
                let lo = self.expr(&args[0]);
                let hi = self.expr(&args[1]);
                let r = self.fb.emit1(Op::StrSlice { s: rv, lo: lo.val(), hi: hi.val() }, It::Str);
                Opnd::owned(r)
            }
            Builtin::StrStartsWith | Builtin::StrEndsWith | Builtin::StrIncludes | Builtin::StrIndexOf => {
                let p = self.expr(&args[0]);
                self.hold(p);
                let name = match b {
                    Builtin::StrStartsWith => "str_starts",
                    Builtin::StrEndsWith => "str_ends",
                    _ => "str_index_of",
                };
                let h = self.lw.helper(name);
                let rt = if name == "str_index_of" { It::I32 } else { It::Bool };
                let r = self.fb.emit1(Op::Call { callee: h, args: vec![rv, p.val()] }, rt);
                self.unhold(p);
                self.drop_o(p);
                if b == Builtin::StrIncludes {
                    let z = self.const_i32(0);
                    let ge = self.fb.emit1(Op::Cmp { op: CmpOp::Ge, a: r, b: z }, It::Bool);
                    Opnd::borrowed(ge)
                } else {
                    Opnd::borrowed(r)
                }
            }
            Builtin::StrBytes => {
                let h = self.lw.helper("str_bytes");
                let r = self.fb.emit1(Op::Call { callee: h, args: vec![rv] }, It::Array(Box::new(It::U8)));
                Opnd::owned(r)
            }
            Builtin::StrChars => {
                let h = self.lw.helper("str_chars");
                let r = self.fb.emit1(Op::Call { callee: h, args: vec![rv] }, It::Array(Box::new(It::U32)));
                Opnd::owned(r)
            }
            Builtin::NumToString | Builtin::BoolToString => {
                let s = self.to_string_op(Opnd { v: Some(rv), own: false });
                s
            }
            Builtin::NumWrapping | Builtin::NumSaturating => {
                let info = self.mi().calls.get(&e.id).cloned().unwrap_or_default();
                let target = info.type_args.first().map(|t| self.cty(*t));
                let tr = target.and_then(|t| self.lw.repr(t));
                match tr {
                    Some(tr) if tr != self.fb.ty(rv).clone() => {
                        let ovf = if b == Builtin::NumWrapping { Ovf::Wrap } else { Ovf::Sat };
                        let r = self.fb.emit1(Op::Conv { ovf, a: rv }, tr);
                        Opnd::borrowed(r)
                    }
                    _ => Opnd::borrowed(rv),
                }
            }
            Builtin::Downcast => self.downcast(e, rv, rty),
            Builtin::PrimHash => self.emit_prim_hash(rv),
            Builtin::PrimEquals => {
                let other = self.expr(&args[0]);
                self.hold(other);
                let r = self.emit_prim_equals(rv, other.val());
                self.unhold(other);
                self.drop_o(other);
                r
            }
            _ => {
                self.unsupported("this builtin", e.span);
                Opnd::void()
            }
        };
        // a result that borrows from the receiver would dangle: make everything owned or scalar
        self.unhold(recv);
        self.drop_o(recv);
        out
    }

    fn downcast(&mut self, e: &Expr, rv: ValueId, rty: Ty) -> Opnd {
        let info = self.mi().calls.get(&e.id).cloned().unwrap_or_default();
        let Some(target) = info.type_args.first().map(|t| self.cty(*t)) else { return Opnd::void() };
        let mask_idx: Option<u32> = match self.kind(target) {
            TyKind::Adt(d, _) => match self.def_kind(d) {
                DefKind::Variant { index, .. } => Some(index),
                _ => None,
            },
            _ => None,
        };
        let rr = self.repr(rty).unwrap_or(It::I32);
        let Some(vi) = mask_idx else {
            self.unsupported("`downcast` to a union", e.span);
            return Opnd::void();
        };
        let tag = self.fb.emit1(Op::Tag { obj: rv }, It::U32);
        let ix = self.fb.emit1(Op::Const(Const::Int(i128::from(vi))), It::U32);
        let hit = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: tag, b: ix }, It::Bool);
        let yes = self.new_block("dc_yes");
        let no = self.new_block("dc_no");
        let mut j = self.new_join(Some(rr.clone()), Vec::new());
        self.fb.term(Terminator::BrIf {
            cond: hit,
            then: tgt(yes, &[]),
            els: tgt(no, &[]),
        });
        self.start_block(yes);
        self.retain(rv);
        let o = self.fb.emit1(Op::Opt { val: rv }, rr.clone());
        self.jump_join(&mut j, Some(o));
        self.start_block(no);
        let n = self.fb.emit1(Op::Null, rr);
        self.jump_join(&mut j, Some(n));
        let r = self.enter_join(&j).expect("join result");
        Opnd::owned(r)
    }

    fn emit_prim_hash(&mut self, rv: ValueId) -> Opnd {
        let ty = self.fb.ty(rv).clone();
        if ty == It::Str {
            let h = self.lw.helper("hash_str");
            let r = self.fb.emit1(Op::Call { callee: h, args: vec![rv] }, It::U64);
            return Opnd::borrowed(r);
        }
        let u = match ty {
            It::U64 => rv,
            It::Bool => {
                let one = self.fb.emit1(Op::Const(Const::Int(1)), It::U64);
                let zero = self.fb.emit1(Op::Const(Const::Int(0)), It::U64);
                self.fb.emit1(Op::Select { cond: rv, a: one, b: zero }, It::U64)
            }
            It::F64 => {
                // Canonicalise -0.0 -> +0.0 so hash(-0.0) === hash(+0.0) matches IEEE ===.
                let pz = self.fb.emit1(Op::Const(Const::F64(0.0f64.to_bits())), It::F64);
                let norm = self.fb.emit1(Op::Bin { op: IrBinOp::Add, ovf: Ovf::Trap, a: rv, b: pz }, It::F64);
                self.fb.emit1(Op::Bitcast { a: norm }, It::U64)
            }
            It::F32 => {
                let pz = self.fb.emit1(Op::Const(Const::F32(0.0f32.to_bits())), It::F32);
                let norm = self.fb.emit1(Op::Bin { op: IrBinOp::Add, ovf: Ovf::Trap, a: rv, b: pz }, It::F32);
                let bits = self.fb.emit1(Op::Bitcast { a: norm }, It::U32);
                self.fb.emit1(Op::Conv { ovf: Ovf::Wrap, a: bits }, It::U64)
            }
            t if t.is_int() => self.fb.emit1(Op::Conv { ovf: Ovf::Wrap, a: rv }, It::U64),
            _ => {
                self.lw.unsupported("hashing this type", None);
                return Opnd::void();
            }
        };
        let h = self.lw.helper("hash_u64");
        let r = self.fb.emit1(Op::Call { callee: h, args: vec![u] }, It::U64);
        Opnd::borrowed(r)
    }

    fn emit_prim_equals(&mut self, a: ValueId, b: ValueId) -> Opnd {
        let ty = self.fb.ty(a).clone();
        let r = if ty == It::Str {
            self.fb.emit1(Op::StrEq { a, b }, It::Bool)
        } else if matches!(ty, It::F64 | It::F32) {
            // SameValueZero for float key equality: -0.0 equals +0.0, and NaN equals NaN.
            // Matches emit_prim_hash which canonicalises -0.0 -> +0.0 and NaN -> canonical qNaN.
            let eq = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a, b }, It::Bool);
            let a_nan = self.fb.emit1(Op::Cmp { op: CmpOp::Ne, a, b: a }, It::Bool);
            let b_nan = self.fb.emit1(Op::Cmp { op: CmpOp::Ne, a: b, b }, It::Bool);
            let both_nan = self.fb.emit1(
                Op::Bin { op: IrBinOp::And, ovf: Ovf::Trap, a: a_nan, b: b_nan },
                It::Bool,
            );
            self.fb.emit1(
                Op::Bin { op: IrBinOp::Or, ovf: Ovf::Trap, a: eq, b: both_nan },
                It::Bool,
            )
        } else {
            self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a, b }, It::Bool)
        };
        Opnd::borrowed(r)
    }

    fn range_call(&mut self, args: &[Expr]) -> Opnd {
        let a = self.expr(&args[0]);
        let b = self.expr(&args[1]);
        let h = self.lw.helper("range");
        let r = self.fb.emit1(
            Op::Call {
                callee: h,
                args: vec![a.val(), b.val()],
            },
            It::Array(Box::new(It::I32)),
        );
        Opnd::owned(r)
    }
}
