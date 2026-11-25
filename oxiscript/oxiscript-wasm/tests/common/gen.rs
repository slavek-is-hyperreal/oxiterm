//! Seeded random generator of well-typed scalar OxiIR (arithmetic, control
//! flow, calls). Every function is exported so tests can drive it directly.
#![allow(dead_code, clippy::pedantic)]

use oxiscript_ir::build::{renumber, target, FuncBuilder};
use oxiscript_ir::*;

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

const INTS: [Ty; 8] = [
    Ty::I8,
    Ty::I16,
    Ty::I32,
    Ty::I64,
    Ty::U8,
    Ty::U16,
    Ty::U32,
    Ty::U64,
];

pub fn rand_ty(r: &mut Rng) -> Ty {
    match r.below(12) {
        0 => Ty::Bool,
        1 => Ty::F32,
        2 => Ty::F64,
        n => INTS[n % 8].clone(),
    }
}

fn int_range(t: &Ty) -> (i128, i128) {
    let b = t.bits().unwrap();
    if t.is_signed_int() {
        (-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1)
    } else {
        (0, (1i128 << b) - 1)
    }
}

/// Interesting values of a type, as IR constants.
/// An arbitrary (non-edge-biased) value of the type.
pub fn rand_const(r: &mut Rng, t: &Ty) -> Const {
    match t {
        Ty::Bool => Const::Bool(r.chance(50)),
        Ty::F32 => Const::F32((r.next() as u32) & 0x7f7f_ffff | ((r.next() as u32) & 0x8000_0000)),
        Ty::F64 => Const::F64((r.next() & 0x7fef_ffff_ffff_ffff) | (r.next() & 0x8000_0000_0000_0000)),
        _ => {
            let (lo, hi) = int_range(t);
            let span = (hi - lo + 1) as u128;
            let raw = (u128::from(r.next()) << 64 | u128::from(r.next())) % span;
            Const::Int(lo + raw as i128)
        }
    }
}

pub fn edge_consts(t: &Ty) -> Vec<Const> {
    match t {
        Ty::Bool => vec![Const::Bool(false), Const::Bool(true)],
        Ty::F32 => [0.0f32, -0.0, 1.0, -1.5, 0.5, 2.5, 3.0e9, -3.0e9, 1.0e30, f32::INFINITY, f32::NEG_INFINITY, f32::NAN, 127.9, -128.9, 255.5, 65535.7, 2147483520.0]
            .iter()
            .map(|f| Const::F32(f.to_bits()))
            .collect(),
        Ty::F64 => [0.0f64, -0.0, 1.0, -1.5, 0.5, 2.5, 4.3e9, -2147483648.5, -2147483649.0, 9.3e18, -9.3e18, 1.0e300, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 18446744073709551615.0, 255.5, -0.9]
            .iter()
            .map(|f| Const::F64(f.to_bits()))
            .collect(),
        _ => {
            let (lo, hi) = int_range(t);
            let mut v = vec![0, 1, 2, 3, 7, 8, 31, 32, 63, 64, -1, lo, hi, lo + 1, hi - 1, hi / 2, 100, 255, -128];
            v.retain(|x| *x >= lo && *x <= hi);
            v.into_iter().map(Const::Int).collect()
        }
    }
}

struct G<'a> {
    r: &'a mut Rng,
    b: FuncBuilder,
    n: usize,
    callees: &'a [(Vec<Ty>, Vec<Ty>)],
}

type Env = Vec<(ValueId, Ty)>;

impl<'a> G<'a> {
    fn name(&mut self) -> String {
        self.n += 1;
        format!("v{}", self.n)
    }

