//! Per-function lowering state: SSA environment, scopes, joins, ownership helpers.

use crate::ctx::Lw;
use crate::fb::{tgt, Fb};
use oxiscript_check::{Builtin, DefId, DefKind, LocalId, ModuleInfo, ParamId, Res, Ty, TyKind};
use oxiscript_ir::{BlockId, Const, Op, Terminator, Ty as It, ValueId};
use oxiscript_syntax::visit::{walk_expr, Visitor};
use oxiscript_syntax::{Expr, ExprKind, NodeId, Span};
use std::collections::HashMap;

pub(crate) type Key = u32;
/// Synthetic key of the virtual field `i` of `this` while a constructor body is lowered.
pub(crate) const THIS_FIELD: Key = 0x8000_0000;
/// Synthetic keys of hidden temporaries (for-of arrays, switch scrutinees, ...).
pub(crate) const TMP_BASE: Key = 0x4000_0000;

#[derive(Clone, Debug)]
pub(crate) enum Slot {
    Ssa { v: ValueId, owned: bool },
    /// A shared cell holding a captured and mutated variable.
    Cell { cell: ValueId, owned: bool, ty: String },
    /// A capture inside a closure body: field of the closure environment.
    Cap {
        env: ValueId,
        env_ty: String,
        field: String,
        cell: Option<String>,
    },
}

/// The result of lowering an expression: a value (none for void) and whether the
/// reference is owned (count 1, must be consumed or released) or borrowed.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Opnd {
    pub v: Option<ValueId>,
    pub own: bool,
}

impl Opnd {
    pub fn void() -> Opnd {
        Opnd { v: None, own: false }
    }
    pub fn owned(v: ValueId) -> Opnd {
        Opnd { v: Some(v), own: true }
    }
    pub fn borrowed(v: ValueId) -> Opnd {
        Opnd { v: Some(v), own: false }
    }
    pub fn val(&self) -> ValueId {
        self.v.expect("value expected")
    }
}

#[derive(Clone)]
pub(crate) struct Frame {
    pub label: Option<String>,
    pub brk: BlockId,
    pub cont: Option<BlockId>,
    pub scope_depth: usize,
    pub brk_set: Vec<Key>,
    pub cont_set: Vec<Key>,
    pub brk_reached: bool,
    pub cont_hit: bool,
}

pub(crate) struct Join {
    pub blk: BlockId,
    pub set: Vec<Key>,
    pub has_result: bool,
    pub reached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FnKind {
    Plain,
    /// `mutating` struct method: the first result is the updated receiver.
    Mutating,
    Closure,
}

pub(crate) struct Fx<'l, 'c, 'a> {
    pub lw: &'l mut Lw<'c, 'a>,
    pub fb: Fb,
    pub m: usize,
    pub subst: Vec<(ParamId, Ty)>,
    pub env: HashMap<Key, Slot>,
    pub scopes: Vec<Vec<Key>>,
    pub loops: Vec<Frame>,
    pub held: Vec<ValueId>,
    /// Owned chain bases released by `flush_pending` once the borrowed chain has been used.
    pub pending_release: Vec<ValueId>,
    pub dead: bool,
    pub kind: FnKind,
    /// Language return type of the function being lowered (concrete).
    pub ret_ty: Ty,
    pub this_key: Option<Key>,
    pub next_tmp: u32,
    pub fn_def: Option<DefId>,
    /// Locals assigned somewhere in the function body (outside closures), for ownership decisions.
    pub assigned_fn: std::collections::HashSet<Key>,
    /// Constructor under lowering: the class and its IR type name (virtual `this`).
    pub ctor: Option<CtorState>,
    pub ctor_base_scopes: usize,
}

pub(crate) struct CtorState {
    pub class: DefId,
    pub ir_ty: String,
    pub args: Vec<Ty>,
    pub materialised: Option<ValueId>,
}

