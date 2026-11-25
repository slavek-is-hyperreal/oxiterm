//! A small reference interpreter for the scalar, control-flow and call part
//! of OxiIR, written straight from `docs/oxiscript/ir.md` section 3 (exact
//! integer math in `i128`, Rust floats) so it shares no code and no tricks
//! with the code generator. Used for differential testing.
#![allow(dead_code, clippy::pedantic)]

use oxiscript_ir::*;

#[derive(Clone, Copy, Debug)]
pub enum V {
    I(i128),
    F32(f32),
    F64(f64),
}

impl V {
    pub fn int(self) -> i128 {
        match self {
            V::I(v) => v,
            _ => panic!("not an int"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Out {
    Trap(TrapKind),
    /// Step budget exceeded: the case is skipped.
    Fuel,
}

pub fn bits_of(t: &Ty) -> u32 {
    match t {
        Ty::Bool => 1,
        _ => t.bits().expect("scalar"),
    }
}

pub fn range(t: &Ty) -> (i128, i128) {
    if *t == Ty::Bool {
        return (0, 1);
    }
    let b = bits_of(t);
    if t.is_signed_int() {
        (-(1i128 << (b - 1)), (1i128 << (b - 1)) - 1)
    } else {
        (0, (1i128 << b) - 1)
    }
}

pub fn wrap(t: &Ty, v: i128) -> i128 {
    if *t == Ty::Bool {
        return i128::from(v != 0);
    }
    let b = bits_of(t);
    let m = v & ((1i128 << b) - 1);
    if t.is_signed_int() && m >= (1i128 << (b - 1)) {
        m - (1i128 << b)
    } else {
        m
    }
}

fn clamp(t: &Ty, v: i128) -> i128 {
    let (lo, hi) = range(t);
    v.clamp(lo, hi)
}

pub fn same(a: V, b: V) -> bool {
    match (a, b) {
        (V::I(x), V::I(y)) => x == y,
        (V::F32(x), V::F32(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        (V::F64(x), V::F64(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        _ => false,
    }
}

pub struct Interp<'a> {
    pub m: &'a Module,
    pub steps: u64,
    pub max_steps: u64,
}

type Res<T> = Result<T, Out>;

macro_rules! fop {
    ($a:expr, $b:expr, $op:tt) => {
        match ($a, $b) {
            (V::F32(x), V::F32(y)) => V::F32(x $op y),
            (V::F64(x), V::F64(y)) => V::F64(x $op y),
            _ => panic!("float op on non-floats"),
        }
    };
}

fn fmin32(x: f32, y: f32) -> f32 {
    if x.is_nan() || y.is_nan() {
        f32::NAN
    } else if x == y {
        if x.is_sign_negative() { x } else { y }
    } else if x < y { x } else { y }
}
fn fmax32(x: f32, y: f32) -> f32 {
    if x.is_nan() || y.is_nan() {
        f32::NAN
    } else if x == y {
        if x.is_sign_negative() { y } else { x }
    } else if x > y { x } else { y }
}
fn fmin64(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == y {
        if x.is_sign_negative() { x } else { y }
    } else if x < y { x } else { y }
}
fn fmax64(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == y {
        if x.is_sign_negative() { y } else { x }
    } else if x > y { x } else { y }
}

impl<'a> Interp<'a> {
    pub fn new(m: &'a Module) -> Self {
        Interp {
            m,
            steps: 0,
            max_steps: 400_000,
        }
    }

    pub fn call(&mut self, name: &str, args: &[V]) -> Res<Vec<V>> {
        let f = self.m.func(name).expect("function");
        self.run(f, args, 0)
    }

    fn run(&mut self, f: &Func, args: &[V], depth: u32) -> Res<Vec<V>> {
        if depth > 64 {
            return Err(Out::Fuel);
        }
        let mut vals: Vec<Option<V>> = vec![None; f.values.len()];
        for (p, a) in f.params.iter().zip(args) {
            // the host passes raw bits; canonicalise like the wrapper does
            let t = f.ty(p.value);
            vals[p.value.0 as usize] = Some(match a {
                V::I(v) if t.is_int() || *t == Ty::Bool => V::I(wrap(t, *v)),
                v => *v,
            });
        }
        let mut b = 0usize;
        loop {
            let blk = &f.blocks[b];
            for i in &blk.insts {
                self.steps += 1;
                if self.steps > self.max_steps {
                    return Err(Out::Fuel);
                }
                self.inst(f, i, &mut vals, depth)?;
            }
            self.steps += 1;
            let get = |v: &ValueId, vals: &Vec<Option<V>>| vals[v.0 as usize].expect("defined");
            let mut go = |t: &Target, vals: &mut Vec<Option<V>>| -> usize {
                let vs: Vec<V> = t.args.iter().map(|a| get(a, vals)).collect();
                let dest = &f.blocks[t.block.0 as usize];
                for (p, v) in dest.params.iter().zip(vs) {
                    vals[p.0 as usize] = Some(v);
                }
                t.block.0 as usize
            };
            match &blk.term {
                Terminator::Br(t) => b = go(t, &mut vals),
                Terminator::BrIf { cond, then, els } => {
                    let c = get(cond, &vals).int() != 0;
                    b = go(if c { then } else { els }, &mut vals);
                }
                Terminator::Switch {
                    value,
                    cases,
                    default,
                } => {
                    let v = get(value, &vals).int();
                    let t = cases
                        .iter()
                        .find(|(k, _)| i128::from(*k) == v)
                        .map_or(default, |c| &c.1);
                    b = go(t, &mut vals);
                }
                Terminator::Ret(vs) => {
                    return Ok(vs.iter().map(|v| get(v, &vals)).collect());
                }
                Terminator::Trap(k) => return Err(Out::Trap(*k)),
                _ => panic!("terminator"),
            }
        }
    }

    fn inst(&mut self, f: &Func, i: &Inst, vals: &mut Vec<Option<V>>, depth: u32) -> Res<()> {
        let g = |v: &ValueId, vals: &Vec<Option<V>>| vals[v.0 as usize].expect("defined");
        let r0 = i.results.first().copied();
        let rt = r0.map(|r| f.ty(r).clone());
        let out: Vec<V> = match &i.op {
            Op::Const(c) => vec![match c {
                Const::Bool(b) => V::I(i128::from(*b)),
                Const::Int(v) => V::I(*v),
                Const::F32(b) => V::F32(f32::from_bits(*b)),
                Const::F64(b) => V::F64(f64::from_bits(*b)),
            }],
            Op::Bin { op, ovf, a, b } => {
                let t = rt.clone().unwrap();
                vec![self.bin(*op, *ovf, &t, g(a, vals), g(b, vals))?]
            }
            Op::Un { op, ovf, a } => {
                let t = rt.clone().unwrap();
                vec![self.un(*op, *ovf, &t, g(a, vals))?]
            }
            Op::Cmp { op, a, b } => {
                let (x, y) = (g(a, vals), g(b, vals));
                let r = match (x, y) {
                    (V::I(x), V::I(y)) => cmp(*op, x.partial_cmp(&y)),
                    (V::F32(x), V::F32(y)) => cmp(*op, x.partial_cmp(&y)),
                    (V::F64(x), V::F64(y)) => cmp(*op, x.partial_cmp(&y)),
                    _ => panic!(),
                };
                vec![V::I(i128::from(r))]
            }
            Op::Conv { ovf, a } => {
                let from = f.ty(*a).clone();
                vec![self.conv(*ovf, &from, &rt.clone().unwrap(), g(a, vals))?]
            }
            Op::Bitcast { a } => {
                let to = rt.clone().unwrap();
                vec![match g(a, vals) {
                    V::F32(x) => {
                        let b = if x.is_nan() { 0x7fc0_0000 } else { x.to_bits() };
                        V::I(wrap(&to, i128::from(b)))
                    }
                    V::F64(x) => {
                        let b = if x.is_nan() {
                            0x7ff8_0000_0000_0000
                        } else {
                            x.to_bits()
                        };
                        V::I(wrap(&to, i128::from(b)))
                    }
                    V::I(v) => {
                        if to == Ty::F32 {
                            V::F32(f32::from_bits(v as u32))
                        } else {
                            V::F64(f64::from_bits(v as u64))
                        }
                    }
                }]
            }
            Op::Select { cond, a, b } => {
                vec![if g(cond, vals).int() != 0 {
                    g(a, vals)
                } else {
                    g(b, vals)
                }]
            }
            Op::Call { callee, args } => {
                let callee_f = self.m.func(callee).expect("internal callee");
                let a: Vec<V> = args.iter().map(|v| g(v, vals)).collect();
                self.run(callee_f, &a, depth + 1)?
            }
            other => panic!("interpreter: unsupported op {other:?}"),
        };
        for (r, v) in i.results.iter().zip(out) {
            vals[r.0 as usize] = Some(v);
        }
        Ok(())
    }

    fn bin(&mut self, op: BinOp, ovf: Ovf, t: &Ty, a: V, b: V) -> Res<V> {
        if t.is_float() {
            return Ok(match op {
                BinOp::Add => fop!(a, b, +),
                BinOp::Sub => fop!(a, b, -),
                BinOp::Mul => fop!(a, b, *),
                BinOp::Div => fop!(a, b, /),
                BinOp::Min => match (a, b) {
                    (V::F32(x), V::F32(y)) => V::F32(fmin32(x, y)),
                    (V::F64(x), V::F64(y)) => V::F64(fmin64(x, y)),
                    _ => panic!(),
                },
                BinOp::Max => match (a, b) {
                    (V::F32(x), V::F32(y)) => V::F32(fmax32(x, y)),
                    (V::F64(x), V::F64(y)) => V::F64(fmax64(x, y)),
                    _ => panic!(),
                },
                _ => panic!("float bin {op:?}"),
            });
        }
        let (x, y) = (a.int(), b.int());
        let bits = bits_of(t);
        let (lo, hi) = range(t);
        let fit = |v: i128| -> Res<V> {
            Ok(V::I(match ovf {
                Ovf::Trap => {
                    if v < lo || v > hi {
                        return Err(Out::Trap(TrapKind::Overflow));
                    }
                    v
                }
                Ovf::Wrap => wrap(t, v),
                Ovf::Sat => clamp(t, v),
            }))
        };
        match op {
            BinOp::Add => fit(x + y),
            BinOp::Sub => fit(x - y),
            BinOp::Mul => fit(if ovf == Ovf::Wrap { x.wrapping_mul(y) } else { x.saturating_mul(y) }),
            BinOp::Div | BinOp::Rem => {
                if y == 0 {
                    return Err(Out::Trap(TrapKind::DivByZero));
                }
                if t.is_signed_int() && x == lo && y == -1 {
                    return match (ovf, op) {
                        (Ovf::Wrap, BinOp::Div) => Ok(V::I(lo)),
                        (Ovf::Wrap, _) => Ok(V::I(0)),
                        _ => Err(Out::Trap(TrapKind::Overflow)),
                    };
                }
                Ok(V::I(if op == BinOp::Div { x / y } else { x % y }))
            }
            BinOp::And | BinOp::Or | BinOp::Xor => {
                let (ux, uy) = (x & ((1i128 << bits) - 1), y & ((1i128 << bits) - 1));
                let r = match op {
                    BinOp::And => ux & uy,
                    BinOp::Or => ux | uy,
                    _ => ux ^ uy,
                };
                Ok(V::I(wrap(t, r)))
            }
            BinOp::Shl | BinOp::Shr => {
                let amt = if ovf == Ovf::Trap {
                    if y < 0 || y >= i128::from(bits) {
                        return Err(Out::Trap(TrapKind::Bounds));
                    }
                    y
                } else {
                    (y & (i128::from(bits) - 1)) as i128
                };
                Ok(V::I(if op == BinOp::Shl {
                    wrap(t, x << amt)
                } else {
                    x >> amt
                }))
            }
            _ => panic!("int bin {op:?}"),
        }
    }

    fn un(&mut self, op: UnOp, ovf: Ovf, t: &Ty, a: V) -> Res<V> {
        if t.is_float() {
            return Ok(match (op, a) {
                (UnOp::Neg, V::F32(x)) => V::F32(-x),
                (UnOp::Neg, V::F64(x)) => V::F64(-x),
                (UnOp::Abs, V::F32(x)) => V::F32(x.abs()),
                (UnOp::Abs, V::F64(x)) => V::F64(x.abs()),
                (UnOp::Sqrt, V::F32(x)) => V::F32(x.sqrt()),
                (UnOp::Sqrt, V::F64(x)) => V::F64(x.sqrt()),
                (UnOp::Floor, V::F32(x)) => V::F32(x.floor()),
                (UnOp::Floor, V::F64(x)) => V::F64(x.floor()),
                (UnOp::Ceil, V::F32(x)) => V::F32(x.ceil()),
                (UnOp::Ceil, V::F64(x)) => V::F64(x.ceil()),
                (UnOp::Trunc, V::F32(x)) => V::F32(x.trunc()),
                (UnOp::Trunc, V::F64(x)) => V::F64(x.trunc()),
                (UnOp::Nearest, V::F32(x)) => V::F32(x.round_ties_even()),
                (UnOp::Nearest, V::F64(x)) => V::F64(x.round_ties_even()),
                _ => panic!("float un"),
            });
        }
        let x = a.int();
        let bits = bits_of(t);
        let (lo, hi) = range(t);
        let u = x & ((1i128 << bits) - 1);
        Ok(V::I(match op {
            UnOp::Neg => {
                let r = -x;
                if ovf == Ovf::Trap {
                    if r < lo || r > hi {
                        return Err(Out::Trap(TrapKind::Overflow));
                    }
                    r
                } else {
                    wrap(t, r)
                }
            }
            UnOp::Not => {
                if *t == Ty::Bool {
                    i128::from(x == 0)
                } else {
                    wrap(t, !u)
                }
            }
            UnOp::Clz => i128::from((u as u128).leading_zeros()) - (128 - i128::from(bits)),
            UnOp::Ctz => {
                if u == 0 {
                    i128::from(bits)
                } else {
                    i128::from((u as u128).trailing_zeros())
                }
            }
            UnOp::Popcnt => i128::from((u as u128).count_ones()),
            _ => panic!("int un"),
        }))
    }

    fn conv(&mut self, ovf: Ovf, from: &Ty, to: &Ty, a: V) -> Res<V> {
        match (a, to.is_float()) {
            (V::I(v), true) => Ok(if *to == Ty::F32 {
                V::F32(v as f32)
            } else {
                V::F64(v as f64)
            }),
            (V::F32(x), true) => Ok(V::F64(f64::from(x))),
            (V::F64(x), true) => Ok(V::F32(x as f32)),
            (V::I(v), false) => {
                let (lo, hi) = range(to);
                Ok(V::I(match ovf {
                    Ovf::Wrap => wrap(to, v),
                    Ovf::Sat => clamp(to, v),
                    Ovf::Trap => {
                        if v < lo || v > hi {
                            return Err(Out::Trap(TrapKind::Overflow));
                        }
                        v
                    }
                }))
            }
            (fl, false) => {
                let x = match fl {
                    V::F32(x) => f64::from(x),
                    V::F64(x) => x,
                    _ => unreachable!(),
                };
                let (lo, hi) = range(to);
                let _ = from;
                if x.is_nan() {
                    return if ovf == Ovf::Sat {
                        Ok(V::I(0))
                    } else {
                        Err(Out::Trap(TrapKind::Overflow))
                    };
                }
                let tr = x.trunc();
                let v: i128 = if tr.abs() > 1e30 {
                    if tr > 0.0 { i128::MAX / 2 } else { i128::MIN / 2 }
                } else {
                    tr as i128
                };
                match ovf {
                    Ovf::Sat => Ok(V::I(clamp(to, v))),
                    _ => {
                        if v < lo || v > hi {
                            Err(Out::Trap(TrapKind::Overflow))
                        } else {
                            Ok(V::I(v))
                        }
                    }
                }
            }
        }
    }
}

fn cmp(op: CmpOp, o: Option<std::cmp::Ordering>) -> bool {
    use std::cmp::Ordering::*;
    match (op, o) {
        (CmpOp::Ne, None) => true,
        (_, None) => false,
        (CmpOp::Eq, Some(o)) => o == Equal,
        (CmpOp::Ne, Some(o)) => o != Equal,
        (CmpOp::Lt, Some(o)) => o == Less,
        (CmpOp::Le, Some(o)) => o != Greater,
        (CmpOp::Gt, Some(o)) => o == Greater,
        (CmpOp::Ge, Some(o)) => o != Less,
    }
}