    fn konst(&mut self, blk: BlockId, env: &mut Env, t: &Ty) -> ValueId {
        let cs = edge_consts(t);
        let c = if self.r.chance(50) {
            // arbitrary value
            match t {
                Ty::F32 => Const::F32((self.r.next() as u32) & 0x7f7f_ffff | ((self.r.next() as u32) & 0x8000_0000)),
                Ty::F64 => Const::F64(f64::from_bits(self.r.next() & 0x7fef_ffff_ffff_ffff).to_bits()),
                Ty::Bool => Const::Bool(self.r.chance(50)),
                _ => {
                    let (lo, hi) = int_range(t);
                    let span = (hi - lo + 1) as u128;
                    Const::Int(lo + (((u128::from(self.r.next()) * 0x1_0000_0001) ^ u128::from(self.r.next())) % span) as i128)
                }
            }
        } else {
            cs[self.r.below(cs.len())].clone()
        };
        let nm = self.name();
        let v = self.b.inst1(blk, Op::Const(c), &nm, t.clone());
        env.push((v, t.clone()));
        v
    }

    fn pick(&mut self, blk: BlockId, env: &mut Env, t: &Ty) -> ValueId {
        let c: Vec<ValueId> = env.iter().filter(|e| e.1 == *t).map(|e| e.0).collect();
        if c.is_empty() || self.r.chance(12) {
            return self.konst(blk, env, t);
        }
        c[self.r.below(c.len())]
    }

    fn emit(&mut self, blk: BlockId, env: &mut Env, op: Op, t: Ty) -> ValueId {
        let nm = self.name();
        let v = self.b.inst1(blk, op, &nm, t.clone());
        env.push((v, t));
        v
    }

    fn ovf(&mut self, allow_sat: bool) -> Ovf {
        match self.r.below(if allow_sat { 3 } else { 2 }) {
            0 => Ovf::Trap,
            1 => Ovf::Wrap,
            _ => Ovf::Sat,
        }
    }

    /// One straight-line instruction producing a value of type `t` (or a comparison etc.).
    fn stmt(&mut self, blk: BlockId, env: &mut Env) {
        let t = rand_ty(self.r);
        self.compute(blk, env, &t);
    }

