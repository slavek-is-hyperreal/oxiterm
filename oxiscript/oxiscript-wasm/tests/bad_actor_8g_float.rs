//! BAD ACTOR 8.G: float -> int conversions at every boundary, all 8 integer
//! destinations, f32 and f64 sources, trapping and saturating, checked
//! against an exact (i128 / f64 arithmetic) model.
#![allow(clippy::pedantic)]
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::parse_module;
use oxiscript_runtime::{Error, Value};
use std::fmt::Write as _;

const TYS: [(&str, u32, bool); 8] = [
    ("i8", 8, true),
    ("i16", 16, true),
    ("i32", 32, true),
    ("i64", 64, true),
    ("u8", 8, false),
    ("u16", 16, false),
    ("u32", 32, false),
    ("u64", 64, false),
];

fn range(bits: u32, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}

fn up64(x: f64) -> f64 {
    if x.is_nan() || x == f64::INFINITY { return x; }
    if x == 0.0 { return f64::from_bits(1); }
    let b = x.to_bits();
    f64::from_bits(if x > 0.0 { b + 1 } else { b - 1 })
}
fn down64(x: f64) -> f64 { -up64(-x) }
fn up32(x: f32) -> f32 {
    if x.is_nan() || x == f32::INFINITY { return x; }
    if x == 0.0 { return f32::from_bits(1); }
    let b = x.to_bits();
    f32::from_bits(if x > 0.0 { b + 1 } else { b - 1 })
}
fn down32(x: f32) -> f32 { -up32(-x) }

fn model(x: f64, bits: u32, signed: bool, sat: bool) -> Result<i128, ()> {
    let (lo, hi) = range(bits, signed);
    if x.is_nan() {
        return if sat { Ok(0) } else { Err(()) };
    }
    // exact: x is a finite or infinite f64; truncate toward zero
    if x.is_infinite() {
        return if sat { Ok(if x > 0.0 { hi } else { lo }) } else { Err(()) };
    }
    let t = x.trunc();
    // |t| may exceed i128? f64 max ~1.8e308: clamp first
    let v: i128 = if t >= 1.7e38 { i128::MAX } else if t <= -1.7e38 { i128::MIN } else { t as i128 };
    if v >= lo && v <= hi {
        Ok(v)
    } else if sat {
        Ok(v.clamp(lo, hi))
    } else {
        Err(())
    }
}

#[test]
fn float_to_int_boundaries_match_model() {
    let mut s = String::from("oxiir 1\nmodule \"f2i\"\n");
    for (t, _, _) in TYS {
        for (f, ft) in [("f32", "f32"), ("f64", "f64")] {
            for (m, o) in [("t", ""), ("s", ".sat")] {
                let _ = write!(s, "export func @{f}_{t}_{m}(%x: {ft}) -> {t} {{\ne:\n  %r: {t} = conv{o} %x\n  ret %r\n}}\n");
            }
        }
    }
    let m = parse_module(&s).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    let mut cases = 0usize;
    for (t, bits, signed) in TYS {
        let (lo, hi) = range(bits, signed);
        let mut vals64: Vec<f64> = vec![0.0, -0.0, 0.5, -0.5, 0.999999, -0.999999, 1.0, -1.0, 1.5, -1.5, 254.9, 255.0, 255.5, 256.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, f64::MIN_POSITIVE, 5e-324];
        for b in [lo as f64, hi as f64, (lo - 1) as f64, (hi + 1) as f64, (lo as f64) - 0.5, (hi as f64) + 0.5] {
            vals64.extend([b, up64(b), down64(b), up64(up64(b)), down64(down64(b))]);
        }
        let mut vals32: Vec<f32> = vals64.iter().map(|v| *v as f32).collect();
        for b in [lo as f32, hi as f32, (lo - 1) as f32, (hi + 1) as f32] {
            vals32.extend([b, up32(b), down32(b), up32(up32(b)), down32(down32(b))]);
        }
        for (mname, sat) in [("t", false), ("s", true)] {
            for &x in &vals64 {
                check(&mut rt, &h, &wasm, &format!("f64_{t}_{mname}"), Value::F64(x), x, bits, signed, sat);
                cases += 1;
            }
            for &x in &vals32 {
                check(&mut rt, &h, &wasm, &format!("f32_{t}_{mname}"), Value::F32(x), f64::from(x), bits, signed, sat);
                cases += 1;
            }
        }
    }
    eprintln!("float->int boundary cases: {cases}");
}

#[allow(clippy::too_many_arguments)]
fn check(rt: &mut oxiscript_runtime::Runtime, h: &oxiscript_runtime::Host, wasm: &[u8], f: &str, arg: Value, x: f64, bits: u32, signed: bool, sat: bool) {
    if rt.is_poisoned() {
        *rt = instantiate(h, wasm);
    }
    let want = model(x, bits, signed, sat);
    let got = rt.call_export(f, std::slice::from_ref(&arg));
    match (want, got) {
        (Ok(w), Ok(g)) => {
            let gv: i128 = match g[0] {
                Value::I32(v) => {
                    // canonical: sign/zero extended per type
                    if signed { i128::from(v) } else { i128::from(v as u32) }
                }
                Value::I64(v) => {
                    if signed { i128::from(v) } else { i128::from(v as u64) }
                }
                _ => panic!(),
            };
            assert_eq!(gv, w, "{f}({arg:?}) = {gv}, want {w}");
        }
        (Err(()), Err(Error::Trap(t))) => assert_eq!(t.guest_kind, Some(TrapKind::Overflow), "{f}({arg:?})"),
        (w, g) => panic!("{f}({arg:?}): model {w:?} vs wasm {g:?}"),
    }
}
