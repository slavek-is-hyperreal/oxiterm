//! Expression lowering.

use crate::ctx::{DefAst, GKey};
use crate::fb::tgt;
use crate::fx::{strip, AssignScan, Fx, Key, Opnd, Slot, THIS_FIELD};
use oxiscript_check::{Builtin, DefId, DefKind, Discr, LocalId, ParamId, Res, Ty, TyKind};
use oxiscript_ir::{BinOp as IB, CmpOp, Const, FieldRef, Op, Ovf, Terminator, Ty as It, TypeRef, UnOp as IU, ValueId};
use oxiscript_syntax::visit::Visitor;
use oxiscript_syntax::{BinOp, Expr, ExprKind, LitKind, ObjectField, TemplatePart, UnOp};

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    /// Locals assigned inside the expression/statement fragments (candidates for join parameters).
    pub fn assigned_in_exprs(&mut self, es: &[&Expr]) -> Vec<Key> {
        let mut sc = AssignScan {
            mi: &self.lw.ck.modules[self.m],
            tys: &self.lw.ck.tys,
            defs: &self.lw.ck.defs,
            fns: &self.lw.ck.fns,
            keys: Vec::new(),
        };
        for e in es {
            sc.visit_expr(e);
        }
        let keys = sc.keys;
        self.ssa_owned(&keys)
    }

    pub fn with_ctx<R>(&mut self, m: usize, subst: Vec<(ParamId, Ty)>, f: impl FnOnce(&mut Self) -> R) -> R {
        let (om, os) = (self.m, std::mem::replace(&mut self.subst, subst));
        self.m = m;
        let r = f(self);
        self.m = om;
        self.subst = os;
        r
    }

    /// Lower an expression including the implicit conversion the checker recorded for it.
    pub fn expr(&mut self, e: &Expr) -> Opnd {
        let saved = self.fb.loc;
        self.loc_of(e.span);
        let o = self.expr_raw(e);
        let o = match self.mi().coercions.get(&e.id).copied() {
            Some(c) => self.coerce(o, c.from, c.to),
            None => o,
        };
        self.fb.loc = saved;
        o
    }

    /// Lower an expression that must produce a value; diverging code yields a dummy.
    pub fn expr_val(&mut self, e: &Expr, ty: &It) -> Opnd {
        let o = self.expr(e);
        if o.v.is_none() {
            let d = self.dummy(ty);
            return Opnd::borrowed(d);
        }
        o
    }

    pub fn is_simple(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::This => true,
            ExprKind::Paren(i) => self.is_simple(i),
            ExprKind::Unary { expr, .. } => self.is_simple(expr),
            ExprKind::Binary { lhs, rhs, .. } => self.is_simple(lhs) && self.is_simple(rhs),
            ExprKind::Member { obj, .. } => self.is_simple(obj),
            _ => false,
        }
    }

    fn expr_raw(&mut self, e: &Expr) -> Opnd {
        match &e.kind {
            ExprKind::Lit(l) => self.literal(e, &l.kind),
            ExprKind::Template(parts) => self.template(e, parts),
            ExprKind::Ident(_) | ExprKind::This => self.ident(e),
            ExprKind::Array(xs) => self.array_lit(e, xs),
            ExprKind::Object(fs) => self.object_lit(e, fs),
            ExprKind::Unary { op, expr } => self.unary(e, *op, expr),
            ExprKind::Binary { op, lhs, rhs } => self.binary(e, *op, lhs, rhs),
            ExprKind::Assign { op, target, value } => self.assign(e, *op, target, value, true),
            ExprKind::Update { op, target, prefix } => self.update_expr(e, *op, target, *prefix),
            ExprKind::Call { .. } => self.call_expr(e),
            ExprKind::New { args, .. } => self.new_expr(e, args),
            ExprKind::Member { obj, name, optional } => self.member(e, obj, &name.name, *optional),
            ExprKind::Index { obj, index, optional } => self.index_expr(e, obj, index, *optional),
            ExprKind::Try(inner) => self.try_expr(e, inner),
            ExprKind::Cast { expr, .. } => self.cast_expr(e, expr),
            ExprKind::Cond { cond, then, els } => self.cond_expr(e, cond, then, els),
            ExprKind::Arrow(a) => self.arrow(e, a),
            ExprKind::Switch(s) => self.switch_expr(e, s),
            ExprKind::Paren(i) => self.expr(i),
        }
    }

    // ------------------------------------------------------------------ coercion and narrowing

    pub fn coerce(&mut self, o: Opnd, from: Ty, to: Ty) -> Opnd {
        let (from, to) = (self.cty(from), self.cty(to));
        if from == to {
            return o;
        }
        let (fk, tk) = (self.kind(from), self.kind(to));
        if matches!(tk, TyKind::Void) {
            self.drop_o(o);
            return Opnd::void();
        }
        if matches!(fk, TyKind::Never) {
            return match self.lw.repr(to) {
                Some(t) => {
                    let d = self.dummy(&t);
                    Opnd::borrowed(d)
                }
                None => Opnd::void(),
            };
        }
        match (&fk, &tk) {
            (TyKind::Null, TyKind::Nullable(_)) => {
                let t = self.lw.repr(to).unwrap_or(It::I32);
                let v = self.fb.emit1(Op::Null, t);
                Opnd::owned(v)
            }
            (TyKind::Nullable(_), TyKind::Nullable(_)) => o,
            (_, TyKind::Nullable(_)) if !matches!(fk, TyKind::Null) => self.wrap_nullable(o, to),
            _ => o,
        }
    }

    /// `T` -> `T?` where `to` is the nullable type.
    pub fn wrap_nullable(&mut self, o: Opnd, to: Ty) -> Opnd {
        let Some(v) = o.v else { return o };
        let target = self.lw.repr(to).unwrap_or(It::I32);
        let vt = self.fb.ty(v).clone();
        if vt.is_managed() {
            // a pure re-typing: the owned count moves
            let r = self.fb.emit1(Op::Opt { val: v }, target);
            return Opnd { v: Some(r), own: o.own };
        }
        // nullable scalar: box it
        let nb = self.lw.box_type(&vt);
        let obj = self.fb.emit1(Op::Alloc(TypeRef { ty: nb.clone(), variant: None }), It::Ref(nb.clone()));
        self.fb.emit0(Op::SetField {
            field: FieldRef {
                ty: nb,
                variant: None,
                field: "v".into(),
            },
            obj,
            val: v,
            init: true,
        });
        let r = self.fb.emit1(Op::Opt { val: obj }, target);
        Opnd::owned(r)
    }

    /// Convert a value stored with type `from` to the (narrowed) type `to` of the expression.
    pub fn narrow(&mut self, o: Opnd, from: Ty, to: Ty) -> Opnd {
        if from == to {
            return o;
        }
        let (Some(v), Some(tr)) = (o.v, self.repr(to)) else { return o };
        let vt = self.fb.ty(v).clone();
        if vt == tr {
            return o;
        }
        if let It::Opt(inner) = &vt {
            if **inner == tr {
                let r = self.fb.emit1(Op::Unopt { val: v }, tr);
                return Opnd { v: Some(r), own: o.own };
            }
            // boxed scalar
            if let It::Ref(nb) = &**inner {
                if tr.is_scalar() && self.lw.box_type(&tr) == *nb {
                    let b = self.fb.emit1(Op::Unopt { val: v }, It::Ref(nb.clone()));
                    let x = self.fb.emit1(
                        Op::GetField {
                            field: FieldRef {
                                ty: nb.clone(),
                                variant: None,
                                field: "v".into(),
                            },
                            obj: b,
                        },
                        tr,
                    );
                    if o.own {
                        self.release(b);
                    }
                    return Opnd::borrowed(x);
                }
            }
        }
        o
    }

    // ------------------------------------------------------------------ literals

    fn literal(&mut self, e: &Expr, k: &LitKind) -> Opnd {
        let t = self.ty_of(e.id);
        match k {
            LitKind::Int { value, .. } => {
                let Some(tr) = self.lw.repr(t) else { return Opnd::void() };
                let v = match tr {
                    It::F32 => self.fb.emit1(Op::Const(Const::F32((*value as f32).to_bits())), It::F32),
                    It::F64 => self.fb.emit1(Op::Const(Const::F64((*value as f64).to_bits())), It::F64),
                    other => self.fb.emit1(Op::Const(Const::Int(*value as i128)), other),
                };
                Opnd::borrowed(v)
            }
            LitKind::Float { text, .. } => {
                let tr = self.lw.repr(t).unwrap_or(It::F64);
                let v = if tr == It::F32 {
                    let f: f32 = text.parse().unwrap_or(0.0);
                    self.fb.emit1(Op::Const(Const::F32(f.to_bits())), It::F32)
                } else {
                    let f: f64 = text.parse().unwrap_or(0.0);
                    self.fb.emit1(Op::Const(Const::F64(f.to_bits())), It::F64)
                };
                Opnd::borrowed(v)
            }
            LitKind::Str(s) => {
                let v = self.str_const(s);
                Opnd::borrowed(v)
            }
            LitKind::Bool(b) => {
                let v = self.const_bool(*b);
                Opnd::borrowed(v)
            }
            LitKind::Null => Opnd::void(),
        }
    }

    // ------------------------------------------------------------------ strings

    /// Text of a value of a printable type, as a string operand.
    pub fn to_string_op(&mut self, o: Opnd) -> Opnd {
        let Some(v) = o.v else { return o };
        let t = self.fb.ty(v).clone();
        match t {
            It::Str => o,
            It::Bool => {
                let h = self.lw.helper("bool_str");
                let r = self.fb.emit1(Op::Call { callee: h, args: vec![v] }, It::Str);
                Opnd::owned(r)
            }
            It::F64 | It::F32 => {
                let x = if t == It::F32 {
                    self.fb.emit1(Op::Conv { ovf: Ovf::Trap, a: v }, It::F64)
                } else {
                    v
                };
                let r = self.fb.emit1(Op::StrFromF64 { x }, It::Str);
                Opnd::owned(r)
            }
            t if t.is_int() => {
                let signed = t.is_signed_int();
                let (h, arg) = if signed {
                    let w = if t == It::I64 {
                        v
                    } else {
                        self.fb.emit1(Op::Conv { ovf: Ovf::Trap, a: v }, It::I64)
                    };
                    (self.lw.helper("itoa"), w)
                } else {
                    let w = if t == It::U64 {
                        v
                    } else {
                        self.fb.emit1(Op::Conv { ovf: Ovf::Trap, a: v }, It::U64)
                    };
                    (self.lw.helper("utoa"), w)
                };
                let r = self.fb.emit1(Op::Call { callee: h, args: vec![arg] }, It::Str);
                Opnd::owned(r)
            }
            _ => {
                self.lw.unsupported("converting this type to text", None);
                o
            }
        }
    }

    /// `a ++ b` of two string operands (both are dropped if owned).
    pub fn concat(&mut self, a: Opnd, b: Opnd) -> Opnd {
        let r = self.fb.emit1(Op::StrConcat { a: a.val(), b: b.val() }, It::Str);
        self.drop_o(a);
        self.drop_o(b);
        Opnd::owned(r)
    }

    fn template(&mut self, e: &Expr, parts: &[TemplatePart]) -> Opnd {
        let _ = e;
        let mut acc: Option<Opnd> = None;
        for (i, p) in parts.iter().enumerate() {
            let later_expr = parts[i + 1..].iter().any(|q| matches!(q, TemplatePart::Expr(_)));
            let (piece, is_const) = match p {
                TemplatePart::Str(s) => {
                    if s.is_empty() {
                        continue;
                    }
                    let v = self.str_const(s);
                    (Opnd::borrowed(v), true)
                }
                TemplatePart::Expr(x) => {
                    let o = self.expr(x);
                    (self.to_string_op(o), false)
                }
            };
            acc = Some(match acc {
                None => piece,
                Some(a) => {
                    // `a` was held across the evaluation of `piece`; both are consumed here
                    self.unhold(a);
                    self.concat(a, piece)
                }
            });
            if later_expr {
                // keep the accumulator alive (and owned) across a later expression that may `?`-exit or
                // reassign a variable the accumulator borrows from
                let mut a = acc.take().expect("acc");
                if !a.own && !is_const {
                    if let Some(v) = self.own_of(a) {
                        a = Opnd::owned(v);
                    }
                }
                self.hold(a);
                acc = Some(a);
            }
        }
        acc.unwrap_or_else(|| {
            let v = self.str_const("");
            Opnd::borrowed(v)
        })
    }

    // ------------------------------------------------------------------ identifiers and variables

    fn ident(&mut self, e: &Expr) -> Opnd {
        match self.res_of(e.id) {
            Some(Res::Local(l) | Res::This(l)) => {
                if matches!(self.res_of(e.id), Some(Res::This(_))) && self.ctor.as_ref().is_some_and(|c| c.materialised.is_none()) {
                    self.materialise_ctor_this();
                }
                let o = self.read_local(l);
                let declared = self.local_info(l).ty;
                let actual = self.ty_of(e.id);
                let declared = self.cty(declared);
                let actual_c = self.cty(actual);
                if matches!(self.kind(actual_c), TyKind::Null) && matches!(self.kind(declared), TyKind::Nullable(_)) {
                    // narrowed to exactly `null`: behaves like the `null` literal (a typed null in any nullable context)
                    self.drop_o(o);
                    return Opnd::void();
                }
                self.narrow(o, declared, actual)
            }
            Some(Res::Def(d)) => self.def_value(e, d),
            _ => {
                self.unsupported("this identifier", e.span);
                Opnd::void()
            }
        }
    }

    pub fn read_local(&mut self, l: LocalId) -> Opnd {
        let lt = self.local_info(l).ty;
        if self.repr(lt).is_none() {
            return Opnd::void();
        }
        self.read_key(l.0, self.local_mut(l))
    }

    pub fn local_mut(&self, l: LocalId) -> bool {
        self.mi().locals[l.0 as usize].mutated || self.assigned_fn.contains(&l.0)
    }

    pub fn read_key(&mut self, key: Key, mutated: bool) -> Opnd {
        let Some(slot) = self.env.get(&key).cloned() else {
            self.lw.unsupported("a variable without a lowering slot", None);
            return Opnd::void();
        };
        match slot {
            Slot::Ssa { v, .. } => {
                if mutated && self.is_mg(v) {
                    self.retain(v);
                    Opnd::owned(v)
                } else {
                    Opnd::borrowed(v)
                }
            }
            Slot::Cell { cell, ty, .. } => self.load_cell(cell, &ty),
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
                    Some(cty) => self.load_cell(c, &cty),
                    None => {
                        if self.is_mg(c) {
                            self.retain(c);
                        }
                        Opnd::owned(c)
                    }
                }
            }
        }
    }

    pub fn env_field_ty(&self, env_ty: &str, field: &str) -> It {
        if let Some(t) = self.lw.type_decl(env_ty) {
            if let oxiscript_ir::TypeKind::Closure { captures } = &t.kind {
                if let Some(f) = captures.iter().find(|f| f.name == field) {
                    return f.ty.clone();
                }
            }
        }
        It::I32
    }

    pub fn cell_field_ty(&self, cell_ty: &str) -> It {
        if let Some(t) = self.lw.type_decl(cell_ty) {
            if let oxiscript_ir::TypeKind::Struct { fields } = &t.kind {
                return fields[0].ty.clone();
            }
        }
        It::I32
    }

    pub(crate) fn load_cell(&mut self, cell: ValueId, cty: &str) -> Opnd {
        let ft = self.cell_field_ty(cty);
        let v = self.fb.emit1(
            Op::GetField {
                field: FieldRef {
                    ty: cty.to_string(),
                    variant: None,
                    field: "v".into(),
                },
                obj: cell,
            },
            ft,
        );
        if self.is_mg(v) {
            self.retain(v);
        }
        Opnd::owned(v)
    }

    /// A reference to a function, global or enum member by name.
    fn def_value(&mut self, e: &Expr, d: DefId) -> Opnd {
        match self.def_kind(d) {
            DefKind::Global => {
                let t = self.lw.ck.globals[&d].ty;
                let t = self.cty(t);
                let actual = self.ty_of(e.id);
                let o = self.global_read(GKey::Global(d), t);
                self.narrow(o, t, actual)
            }
            DefKind::Fn => self.fn_value(d, e),
            _ => {
                self.unsupported("this definition used as a value", e.span);
                Opnd::void()
            }
        }
    }

    pub fn global_read(&mut self, key: GKey, ty: Ty) -> Opnd {
        let g = self.lw.global_ir(key, ty);
        let sty = if g.wrapped { It::Opt(Box::new(g.vty.clone())) } else { g.vty.clone() };
        let v = self.fb.emit1(Op::GlobalGet { name: g.sym.clone() }, sty);
        let v = if g.wrapped {
            self.fb.emit1(Op::Unopt { val: v }, g.vty.clone())
        } else {
            v
        };
        if self.is_mg(v) {
            self.retain(v);
        }
        Opnd::owned(v)
    }

    // ------------------------------------------------------------------ arrays and objects

    fn array_lit(&mut self, e: &Expr, xs: &[Expr]) -> Opnd {
        let t = self.ty_of(e.id);
        let at = self.lw.repr(t).unwrap_or(It::Array(Box::new(It::I32)));
        let arr = self.fb.emit1(Op::ArrayEmpty, at);
        self.hold(Opnd::owned(arr));
        for x in xs {
            let v = self.expr(x);
            let Some(v) = self.own_of(v) else { continue };
            self.fb.emit0(Op::ArrayPush { arr, val: v });
        }
        self.unhold(Opnd::owned(arr));
        Opnd::owned(arr)
    }

    fn object_lit(&mut self, e: &Expr, fields: &[ObjectField]) -> Opnd {
        let Some(&def) = self.mi().object_lits.get(&e.id) else {
            self.unsupported("this object literal", e.span);
            return Opnd::void();
        };
        let ty = self.ty_of(e.id);
        let TyKind::Adt(_, args) = self.kind(ty) else {
            self.unsupported("this object literal type", e.span);
            return Opnd::void();
        };
        let args: Vec<Ty> = args.to_vec();
        let rec = self.lw.ck.records[&def].clone();
        let (ir_ty, variant): (String, Option<String>) = match self.def_kind(def) {
            DefKind::Variant { union: Some(u), index } => {
                let uargs = self.lw.variant_union_args(u, index, &args);
                let un = self.lw.union_name(u, &uargs);
                let vn = self.lw.variant_name(&un, index);
                (un, Some(vn))
            }
            _ => (self.lw.struct_name(def, &args), None),
        };
        let map = self.lw.tparam_map(def, &args);
        // evaluate given values in source order
        let mut vals: Vec<(usize, Option<ValueId>)> = Vec::new();
        let mut held: Vec<ValueId> = Vec::new();
        let discr = rec.discr.as_ref().map(|(n, _)| n.clone());
        for f in fields {
            if Some(&f.name.name) == discr.as_ref() {
                continue;
            }
            let Some(idx) = rec.fields.iter().position(|x| !x.is_static && x.name == f.name.name) else {
                continue;
            };
            let o = match &f.value {
                Some(v) => self.expr(v),
                None => self.shorthand(f),
            };
            let v = self.own_of(o);
            if let Some(v) = v {
                if self.is_mg(v) {
                    self.held.push(v);
                    held.push(v);
                }
            }
            vals.push((idx, v));
        }
        // defaults
        for (idx, fi) in rec.fields.iter().enumerate() {
            if fi.is_static || vals.iter().any(|(i, _)| *i == idx) {
                continue;
            }
            if fi.has_init {
                if let Some(init) = self.field_init_expr(def, &fi.name) {
                    let m = self.lw.ck.defs[def.0 as usize].module as usize;
                    let o = self.with_ctx(m, map.clone(), |fx| fx.expr(init));
                    let v = self.own_of(o);
                    if let Some(v) = v {
                        if self.is_mg(v) {
                            self.held.push(v);
                            held.push(v);
                        }
                    }
                    vals.push((idx, v));
                }
            }
        }
        // allocate and initialise in declaration order, straight after the alloc
        for h in &held {
            if let Some(p) = self.held.iter().rposition(|x| x == h) {
                self.held.remove(p);
            }
        }
        let obj = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: ir_ty.clone(),
                variant: variant.clone(),
            }),
            It::Ref(ir_ty.clone()),
        );
        let mut order: Vec<(usize, Option<ValueId>)> = vals;
        order.sort_by_key(|(i, _)| *i);
        for (idx, v) in order {
            let Some(v) = v else { continue };
            self.fb.emit0(Op::SetField {
                field: FieldRef {
                    ty: ir_ty.clone(),
                    variant: variant.clone(),
                    field: rec.fields[idx].name.clone(),
                },
                obj,
                val: v,
                init: true,
            });
        }
        Opnd::owned(obj)
    }

    pub fn field_init_expr(&self, def: DefId, name: &str) -> Option<&'a Expr> {
        let DefAst::Record(rd) = self.lw.defast.get(&def)? else { return None };
        for m in &rd.members {
            if let oxiscript_syntax::MemberKind::Field(f) = &m.kind {
                if f.name.name == name && !f.is_static {
                    return f.init.as_ref();
                }
            }
        }
        None
    }

    fn shorthand(&mut self, f: &ObjectField) -> Opnd {
        let o = match self.res_of(f.id) {
            Some(Res::Local(l) | Res::This(l)) => {
                let o = self.read_local(l);
                let declared = self.local_info(l).ty;
                let declared = self.cty(declared);
                let from = self.mi().coercions.get(&f.id).map(|c| c.from);
                match from {
                    Some(fr) => {
                        let fr = self.cty(fr);
                        self.narrow(o, declared, fr)
                    }
                    None => o,
                }
            }
            Some(Res::Def(d)) if self.def_kind(d) == DefKind::Global => {
                let t = self.lw.ck.globals[&d].ty;
                let t = self.cty(t);
                self.global_read(GKey::Global(d), t)
            }
            _ => {
                self.unsupported("this shorthand field", f.span);
                return Opnd::void();
            }
        };
        match self.mi().coercions.get(&f.id).copied() {
            Some(c) => self.coerce(o, c.from, c.to),
            None => o,
        }
    }

    // ------------------------------------------------------------------ member and index reads

    /// An object expression used only to read from (borrowed when that is safe).
    pub fn borrow_obj(&mut self, obj: &Expr, next_is_simple: bool) -> Opnd {
        let o = strip(obj);
        // a chain of field reads from a stable base needs no retains as long as nothing runs between
        if next_is_simple {
            if let ExprKind::Member { obj: inner, optional: false, .. } = &o.kind {
                if let Some(Res::Field { owner, index }) = self.res_of(o.id) {
                    if self.mi().coercions.get(&o.id).is_none() && !self.is_ctor_this(inner) {
                        let declared = self.field_decl_ty(inner, owner, index);
                        let actual = self.ty_of(o.id);
                        let (dr, ar) = (self.repr(declared), self.repr(actual));
                        if let (Some(dr), Some(ar)) = (dr, ar) {
                            if dr == ar {
                                let b = self.borrow_obj(inner, true);
                                if let Some(bv) = b.v {
                                    let (ir_ty, variant, fname) = self.field_ref(inner, owner, index);
                                    let v = self.fb.emit1(
                                        Op::GetField {
                                            field: FieldRef {
                                                ty: ir_ty,
                                                variant,
                                                field: fname,
                                            },
                                            obj: bv,
                                        },
                                        dr,
                                    );
                                    // an owned base stays alive until the caller is done with the chain
                                    if b.own && self.is_mg(bv) {
                                        self.held.push(bv);
                                        self.pending_release.push(bv);
                                    }
                                    return Opnd::borrowed(v);
                                }
                                return b;
                            }
                        }
                    }
                }
            }
        }
        self.expr(obj)
    }

    pub fn flush_pending(&mut self) {
        let p = std::mem::take(&mut self.pending_release);
        for v in p.iter().rev() {
            if let Some(i) = self.held.iter().rposition(|x| x == v) {
                self.held.remove(i);
            }
            if !self.dead {
                self.release(*v);
            }
        }
    }

    pub fn is_ctor_this(&self, e: &Expr) -> bool {
        self.ctor.as_ref().is_some_and(|c| c.materialised.is_none()) && matches!(strip(e).kind, ExprKind::This)
    }

    /// The type of field `index` of `owner` as seen through the object expression `obj`.
    pub fn field_decl_ty(&mut self, obj: &Expr, owner: DefId, index: u32) -> Ty {
        let ot = self.ty_of(strip(obj).id);
        let args: Vec<Ty> = match self.kind(ot) {
            TyKind::Adt(_, a) => a.to_vec(),
            TyKind::Variants(_, a, _) => a.to_vec(),
            _ => Vec::new(),
        };
        let fty = self.lw.ck.records[&owner].fields[index as usize].ty;
        let map = self.lw.tparam_map(owner, &args);
        self.lw.subst(fty, &map)
    }

    /// IR (type name, variant, field name) of a field read through `obj`.
    pub fn field_ref(&mut self, obj: &Expr, owner: DefId, index: u32) -> (String, Option<String>, String) {
        let ot = self.ty_of(strip(obj).id);
        let args: Vec<Ty> = match self.kind(ot) {
            TyKind::Adt(_, a) => a.to_vec(),
            TyKind::Variants(_, a, _) => a.to_vec(),
            _ => Vec::new(),
        };
        let fname = self.lw.ck.records[&owner].fields[index as usize].name.clone();
        match self.def_kind(owner) {
            DefKind::Variant { union: Some(u), index: vi } => {
                let uargs = self.lw.variant_union_args(u, vi, &args);
                let un = self.lw.union_name(u, &uargs);
                let vn = self.lw.variant_name(&un, vi);
                (un, Some(vn), fname)
            }
            _ => (self.lw.struct_name(owner, &args), None, fname),
        }
    }

    fn member(&mut self, e: &Expr, obj: &Expr, name: &str, optional: bool) -> Opnd {
        let _ = name;
        match self.res_of(e.id) {
            Some(Res::Field { owner, index }) => {
                let declared = self.field_decl_ty(obj, owner, index);
                let actual = self.ty_of(e.id);
                // virtual `this` of a constructor
                if self.is_ctor_this(obj) {
                    let o = self.read_key(THIS_FIELD | index, true);
                    let declared = self.cty(declared);
                    return self.narrow(o, declared, actual);
                }
                if optional {
                    return self.optional_member(e, obj, owner, index, declared);
                }
                let simple = true;
                let b = self.borrow_obj(obj, simple);
                let Some(bv) = b.v else { return Opnd::void() };
                let (ir_ty, variant, fname) = self.field_ref(obj, owner, index);
                let Some(fr) = self.repr(declared) else {
                    self.drop_o(b);
                    self.flush_pending();
                    return Opnd::void();
                };
                let v = self.fb.emit1(
                    Op::GetField {
                        field: FieldRef {
                            ty: ir_ty,
                            variant,
                            field: fname,
                        },
                        obj: bv,
                    },
                    fr,
                );
                if self.is_mg(v) {
                    self.retain(v);
                }
                self.drop_o(b);
                self.flush_pending();
                let declared = self.cty(declared);
                self.narrow(Opnd::owned(v), declared, actual)
            }
            Some(Res::StaticField { owner, index }) => {
                let fty = self.lw.ck.records[&owner].fields[index as usize].ty;
                let fty = self.cty(fty);
                let actual = self.ty_of(e.id);
                let o = self.global_read(GKey::Static(owner, index), fty);
                self.narrow(o, fty, actual)
            }
            Some(Res::EnumMember { def, index }) => {
                let sig = &self.lw.ck.enums[&def];
                let (repr, val) = (sig.repr, sig.members[index as usize].1);
                let v = self.fb.emit1(Op::Const(Const::Int(val)), crate::types::num_ir(repr));
                Opnd::borrowed(v)
            }
            Some(Res::Discriminant { union }) => self.discriminant(e, obj, union),
            Some(Res::Builtin(b @ (Builtin::ArrayLength | Builtin::StrByteLength))) if optional => self.optional_builtin_member(e, obj, b),
            Some(Res::Builtin(Builtin::ArrayLength)) => {
                let b = self.borrow_obj(obj, true);
                let Some(bv) = b.v else { return Opnd::void() };
                let r = self.fb.emit1(Op::ArrayLen { arr: bv }, It::I32);
                self.drop_o(b);
                self.flush_pending();
                Opnd::borrowed(r)
            }
            Some(Res::Builtin(Builtin::StrByteLength)) => {
                let b = self.borrow_obj(obj, true);
                let Some(bv) = b.v else { return Opnd::void() };
                let r = self.fb.emit1(Op::StrLen { s: bv }, It::I32);
                self.drop_o(b);
                self.flush_pending();
                Opnd::borrowed(r)
            }
            Some(Res::Def(d)) => {
                let o = self.def_value(e, d);
                o
            }
            _ => {
                self.unsupported("this member access", e.span);
                Opnd::void()
            }
        }
    }

    /// `obj?.<something>`: null-check `obj`, run `f` on the unwrapped object (which `f` must
    /// drop) and wrap its result (of language type `.1`) into the nullable result type.
    pub fn optional_chain(&mut self, e: &Expr, obj: &Expr, f: impl FnOnce(&mut Self, Opnd) -> (Opnd, Ty)) -> Opnd {
        let ot = self.ty_of(strip(obj).id);
        let rty = self.ty_of(e.id);
        let rr = self.repr(rty).unwrap_or(It::I32);
        let o = self.expr(obj);
        let Some(ov) = o.v else { return Opnd::void() };
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
        if o.own {
            self.release(ov);
        }
        self.jump_join(&mut j, Some(nv));
        self.start_block(some);
        let inner = match self.kind(ot) {
            TyKind::Nullable(i) => i,
            _ => ot,
        };
        let unwrapped = self.narrow(o, ot, inner);
        let (val, vty) = f(self, unwrapped);
        let w = self.wrap_nullable_if_needed(val, vty, rty);
        let wv = match w.v {
            Some(_) => self.own_of(w).expect("value"),
            None => self.fb.emit1(Op::Null, It::I32),
        };
        self.jump_join(&mut j, Some(wv));
        let r = self.enter_join(&j).expect("join result");
        Opnd::owned(r)
    }

    fn optional_member(&mut self, e: &Expr, obj: &Expr, owner: DefId, index: u32, declared: Ty) -> Opnd {
        let ot = self.ty_of(strip(obj).id);
        let inner = match self.kind(ot) {
            TyKind::Nullable(i) => i,
            _ => ot,
        };
        let args: Vec<Ty> = match self.kind(inner) {
            TyKind::Adt(_, a) => a.to_vec(),
            TyKind::Variants(_, a, _) => a.to_vec(),
            _ => Vec::new(),
        };
        let fname = self.lw.ck.records[&owner].fields[index as usize].name.clone();
        let ir_ty = match self.def_kind(owner) {
            DefKind::Variant { union: Some(u), index: vi } => {
                let ua = self.lw.variant_union_args(u, vi, &args);
                self.lw.union_name(u, &ua)
            }
            _ => self.lw.struct_name(owner, &args),
        };
        let variant = match self.def_kind(owner) {
            DefKind::Variant { index: vi, .. } => Some(self.lw.variant_name(&ir_ty, vi)),
            _ => None,
        };
        let fr = self.repr(declared).unwrap_or(It::I32);
        let declared = self.cty(declared);
        self.optional_chain(e, obj, move |fx, un| {
            let ub = un.val();
            let fv = fx.fb.emit1(
                Op::GetField {
                    field: FieldRef {
                        ty: ir_ty,
                        variant,
                        field: fname,
                    },
                    obj: ub,
                },
                fr,
            );
            if fx.is_mg(fv) {
                fx.retain(fv);
            }
            fx.drop_o(un);
            (Opnd::owned(fv), declared)
        })
    }

    fn optional_builtin_member(&mut self, e: &Expr, obj: &Expr, b: Builtin) -> Opnd {
        self.optional_chain(e, obj, move |fx, un| {
            let uv = un.val();
            let r = match b {
                Builtin::ArrayLength => fx.fb.emit1(Op::ArrayLen { arr: uv }, It::I32),
                _ => fx.fb.emit1(Op::StrLen { s: uv }, It::I32),
            };
            fx.drop_o(un);
            (Opnd::borrowed(r), oxiscript_check::ty::I32)
        })
    }

    /// Wrap `o` of type `from` into the nullable type `to` unless it already is nullable.
    pub fn wrap_nullable_if_needed(&mut self, o: Opnd, from: Ty, to: Ty) -> Opnd {
        if from == to {
            return o;
        }
        if matches!(self.kind(from), TyKind::Nullable(_)) {
            return o;
        }
        self.wrap_nullable(o, to)
    }

    fn index_expr(&mut self, e: &Expr, obj: &Expr, index: &Expr, optional: bool) -> Opnd {
        if optional {
            return self.optional_index(e, obj, index);
        }
        let simple = self.is_simple(index);
        let arr = self.borrow_obj(obj, simple);
        let Some(av) = arr.v else { return Opnd::void() };
        if !simple {
            self.hold(arr);
        }
        let idx = self.expr(index);
        let Some(iv) = idx.v else { return Opnd::void() };
        if !simple {
            self.unhold(arr);
        }
        let et = match self.fb.ty(av).clone() {
            It::Array(el) => *el,
            _ => It::I32,
        };
        let v = self.fb.emit1(Op::ArrayGet { arr: av, idx: iv }, et);
        if self.is_mg(v) {
            self.retain(v);
        }
        self.drop_o(arr);
        self.flush_pending();
        Opnd::owned(v)
    }

    fn optional_index(&mut self, e: &Expr, obj: &Expr, index: &Expr) -> Opnd {
        let ot = self.ty_of(strip(obj).id);
        let inner = match self.kind(ot) {
            TyKind::Nullable(i) => i,
            _ => ot,
        };
        let elem_ty = match self.kind(inner) {
            TyKind::Array(t) => t,
            _ => inner,
        };
        self.optional_chain(e, obj, move |fx, un| {
            let uv = un.val();
            fx.hold(un);
            let idx = fx.expr(index);
            fx.unhold(un);
            let iv = idx.val();
            let et = match fx.fb.ty(uv).clone() {
                It::Array(el) => *el,
                _ => It::I32,
            };
            let v = fx.fb.emit1(Op::ArrayGet { arr: uv, idx: iv }, et);
            if fx.is_mg(v) {
                fx.retain(v);
            }
            fx.drop_o(un);
            (Opnd::owned(v), elem_ty)
        })
    }

    /// `u.kind` of a discriminated union.
    fn discriminant(&mut self, e: &Expr, obj: &Expr, union: DefId) -> Opnd {
        let b = self.expr(obj);
        let Some(bv) = b.v else { return Opnd::void() };
        let tag = self.fb.emit1(Op::Tag { obj: bv }, It::U32);
        self.drop_o(b);
        let sig = self.lw.ck.unions[&union].clone();
        let dt = self.ty_of(e.id);
        let rr = self.repr(dt).unwrap_or(It::I32);
        match rr {
            It::Str => {
                // chain of selects through blocks: tag -> string constant
                let mut j = self.new_join(Some(It::Str), Vec::new());
                let mut targets = Vec::new();
                let mut blocks = Vec::new();
                for (i, v) in sig.variants.iter().enumerate() {
                    let blk = self.new_block("kind");
                    targets.push((i as i64, tgt(blk, &[])));
                    blocks.push((blk, v.value.clone()));
                }
                let dflt = self.new_block("kdef");
                self.terminate_keep(Terminator::Switch {
                    value: tag,
                    cases: targets,
                    default: tgt(dflt, &[]),
                });
                for (blk, val) in blocks {
                    self.start_block(blk);
                    let s = match val {
                        Discr::Str(s) => s,
                        _ => String::new(),
                    };
                    let sv = self.str_const(&s);
                    self.retain(sv);
                    self.jump_join(&mut j, Some(sv));
                }
                self.start_block(dflt);
                self.terminate(Terminator::Trap(oxiscript_ir::TrapKind::Unreachable));
                let r = self.enter_join(&j).expect("join result");
                // the string constants are immortal; the join parameter is owned
                Opnd::owned(r)
            }
            t => {
                // integer or bool discriminants: select over the tag
                let mut acc: Option<ValueId> = None;
                for (i, v) in sig.variants.iter().enumerate().rev() {
                    let c = match (&v.value, &t) {
                        (Discr::Int(n), tt) if tt.is_int() => self.fb.emit1(Op::Const(Const::Int(*n)), t.clone()),
                        (Discr::Bool(b), It::Bool) => self.const_bool(*b),
                        _ => self.dummy(&t),
                    };
                    acc = Some(match acc {
                        None => c,
                        Some(prev) => {
                            let ix = self.fb.emit1(Op::Const(Const::Int(i as i128)), It::U32);
                            let is = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: tag, b: ix }, It::Bool);
                            self.fb.emit1(Op::Select { cond: is, a: c, b: prev }, t.clone())
                        }
                    });
                }
                Opnd::borrowed(acc.unwrap_or_else(|| self.dummy(&t)))
            }
        }
    }

    pub fn terminate_keep(&mut self, t: Terminator) {
        self.fb.term(t);
        self.dead = true;
    }

    // ------------------------------------------------------------------ operators

    fn unary(&mut self, e: &Expr, op: UnOp, inner: &Expr) -> Opnd {
        let t = self.ty_of(e.id);
        match op {
            UnOp::Neg => {
                // fold negative literals (`-128i8` is a literal whose magnitude does not fit)
                if let ExprKind::Lit(l) = &strip(inner).kind {
                    let tr = self.lw.repr(t).unwrap_or(It::I32);
                    match (&l.kind, &tr) {
                        (LitKind::Int { value, .. }, t) if t.is_int() => {
                            let v = self.fb.emit1(Op::Const(Const::Int(-(*value as i128))), tr.clone());
                            return Opnd::borrowed(v);
                        }
                        (LitKind::Int { value, .. }, It::F64) => {
                            let v = self.fb.emit1(Op::Const(Const::F64((-(*value as f64)).to_bits())), It::F64);
                            return Opnd::borrowed(v);
                        }
                        (LitKind::Int { value, .. }, It::F32) => {
                            let v = self.fb.emit1(Op::Const(Const::F32((-(*value as f32)).to_bits())), It::F32);
                            return Opnd::borrowed(v);
                        }
                        (LitKind::Float { text, .. }, It::F64) => {
                            let f: f64 = text.parse().unwrap_or(0.0);
                            let v = self.fb.emit1(Op::Const(Const::F64((-f).to_bits())), It::F64);
                            return Opnd::borrowed(v);
                        }
                        (LitKind::Float { text, .. }, It::F32) => {
                            let f: f32 = text.parse().unwrap_or(0.0);
                            let v = self.fb.emit1(Op::Const(Const::F32((-f).to_bits())), It::F32);
                            return Opnd::borrowed(v);
                        }
                        _ => {}
                    }
                }
                let o = self.expr(inner);
                let Some(v) = o.v else { return o };
                let ty = self.fb.ty(v).clone();
                let r = self.fb.emit1(Op::Un { op: IU::Neg, ovf: Ovf::Trap, a: v }, ty);
                Opnd::borrowed(r)
            }
            UnOp::Not | UnOp::BitNot => {
                let o = self.expr(inner);
                let Some(v) = o.v else { return o };
                let ty = self.fb.ty(v).clone();
                let r = self.fb.emit1(Op::Un { op: IU::Not, ovf: Ovf::Trap, a: v }, ty);
                Opnd::borrowed(r)
            }
        }
    }

    fn is_null_lit(e: &Expr) -> bool {
        matches!(&strip(e).kind, ExprKind::Lit(l) if matches!(l.kind, LitKind::Null))
    }

    fn binary(&mut self, e: &Expr, op: BinOp, lhs: &Expr, rhs: &Expr) -> Opnd {
        match op {
            BinOp::And | BinOp::Or => return self.short_circuit(e, op, lhs, rhs),
            BinOp::Coalesce => return self.coalesce(e, lhs, rhs),
            _ => {}
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) && (Self::is_null_lit(lhs) || Self::is_null_lit(rhs)) {
            let other = if Self::is_null_lit(lhs) { rhs } else { lhs };
            let o = self.expr(other);
            let Some(v) = o.v else {
                return Opnd::borrowed(self.const_bool(op == BinOp::Eq));
            };
            let r = self.fb.emit1(Op::IsNull { val: v }, It::Bool);
            self.drop_o(o);
            let r = if op == BinOp::Ne {
                self.fb.emit1(Op::Un { op: IU::Not, ovf: Ovf::Trap, a: r }, It::Bool)
            } else {
                r
            };
            return Opnd::borrowed(r);
        }
        if matches!(op, BinOp::Eq | BinOp::Ne) {
            if let Some(o) = self.discr_compare(op, lhs, rhs).or_else(|| self.discr_compare(op, rhs, lhs)) {
                return o;
            }
        }
        let a = self.expr(lhs);
        self.hold(a);
        let b = self.expr(rhs);
        self.unhold(a);
        let (Some(av), Some(bv)) = (a.v, b.v) else {
            self.drop_o(a);
            self.drop_o(b);
            return Opnd::void();
        };
        let r = self.binop_values(e, op, av, bv);
        self.drop_o(a);
        self.drop_o(b);
        r
    }

    /// `u.kind === "lit"` compares the tag directly.
    fn discr_compare(&mut self, op: BinOp, d: &Expr, l: &Expr) -> Option<Opnd> {
        let ExprKind::Member { obj, .. } = &strip(d).kind else { return None };
        let Some(Res::Discriminant { union }) = self.res_of(strip(d).id) else { return None };
        let ExprKind::Lit(lit) = &strip(l).kind else { return None };
        let want = match &lit.kind {
            LitKind::Str(s) => Discr::Str(s.clone()),
            LitKind::Int { value, .. } => Discr::Int(*value as i128),
            LitKind::Bool(b) => Discr::Bool(*b),
            _ => return None,
        };
        let idx = self.lw.ck.unions[&union].variants.iter().position(|v| v.value == want);
        let o = self.expr(obj);
        let ov = o.v?;
        let r = match idx {
            Some(i) => {
                let tag = self.fb.emit1(Op::Tag { obj: ov }, It::U32);
                let ix = self.fb.emit1(Op::Const(Const::Int(i as i128)), It::U32);
                self.fb.emit1(Op::Cmp { op: if op == BinOp::Eq { CmpOp::Eq } else { CmpOp::Ne }, a: tag, b: ix }, It::Bool)
            }
            None => self.const_bool(op == BinOp::Ne),
        };
        self.drop_o(o);
        Some(Opnd::borrowed(r))
    }

    /// Apply a binary operator to two lowered operands of the same IR type.
    pub fn binop_values(&mut self, e: &Expr, op: BinOp, av: ValueId, bv: ValueId) -> Opnd {
        let t = self.fb.ty(av).clone();
        // strings
        if t == It::Str {
            return match op {
                BinOp::Add => {
                    let r = self.fb.emit1(Op::StrConcat { a: av, b: bv }, It::Str);
                    Opnd::owned(r)
                }
                BinOp::Eq | BinOp::Ne => {
                    let r = self.fb.emit1(Op::StrEq { a: av, b: bv }, It::Bool);
                    let r = if op == BinOp::Ne {
                        self.fb.emit1(Op::Un { op: IU::Not, ovf: Ovf::Trap, a: r }, It::Bool)
                    } else {
                        r
                    };
                    Opnd::borrowed(r)
                }
                BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                    let c = self.fb.emit1(Op::StrCmp { a: av, b: bv }, It::I32);
                    let z = self.const_i32(0);
                    let cop = cmp_of(op);
                    let r = self.fb.emit1(Op::Cmp { op: cop, a: c, b: z }, It::Bool);
                    Opnd::borrowed(r)
                }
                _ => {
                    self.lw.err("OXS-E0703", format!("{op:?} on strings"), Some(e.span));
                    Opnd::void()
                }
            };
        }
        if matches!(op, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge) {
            let r = self.fb.emit1(Op::Cmp { op: cmp_of(op), a: av, b: bv }, It::Bool);
            return Opnd::borrowed(r);
        }
        let (ibo, ovf) = match op {
            BinOp::Add => (IB::Add, Ovf::Trap),
            BinOp::Sub => (IB::Sub, Ovf::Trap),
            BinOp::Mul => (IB::Mul, Ovf::Trap),
            BinOp::Div => (IB::Div, Ovf::Trap),
            BinOp::Rem => (IB::Rem, Ovf::Trap),
            BinOp::AddWrap => (IB::Add, Ovf::Wrap),
            BinOp::SubWrap => (IB::Sub, Ovf::Wrap),
            BinOp::MulWrap => (IB::Mul, Ovf::Wrap),
            BinOp::AddSat => (IB::Add, Ovf::Sat),
            BinOp::SubSat => (IB::Sub, Ovf::Sat),
            BinOp::MulSat => (IB::Mul, Ovf::Sat),
            BinOp::BitAnd => (IB::And, Ovf::Trap),
            BinOp::BitOr => (IB::Or, Ovf::Trap),
            BinOp::BitXor => (IB::Xor, Ovf::Trap),
            BinOp::Shl => (IB::Shl, Ovf::Trap),
            BinOp::Shr => (IB::Shr, Ovf::Trap),
            _ => {
                self.lw.err("OXS-E0703", format!("{op:?}"), Some(e.span));
                return Opnd::void();
            }
        };
        if t.is_float() && ibo == IB::Rem {
            self.lw.err("OXS-E0703", "remainder on floats".to_string(), Some(e.span));
            return Opnd::void();
        }
        let r = self.fb.emit1(Op::Bin { op: ibo, ovf, a: av, b: bv }, t);
        Opnd::borrowed(r)
    }

    fn short_circuit(&mut self, e: &Expr, op: BinOp, lhs: &Expr, rhs: &Expr) -> Opnd {
        let _ = e;
        let a = self.expr(lhs);
        let Some(av) = a.v else { return Opnd::void() };
        let set = self.assigned_in_exprs(&[rhs]);
        let snap = self.env.clone();
        let mut j = self.new_join(Some(It::Bool), set);
        let rhs_blk = self.new_block("rhs");
        let skip = self.new_block("skip");
        let (then, els) = if op == BinOp::And { (rhs_blk, skip) } else { (skip, rhs_blk) };
        self.fb.term(Terminator::BrIf {
            cond: av,
            then: tgt(then, &[]),
            els: tgt(els, &[]),
        });
        self.start_block(rhs_blk);
        let b = self.expr(rhs);
        let bv = b.v.unwrap_or_else(|| self.const_bool(false));
        self.jump_join(&mut j, Some(bv));
        self.env = snap;
        self.start_block(skip);
        let c = self.const_bool(op == BinOp::Or);
        self.jump_join(&mut j, Some(c));
        let r = self.enter_join(&j).expect("join result");
        Opnd::borrowed(r)
    }

    fn coalesce(&mut self, e: &Expr, lhs: &Expr, rhs: &Expr) -> Opnd {
        let rty = self.ty_of(e.id);
        let Some(rr) = self.repr(rty) else { return Opnd::void() };
        let lty = self.ty_of(strip(lhs).id);
        let a = self.expr(lhs);
        let Some(av) = a.v else {
            let lc = self.cty(lty);
            if matches!(self.kind(lc), TyKind::Null) {
                // `null ?? x` (a literal or a local narrowed to null): always `x`
                return self.expr_val(rhs, &rr);
            }
            return Opnd::void();
        };
        let set = self.assigned_in_exprs(&[rhs]);
        let snap = self.env.clone();
        let mut j = self.new_join(Some(rr.clone()), set);
        let isn = self.fb.emit1(Op::IsNull { val: av }, It::Bool);
        let some = self.new_block("some");
        let none = self.new_block("none");
        self.fb.term(Terminator::BrIf {
            cond: isn,
            then: tgt(none, &[]),
            els: tgt(some, &[]),
        });
        self.start_block(some);
        let inner = match self.kind(lty) {
            TyKind::Nullable(i) => i,
            _ => lty,
        };
        let unwrapped = if self.fb.ty(av) == &rr { a } else { self.narrow(a, lty, inner) };
        let uv = self.own_of(unwrapped).expect("value");
        self.jump_join(&mut j, Some(uv));
        self.env = snap;
        self.start_block(none);
        self.drop_o(a);
        let b = self.expr_val(rhs, &rr);
        let bv = self.own_of(b).expect("value");
        self.jump_join(&mut j, Some(bv));
        let r = self.enter_join(&j).expect("join result");
        Opnd::owned(r)
    }

    fn cond_expr(&mut self, e: &Expr, c: &Expr, x: &Expr, y: &Expr) -> Opnd {
        let rty = self.ty_of(e.id);
        let rr = self.repr(rty);
        let co = self.expr(c);
        let cv = co.val();
        let set = self.assigned_in_exprs(&[x, y]);
        let snap = self.env.clone();
        let mut j = self.new_join(rr.clone(), set);
        let tb = self.new_block("then");
        let eb = self.new_block("else");
        self.fb.term(Terminator::BrIf {
            cond: cv,
            then: tgt(tb, &[]),
            els: tgt(eb, &[]),
        });
        self.start_block(tb);
        let xo = self.cond_arm(x, &rr);
        self.jump_join(&mut j, xo);
        self.env = snap.clone();
        self.start_block(eb);
        let yo = self.cond_arm(y, &rr);
        self.jump_join(&mut j, yo);
        self.env = snap;
        let r = self.enter_join(&j);
        match r {
            Some(v) => Opnd::owned(v),
            None => Opnd::void(),
        }
    }

    fn cond_arm(&mut self, x: &Expr, rr: &Option<It>) -> Option<ValueId> {
        match rr {
            Some(t) => {
                let o = self.expr_val(x, t);
                self.own_of(o)
            }
            None => {
                let o = self.expr(x);
                self.drop_o(o);
                None
            }
        }
    }

    fn cast_expr(&mut self, e: &Expr, inner: &Expr) -> Opnd {
        let o = self.expr(inner);
        let Some(v) = o.v else { return o };
        let target = self.ty_of(e.id);
        let Some(tr) = self.lw.repr(target) else { return o };
        let vt = self.fb.ty(v).clone();
        if vt == tr || !(vt.is_numeric() && tr.is_numeric()) {
            return o;
        }
        let r = self.fb.emit1(Op::Conv { ovf: Ovf::Trap, a: v }, tr);
        Opnd::borrowed(r)
    }

    // ------------------------------------------------------------------ try operator

    fn try_expr(&mut self, e: &Expr, inner: &Expr) -> Opnd {
        let o = self.expr(inner);
        let Some(rv) = o.v else { return Opnd::void() };
        let ity = self.ty_of(inner.id);
        // the union IR type and its variants
        let (u, uargs) = match self.kind(ity) {
            TyKind::Adt(d, a) if self.def_kind(d) == DefKind::Union => (d, a.to_vec()),
            TyKind::Variants(d, a, _) => (d, a.to_vec()),
            _ => {
                self.unsupported("`?` on this type", e.span);
                return Opnd::void();
            }
        };
        let un = self.lw.union_name(u, &uargs);
        let ok_name = self.lw.variant_name(&un, 0);
        let err_name = self.lw.variant_name(&un, 1);
        let tag = self.fb.emit1(Op::Tag { obj: rv }, It::U32);
        let zero = self.fb.emit1(Op::Const(Const::Int(0)), It::U32);
        let is_ok = self.fb.emit1(Op::Cmp { op: CmpOp::Eq, a: tag, b: zero }, It::Bool);
        let okb = self.new_block("ok");
        let errb = self.new_block("err");
        self.fb.term(Terminator::BrIf {
            cond: is_ok,
            then: tgt(okb, &[]),
            els: tgt(errb, &[]),
        });
        // err path: build the function's error result and return it
        self.start_block(errb);
        let ret_t = self.ret_ty;
        let (ru, rargs) = match self.kind(ret_t) {
            TyKind::Adt(d, a) if self.def_kind(d) == DefKind::Union => (d, a.to_vec()),
            TyKind::Variants(d, a, _) => (d, a.to_vec()),
            _ => {
                self.unsupported("`?` in a function that does not return a Result", e.span);
                return Opnd::void();
            }
        };
        let run = self.lw.union_name(ru, &rargs);
        let rerr = self.lw.variant_name(&run, 1);
        let src_err_ty = self.variant_field_ty(&un, &err_name, "error");
        let ev = self.fb.emit1(
            Op::GetField {
                field: FieldRef {
                    ty: un.clone(),
                    variant: Some(err_name.clone()),
                    field: "error".into(),
                },
                obj: rv,
            },
            src_err_ty,
        );
        if self.is_mg(ev) {
            self.retain(ev);
        }
        let out = self.fb.emit1(
            Op::Alloc(TypeRef {
                ty: run.clone(),
                variant: Some(rerr.clone()),
            }),
            It::Ref(run.clone()),
        );
        self.fb.emit0(Op::SetField {
            field: FieldRef {
                ty: run,
                variant: Some(rerr),
                field: "error".into(),
            },
            obj: out,
            val: ev,
            init: true,
        });
        if o.own {
            self.release(rv);
        }
        self.emit_return(Some(out));
        // ok path
        self.start_block(okb);
        let vt = self.variant_field_ty_opt(&un, &ok_name, "value");
        let res = match vt {
            Some(vt) => {
                let v = self.fb.emit1(
                    Op::GetField {
                        field: FieldRef {
                            ty: un,
                            variant: Some(ok_name),
                            field: "value".into(),
                        },
                        obj: rv,
                    },
                    vt,
                );
                if self.is_mg(v) {
                    self.retain(v);
                }
                Opnd::owned(v)
            }
            None => Opnd::void(),
        };
        if o.own {
            self.release(rv);
        }
        res
    }

    pub fn variant_field_ty_opt(&self, union: &str, variant: &str, field: &str) -> Option<It> {
        if let Some(t) = self.lw.type_decl(union) {
            if let oxiscript_ir::TypeKind::Union { variants } = &t.kind {
                if let Some(v) = variants.iter().find(|v| v.name == variant) {
                    return v.fields.iter().find(|f| f.name == field).map(|f| f.ty.clone());
                }
            }
        }
        None
    }

    pub fn variant_field_ty(&self, union: &str, variant: &str, field: &str) -> It {
        self.variant_field_ty_opt(union, variant, field).unwrap_or(It::I32)
    }

}

pub(crate) fn cmp_of(op: BinOp) -> CmpOp {
    match op {
        BinOp::Eq => CmpOp::Eq,
        BinOp::Ne => CmpOp::Ne,
        BinOp::Lt => CmpOp::Lt,
        BinOp::Le => CmpOp::Le,
        BinOp::Gt => CmpOp::Gt,
        _ => CmpOp::Ge,
    }
}
