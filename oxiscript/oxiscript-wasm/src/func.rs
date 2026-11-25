//! Lowering of one OxiIR function to a Wasm function body.

use crate::body::{Body, Mem};
use crate::cfg::{self, Cfg};
use crate::cx::Cx;
use crate::helpers::{int_range, ty_code, HKey};
use crate::modlayout::ty_key;
use crate::stubrt::G_TRAP;
use crate::{Diagnostic, E_IRREDUCIBLE, E_LIMIT, E_UNSUPPORTED};
use oxiscript_abi::layout;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::{
    BinOp, CmpOp, Const, Func, Inst, Op, Ovf, Target, Terminator, Ty, UnOp, ValueId,
};
use std::collections::HashMap;
use wasm_encoder::{BlockType, Function, Ieee32, Ieee64, ValType};

/// Structuring recursion limit. A debug-build level costs ~2.4 KiB of Rust
/// stack (measured), so 300 levels fit a 2 MiB test thread; release/wasm32
/// frames are several times smaller.
pub(crate) const MAX_DEPTH: u32 = 300;

pub(crate) fn vt(t: &Ty) -> ValType {
    match t {
        Ty::I64 | Ty::U64 => ValType::I64,
        Ty::F32 => ValType::F32,
        Ty::F64 => ValType::F64,
        _ => crate::target::PTR_VT,
    }
}

pub(crate) fn mem_of(t: &Ty) -> Mem {
    match t {
        Ty::Bool | Ty::U8 => Mem::U8,
        Ty::I8 => Mem::I8,
        Ty::I16 => Mem::I16,
        Ty::U16 => Mem::U16,
        Ty::I64 | Ty::U64 => Mem::I64,
        Ty::F32 => Mem::F32,
        Ty::F64 => Mem::F64,
        _ => Mem::I32,
    }
}

fn ibits(t: &Ty) -> u32 {
    t.bits().unwrap_or(32)
}

fn is_signed(t: &Ty) -> bool {
    t.is_signed_int()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Frame {
    Loop(usize),
    Block(usize),
    If,
}

enum Plan {
    Table { min: i64, max: i64 },
    Chain,
}

pub(crate) struct FnGen<'c, 'a> {
    pub(crate) cx: &'c mut Cx<'a>,
    f: &'a Func,
    pub(crate) b: Body,
    loc: Vec<u32>,
    cfg: Cfg,
    plans: HashMap<usize, Plan>,
    ctx: Vec<Frame>,
    depth: u32,
    no_tail: u32,
    t32: Vec<u32>,
    t64: Vec<u32>,
    tf32: u32,
    tf64: u32,
}

type R<T = ()> = Result<T, Diagnostic>;

pub(crate) fn gen_func<'a>(cx: &mut Cx<'a>, f: &'a Func) -> R<Function> {
    let mut cfg = cfg::analyze(f).map_err(|label| {
        Diagnostic::new(
            E_IRREDUCIBLE,
            format!("irreducible control flow (retreating edge into block `{label}` that does not dominate its source)"),
            Some(f.name.clone()),
        )
    })?;
    let mut plans = HashMap::new();
    for &bi in &cfg.rpo {
        if let Terminator::Switch {
            value,
            cases,
            default,
        } = &f.blocks[bi].term
        {
            let wide = ibits(f.ty(*value)) == 64;
            let no_args = default.args.is_empty() && cases.iter().all(|(_, t)| t.args.is_empty());
            let plan = if !wide && no_args && cases.len() >= 3 {
                let min = cases.iter().map(|c| c.0).min().unwrap_or(0);
                let max = cases.iter().map(|c| c.0).max().unwrap_or(0);
                let range = i128::from(max) - i128::from(min) + 1;
                if range <= 1024 && range <= 4 * cases.len() as i128 {
                    Plan::Table { min, max }
                } else {
                    Plan::Chain
                }
            } else {
                Plan::Chain
            };
            if matches!(plan, Plan::Table { .. }) {
                for t in cases.iter().map(|c| &c.1).chain(std::iter::once(default)) {
                    cfg.forced[t.block.0 as usize] = true;
                }
            }
            plans.insert(bi, plan);
        }
    }
    let mut b = Body::new(f.params.len());
    let mut loc = vec![u32::MAX; f.values.len()];
    for (i, p) in f.params.iter().enumerate() {
        loc[p.value.0 as usize] = i as u32;
    }
    for ty in [ValType::I32, ValType::I64, ValType::F32, ValType::F64] {
        for (i, v) in f.values.iter().enumerate() {
            if loc[i] == u32::MAX && vt(&v.ty) == ty {
                loc[i] = b.local(ty);
            }
        }
    }
    let mut g = FnGen {
        cx,
        f,
        b,
        loc,
        cfg,
        plans,
        ctx: Vec::new(),
        depth: 0,
        no_tail: 0,
        t32: Vec::new(),
        t64: Vec::new(),
        tf32: u32::MAX,
        tf64: u32::MAX,
    };
    g.do_tree(0).map_err(|mut d| {
        d.func.get_or_insert_with(|| f.name.clone());
        d
    })?;
    g.b.s().unreachable();
    if g.b.declared_locals() + f.params.len() > crate::LIMIT_LOCALS {
        return Err(Diagnostic::new(
            E_LIMIT,
            format!(
                "function needs {} Wasm locals; the limit is {}",
                g.b.declared_locals() + f.params.len(),
                crate::LIMIT_LOCALS
            ),
            Some(f.name.clone()),
        ));
    }
    if g.b.code_len() > crate::LIMIT_BODY_BYTES {
        return Err(Diagnostic::new(
            E_LIMIT,
            "function body exceeds the Wasm function size limit",
            Some(f.name.clone()),
        ));
    }
    Ok(g.b.finish())
}