    fn compute(&mut self, blk: BlockId, env: &mut Env, t: &Ty) -> ValueId {
        let k = self.r.below(100);
        if *t == Ty::Bool {
            return match k % 4 {
                0 | 1 => {
                    let ot = loop {
                        let c = rand_ty(self.r);
                        if c != Ty::Bool || self.r.chance(20) {
                            break c;
                        }
                    };
                    let a = self.pick(blk, env, &ot);
                    let b = self.pick(blk, env, &ot);
                    let ops = [CmpOp::Eq, CmpOp::Ne, CmpOp::Lt, CmpOp::Le, CmpOp::Gt, CmpOp::Ge];
                    let mut op = ops[self.r.below(6)];
                    if ot == Ty::Bool && !matches!(op, CmpOp::Eq | CmpOp::Ne) {
                        op = CmpOp::Eq;
                    }
                    self.emit(blk, env, Op::Cmp { op, a, b }, Ty::Bool)
                }
                2 => {
                    let a = self.pick(blk, env, t);
                    let b = self.pick(blk, env, t);
                    let op = [BinOp::And, BinOp::Or, BinOp::Xor][self.r.below(3)];
                    self.emit(blk, env, Op::Bin { op, ovf: Ovf::Trap, a, b }, Ty::Bool)
                }
                _ => {
                    let a = self.pick(blk, env, t);
                    self.emit(blk, env, Op::Un { op: UnOp::Not, ovf: Ovf::Trap, a }, Ty::Bool)
                }
            };
        }
        if k < 6 {
            let c = self.pick(blk, env, &Ty::Bool);
            let a = self.pick(blk, env, t);
            let b = self.pick(blk, env, t);
            return self.emit(blk, env, Op::Select { cond: c, a, b }, t.clone());
        }
        if k < 22 {
            // conversion from another numeric type
            let from = loop {
                let c = rand_ty(self.r);
                if c != Ty::Bool && c != *t {
                    break c;
                }
            };
            let ovf = if from.is_float() || t.is_float() {
                if t.is_int() { [Ovf::Trap, Ovf::Sat][self.r.below(2)] } else { Ovf::Trap }
            } else {
                self.ovf(true)
            };
            let a = self.pick(blk, env, &from);
            return self.emit(blk, env, Op::Conv { ovf, a }, t.clone());
        }
        if k < 27 && matches!(t, Ty::I32 | Ty::U32 | Ty::I64 | Ty::U64 | Ty::F32 | Ty::F64) {
            let from = match t {
                Ty::I32 | Ty::U32 => Ty::F32,
                Ty::I64 | Ty::U64 => Ty::F64,
                Ty::F32 => [Ty::I32, Ty::U32][self.r.below(2)].clone(),
                _ => [Ty::I64, Ty::U64][self.r.below(2)].clone(),
            };
            let a = self.pick(blk, env, &from);
            return self.emit(blk, env, Op::Bitcast { a }, t.clone());
        }
        if k < 36 && !self.callees.is_empty() {
            let ci = self.r.below(self.callees.len());
            let (ps, rts) = self.callees[ci].clone();
            let args: Vec<ValueId> = ps.iter().map(|p| self.pick(blk, env, p)).collect();
            let names: Vec<String> = rts.iter().map(|_| self.name()).collect();
            let res: Vec<(&str, Ty)> = names.iter().map(String::as_str).zip(rts.iter().cloned()).collect();
            let vs = self.b.inst(blk, Op::Call { callee: format!("f{ci}"), args }, &res);
            for (v, t2) in vs.iter().zip(&rts) {
                env.push((*v, t2.clone()));
            }
            if let Some(i) = rts.iter().position(|x| x == t) {
                return vs[i];
            }
        }
        if t.is_float() {
            let a = self.pick(blk, env, t);
            if k < 60 {
                let b = self.pick(blk, env, t);
                let op = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Min, BinOp::Max][self.r.below(6)];
                return self.emit(blk, env, Op::Bin { op, ovf: Ovf::Trap, a, b }, t.clone());
            }
            let op = [UnOp::Neg, UnOp::Abs, UnOp::Sqrt, UnOp::Floor, UnOp::Ceil, UnOp::Trunc, UnOp::Nearest][self.r.below(7)];
            return self.emit(blk, env, Op::Un { op, ovf: Ovf::Trap, a }, t.clone());
        }
        // integer
        let a = self.pick(blk, env, t);
        if k < 80 {
            let b = self.pick(blk, env, t);
            let op = [BinOp::Add, BinOp::Sub, BinOp::Mul, BinOp::Div, BinOp::Rem, BinOp::And, BinOp::Or, BinOp::Xor, BinOp::Shl, BinOp::Shr][self.r.below(10)];
            let ovf = match op {
                BinOp::Add | BinOp::Sub | BinOp::Mul => self.ovf(true),
                BinOp::And | BinOp::Or | BinOp::Xor => Ovf::Trap,
                _ => self.ovf(false),
            };
            let b = if matches!(op, BinOp::Shl | BinOp::Shr) && self.r.chance(60) {
                // usually a sane shift amount
                let amt = self.r.below(t.bits().unwrap() as usize) as i128;
                let nm = self.name();
                let v = self.b.inst1(blk, Op::Const(Const::Int(amt)), &nm, t.clone());
                env.push((v, t.clone()));
                v
            } else {
                b
            };
            return self.emit(blk, env, Op::Bin { op, ovf, a, b }, t.clone());
        }
        let op = [UnOp::Neg, UnOp::Not, UnOp::Clz, UnOp::Ctz, UnOp::Popcnt][self.r.below(5)];
        let ovf = if op == UnOp::Neg { self.ovf(false) } else { Ovf::Trap };
        self.emit(blk, env, Op::Un { op, ovf, a }, t.clone())
    }

    /// Generate statements into `cur`; returns the block where control continues.
    fn seq(&mut self, mut cur: BlockId, env: &mut Env, depth: u32, n: usize) -> BlockId {
        for _ in 0..n {
            let k = self.r.below(100);
            if k < 70 || depth >= 3 {
                self.stmt(cur, env);
            } else if k < 82 {
                cur = self.diamond(cur, env, depth);
            } else if k < 92 {
                cur = self.counted_loop(cur, env, depth);
            } else if k < 98 {
                cur = self.switch(cur, env, depth);
            } else if k < 99 {
                cur = self.guard(cur, env);
            }
        }
        cur
    }

    fn guard(&mut self, cur: BlockId, env: &mut Env) -> BlockId {
        let c = self.pick(cur, env, &Ty::Bool);
        let ok = self.b.block("ok");
        let bad = self.b.block("bad");
        self.b.terminate(cur, Terminator::BrIf { cond: c, then: target(bad, &[]), els: target(ok, &[]) });
        let k = TrapKind::ALL[self.r.below(TrapKind::ALL.len())];
        self.b.terminate(bad, Terminator::Trap(k));
        ok
    }

    fn diamond(&mut self, cur: BlockId, env: &mut Env, depth: u32) -> BlockId {
        let c = self.pick(cur, env, &Ty::Bool);
        let m = self.b.block("merge");
        let nres = self.r.below(3);
        let tys: Vec<Ty> = (0..nres).map(|_| rand_ty(self.r)).collect();
        if self.r.chance(35) {
            // both edges go straight to the join, carrying arguments
            let a1: Vec<ValueId> = tys.iter().map(|t| self.pick(cur, env, t)).collect();
            let a2: Vec<ValueId> = tys.iter().map(|t| self.pick(cur, env, t)).collect();
            self.b.terminate(cur, Terminator::BrIf { cond: c, then: target(m, &a1), els: target(m, &a2) });
        } else {
            let (t1, e1) = (self.b.block("then"), self.b.block("else"));
            self.b.terminate(cur, Terminator::BrIf { cond: c, then: target(t1, &[]), els: target(e1, &[]) });
            let mut ends = Vec::new();
            for blk in [t1, e1] {
                let mut env2 = env.clone();
                let n = self.r.below(4);
                let end = self.seq(blk, &mut env2, depth + 1, n);
                let args: Vec<ValueId> = tys.iter().map(|t| self.pick(end, &mut env2, t)).collect();
                ends.push((end, args));
            }
            for (end, args) in ends {
                self.b.terminate(end, Terminator::Br(target(m, &args)));
            }
        }
        for t in &tys {
            let nm = self.name();
            let p = self.b.block_param(m, &nm, t.clone());
            env.push((p, t.clone()));
        }
        m
    }

    fn counted_loop(&mut self, cur: BlockId, env: &mut Env, depth: u32) -> BlockId {
        let (head, body, exit) = (self.b.block("head"), self.b.block("body"), self.b.block("exit"));
        let at = rand_ty(self.r);
        let i0 = self.konst(cur, env, &Ty::I32);
        let _ = i0;
        let zero = self.b.inst1(cur, Op::Const(Const::Int(0)), "z", Ty::I32);
        let n = self.name();
        let _ = n;
        let acc0 = self.pick(cur, env, &at);
        self.b.terminate(cur, Terminator::Br(target(head, &[zero, acc0])));
        let nm = self.name();
        let i = self.b.block_param(head, &nm, Ty::I32);
        let nm = self.name();
        let acc = self.b.block_param(head, &nm, at.clone());
        let trips = 1 + self.r.below(6) as i128;
        let lim = self.b.inst1(head, Op::Const(Const::Int(trips)), "lim", Ty::I32);
        let c = self.b.inst1(head, Op::Cmp { op: CmpOp::Lt, a: i, b: lim }, "c", Ty::Bool);
        self.b.terminate(head, Terminator::BrIf { cond: c, then: target(body, &[]), els: target(exit, &[acc]) });
        let mut env2 = env.clone();
        env2.push((i, Ty::I32));
        env2.push((acc, at.clone()));
        let nstm = 1 + self.r.below(4);
        let end = self.seq(body, &mut env2, depth + 1, nstm);
        let acc2 = self.pick(end, &mut env2, &at);
        let one = self.b.inst1(end, Op::Const(Const::Int(1)), "one", Ty::I32);
        let i2 = self.b.inst1(end, Op::Bin { op: BinOp::Add, ovf: Ovf::Wrap, a: i, b: one }, "i2", Ty::I32);
        self.b.terminate(end, Terminator::Br(target(head, &[i2, acc2])));
        let nm = self.name();
        let res = self.b.block_param(exit, &nm, at.clone());
        env.push((res, at));
        exit
    }

    fn switch(&mut self, cur: BlockId, env: &mut Env, depth: u32) -> BlockId {
        let v = self.pick(cur, env, &Ty::I32);
        let three = self.b.inst1(cur, Op::Const(Const::Int(3)), "m3", Ty::I32);
        let key = self.b.inst1(cur, Op::Bin { op: BinOp::And, ovf: Ovf::Trap, a: v, b: three }, "key", Ty::I32);
        let nb = 3 + self.r.below(2);
        if self.r.chance(40) {
            // every case goes straight to the join with its own arguments (chain with moves)
            let m = self.b.block("smerge");
            let t = rand_ty(self.r);
            let mut cases: Vec<(i64, Target)> = Vec::new();
            for k in 0..nb - 1 {
                let a = self.pick(cur, env, &t);
                cases.push((k as i64, target(m, &[a])));
            }
            let a = self.pick(cur, env, &t);
            self.b.terminate(cur, Terminator::Switch { value: key, cases, default: target(m, &[a]) });
            let nm = self.name();
            let p = self.b.block_param(m, &nm, t.clone());
            env.push((p, t));
            return m;
        }
        let blocks: Vec<BlockId> = (0..nb).map(|_| self.b.block("case")).collect();
        let m = self.b.block("smerge");
        let cases: Vec<(i64, Target)> = (0..nb - 1).map(|k| (k as i64, target(blocks[k], &[]))).collect();
        self.b.terminate(cur, Terminator::Switch { value: key, cases, default: target(blocks[nb - 1], &[]) });
        let t = rand_ty(self.r);
        let mut ends = Vec::new();
        for blk in blocks {
            let mut env2 = env.clone();
            let n = self.r.below(3);
            let end = self.seq(blk, &mut env2, depth + 1, n);
            let a = self.pick(end, &mut env2, &t);
            ends.push((end, a));
        }
        for (end, a) in ends {
            self.b.terminate(end, Terminator::Br(target(m, &[a])));
        }
        let nm = self.name();
        let p = self.b.block_param(m, &nm, t.clone());
        env.push((p, t));
        m
    }
}