impl<'l, 'c, 'a> Fx<'l, 'c, 'a> {
    pub fn new(lw: &'l mut Lw<'c, 'a>, name: &str, m: usize, subst: Vec<(ParamId, Ty)>) -> Fx<'l, 'c, 'a> {
        Fx {
            lw,
            fb: Fb::new(name),
            m,
            subst,
            env: HashMap::new(),
            scopes: vec![Vec::new()],
            loops: Vec::new(),
            held: Vec::new(),
            pending_release: Vec::new(),
            dead: false,
            kind: FnKind::Plain,
            ret_ty: oxiscript_check::ty::VOID,
            this_key: None,
            next_tmp: 0,
            fn_def: None,
            assigned_fn: std::collections::HashSet::new(),
            ctor: None,
            ctor_base_scopes: 0,
        }
    }

    // ---------------------------------------------------------------- tables

    pub fn mi(&self) -> &ModuleInfo {
        &self.lw.ck.modules[self.m]
    }

    pub fn res_of(&self, id: NodeId) -> Option<Res> {
        self.mi().res.get(&id).copied()
    }

    /// Concrete type of an expression node (after narrowing), substituted for the current instance.
    pub fn ty_of(&mut self, id: NodeId) -> Ty {
        let t = self
            .mi()
            .expr_ty
            .get(id.0 as usize)
            .copied()
            .unwrap_or(oxiscript_check::ty::ERROR);
        self.cty(t)
    }

    pub fn cty(&mut self, t: Ty) -> Ty {
        if self.subst.is_empty() {
            t
        } else {
            let m = self.subst.clone();
            self.lw.ck.tys.subst(t, &m)
        }
    }

    pub fn kind(&self, t: Ty) -> TyKind {
        self.lw.ck.tys.kind(t).clone()
    }

    pub fn repr(&mut self, t: Ty) -> Option<It> {
        let t = self.cty(t);
        self.lw.repr(t)
    }

    pub fn loc_of(&mut self, sp: Span) {
        self.fb.loc = Some(oxiscript_ir::Loc {
            file: sp.file.0,
            lo: sp.lo,
            hi: sp.hi,
        });
    }

    pub fn unsupported(&mut self, what: &str, sp: Span) {
        self.lw.unsupported(what, Some(sp));
    }

    pub fn local_info(&self, l: LocalId) -> oxiscript_check::LocalInfo {
        self.mi().locals[l.0 as usize].clone()
    }

    // ---------------------------------------------------------------- emission helpers

    pub fn retain(&mut self, v: ValueId) {
        self.fb.emit0(Op::Retain { val: v });
    }

    pub fn release(&mut self, v: ValueId) {
        self.fb.emit0(Op::Release { val: v });
    }

    pub fn is_mg(&self, v: ValueId) -> bool {
        self.fb.is_managed(v)
    }

    /// Make an operand owned (retaining a borrowed managed value).
    pub fn own_of(&mut self, o: Opnd) -> Option<ValueId> {
        let v = o.v?;
        if self.is_mg(v) && !o.own {
            self.retain(v);
        }
        Some(v)
    }

    /// Release an operand that is owned and managed.
    pub fn drop_o(&mut self, o: Opnd) {
        if let Some(v) = o.v {
            if o.own && self.is_mg(v) {
                self.release(v);
            }
        }
    }

    pub fn const_i32(&mut self, n: i128) -> ValueId {
        self.fb.emit1(Op::Const(Const::Int(n)), It::I32)
    }

    pub fn const_bool(&mut self, b: bool) -> ValueId {
        self.fb.emit1(Op::Const(Const::Bool(b)), It::Bool)
    }

    pub fn str_const(&mut self, s: &str) -> ValueId {
        let d = self.lw.str_data(s);
        self.fb.emit1(Op::StrConst { data: d }, It::Str)
    }

    /// A value of type `ty` that is never observed (used after diverging code).
    pub fn dummy(&mut self, ty: &It) -> ValueId {
        match ty {
            It::Bool => self.const_bool(false),
            t if t.is_int() => self.fb.emit1(Op::Const(Const::Int(0)), t.clone()),
            It::F32 => self.fb.emit1(Op::Const(Const::F32(0)), It::F32),
            It::F64 => self.fb.emit1(Op::Const(Const::F64(0)), It::F64),
            It::Str => {
                let v = self.str_const("");
                v
            }
            It::Opt(_) => self.fb.emit1(Op::Null, ty.clone()),
            other => {
                // a non-null managed reference: null re-typed (never executed)
                let o = It::Opt(Box::new(other.clone()));
                let n = self.fb.emit1(Op::Null, o);
                self.fb.emit1(Op::Unopt { val: n }, other.clone())
            }
        }
    }

    // ---------------------------------------------------------------- control flow

    /// End the current block; code that follows is dead until a new block is entered.
    pub fn terminate(&mut self, t: Terminator) {
        self.fb.term(t);
        let b = self.fb.new_block("dead");
        self.fb.cur = b;
        self.dead = true;
    }

    pub fn start_block(&mut self, b: BlockId) {
        self.fb.cur = b;
        self.dead = false;
    }

    pub fn new_block(&mut self, hint: &str) -> BlockId {
        self.fb.new_block(hint)
    }

    /// Current IR values of the variables in `set` (consumed by the edge that carries them).
    pub fn join_args(&mut self, set: &[Key]) -> Vec<ValueId> {
        set.iter()
            .map(|k| match self.env.get(k) {
                Some(Slot::Ssa { v, .. }) => *v,
                _ => panic!("join variable {k} is not an SSA slot"),
            })
            .collect()
    }

    pub fn new_join(&mut self, result: Option<It>, set: Vec<Key>) -> Join {
        let blk = self.fb.new_block("join");
        if let Some(t) = &result {
            self.fb.block_param(blk, "r", t.clone());
        }
        for k in &set {
            let ty = match self.env.get(k) {
                Some(Slot::Ssa { v, .. }) => self.fb.ty(*v).clone(),
                _ => It::I32,
            };
            self.fb.block_param(blk, "j", ty);
        }
        Join {
            blk,
            set,
            has_result: result.is_some(),
            reached: false,
        }
    }

    /// Jump to a join from the current (live) block, passing the result and the join variables.
    pub fn jump_join(&mut self, j: &mut Join, result: Option<ValueId>) {
        if self.dead {
            return;
        }
        let mut args = Vec::new();
        if j.has_result {
            args.push(result.expect("join result"));
        }
        args.extend(self.join_args(&j.set));
        j.reached = true;
        self.terminate(Terminator::Br(tgt(j.blk, &args)));
    }

    /// Continue in a join block; its parameters become the current values of the join variables.
    pub fn enter_join(&mut self, j: &Join) -> Option<ValueId> {
        self.fb.cur = j.blk;
        self.dead = !j.reached;
        let params = self.fb.f.blocks[j.blk.0 as usize].params.clone();
        let mut it = params.into_iter();
        let res = if j.has_result { it.next() } else { None };
        for k in &j.set {
            let p = it.next().expect("join param");
            self.env.insert(*k, Slot::Ssa { v: p, owned: true });
        }
        res
    }

    /// Variables of `cands` that are owned SSA slots right now (only those can travel through joins).
    pub fn ssa_owned(&self, cands: &[Key]) -> Vec<Key> {
        cands
            .iter()
            .copied()
            .filter(|k| matches!(self.env.get(k), Some(Slot::Ssa { owned: true, .. })))
            .collect()
    }

    // ---------------------------------------------------------------- scopes

    pub fn push_scope(&mut self) {
        self.scopes.push(Vec::new());
    }

    /// Release the owned slots of scope `i`.
    fn release_scope(&mut self, i: usize) {
        let keys = self.scopes[i].clone();
        for k in keys.iter().rev() {
            match self.env.get(k).cloned() {
                Some(Slot::Ssa { v, owned: true }) if self.is_mg(v) => self.release(v),
                Some(Slot::Cell { cell, owned: true, .. }) => self.release(cell),
                _ => {}
            }
        }
    }

    pub fn pop_scope(&mut self) {
        let i = self.scopes.len() - 1;
        if !self.dead {
            self.release_scope(i);
        }
        let keys = self.scopes.pop().unwrap_or_default();
        for k in keys {
            self.env.remove(&k);
        }
    }

    /// Release scopes `depth..` without popping them (for jumps out of them).
    pub fn release_scopes_from(&mut self, depth: usize) {
        if self.dead {
            return;
        }
        for i in (depth..self.scopes.len()).rev() {
            self.release_scope(i);
        }
    }

    pub fn declare(&mut self, key: Key, slot: Slot) {
        self.env.insert(key, slot);
        if let Some(s) = self.scopes.last_mut() {
            s.push(key);
        }
    }

    pub fn tmp_key(&mut self) -> Key {
        self.next_tmp += 1;
        TMP_BASE + self.next_tmp
    }

    // ---------------------------------------------------------------- held temporaries

    pub fn hold(&mut self, o: Opnd) {
        if let Some(v) = o.v {
            if o.own && self.is_mg(v) {
                self.held.push(v);
            }
        }
    }

    /// Stop holding `o` (it is consumed or released by the caller).
    pub fn unhold(&mut self, o: Opnd) {
        if let Some(v) = o.v {
            if o.own && self.is_mg(v) {
                if let Some(p) = self.held.iter().rposition(|x| *x == v) {
                    self.held.remove(p);
                }
            }
        }
    }

    pub fn release_held(&mut self) {
        if self.dead {
            return;
        }
        let h = self.held.clone();
        for v in h.iter().rev() {
            self.release(*v);
        }
    }

    // ---------------------------------------------------------------- locals

    pub fn key_of(l: LocalId) -> Key {
        l.0
    }

    /// Is the local kept in a shared cell (captured by a closure and assigned after initialisation)?
    pub fn is_cell(&self, l: LocalId) -> bool {
        let i = &self.mi().locals[l.0 as usize];
        i.captured && (i.mutated || self.assigned_fn.contains(&l.0))
    }

    /// Bind a local to an owned initial value (creating its cell when needed).
    pub fn declare_local(&mut self, l: LocalId, init: Option<ValueId>) {
        let key = Self::key_of(l);
        let Some(v) = init else {
            return; // void-typed local: nothing to hold
        };
        if self.is_cell(l) {
            let ty = self.fb.ty(v).clone();
            let cname = self.lw.cell_type(&ty);
            let cell = self.fb.emit1(Op::Alloc(oxiscript_ir::TypeRef { ty: cname.clone(), variant: None }), It::Ref(cname.clone()));
            self.fb.emit0(Op::SetField {
                field: oxiscript_ir::FieldRef {
                    ty: cname.clone(),
                    variant: None,
                    field: "v".into(),
                },
                obj: cell,
                val: v,
                init: true,
            });
            self.declare(key, Slot::Cell { cell, owned: true, ty: cname });
        } else {
            self.declare(key, Slot::Ssa { v, owned: true });
        }
    }

    pub fn def_kind(&self, d: DefId) -> DefKind {
        self.lw.ck.defs[d.0 as usize].kind
    }

    pub fn builtin_of(&self, id: NodeId) -> Option<Builtin> {
        match self.res_of(id) {
            Some(Res::Builtin(b)) => Some(b),
            _ => None,
        }
    }
}

// -------------------------------------------------------------------- assigned-variable scan

/// Locals assigned (directly, or through a chain of struct fields) inside an AST fragment,
/// not descending into closure bodies (those variables live in cells).
pub(crate) struct AssignScan<'m, 't> {
    pub mi: &'m ModuleInfo,
    pub tys: &'t oxiscript_check::Interner,
    pub defs: &'t [oxiscript_check::DefInfo],
    pub fns: &'t HashMap<DefId, oxiscript_check::FnSig>,
    pub keys: Vec<Key>,
}

impl<'m, 't> AssignScan<'m, 't> {
    fn add(&mut self, k: Key) {
        if !self.keys.contains(&k) {
            self.keys.push(k);
        }
    }