impl<'c, 'a> FnGen<'c, 'a> {
    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        Err(Diagnostic::new(
            E_UNSUPPORTED,
            msg,
            Some(self.f.name.clone()),
        ))
    }

    fn l(&self, v: ValueId) -> u32 {
        self.loc[v.0 as usize]
    }
    fn ty(&self, v: ValueId) -> &'a Ty {
        let f: &'a Func = self.f;
        f.ty(v)
    }
    fn get(&mut self, v: ValueId) {
        let l = self.l(v);
        self.b.get(l);
    }
    fn set(&mut self, v: ValueId) {
        let l = self.l(v);
        self.b.set(l);
    }
    fn temp32(&mut self, n: usize) -> u32 {
        while self.t32.len() <= n {
            let l = self.b.local(ValType::I32);
            self.t32.push(l);
        }
        self.t32[n]
    }
    fn temp64(&mut self, n: usize) -> u32 {
        while self.t64.len() <= n {
            let l = self.b.local(ValType::I64);
            self.t64.push(l);
        }
        self.t64[n]
    }
    fn trap(&mut self, k: TrapKind) {
        self.b.i32c(k.code() as i32);
        self.b.s().global_set(G_TRAP).unreachable();
    }
    /// Condition (i32) on the stack: trap if non-zero.
    fn trap_if(&mut self, k: TrapKind) {
        self.b.s().if_(BlockType::Empty);
        self.trap(k);
        self.b.s().end();
    }
    fn call_rt(&mut self, name: &str) {
        let i = self.cx.rt(name);
        self.b.s().call(i);
    }

    /// Re-canonicalise an `i32` on the stack for integer type `t`.
    fn canon(&mut self, t: &Ty) {
        match t {
            Ty::I8 => {
                self.b.s().i32_extend8_s();
            }
            Ty::I16 => {
                self.b.s().i32_extend16_s();
            }
            Ty::U8 => {
                self.b.i32c(0xff);
                self.b.s().i32_and();
            }
            Ty::U16 => {
                self.b.i32c(0xffff);
                self.b.s().i32_and();
            }
            _ => {}
        }
    }

    /// Push the canonical-NaN form of the float in local `l`.
    pub(crate) fn push_canon_float(&mut self, l: u32, is64: bool) {
        push_canon_float_body(&mut self.b, l, is64);
    }

    /// Canonicalise the float on top of the stack (via a scratch local).
    fn canon_top(&mut self, is64: bool) {
        let t = if is64 {
            if self.tf64 == u32::MAX {
                self.tf64 = self.b.local(ValType::F64);
            }
            self.tf64
        } else {
            if self.tf32 == u32::MAX {
                self.tf32 = self.b.local(ValType::F32);
            }
            self.tf32
        };
        self.b.tee(t);
        self.b.s().drop();
        self.push_canon_float(t, is64);
    }

    /// Push value `v` ready to be stored to memory (floats canonicalised in `safety`).
    fn push_store_val(&mut self, v: ValueId) {
        let t = self.ty(v);
        if self.cx.safety && t.is_float() {
            let l = self.l(v);
            self.push_canon_float(l, *t == Ty::F64);
        } else {
            self.get(v);
        }
    }

    // ---------------------------------------------------------------- CFG

    fn label_depth(&self, want: Frame) -> R<u32> {
        for (d, fr) in self.ctx.iter().rev().enumerate() {
            if *fr == want {
                return Ok(d as u32);
            }
        }
        Err(Diagnostic::new(
            crate::I_INTERNAL,
            format!("structuring bug: no enclosing frame for {want:?}"),
            Some(self.f.name.clone()),
        ))
    }

    fn is_label(&self, from: usize, t: usize) -> bool {
        self.cfg.num[t] <= self.cfg.num[from] || self.cfg.is_merge(t)
    }

    fn label_of(&self, from: usize, t: usize) -> R<u32> {
        if self.cfg.num[t] <= self.cfg.num[from] {
            self.label_depth(Frame::Loop(t))
        } else {
            self.label_depth(Frame::Block(t))
        }
    }

    fn enter(&mut self) -> R {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Diagnostic::new(
                E_LIMIT,
                "control-flow nesting too deep",
                Some(self.f.name.clone()),
            ));
        }
        Ok(())
    }

    fn do_tree(&mut self, x: usize) -> R {
        self.enter()?;
        let r = self.do_tree_inner(x);
        self.depth -= 1;
        r
    }

    fn do_tree_inner(&mut self, mut x: usize) -> R {
        loop {
            let header = self.cfg.is_header[x];
            if header {
                self.ctx.push(Frame::Loop(x));
                self.b.s().loop_(BlockType::Empty);
            }
            let mut ys: Vec<usize> = self.cfg.children[x]
                .iter()
                .copied()
                .filter(|c| self.cfg.is_merge(*c))
                .collect();
            ys.sort_by_key(|y| std::cmp::Reverse(self.cfg.num[*y]));
            for &y in &ys {
                self.ctx.push(Frame::Block(y));
                self.b.s().block(BlockType::Empty);
            }
            self.node_body(x)?;
            let mut next = None;
            for (k, &y) in ys.iter().enumerate().rev() {
                self.ctx.pop();
                self.b.s().end();
                if k == 0 && !header {
                    next = Some(y); // outermost: continue iteratively
                } else {
                    self.do_tree(y)?;
                }
            }
            if header {
                self.ctx.pop();
                self.b.s().end();
            }
            match next {
                Some(y) => x = y,
                None => return Ok(()),
            }
        }
    }

    /// Is `z` plain straight-line code (so a linear chain can continue into it)?
    fn simple(&self, z: usize) -> bool {
        !self.cfg.is_header[z] && !self.cfg.children[z].iter().any(|c| self.cfg.is_merge(*c))
    }

    fn node_body(&mut self, x: usize) -> R {
        let mut cur = x;
        loop {
            let f: &'a Func = self.f;
            for inst in &f.blocks[cur].insts {
                self.inst(inst)?;
            }
            match self.term(cur)? {
                Some(z) if self.simple(z) => cur = z,
                Some(z) => {
                    self.do_tree(z)?;
                    return Ok(());
                }
                None => return Ok(()),
            }
        }
    }

    fn moves(&mut self, t: &Target) {
        if t.args.is_empty() {
            return;
        }
        for a in &t.args {
            self.get(*a);
        }
        let f: &'a Func = self.f;
        for p in f.blocks[t.block.0 as usize].params.iter().rev() {
            self.set(*p);
        }
    }

    /// Emit the edge `from -> t`. Returns the block to emit inline when it is
    /// neither a loop back edge nor a merge point.
    fn branch(&mut self, from: usize, t: &Target) -> R<Option<usize>> {
        self.moves(t);
        let tb = t.block.0 as usize;
        if self.cfg.num[tb] <= self.cfg.num[from] {
            let d = self.label_depth(Frame::Loop(tb))?;
            self.b.s().br(d);
            Ok(None)
        } else if self.cfg.is_merge(tb) {
            if self.no_tail == 0 && self.ctx.last() == Some(&Frame::Block(tb)) {
                return Ok(None);
            }
            let d = self.label_depth(Frame::Block(tb))?;
            self.b.s().br(d);
            Ok(None)
        } else {
            Ok(Some(tb))
        }
    }

    /// Emit an edge as a nested arm (inside an `if`).
    fn arm(&mut self, from: usize, t: &Target) -> R {
        self.ctx.push(Frame::If);
        self.no_tail += 1;
        let r = self.branch(from, t)?;
        let res = match r {
            Some(z) => self.do_tree(z),
            None => Ok(()),
        };
        self.no_tail -= 1;
        self.ctx.pop();
        res
    }

    fn term(&mut self, x: usize) -> R<Option<usize>> {
        let f: &'a Func = self.f;
        let term = &f.blocks[x].term;
        match term {
            Terminator::Br(t) => self.branch(x, t),
            Terminator::Ret(vs) => {
                for v in vs {
                    self.get(*v);
                }
                self.b.s().return_();
                Ok(None)
            }
            Terminator::Trap(k) => {
                self.trap(*k);
                Ok(None)
            }
            Terminator::BrIf { cond, then, els } => self.term_brif(x, *cond, then, els),
            Terminator::Switch {
                value,
                cases,
                default,
            } => self.term_switch(x, *value, cases, default),
            _ => self.err("unsupported terminator"),
        }
    }

    #[inline(never)]
    fn term_brif(&mut self, x: usize, cond: ValueId, then: &Target, els: &Target) -> R<Option<usize>> {
        let cond = &cond;

                let (tb, eb) = (then.block.0 as usize, els.block.0 as usize);
                let tl = self.is_label(x, tb) && then.args.is_empty();
                let el = self.is_label(x, eb) && els.args.is_empty();
                if tl {
                    self.get(*cond);
                    let d = self.label_of(x, tb)?;
                    self.b.s().br_if(d);
                    return self.branch(x, els);
                }
                if el {
                    self.get(*cond);
                    self.b.s().i32_eqz();
                    let d = self.label_of(x, eb)?;
                    self.b.s().br_if(d);
                    return self.branch(x, then);
                }
                let size = |g: &Self, b: usize| {
                    if g.is_label(x, b) {
                        0
                    } else {
                        g.cfg.subtree[b]
                    }
                };
                if size(self, tb) <= size(self, eb) {
                    self.get(*cond);
                    self.b.s().if_(BlockType::Empty);
                    self.arm(x, then)?;
                    self.b.s().end();
                    self.branch(x, els)
                } else {
                    self.get(*cond);
                    self.b.s().i32_eqz().if_(BlockType::Empty);
                    self.arm(x, els)?;
                    self.b.s().end();
                    self.branch(x, then)
                }
            
    }

    #[inline(never)]
    fn term_switch(&mut self, x: usize, value: ValueId, cases: &[(i64, Target)], default: &Target) -> R<Option<usize>> {
        let value = &value;

                let wide = ibits(self.ty(*value)) == 64;
                match self.plans.get(&x) {
                    Some(Plan::Table { min, max }) => {
                        let (min, max) = (*min, *max);
                        self.get(*value);
                        if min != 0 {
                            self.b.i32c(min as i32);
                            self.b.s().i32_sub();
                        }
                        let dl = self.label_of(x, default.block.0 as usize)?;
                        let mut labels = Vec::new();
                        for k in min..=max {
                            let t = cases
                                .iter()
                                .find(|c| c.0 == k)
                                .map_or(default, |c| &c.1);
                            labels.push(self.label_of(x, t.block.0 as usize)?);
                        }
                        self.b.s().br_table(labels, dl);
                        Ok(None)
                    }
                    _ => {
                        for (k, t) in cases {
                            self.get(*value);
                            if wide {
                                self.b.i64c(*k);
                                self.b.s().i64_eq();
                            } else {
                                self.b.i32c(*k as i32);
                                self.b.s().i32_eq();
                            }
                            let tb = t.block.0 as usize;
                            if self.is_label(x, tb) && t.args.is_empty() {
                                let d = self.label_of(x, tb)?;
                                self.b.s().br_if(d);
                            } else {
                                self.b.s().if_(BlockType::Empty);
                                self.arm(x, t)?;
                                self.b.s().end();
                            }
                        }
                        self.branch(x, default)
                    }
                }
            
    }

    // --------------------------------------------------------- instructions

    fn bind(&mut self, res: &[ValueId], host: bool) {
        for r in res.iter().rev() {
            let t = self.ty(*r);
            if host {
                self.canon_host_result(t);
            }
            self.set(*r);
        }
    }

    fn canon_host_result(&mut self, t: &Ty) {
        if *t == Ty::Bool {
            self.b.s().i32_eqz().i32_eqz();
        } else {
            self.canon(t);
        }
    }

    fn field(&self, fr: &oxiscript_ir::FieldRef) -> R<(u32, Ty, Option<u32>)> {
        let (rec, var) = self
            .cx
            .lay
            .rec(&fr.ty, fr.variant.as_deref())
            .ok_or_else(|| Diagnostic::new(E_UNSUPPORTED, "unknown field owner", None))?;
        let i = rec
            .ix
            .get(&fr.field)
            .copied()
            .ok_or_else(|| Diagnostic::new(E_UNSUPPORTED, "unknown field", None))?;
        Ok((rec.offsets[i], rec.tys[i].clone(), var))
    }

    fn tag_check(&mut self, obj: ValueId, idx: u32) {
        self.get(obj);
        self.b.load(Mem::I32, layout::UNION_OFF_TAG);
        self.b.i32c(idx as i32);
        self.b.s().i32_ne();
        self.trap_if(TrapKind::BadCast);
    }

    #[allow(clippy::too_many_lines)]
    fn inst(&mut self, i: &Inst) -> R {
        let res = &i.results;
        match &i.op {
            Op::Const(c) => {
                let r = res[0];
                let t = self.ty(r);
                match c {
                    Const::Bool(v) => {
                        self.b.i32c(i32::from(*v));
                    }
                    Const::Int(v) => {
                        if vt(t) == ValType::I64 {
                            self.b.i64c(*v as i64);
                        } else {
                            self.b.i32c(*v as i64 as i32);
                        }
                    }
                    Const::F32(bits) => {
                        self.b.s().f32_const(Ieee32::new(*bits));
                    }
                    Const::F64(bits) => {
                        self.b.s().f64_const(Ieee64::new(*bits));
                    }
                }
                self.set(r);
            }
            Op::Null => {
                self.b.i32c(0);
                self.set(res[0]);
            }
            Op::Bin { op, ovf, a, b } => self.bin(*op, *ovf, *a, *b, res[0])?,
            Op::Un { op, ovf, a } => self.un(*op, *ovf, *a, res[0])?,
            Op::Cmp { op, a, b } => {
                let t = self.ty(*a);
                self.get(*a);
                self.get(*b);
                self.cmp(*op, t);
                self.set(res[0]);
            }
            Op::Conv { ovf, a } => self.conv(*ovf, *a, res[0])?,
            Op::Bitcast { a } => {
                let (from, to) = (self.ty(*a), self.ty(res[0]));
                match (from, to) {
                    (Ty::F32, _) => {
                        let l = self.l(*a);
                        self.push_canon_float(l, false);
                        self.b.s().i32_reinterpret_f32();
                    }
                    (Ty::F64, _) => {
                        let l = self.l(*a);
                        self.push_canon_float(l, true);
                        self.b.s().i64_reinterpret_f64();
                    }
                    (_, Ty::F32) => {
                        self.get(*a);
                        self.b.s().f32_reinterpret_i32();
                    }
                    (_, Ty::F64) => {
                        self.get(*a);
                        self.b.s().f64_reinterpret_i64();
                    }
                    _ => self.get(*a),
                }
                self.set(res[0]);
            }
            Op::Select { cond, a, b } => {
                self.get(*a);
                self.get(*b);
                self.get(*cond);
                self.b.s().select();
                self.set(res[0]);
            }
            Op::Alloc(tr) => {
                let (id, var) = {
                    let (rec, var) = self
                        .cx
                        .lay
                        .rec(&tr.ty, tr.variant.as_deref())
                        .ok_or_else(|| Diagnostic::new(E_UNSUPPORTED, "alloc of unknown type", None))?;
                    (rec.id, var)
                };
                self.b.i32c(id as i32);
                self.call_rt("oxi_alloc");
                self.set(res[0]);
                if let Some(v) = var {
                    let l = self.l(res[0]);
                    self.b.get(l).i32c(v as i32);
                    self.b.store(Mem::I32, layout::UNION_OFF_TAG);
                }
            }
            Op::GetField { field, obj } => {
                let (off, fty, var) = self.field(field)?;
                if let Some(v) = var {
                    self.tag_check(*obj, v);
                }
                self.get(*obj);
                self.b.load(mem_of(&fty), off);
                self.set(res[0]);
            }
            Op::SetField {
                field,
                obj,
                val,
                init,
            } => {
                let (off, fty, var) = self.field(field)?;
                if let (Some(v), false) = (var, *init) {
                    self.tag_check(*obj, v);
                }
                let managed = fty.is_managed();
                let old = if managed && !*init {
                    let t = self.temp32(0);
                    self.get(*obj);
                    self.b.load(Mem::I32, off);
                    self.b.set(t);
                    Some(t)
                } else {
                    None
                };
                self.get(*obj);
                self.push_store_val(*val);
                self.b.store(mem_of(&fty), off);
                if let Some(t) = old {
                    self.b.get(t);
                    self.call_rt("oxi_release");
                }
            }
            Op::Tag { obj } => {
                self.get(*obj);
                self.b.load(Mem::I32, layout::UNION_OFF_TAG);
                self.set(res[0]);
            }
            Op::IsUnique { obj } => {
                self.get(*obj);
                self.call_rt("oxi_is_unique");
                self.set(res[0]);
            }
            Op::Clone { obj } => {
                self.get(*obj);
                self.call_rt("oxi_clone");
                self.set(res[0]);
            }
            Op::ArrayNew { len } => {
                let Ty::Array(e) = self.ty(res[0]) else {
                    return self.err("array.new result is not an array");
                };
                let id = *self.cx.lay.array_ids.get(&ty_key(e)).expect("array type registered");
                // n < 0 traps `bounds` here, not in the runtime (whose trap cell is separate)
                self.get(*len);
                self.b.i32c(0);
                self.b.s().i32_lt_s();
                self.trap_if(TrapKind::Bounds);
                self.b.i32c(id as i32);
                self.get(*len);
                self.call_rt("oxi_array_new");
                self.set(res[0]);
            }
            Op::ArrayLen { arr } => {
                self.get(*arr);
                self.b.load(Mem::I32, layout::ARR_OFF_LEN);
                self.set(res[0]);
            }
            Op::ArrayGet { arr, idx } => {
                let Ty::Array(e) = self.ty(*arr) else {
                    return self.err("array.get on a non-array");
                };
                self.elem_addr(*arr, *idx, e);
                self.b.load(mem_of(e), 0);
                self.set(res[0]);
            }
            Op::ArraySet { arr, idx, val } => {
                let Ty::Array(e) = self.ty(*arr) else {
                    return self.err("array.set on a non-array");
                };
                let ta = self.temp32(1);
                self.elem_addr(*arr, *idx, e);
                self.b.set(ta);
                let old = if e.is_managed() {
                    let t = self.temp32(0);
                    self.b.get(ta).load(Mem::I32, 0);
                    self.b.set(t);
                    Some(t)
                } else {
                    None
                };
                self.b.get(ta);
                self.push_store_val(*val);
                self.b.store(mem_of(e), 0);
                if let Some(t) = old {
                    self.b.get(t);
                    self.call_rt("oxi_release");
                }
            }
            Op::ArrayPush { arr, val } => {
                let Ty::Array(e) = self.ty(*arr) else {
                    return self.err("array.push on a non-array");
                };
                let tl = self.temp32(0);
                self.get(*arr);
                self.b.load(Mem::I32, layout::ARR_OFF_LEN);
                self.b.set(tl);
                self.b.get(tl);
                self.get(*arr);
                self.b.load(Mem::I32, layout::ARR_OFF_CAP);
                self.b.s().i32_eq().if_(BlockType::Empty);
                self.b.get(tl).i32c(0x7fff_ffff);
                self.b.s().i32_eq();
                self.trap_if(TrapKind::OutOfMemory);
                self.get(*arr);
                self.b.get(tl).i32c(1);
                self.b.s().i32_add();
                self.call_rt("oxi_array_grow");
                self.b.s().end();
                self.get(*arr);
                self.b.load(Mem::I32, layout::ARR_OFF_DATA);
                self.b.get(tl);
                let sh = mem_of(e).align_log2();
                if sh > 0 {
                    self.b.i32c(sh as i32);
                    self.b.s().i32_shl();
                }
                self.b.s().i32_add();
                self.push_store_val(*val);
                self.b.store(mem_of(e), 0);
                self.get(*arr);
                self.b.get(tl).i32c(1);
                self.b.s().i32_add();
                self.b.store(Mem::I32, layout::ARR_OFF_LEN);
            }
            Op::StrConst { data } => {
                let ix = *self.cx.lay.data_ix.get(data).expect("verified");
                self.b.i32c(self.cx.lay.str_addr[ix] as i32);
                self.set(res[0]);
            }
            Op::StrLen { s } => {
                self.get(*s);
                self.b.load(Mem::I32, layout::STR_OFF_LEN);
                self.set(res[0]);
            }
            Op::StrFromUtf8 { bytes, len } => {
                // 0 <= len <= array length, else bounds
                self.get(*len);
                self.get(*bytes);
                self.b.load(Mem::I32, layout::ARR_OFF_LEN);
                self.b.s().i32_gt_u();
                self.trap_if(TrapKind::Bounds);
                self.get(*bytes);
                self.b.load(Mem::I32, layout::ARR_OFF_DATA);
                self.get(*len);
                self.call_rt("oxi_str_new");
                self.set(res[0]);
            }
            Op::IsNull { val } => {
                self.get(*val);
                self.b.s().i32_eqz();
                self.set(res[0]);
            }
            Op::Opt { val } => {
                self.get(*val);
                self.set(res[0]);
            }
            Op::Unopt { val } => {
                self.get(*val);
                self.b.s().i32_eqz();
                self.trap_if(TrapKind::NullDeref);
                self.get(*val);
                self.set(res[0]);
            }
            Op::ClosureNew { env, func, captures } => {
                let (id, offs, tys) = {
                    let (rec, _) = self
                        .cx
                        .lay
                        .rec(env, None)
                        .ok_or_else(|| Diagnostic::new(E_UNSUPPORTED, "unknown closure env", None))?;
                    (rec.id, rec.offsets.clone(), rec.tys.clone())
                };
                let tix = self.cx.closure_index(func);
                self.b.i32c(id as i32);
                self.call_rt("oxi_alloc");
                self.set(res[0]);
                let l = self.l(res[0]);
                self.b.get(l).i32c(tix as i32);
                self.b.store(Mem::I32, layout::CLOSURE_OFF_FN);
                for ((c, off), ty) in captures.iter().zip(offs).zip(tys) {
                    self.b.get(l);
                    self.push_store_val(*c);
                    self.b.store(mem_of(&ty), off);
                }
            }
            Op::Call { callee, args } => {
                let m: &'a oxiscript_ir::Module = self.cx.m;
                let is_import = m.import(callee).is_some();
                if let Some(imp) = m.import(callee) {
                    let idx = self.cx.imp_idx[callee];
                    for (a, pt) in args.iter().zip(&imp.sig.params) {
                        self.push_host_arg(*a, pt);
                    }
                    self.b.s().call(idx);
                } else {
                    let idx = *self
                        .cx
                        .fn_idx
                        .get(callee)
                        .ok_or_else(|| Diagnostic::new(E_UNSUPPORTED, format!("unknown callee @{callee}"), None))?;
                    for a in args {
                        self.get(*a);
                    }
                    self.b.s().call(idx);
                }
                self.bind(res, is_import);
            }
            Op::CallClosure { callee, args } => {
                let Ty::Fn(sig) = self.ty(*callee) else {
                    return self.err("call_closure on a non-closure");
                };
                let mut params = vec![ValType::I32];
                params.extend(sig.params.iter().map(vt));
                let results: Vec<ValType> = sig.rets.iter().map(vt).collect();
                let ti = self.cx.sig_type(&params, &results);
                self.get(*callee);
                for a in args {
                    self.get(*a);
                }
                self.get(*callee);
                self.b.load(Mem::I32, layout::CLOSURE_OFF_FN);
                self.b.s().call_indirect(0, ti);
                self.bind(res, false);
            }
            Op::Intrinsic { name, args } => {
                let Some(idx) = self.cx.rt_idx.get(name.as_str()).copied() else {
                    return self.err(format!("unknown intrinsic {name}"));
                };
                for a in args {
                    self.get(*a);
                }
                self.b.s().call(idx);
                for r in res.iter().rev() {
                    self.set(*r);
                }
            }
            Op::Retain { val } => {
                self.get(*val);
                self.call_rt("oxi_retain");
            }
            Op::Release { val } => {
                self.get(*val);
                self.call_rt("oxi_release");
            }
            Op::GlobalGet { name } => {
                let g = self.cx.global_idx[name];
                self.b.s().global_get(g);
                self.set(res[0]);
            }
            Op::GlobalSet { name, val } => {
                let g = self.cx.global_idx[name];
                let m: &'a oxiscript_ir::Module = self.cx.m;
                let gty = &m.global(name).expect("verified").ty;
                if gty.is_managed() {
                    let t = self.temp32(0);
                    self.b.s().global_get(g);
                    self.b.set(t);
                    self.get(*val);
                    self.b.s().global_set(g);
                    self.b.get(t);
                    self.call_rt("oxi_release");
                } else {
                    self.push_store_val(*val);
                    self.b.s().global_set(g);
                }
            }
            Op::Load { ptr, offset } => {
                let t = self.ty(res[0]);
                self.get(*ptr);
                self.b.load(mem_of(t), *offset);
                self.set(res[0]);
            }
            Op::Store { ptr, offset, val } => {
                let t = self.ty(*val);
                self.get(*ptr);
                self.push_store_val(*val);
                self.b.store(mem_of(t), *offset);
            }
            _ => return self.err("unsupported instruction"),
        }
        Ok(())
    }

    /// Push the address of `arr[idx]` after a bounds check.
    fn elem_addr(&mut self, arr: ValueId, idx: ValueId, e: &Ty) {
        self.get(idx);
        self.get(arr);
        self.b.load(Mem::I32, layout::ARR_OFF_LEN);
        self.b.s().i32_ge_u();
        self.trap_if(TrapKind::Bounds);
        self.get(arr);
        self.b.load(Mem::I32, layout::ARR_OFF_DATA);
        self.get(idx);
        let sh = mem_of(e).align_log2();
        if sh > 0 {
            self.b.i32c(sh as i32);
            self.b.s().i32_shl();
        }
        self.b.s().i32_add();
    }

    /// Push one import argument, flattening `str`/`[u8]` to `(ptr, len)`.
    pub(crate) fn push_host_arg(&mut self, a: ValueId, pt: &Ty) {
        match pt {
            Ty::Str => {
                self.get(a);
                self.b.i32c(layout::STR_OFF_BYTES as i32);
                self.b.s().i32_add();
                self.get(a);
                self.b.load(Mem::I32, layout::STR_OFF_LEN);
            }
            Ty::Array(_) => {
                self.get(a);
                self.b.load(Mem::I32, layout::ARR_OFF_DATA);
                self.get(a);
                self.b.load(Mem::I32, layout::ARR_OFF_LEN);
            }
            Ty::F32 | Ty::F64 => {
                let l = self.l(a);
                self.push_canon_float(l, *pt == Ty::F64);
            }
            _ => self.get(a),
        }
    }

    // ------------------------------------------------------------ numerics

    fn cmp(&mut self, op: CmpOp, t: &Ty) {
        let wide = vt(t) == ValType::I64;
        let signed = is_signed(t);
        let mut s = self.b.s();
        match vt(t) {
            ValType::F32 => match op {
                CmpOp::Eq => s.f32_eq(),
                CmpOp::Ne => s.f32_ne(),
                CmpOp::Lt => s.f32_lt(),
                CmpOp::Le => s.f32_le(),
                CmpOp::Gt => s.f32_gt(),
                CmpOp::Ge => s.f32_ge(),
            },
            ValType::F64 => match op {
                CmpOp::Eq => s.f64_eq(),
                CmpOp::Ne => s.f64_ne(),
                CmpOp::Lt => s.f64_lt(),
                CmpOp::Le => s.f64_le(),
                CmpOp::Gt => s.f64_gt(),
                CmpOp::Ge => s.f64_ge(),
            },
            _ if wide => match (op, signed) {
                (CmpOp::Eq, _) => s.i64_eq(),
                (CmpOp::Ne, _) => s.i64_ne(),
                (CmpOp::Lt, true) => s.i64_lt_s(),
                (CmpOp::Lt, false) => s.i64_lt_u(),
                (CmpOp::Le, true) => s.i64_le_s(),
                (CmpOp::Le, false) => s.i64_le_u(),
                (CmpOp::Gt, true) => s.i64_gt_s(),
                (CmpOp::Gt, false) => s.i64_gt_u(),
                (CmpOp::Ge, true) => s.i64_ge_s(),
                (CmpOp::Ge, false) => s.i64_ge_u(),
            },
            _ => match (op, signed) {
                (CmpOp::Eq, _) => s.i32_eq(),
                (CmpOp::Ne, _) => s.i32_ne(),
                (CmpOp::Lt, true) => s.i32_lt_s(),
                (CmpOp::Lt, false) => s.i32_lt_u(),
                (CmpOp::Le, true) => s.i32_le_s(),
                (CmpOp::Le, false) => s.i32_le_u(),
                (CmpOp::Gt, true) => s.i32_gt_s(),
                (CmpOp::Gt, false) => s.i32_gt_u(),
                (CmpOp::Ge, true) => s.i32_ge_s(),
                (CmpOp::Ge, false) => s.i32_ge_u(),
            },
        };
    }

    fn raw_arith(&mut self, op: BinOp, wide: bool) {
        let mut s = self.b.s();
        match (op, wide) {
            (BinOp::Add, false) => s.i32_add(),
            (BinOp::Add, true) => s.i64_add(),
            (BinOp::Sub, false) => s.i32_sub(),
            (BinOp::Sub, true) => s.i64_sub(),
            (BinOp::Mul, false) => s.i32_mul(),
            (BinOp::Mul, true) => s.i64_mul(),
            (BinOp::And, false) => s.i32_and(),
            (BinOp::And, true) => s.i64_and(),
            (BinOp::Or, false) => s.i32_or(),
            (BinOp::Or, true) => s.i64_or(),
            (BinOp::Xor, false) => s.i32_xor(),
            (BinOp::Xor, true) => s.i64_xor(),
            _ => s.unreachable(),
        };
    }

    fn bin(&mut self, op: BinOp, ovf: Ovf, a: ValueId, b: ValueId, dst: ValueId) -> R {
        let t = self.ty(dst);
        if t.is_float() {
            self.get(a);
            self.get(b);
            let is64 = *t == Ty::F64;
            let mut s = self.b.s();
            match (op, is64) {
                (BinOp::Add, false) => s.f32_add(),
                (BinOp::Add, true) => s.f64_add(),
                (BinOp::Sub, false) => s.f32_sub(),
                (BinOp::Sub, true) => s.f64_sub(),
                (BinOp::Mul, false) => s.f32_mul(),
                (BinOp::Mul, true) => s.f64_mul(),
                (BinOp::Div, false) => s.f32_div(),
                (BinOp::Div, true) => s.f64_div(),
                (BinOp::Min, false) => s.f32_min(),
                (BinOp::Min, true) => s.f64_min(),
                (BinOp::Max, false) => s.f32_max(),
                (BinOp::Max, true) => s.f64_max(),
                _ => return self.err("unsupported float operation"),
            };
            if self.cx.safety && matches!(op, BinOp::Min | BinOp::Max) {
                self.canon_top(is64);
            }
            self.set(dst);
            return Ok(());
        }
        if *t == Ty::Bool {
            self.get(a);
            self.get(b);
            self.raw_arith(op, false);
            self.set(dst);
            return Ok(());
        }
        let (la, lb, ld) = (self.l(a), self.l(b), self.l(dst));
        let bits = ibits(t);
        let signed = is_signed(t);
        let wide = bits == 64;
        match op {
            BinOp::And | BinOp::Or | BinOp::Xor => {
                self.b.get(la).get(lb);
                self.raw_arith(op, wide);
                self.b.set(ld);
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul => self.arith(op, ovf, t, la, lb, ld)?,
            BinOp::Div | BinOp::Rem => self.divrem(op, ovf, t, la, lb, ld),
            BinOp::Shl | BinOp::Shr => self.shift(op, ovf, t, la, lb, ld),
            _ => return self.err("unsupported integer operation"),
        }
        let _ = signed;
        Ok(())
    }

    fn arith(&mut self, op: BinOp, ovf: Ovf, t: &Ty, a: u32, b: u32, d: u32) -> R {
        let bits = ibits(t);
        let signed = is_signed(t);
        let wide = bits == 64;
        match ovf {
            Ovf::Wrap => {
                self.b.get(a).get(b);
                self.raw_arith(op, wide);
                self.canon(t);
                self.b.set(d);
            }
            Ovf::Trap => {
                if bits < 32 {
                    self.b.get(a).get(b);
                    self.raw_arith(op, false);
                    self.b.set(d);
                    self.b.get(d);
                    if signed {
                        if bits == 8 {
                            self.b.s().i32_extend8_s();
                        } else {
                            self.b.s().i32_extend16_s();
                        }
                        self.b.get(d);
                        self.b.s().i32_ne();
                    } else {
                        self.b.i32c(if bits == 8 { 0xff } else { 0xffff });
                        self.b.s().i32_gt_u();
                    }
                    self.trap_if(TrapKind::Overflow);
                } else if wide && op == BinOp::Mul {
                    self.b.get(a).get(b);
                    let h = self.cx.helper(HKey::Mul64 { signed });
                    self.b.s().call(h);
                    self.b.set(d);
                } else if op == BinOp::Mul {
                    // 32-bit: multiply in 64 bits and check the range
                    let t64 = self.temp64(0);
                    self.b.get(a);
                    if signed {
                        self.b.s().i64_extend_i32_s();
                    } else {
                        self.b.s().i64_extend_i32_u();
                    }
                    self.b.get(b);
                    if signed {
                        self.b.s().i64_extend_i32_s();
                    } else {
                        self.b.s().i64_extend_i32_u();
                    }
                    self.b.s().i64_mul();
                    self.b.tee(t64);
                    self.b.s().i32_wrap_i64();
                    self.b.set(d);
                    if signed {
                        self.b.get(d);
                        self.b.s().i64_extend_i32_s();
                        self.b.get(t64);
                        self.b.s().i64_ne();
                    } else {
                        self.b.get(t64);
                        self.b.i64c(0xFFFF_FFFF);
                        self.b.s().i64_gt_u();
                    }
                    self.trap_if(TrapKind::Overflow);
                } else {
                    // add / sub, 32 or 64 bits
                    self.b.get(a).get(b);
                    self.raw_arith(op, wide);
                    self.b.set(d);
                    if signed {
                        if op == BinOp::Add {
                            self.b.get(a).get(d);
                            self.xor_w(wide);
                            self.b.get(b).get(d);
                            self.xor_w(wide);
                        } else {
                            self.b.get(a).get(b);
                            self.xor_w(wide);
                            self.b.get(a).get(d);
                            self.xor_w(wide);
                        }
                        if wide {
                            self.b.s().i64_and();
                            self.b.i64c(0);
                            self.b.s().i64_lt_s();
                        } else {
                            self.b.s().i32_and();
                            self.b.i32c(0);
                            self.b.s().i32_lt_s();
                        }
                    } else if op == BinOp::Add {
                        self.b.get(d).get(a);
                        if wide {
                            self.b.s().i64_lt_u();
                        } else {
                            self.b.s().i32_lt_u();
                        }
                    } else {
                        self.b.get(a).get(b);
                        if wide {
                            self.b.s().i64_lt_u();
                        } else {
                            self.b.s().i32_lt_u();
                        }
                    }
                    self.trap_if(TrapKind::Overflow);
                }
            }
            Ovf::Sat => {
                if wide {
                    let code = match op {
                        BinOp::Add => 0,
                        BinOp::Sub => 1,
                        _ => 2,
                    };
                    self.b.get(a).get(b);
                    let h = self.cx.helper(HKey::Sat64 { op: code, signed });
                    self.b.s().call(h);
                    self.b.set(d);
                } else {
                    let (min, max) = int_range(bits, signed);
                    let t64 = self.temp64(0);
                    for v in [a, b] {
                        self.b.get(v);
                        if signed {
                            self.b.s().i64_extend_i32_s();
                        } else {
                            self.b.s().i64_extend_i32_u();
                        }
                    }
                    self.raw_arith(op, true);
                    self.b.set(t64);
                    let unsigned_mul = op == BinOp::Mul && !signed && bits == 32;
                    // t = min(t, max)
                    self.b.get(t64).i64c(max as i64).get(t64).i64c(max as i64);
                    if unsigned_mul {
                        self.b.s().i64_le_u();
                    } else {
                        self.b.s().i64_le_s();
                    }
                    self.b.s().select();
                    self.b.set(t64);
                    // t = max(t, min)
                    self.b.get(t64).i64c(min as i64).get(t64).i64c(min as i64);
                    self.b.s().i64_ge_s().select();
                    self.b.s().i32_wrap_i64();
                    self.b.set(d);
                }
            }
        }
        Ok(())
    }

    fn xor_w(&mut self, wide: bool) {
        if wide {
            self.b.s().i64_xor();
        } else {
            self.b.s().i32_xor();
        }
    }

    fn divrem(&mut self, op: BinOp, ovf: Ovf, t: &Ty, a: u32, b: u32, d: u32) {
        let bits = ibits(t);
        let signed = is_signed(t);
        let wide = bits == 64;
        let div = op == BinOp::Div;
        self.b.get(b);
        if wide {
            self.b.s().i64_eqz();
        } else {
            self.b.s().i32_eqz();
        }
        self.trap_if(TrapKind::DivByZero);
        let native = |g: &mut Self| {
            let mut s = g.b.s();
            match (div, signed, wide) {
                (true, true, false) => s.i32_div_s(),
                (true, true, true) => s.i64_div_s(),
                (true, false, false) => s.i32_div_u(),
                (true, false, true) => s.i64_div_u(),
                (false, true, false) => s.i32_rem_s(),
                (false, true, true) => s.i64_rem_s(),
                (false, false, false) => s.i32_rem_u(),
                (false, false, true) => s.i64_rem_u(),
            };
        };
        if signed && ovf == Ovf::Trap {
            let (min, _) = int_range(bits, true);
            self.b.get(a);
            if wide {
                self.b.i64c(min as i64);
                self.b.s().i64_eq();
            } else {
                self.b.i32c(min as i32);
                self.b.s().i32_eq();
            }
            self.b.get(b);
            if wide {
                self.b.i64c(-1);
                self.b.s().i64_eq();
            } else {
                self.b.i32c(-1);
                self.b.s().i32_eq();
            }
            self.b.s().i32_and();
            self.trap_if(TrapKind::Overflow);
            self.b.get(a).get(b);
            native(self);
        } else if signed && div {
            // wrap: x / -1 = 0 - x (wrapping), avoiding the native overflow trap
            let rt = if wide { ValType::I64 } else { ValType::I32 };
            self.b.get(b);
            if wide {
                self.b.i64c(-1);
                self.b.s().i64_eq();
            } else {
                self.b.i32c(-1);
                self.b.s().i32_eq();
            }
            self.b.s().if_(BlockType::Result(rt));
            if wide {
                self.b.i64c(0);
                self.b.get(a);
                self.b.s().i64_sub();
            } else {
                self.b.i32c(0);
                self.b.get(a);
                self.b.s().i32_sub();
            }
            self.b.s().else_();
            self.b.get(a).get(b);
            native(self);
            self.b.s().end();
        } else {
            self.b.get(a).get(b);
            native(self);
        }
        if !wide {
            self.canon(t);
        }
        self.b.set(d);
    }

    fn shift(&mut self, op: BinOp, ovf: Ovf, t: &Ty, a: u32, b: u32, d: u32) {
        let bits = ibits(t);
        let wide = bits == 64;
        let signed = is_signed(t);
        if ovf == Ovf::Trap {
            self.b.get(b);
            if wide {
                self.b.i64c(64);
                self.b.s().i64_ge_u();
            } else {
                self.b.i32c(bits as i32);
                self.b.s().i32_ge_u();
            }
            self.trap_if(TrapKind::Bounds);
        }
        self.b.get(a).get(b);
        if ovf != Ovf::Trap && bits < 32 {
            self.b.i32c(bits as i32 - 1);
            self.b.s().i32_and();
        }
        let mut s = self.b.s();
        match (op == BinOp::Shl, signed, wide) {
            (true, _, false) => s.i32_shl(),
            (true, _, true) => s.i64_shl(),
            (false, true, false) => s.i32_shr_s(),
            (false, true, true) => s.i64_shr_s(),
            (false, false, false) => s.i32_shr_u(),
            (false, false, true) => s.i64_shr_u(),
        };
        if !wide {
            self.canon(t);
        }
        self.b.set(d);
    }

    fn un(&mut self, op: UnOp, ovf: Ovf, a: ValueId, dst: ValueId) -> R {
        let t = self.ty(dst);
        let (la, ld) = (self.l(a), self.l(dst));
        if t.is_float() {
            let is64 = *t == Ty::F64;
            self.b.get(la);
            let mut s = self.b.s();
            match (op, is64) {
                (UnOp::Neg, false) => s.f32_neg(),
                (UnOp::Neg, true) => s.f64_neg(),
                (UnOp::Abs, false) => s.f32_abs(),
                (UnOp::Abs, true) => s.f64_abs(),
                (UnOp::Sqrt, false) => s.f32_sqrt(),
                (UnOp::Sqrt, true) => s.f64_sqrt(),
                (UnOp::Floor, false) => s.f32_floor(),
                (UnOp::Floor, true) => s.f64_floor(),
                (UnOp::Ceil, false) => s.f32_ceil(),
                (UnOp::Ceil, true) => s.f64_ceil(),
                (UnOp::Trunc, false) => s.f32_trunc(),
                (UnOp::Trunc, true) => s.f64_trunc(),
                (UnOp::Nearest, false) => s.f32_nearest(),
                (UnOp::Nearest, true) => s.f64_nearest(),
                _ => return self.err("unsupported float unary operation"),
            };
            self.b.set(ld);
            return Ok(());
        }
        if *t == Ty::Bool {
            self.b.get(la);
            self.b.s().i32_eqz();
            self.b.set(ld);
            return Ok(());
        }
        let bits = ibits(t);
        let wide = bits == 64;
        let signed = is_signed(t);
        match op {
            UnOp::Neg => {
                if ovf == Ovf::Sat {
                    return self.err("neg.sat is not defined");
                }
                if ovf == Ovf::Trap {
                    self.b.get(la);
                    if signed {
                        let (min, _) = int_range(bits, true);
                        if wide {
                            self.b.i64c(min as i64);
                            self.b.s().i64_eq();
                        } else {
                            self.b.i32c(min as i32);
                            self.b.s().i32_eq();
                        }
                    } else if wide {
                        self.b.s().i64_eqz().i32_eqz();
                    } else {
                        self.b.s().i32_eqz().i32_eqz();
                    }
                    self.trap_if(TrapKind::Overflow);
                }
                if wide {
                    self.b.i64c(0);
                    self.b.get(la);
                    self.b.s().i64_sub();
                } else {
                    self.b.i32c(0);
                    self.b.get(la);
                    self.b.s().i32_sub();
                    self.canon(t);
                }
            }
            UnOp::Not => {
                self.b.get(la);
                if wide {
                    self.b.i64c(-1);
                    self.b.s().i64_xor();
                } else {
                    self.b.i32c(-1);
                    self.b.s().i32_xor();
                    self.canon(t);
                }
            }
            UnOp::Clz | UnOp::Ctz | UnOp::Popcnt => {
                self.b.get(la);
                if wide {
                    let mut s = self.b.s();
                    match op {
                        UnOp::Clz => s.i64_clz(),
                        UnOp::Ctz => s.i64_ctz(),
                        _ => s.i64_popcnt(),
                    };
                } else {
                    let mask = if bits == 32 { -1 } else { (1i32 << bits) - 1 };
                    match op {
                        UnOp::Clz => {
                            if bits < 32 {
                                self.b.i32c(mask);
                                self.b.s().i32_and();
                            }
                            self.b.s().i32_clz();
                            if bits < 32 {
                                self.b.i32c(32 - bits as i32);
                                self.b.s().i32_sub();
                            }
                        }
                        UnOp::Ctz => {
                            if bits < 32 {
                                self.b.i32c(1 << bits);
                                self.b.s().i32_or();
                            }
                            self.b.s().i32_ctz();
                        }
                        _ => {
                            if bits < 32 {
                                self.b.i32c(mask);
                                self.b.s().i32_and();
                            }
                            self.b.s().i32_popcnt();
                        }
                    }
                }
            }
            _ => return self.err("unsupported integer unary operation"),
        }
        self.b.set(ld);
        Ok(())
    }

    fn conv(&mut self, ovf: Ovf, a: ValueId, dst: ValueId) -> R {
        let (from, to) = (self.ty(a), self.ty(dst));
        let la = self.l(a);
        let ld = self.l(dst);
        match (from.is_float(), to.is_float()) {
            (true, true) => {
                self.b.get(la);
                if *from == Ty::F32 {
                    self.b.s().f64_promote_f32();
                } else {
                    self.b.s().f32_demote_f64();
                }
                if self.cx.safety {
                    self.canon_top(*to == Ty::F64);
                }
                self.b.set(ld);
            }
            (false, true) => {
                self.b.get(la);
                let signed = is_signed(from);
                let wide = ibits(from) == 64;
                let mut s = self.b.s();
                match (*to == Ty::F64, wide, signed) {
                    (false, false, true) => s.f32_convert_i32_s(),
                    (false, false, false) => s.f32_convert_i32_u(),
                    (false, true, true) => s.f32_convert_i64_s(),
                    (false, true, false) => s.f32_convert_i64_u(),
                    (true, false, true) => s.f64_convert_i32_s(),
                    (true, false, false) => s.f64_convert_i32_u(),
                    (true, true, true) => s.f64_convert_i64_s(),
                    (true, true, false) => s.f64_convert_i64_u(),
                };
                self.b.set(ld);
            }
            (true, false) => {
                let sat = match ovf {
                    Ovf::Trap => false,
                    Ovf::Sat => true,
                    Ovf::Wrap => return self.err("conv.wrap from a float"),
                };
                self.b.get(la);
                let h = self.cx.helper(HKey::F2I {
                    src64: *from == Ty::F64,
                    dst: ty_code(to),
                    sat,
                });
                self.b.s().call(h);
                self.b.set(ld);
            }
            (false, false) => self.int_conv(ovf, from, to, la, ld),
        }
        Ok(())
    }

    fn int_conv(&mut self, ovf: Ovf, from: &Ty, to: &Ty, a: u32, d: u32) {
        let (fb, fs) = (ibits(from), is_signed(from));
        let (tb, ts) = (ibits(to), is_signed(to));
        let fw = fb == 64;
        let tw = tb == 64;
        if ovf == Ovf::Wrap {
            self.b.get(a);
            match (fw, tw) {
                (false, true) => {
                    if fs {
                        self.b.s().i64_extend_i32_s();
                    } else {
                        self.b.s().i64_extend_i32_u();
                    }
                }
                (true, false) => {
                    self.b.s().i32_wrap_i64();
                    self.canon(to);
                }
                (false, false) => self.canon(to),
                (true, true) => {}
            }
            self.b.set(d);
            return;
        }
        let (smin, smax) = int_range(fb, fs);
        let (dmin, dmax) = int_range(tb, ts);
        if smin >= dmin && smax <= dmax {
            // always fits: pure widening / re-typing
            self.b.get(a);
            match (fw, tw) {
                (false, true) => {
                    if fs {
                        self.b.s().i64_extend_i32_s();
                    } else {
                        self.b.s().i64_extend_i32_u();
                    }
                }
                _ => {}
            }
            self.b.set(d);
            return;
        }
        let t = self.temp64(0);
        self.b.get(a);
        if !fw {
            if fs {
                self.b.s().i64_extend_i32_s();
            } else {
                self.b.s().i64_extend_i32_u();
            }
        }
        self.b.set(t);
        let from_u64 = fb == 64 && !fs;
        let sat = ovf == Ovf::Sat;
        if from_u64 {
            if dmax < (1i128 << 64) - 1 {
                if sat {
                    self.b.get(t).i64c(dmax as i64).get(t).i64c(dmax as i64);
                    self.b.s().i64_le_u().select();
                    self.b.set(t);
                } else {
                    self.b.get(t).i64c(dmax as i64);
                    self.b.s().i64_gt_u();
                    self.trap_if(TrapKind::Overflow);
                }
            }
        } else {
            if dmin > smin {
                if sat {
                    self.b.get(t).i64c(dmin as i64).get(t).i64c(dmin as i64);
                    self.b.s().i64_ge_s().select();
                    self.b.set(t);
                } else {
                    self.b.get(t).i64c(dmin as i64);
                    self.b.s().i64_lt_s();
                    self.trap_if(TrapKind::Overflow);
                }
            }
            if dmax < smax {
                if sat {
                    self.b.get(t).i64c(dmax as i64).get(t).i64c(dmax as i64);
                    self.b.s().i64_le_s().select();
                    self.b.set(t);
                } else {
                    self.b.get(t).i64c(dmax as i64);
                    self.b.s().i64_gt_s();
                    self.trap_if(TrapKind::Overflow);
                }
            }
        }
        self.b.get(t);
        if !tw {
            self.b.s().i32_wrap_i64();
        }
        self.b.set(d);
    }
}

/// Push the canonical-NaN form of the float in local `l` (NaN -> canonical NaN, else itself).
pub(crate) fn push_canon_float_body(b: &mut Body, l: u32, is64: bool) {
    b.get(l);
    if is64 {
        b.s().f64_const(Ieee64::new(0x7ff8_0000_0000_0000));
    } else {
        b.s().f32_const(Ieee32::new(0x7fc0_0000));
    }
    b.get(l).get(l);
    if is64 {
        b.s().f64_eq();
    } else {
        b.s().f32_eq();
    }
    b.s().select();
}