/// Generate a module of `nfuncs` exported functions `f0..`; each may call lower-numbered ones.
pub fn gen_module(seed: u64, nfuncs: usize) -> Module {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut funcs = Vec::new();
    let mut sigs: Vec<(Vec<Ty>, Vec<Ty>)> = Vec::new();
    for fi in 0..nfuncs {
        let nparams = r.below(4);
        let params: Vec<Ty> = (0..nparams).map(|_| rand_ty(&mut r)).collect();
        let nrets = if r.chance(25) { 2 } else { 1 };
        let rets: Vec<Ty> = (0..nrets).map(|_| rand_ty(&mut r)).collect();
        let callees = sigs.clone();
        let mut g = G {
            r: &mut r,
            b: FuncBuilder::new(&format!("f{fi}")).export().rets(rets.clone()),
            n: 0,
            callees: &callees,
        };
        let mut env: Env = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let v = g.b.param(&format!("p{i}"), p.clone(), false);
            env.push((v, p.clone()));
        }
        let entry = g.b.block("entry");
        let n = 3 + g.r.below(10);
        let end = g.seq(entry, &mut env, 0, n);
        let vs: Vec<ValueId> = rets.iter().map(|t| g.pick(end, &mut env, t)).collect();
        g.b.terminate(end, Terminator::Ret(vs));
        let mut f = g.b.finish();
        renumber(&mut f);
        for (i, v) in f.values.iter_mut().enumerate() {
            v.name = format!("v{i}");
        }
        for (i, b) in f.blocks.iter_mut().enumerate() {
            b.label = format!("b{i}");
        }
        funcs.push(f);
        sigs.push((params, rets));
    }
    Module {
        name: format!("gen{seed}"),
        funcs,
        ..Module::default()
    }
}

pub fn sig_of(m: &Module, name: &str) -> (Vec<Ty>, Vec<Ty>) {
    let f = m.func(name).unwrap();
    (
        f.params.iter().map(|p| f.ty(p.value).clone()).collect(),
        f.rets.clone(),
    )
}
