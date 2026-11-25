//! Out-of-line helper functions for arithmetic whose inline form would bloat
//! every use site: 64-bit multiplication with overflow detection, saturating
//! 64-bit arithmetic and float-to-integer conversion without `trunc_sat`.

use crate::body::Body;
use crate::stubrt::G_TRAP;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::Ty;
use wasm_encoder::{BlockType, Function, Ieee32, Ieee64, ValType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum HKey {
    /// 64-bit add/sub/mul (op 0/1/2), signed or not, trapping.
    Mul64 { signed: bool },
    /// Saturating 64-bit op (0 add, 1 sub, 2 mul).
    Sat64 { op: u8, signed: bool },
    /// Float to integer: source f64?, destination type code, saturating?
    F2I { src64: bool, dst: u8, sat: bool },
}

pub(crate) fn ty_code(t: &Ty) -> u8 {
    match t {
        Ty::I8 => 0,
        Ty::I16 => 1,
        Ty::I32 => 2,
        Ty::I64 => 3,
        Ty::U8 => 4,
        Ty::U16 => 5,
        Ty::U32 => 6,
        _ => 7,
    }
}

/// (bits, signed) of an integer type code.
fn code_info(c: u8) -> (u32, bool) {
    match c {
        0 => (8, true),
        1 => (16, true),
        2 => (32, true),
        3 => (64, true),
        4 => (8, false),
        5 => (16, false),
        6 => (32, false),
        _ => (64, false),
    }
}

pub(crate) fn int_range(bits: u32, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}

fn trap(b: &mut Body, k: TrapKind) {
    b.i32c(k.code() as i32);
    b.s().global_set(G_TRAP).unreachable();
}

fn trap_if(b: &mut Body, k: TrapKind) {
    b.s().if_(BlockType::Empty);
    trap(b, k);
    b.s().end();
}

pub(crate) fn sig(k: HKey) -> (Vec<ValType>, Vec<ValType>) {
    use ValType::{F32, F64, I32, I64};
    match k {
        HKey::Mul64 { .. } | HKey::Sat64 { .. } => (vec![I64, I64], vec![I64]),
        HKey::F2I { src64, dst, .. } => {
            let (bits, _) = code_info(dst);
            (
                vec![if src64 { F64 } else { F32 }],
                vec![if bits == 64 { I64 } else { I32 }],
            )
        }
    }
}

pub(crate) fn build(k: HKey) -> Function {
    let (p, _) = sig(k);
    let mut body = Body::new(p.len());
    match k {
        HKey::Mul64 { signed } => mul64(&mut body, signed, false),
        HKey::Sat64 { op: 2, signed } => mul64(&mut body, signed, true),
        HKey::Sat64 { op, signed } => sat_addsub(&mut body, op, signed),
        HKey::F2I { src64, dst, sat } => f2i(&mut body, src64, dst, sat),
    }
    body.finish()
}

fn mul64(b: &mut Body, signed: bool, sat: bool) {
    let (a, bb) = (0u32, 1u32);
    let r = b.local(ValType::I64);
    // overflow action: trap, or return the saturated value
    let ovf = |b: &mut Body| {
        if !sat {
            trap(b, TrapKind::Overflow);
        } else if signed {
            b.i64c(i64::MIN).i64c(i64::MAX);
            b.get(a).get(bb);
            b.s().i64_xor();
            b.i64c(0);
            b.s().i64_lt_s().select().return_();
        } else {
            b.i64c(-1);
            b.s().return_();
        }
    };
    b.get(a);
    b.s().i64_eqz().if_(BlockType::Empty);
    b.i64c(0);
    b.s().return_().end();
    if signed {
        b.get(a).i64c(-1);
        b.s().i64_eq().if_(BlockType::Empty);
        b.get(bb).i64c(i64::MIN);
        b.s().i64_eq().if_(BlockType::Empty);
        ovf(b);
        b.s().end();
        b.i64c(0).get(bb);
        b.s().i64_sub().return_().end();
    }
    b.get(a).get(bb);
    b.s().i64_mul();
    b.set(r);
    b.get(r).get(a);
    if signed {
        b.s().i64_div_s();
    } else {
        b.s().i64_div_u();
    }
    b.get(bb);
    b.s().i64_ne().if_(BlockType::Empty);
    ovf(b);
    b.s().end();
    b.get(r);
}

fn sat_addsub(b: &mut Body, op: u8, signed: bool) {
    let (a, bb) = (0u32, 1u32);
    let r = b.local(ValType::I64);
    match (op, signed) {
        (0, true) | (1, true) => {
            if op == 0 {
                b.get(a).get(bb);
                b.s().i64_add();
            } else {
                b.get(a).get(bb);
                b.s().i64_sub();
            }
            b.set(r);
            // overflow test
            if op == 0 {
                b.get(a).get(r);
                b.s().i64_xor();
                b.get(bb).get(r);
                b.s().i64_xor().i64_and();
            } else {
                b.get(a).get(bb);
                b.s().i64_xor();
                b.get(a).get(r);
                b.s().i64_xor().i64_and();
            }
            b.i64c(0);
            b.s().i64_lt_s().if_(BlockType::Empty);
            b.i64c(i64::MIN).i64c(i64::MAX);
            b.get(a).i64c(0);
            b.s().i64_lt_s().select().return_().end();
            b.get(r);
        }
        (0, false) => {
            b.get(a).get(bb);
            b.s().i64_add();
            b.set(r);
            b.i64c(-1).get(r);
            b.get(r).get(a);
            b.s().i64_lt_u().select();
        }
        _ => {
            b.i64c(0);
            b.get(a).get(bb);
            b.s().i64_sub();
            b.get(a).get(bb);
            b.s().i64_ge_u().select();
        }
    }
}

fn f2i(b: &mut Body, src64: bool, dst: u8, sat: bool) {
    let x = 0u32;
    let (bits, signed) = code_info(dst);
    let (min, max) = int_range(bits, signed);
    let wide = bits == 64;
    let fconst = |b: &mut Body, v: f64| {
        if src64 {
            b.s().f64_const(Ieee64::from(v));
        } else {
            b.s().f32_const(Ieee32::from(v as f32));
        }
    };
    // exclusive upper bound MAX+1 (a power of two: exact in both float widths)
    let upper = (max + 1) as f64;
    let trunc = |b: &mut Body| {
        let mut s = b.s();
        match (src64, wide, signed && bits != 64 || (!signed && bits < 32)) {
            (true, false, true) => s.i32_trunc_f64_s(),
            (true, false, false) => s.i32_trunc_f64_u(),
            (false, false, true) => s.i32_trunc_f32_s(),
            (false, false, false) => s.i32_trunc_f32_u(),
            (true, true, _) if signed => s.i64_trunc_f64_s(),
            (true, true, _) => s.i64_trunc_f64_u(),
            (false, true, _) if signed => s.i64_trunc_f32_s(),
            (false, true, _) => s.i64_trunc_f32_u(),
        };
    };
    // is MIN-1 exactly representable in the source float?
    let mantissa = if src64 { 53 } else { 24 };
    let lower_exact = bits < mantissa || (!signed && bits <= mantissa);
    let lo = if signed { min as f64 } else { 0.0 };
    if !sat {
        // valid iff (x > MIN-1 | x >= MIN) && x < MAX+1 ; NaN fails both
        b.get(x);
        if signed && lower_exact {
            fconst(b, (min - 1) as f64);
            if src64 {
                b.s().f64_gt();
            } else {
                b.s().f32_gt();
            }
        } else if signed {
            fconst(b, lo);
            if src64 {
                b.s().f64_ge();
            } else {
                b.s().f32_ge();
            }
        } else {
            fconst(b, -1.0);
            if src64 {
                b.s().f64_gt();
            } else {
                b.s().f32_gt();
            }
        }
        b.get(x);
        fconst(b, upper);
        if src64 {
            b.s().f64_lt();
        } else {
            b.s().f32_lt();
        }
        b.s().i32_and().i32_eqz();
        trap_if(b, TrapKind::Overflow);
        b.get(x);
        trunc(b);
    } else {
        let imm = |b: &mut Body, v: i128| {
            if wide {
                b.i64c(v as i64);
            } else {
                b.i32c(v as i32);
            }
        };
        let rt = if wide { ValType::I64 } else { ValType::I32 };
        // NaN -> 0
        b.get(x).get(x);
        if src64 {
            b.s().f64_ne();
        } else {
            b.s().f32_ne();
        }
        b.s().if_(BlockType::Result(rt));
        imm(b, 0);
        b.s().else_();
        b.get(x);
        fconst(b, upper);
        if src64 {
            b.s().f64_ge();
        } else {
            b.s().f32_ge();
        }
        b.s().if_(BlockType::Result(rt));
        imm(b, max);
        b.s().else_();
        b.get(x);
        fconst(b, lo);
        if src64 {
            b.s().f64_le();
        } else {
            b.s().f32_le();
        }
        b.s().if_(BlockType::Result(rt));
        imm(b, min);
        b.s().else_();
        b.get(x);
        trunc(b);
        b.s().end().end().end();
    }
}