    fn is_struct(&self, id: NodeId) -> bool {
        let t = self.mi.expr_ty.get(id.0 as usize).copied();
        match t.map(|t| self.tys.kind(t)) {
            Some(TyKind::Adt(d, _)) => matches!(self.defs[d.0 as usize].kind, DefKind::Struct),
            _ => false,
        }
    }

    fn root(&mut self, e: &Expr) {
        match &e.kind {
            ExprKind::Paren(i) => self.root(i),
            ExprKind::Ident(_) | ExprKind::This => {
                if let Some(Res::Local(l) | Res::This(l)) = self.mi.res.get(&e.id) {
                    self.add(l.0);
                }
            }
            ExprKind::Member { obj, .. } => {
                if let Some(Res::Field { .. }) = self.mi.res.get(&e.id) {
                    let inner = strip(obj);
                    if self.is_struct(inner.id) {
                        self.root(inner);
                    }
                    // a constructor's `this.f = ..` assigns the virtual field
                    if let (ExprKind::This, Some(Res::Field { index, .. })) = (&inner.kind, self.mi.res.get(&e.id)) {
                        self.add(THIS_FIELD | *index);
                    }
                }
            }
            _ => {}
        }
    }
}

pub(crate) fn strip(e: &Expr) -> &Expr {
    let mut e = e;
    while let ExprKind::Paren(i) = &e.kind {
        e = i;
    }
    e
}

impl<'a, 'm, 't> Visitor<'a> for AssignScan<'m, 't> {
    fn visit_expr(&mut self, e: &'a Expr) {
        match &e.kind {
            ExprKind::Arrow(_) => return,
            ExprKind::Assign { target, .. } => self.root(target),
            ExprKind::Update { target, .. } => self.root(target),
            ExprKind::Call { callee, .. } => {
                // a `mutating` method call updates its receiver
                if let ExprKind::Member { obj, .. } = &strip(callee).kind {
                    if let Some(Res::Method(d)) = self.mi.res.get(&strip(callee).id) {
                        if self.fns.get(d).is_some_and(|s| s.mutating) {
                            self.root(obj);
                        }
                    }
                }
            }
            _ => {}
        }
        walk_expr(self, e);
    }
}
