//! Hand-written IR programs, one per instruction family / trap, compiled and
//! run through `oxiscript-runtime`: expected results and trap kinds.
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::parse_module;
use oxiscript_runtime::{Error, Value};
use Value::{F32, F64, I32, I64};

enum E {
    Ok(Vec<Value>),
    Trap(TrapKind),
}
use E::{Ok as Okv, Trap as Tr};

struct Step(&'static str, Vec<Value>, E);

fn s(f: &'static str, a: Vec<Value>, e: E) -> Step {
    Step(f, a, e)
}

fn same_val(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (F32(x), F32(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        (F64(x), F64(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

/// Compile `src`, run the steps (re-instantiating after traps); `leak` checks `deinit` frees everything.
fn run(name: &str, src: &str, steps: Vec<Step>, leak: bool) {
    let m = parse_module(src).unwrap_or_else(|e| panic!("{name}: parse: {e}"));
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    for Step(f, args, want) in steps {
        if rt.is_poisoned() {
            rt = instantiate(&h, &wasm);
        }
        let got = rt.call_export(f, &args);
        match (want, got) {
            (Okv(w), Ok(g)) => {
                assert!(
                    w.len() == g.len() && w.iter().zip(&g).all(|(a, b)| same_val(a, b)),
                    "{name}: {f}{args:?} = {g:?}, want {w:?}"
                );
            }
            (Tr(k), Err(Error::Trap(t))) => {
                assert_eq!(t.guest_kind, Some(k), "{name}: {f}{args:?}: {:?}", t.cause);
            }
            (_, got) => panic!("{name}: {f}{args:?}: unexpected {got:?}"),
        }
    }
    if leak {
        if rt.is_poisoned() {
            return;
        }
        assert_eq!(rt.deinit().unwrap(), 0, "{name}: leaked objects");
    }
}

#[test]
fn p01_add_overflow_i32() {
    run(
        "p01",
        r#"oxiir 1
module "p01"
export func @add(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = add %a, %b
  ret %c
}"#,
        vec![
            s("add", vec![I32(i32::MAX), I32(1)], Tr(TrapKind::Overflow)),
            s("add", vec![I32(i32::MIN), I32(-1)], Tr(TrapKind::Overflow)),
            s("add", vec![I32(1), I32(2)], Okv(vec![I32(3)])),
            s("add", vec![I32(-5), I32(3)], Okv(vec![I32(-2)])),
        ],
        false,
    );
}

#[test]
fn p02_sub_u8() {
    run(
        "p02",
        r#"oxiir 1
module "p02"
export func @sub(%a: u8, %b: u8) -> u8 {
e:
  %c: u8 = sub %a, %b
  ret %c
}
export func @subw(%a: u8, %b: u8) -> u8 {
e:
  %c: u8 = sub.wrap %a, %b
  ret %c
}
export func @subs(%a: u8, %b: u8) -> u8 {
e:
  %c: u8 = sub.sat %a, %b
  ret %c
}"#,
        vec![
            s("sub", vec![I32(3), I32(5)], Tr(TrapKind::Overflow)),
            s("sub", vec![I32(5), I32(3)], Okv(vec![I32(2)])),
            s("subw", vec![I32(3), I32(5)], Okv(vec![I32(254)])),
            s("subs", vec![I32(3), I32(5)], Okv(vec![I32(0)])),
        ],
        false,
    );
}

#[test]
fn p03_mul_i64() {
    run(
        "p03",
        r#"oxiir 1
module "p03"
export func @mul(%a: i64, %b: i64) -> i64 {
e:
  %c: i64 = mul %a, %b
  ret %c
}
export func @mulsat(%a: u64, %b: u64) -> u64 {
e:
  %c: u64 = mul.sat %a, %b
  ret %c
}
export func @mulu(%a: u64, %b: u64) -> u64 {
e:
  %c: u64 = mul %a, %b
  ret %c
}"#,
        vec![
            s("mul", vec![I64(1 << 62), I64(2)], Tr(TrapKind::Overflow)),
            s("mul", vec![I64(-1), I64(i64::MIN)], Tr(TrapKind::Overflow)),
            s("mul", vec![I64(i64::MIN), I64(-1)], Tr(TrapKind::Overflow)),
            s("mul", vec![I64(3), I64(4)], Okv(vec![I64(12)])),
            s("mul", vec![I64(-3), I64(4)], Okv(vec![I64(-12)])),
            s("mulsat", vec![I64(-1), I64(2)], Okv(vec![I64(-1)])),
            s("mulsat", vec![I64(3), I64(4)], Okv(vec![I64(12)])),
            s("mulu", vec![I64(-1), I64(2)], Tr(TrapKind::Overflow)),
            s("mulu", vec![I64(1 << 32), I64(1 << 31)], Okv(vec![I64(1 << 63)])),
        ],
        false,
    );
}

#[test]
fn p04_div_rem() {
    run(
        "p04",
        r#"oxiir 1
module "p04"
export func @div(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = div %a, %b
  ret %c
}
export func @divw(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = div.wrap %a, %b
  ret %c
}
export func @rem(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = rem %a, %b
  ret %c
}
export func @remw(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = rem.wrap %a, %b
  ret %c
}
export func @div8(%a: i8, %b: i8) -> i8 {
e:
  %c: i8 = div %a, %b
  ret %c
}
export func @divu(%a: u32, %b: u32) -> u32 {
e:
  %c: u32 = div %a, %b
  ret %c
}"#,
        vec![
            s("div", vec![I32(1), I32(0)], Tr(TrapKind::DivByZero)),
            s("div", vec![I32(7), I32(2)], Okv(vec![I32(3)])),
            s("div", vec![I32(-7), I32(2)], Okv(vec![I32(-3)])),
            s("div", vec![I32(i32::MIN), I32(-1)], Tr(TrapKind::Overflow)),
            s("divw", vec![I32(i32::MIN), I32(-1)], Okv(vec![I32(i32::MIN)])),
            s("divw", vec![I32(5), I32(0)], Tr(TrapKind::DivByZero)),
            s("rem", vec![I32(i32::MIN), I32(-1)], Tr(TrapKind::Overflow)),
            s("rem", vec![I32(-7), I32(2)], Okv(vec![I32(-1)])),
            s("remw", vec![I32(i32::MIN), I32(-1)], Okv(vec![I32(0)])),
            s("div8", vec![I32(-128), I32(-1)], Tr(TrapKind::Overflow)),
            s("div8", vec![I32(-128), I32(1)], Okv(vec![I32(-128)])),
            s("divu", vec![I32(-2), I32(2)], Okv(vec![I32(0x7fff_ffff)])),
        ],
        false,
    );
}

#[test]
fn p05_shifts() {
    run(
        "p05",
        r#"oxiir 1
module "p05"
export func @shl(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = shl %a, %b
  ret %c
}
export func @shlw(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = shl.wrap %a, %b
  ret %c
}
export func @shr(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = shr %a, %b
  ret %c
}
export func @shru(%a: u32, %b: u32) -> u32 {
e:
  %c: u32 = shr %a, %b
  ret %c
}
export func @shl8w(%a: i8, %b: i8) -> i8 {
e:
  %c: i8 = shl.wrap %a, %b
  ret %c
}
export func @shl8(%a: i8, %b: i8) -> i8 {
e:
  %c: i8 = shl %a, %b
  ret %c
}
export func @shl64(%a: u64, %b: u64) -> u64 {
e:
  %c: u64 = shl %a, %b
  ret %c
}"#,
        vec![
            s("shl", vec![I32(1), I32(32)], Tr(TrapKind::Bounds)),
            s("shl", vec![I32(1), I32(-1)], Tr(TrapKind::Bounds)),
            s("shl", vec![I32(1), I32(31)], Okv(vec![I32(i32::MIN)])),
            s("shlw", vec![I32(1), I32(33)], Okv(vec![I32(2)])),
            s("shr", vec![I32(-8), I32(1)], Okv(vec![I32(-4)])),
            s("shru", vec![I32(-8), I32(1)], Okv(vec![I32(0x7fff_fffc)])),
            s("shl8w", vec![I32(1), I32(9)], Okv(vec![I32(2)])),
            s("shl8", vec![I32(64), I32(1)], Okv(vec![I32(-128)])),
            s("shl8", vec![I32(1), I32(8)], Tr(TrapKind::Bounds)),
            s("shl64", vec![I64(1), I64(64)], Tr(TrapKind::Bounds)),
            s("shl64", vec![I64(1), I64(63)], Okv(vec![I64(i64::MIN)])),
        ],
        false,
    );
}

#[test]
fn p06_neg_not_bits() {
    run(
        "p06",
        r#"oxiir 1
module "p06"
export func @neg(%a: i32) -> i32 {
e:
  %c: i32 = neg %a
  ret %c
}
export func @negw(%a: i32) -> i32 {
e:
  %c: i32 = neg.wrap %a
  ret %c
}
export func @negu(%a: u8) -> u8 {
e:
  %c: u8 = neg %a
  ret %c
}
export func @notu8(%a: u8) -> u8 {
e:
  %c: u8 = not %a
  ret %c
}
export func @clz8(%a: u8) -> u8 {
e:
  %c: u8 = clz %a
  ret %c
}
export func @ctz8(%a: u8) -> u8 {
e:
  %c: u8 = ctz %a
  ret %c
}
export func @pop8(%a: i8) -> i8 {
e:
  %c: i8 = popcnt %a
  ret %c
}
export func @clz16(%a: i16) -> i16 {
e:
  %c: i16 = clz %a
  ret %c
}
export func @ctz64(%a: i64) -> i64 {
e:
  %c: i64 = ctz %a
  ret %c
}"#,
        vec![
            s("neg", vec![I32(i32::MIN)], Tr(TrapKind::Overflow)),
            s("neg", vec![I32(5)], Okv(vec![I32(-5)])),
            s("negw", vec![I32(i32::MIN)], Okv(vec![I32(i32::MIN)])),
            s("negu", vec![I32(0)], Okv(vec![I32(0)])),
            s("negu", vec![I32(1)], Tr(TrapKind::Overflow)),
            s("notu8", vec![I32(0x0f)], Okv(vec![I32(0xf0)])),
            s("clz8", vec![I32(0)], Okv(vec![I32(8)])),
            s("clz8", vec![I32(1)], Okv(vec![I32(7)])),
            s("ctz8", vec![I32(0)], Okv(vec![I32(8)])),
            s("ctz8", vec![I32(0x80)], Okv(vec![I32(7)])),
            s("pop8", vec![I32(-1)], Okv(vec![I32(8)])),
            s("clz16", vec![I32(-1)], Okv(vec![I32(0)])),
            s("ctz64", vec![I64(0)], Okv(vec![I64(64)])),
        ],
        false,
    );
}

#[test]
fn p07_float_to_int() {
    run(
        "p07",
        r#"oxiir 1
module "p07"
export func @f2i(%a: f64) -> i32 {
e:
  %c: i32 = conv %a
  ret %c
}
export func @f2u8s(%a: f64) -> u8 {
e:
  %c: u8 = conv.sat %a
  ret %c
}
export func @f32i64s(%a: f32) -> i64 {
e:
  %c: i64 = conv.sat %a
  ret %c
}
export func @f2u32(%a: f64) -> u32 {
e:
  %c: u32 = conv %a
  ret %c
}
export func @f2i64(%a: f64) -> i64 {
e:
  %c: i64 = conv %a
  ret %c
}
export func @f32i32(%a: f32) -> i32 {
e:
  %c: i32 = conv %a
  ret %c
}"#,
        vec![
            s("f2i", vec![F64(f64::NAN)], Tr(TrapKind::Overflow)),
            s("f2i", vec![F64(2147483647.9)], Okv(vec![I32(i32::MAX)])),
            s("f2i", vec![F64(-2147483648.9)], Okv(vec![I32(i32::MIN)])),
            s("f2i", vec![F64(2147483648.0)], Tr(TrapKind::Overflow)),
            s("f2i", vec![F64(-2147483649.0)], Tr(TrapKind::Overflow)),
            s("f2i", vec![F64(f64::INFINITY)], Tr(TrapKind::Overflow)),
            s("f2i", vec![F64(-0.9)], Okv(vec![I32(0)])),
            s("f2u8s", vec![F64(f64::NAN)], Okv(vec![I32(0)])),
            s("f2u8s", vec![F64(300.5)], Okv(vec![I32(255)])),
            s("f2u8s", vec![F64(-5.0)], Okv(vec![I32(0)])),
            s("f2u8s", vec![F64(7.9)], Okv(vec![I32(7)])),
            s("f32i64s", vec![F32(1e30)], Okv(vec![I64(i64::MAX)])),
            s("f32i64s", vec![F32(-1e30)], Okv(vec![I64(i64::MIN)])),
            s("f2u32", vec![F64(4294967295.5)], Okv(vec![I32(-1)])),
            s("f2u32", vec![F64(4294967296.0)], Tr(TrapKind::Overflow)),
            s("f2u32", vec![F64(-1.0)], Tr(TrapKind::Overflow)),
            s("f2i64", vec![F64(9.3e18)], Tr(TrapKind::Overflow)),
            s("f2i64", vec![F64(-9223372036854775808.0)], Okv(vec![I64(i64::MIN)])),
            s("f32i32", vec![F32(2147483648.0)], Tr(TrapKind::Overflow)),
            s("f32i32", vec![F32(-2147483648.0)], Okv(vec![I32(i32::MIN)])),
        ],
        false,
    );
}

#[test]
fn p08_int_conversions() {
    run(
        "p08",
        r#"oxiir 1
module "p08"
export func @i64u8(%a: i64) -> u8 {
e:
  %c: u8 = conv %a
  ret %c
}
export func @wrap8(%a: i64) -> i8 {
e:
  %c: i8 = conv.wrap %a
  ret %c
}
export func @sat16(%a: i32) -> u16 {
e:
  %c: u16 = conv.sat %a
  ret %c
}
export func @u64i64(%a: u64) -> i64 {
e:
  %c: i64 = conv %a
  ret %c
}
export func @i32u64(%a: i32) -> u64 {
e:
  %c: u64 = conv %a
  ret %c
}
export func @u32f32(%a: u32) -> f32 {
e:
  %c: f32 = conv %a
  ret %c
}
export func @u64f64(%a: u64) -> f64 {
e:
  %c: f64 = conv %a
  ret %c
}
export func @i8i64(%a: i8) -> i64 {
e:
  %c: i64 = conv %a
  ret %c
}
export func @u8i8(%a: u8) -> i8 {
e:
  %c: i8 = conv %a
  ret %c
}
export func @sat64(%a: u64) -> i32 {
e:
  %c: i32 = conv.sat %a
  ret %c
}"#,
        vec![
            s("i64u8", vec![I64(255)], Okv(vec![I32(255)])),
            s("i64u8", vec![I64(256)], Tr(TrapKind::Overflow)),
            s("i64u8", vec![I64(-1)], Tr(TrapKind::Overflow)),
            s("wrap8", vec![I64(200)], Okv(vec![I32(-56)])),
            s("sat16", vec![I32(-1)], Okv(vec![I32(0)])),
            s("sat16", vec![I32(70000)], Okv(vec![I32(65535)])),
            s("u64i64", vec![I64(-1)], Tr(TrapKind::Overflow)),
            s("u64i64", vec![I64(5)], Okv(vec![I64(5)])),
            s("i32u64", vec![I32(-1)], Tr(TrapKind::Overflow)),
            s("i32u64", vec![I32(7)], Okv(vec![I64(7)])),
            s("u32f32", vec![I32(-1)], Okv(vec![F32(4294967296.0)])),
            s("u64f64", vec![I64(-1)], Okv(vec![F64(18446744073709551615.0)])),
            s("i8i64", vec![I32(-3)], Okv(vec![I64(-3)])),
            s("u8i8", vec![I32(200)], Tr(TrapKind::Overflow)),
            s("sat64", vec![I64(-1)], Okv(vec![I32(i32::MAX)])),
        ],
        false,
    );
}

#[test]
fn p09_bitcast_nan_canonical() {
    run(
        "p09",
        r#"oxiir 1
module "p09"
export func @nanbits() -> u32 {
e:
  %n: f32 = const nan:0x7fc00001
  %b: u32 = bitcast %n
  ret %b
}
export func @nan64() -> u64 {
e:
  %n: f64 = const nan:0xfff8000000000123
  %b: u64 = bitcast %n
  ret %b
}
export func @negzero() -> i64 {
e:
  %n: f64 = const -0.0
  %b: i64 = bitcast %n
  ret %b
}
export func @tof(%a: i32) -> f32 {
e:
  %b: f32 = bitcast %a
  ret %b
}
export func @roundtrip(%a: f32) -> f32 {
e:
  %b: u32 = bitcast %a
  %c: f32 = bitcast %b
  ret %c
}"#,
        vec![
            s("nanbits", vec![], Okv(vec![I32(0x7fc0_0000)])),
            s("nan64", vec![], Okv(vec![I64(0x7ff8_0000_0000_0000)])),
            s("negzero", vec![], Okv(vec![I64(i64::MIN)])),
            s("tof", vec![I32(0x3fc0_0000)], Okv(vec![F32(1.5)])),
            s("roundtrip", vec![F32(-2.25)], Okv(vec![F32(-2.25)])),
        ],
        false,
    );
}

#[test]
fn p10_float_ops() {
    run(
        "p10",
        r#"oxiir 1
module "p10"
export func @min(%a: f64, %b: f64) -> f64 {
e:
  %c: f64 = min %a, %b
  ret %c
}
export func @max(%a: f64, %b: f64) -> f64 {
e:
  %c: f64 = max %a, %b
  ret %c
}
export func @near(%a: f64) -> f64 {
e:
  %c: f64 = nearest %a
  ret %c
}
export func @floor(%a: f32) -> f32 {
e:
  %c: f32 = floor %a
  ret %c
}
export func @ceil(%a: f64) -> f64 {
e:
  %c: f64 = ceil %a
  ret %c
}
export func @trunc(%a: f64) -> f64 {
e:
  %c: f64 = trunc %a
  ret %c
}
export func @sqrt(%a: f64) -> f64 {
e:
  %c: f64 = sqrt %a
  ret %c
}
export func @abs(%a: f64) -> f64 {
e:
  %c: f64 = abs %a
  ret %c
}
export func @neg(%a: f64) -> f64 {
e:
  %c: f64 = neg %a
  ret %c
}
export func @div(%a: f64, %b: f64) -> f64 {
e:
  %c: f64 = div %a, %b
  ret %c
}
export func @lt(%a: f64, %b: f64) -> bool {
e:
  %c: bool = lt %a, %b
  ret %c
}
export func @ne(%a: f64, %b: f64) -> bool {
e:
  %c: bool = ne %a, %b
  ret %c
}
export func @sel(%c: bool, %a: f64, %b: f64) -> f64 {
e:
  %r: f64 = select %c, %a, %b
  ret %r
}"#,
        vec![
            s("min", vec![F64(-0.0), F64(0.0)], Okv(vec![F64(-0.0)])),
            s("min", vec![F64(0.0), F64(-0.0)], Okv(vec![F64(-0.0)])),
            s("max", vec![F64(-0.0), F64(0.0)], Okv(vec![F64(0.0)])),
            s("max", vec![F64(f64::NAN), F64(1.0)], Okv(vec![F64(f64::NAN)])),
            s("near", vec![F64(2.5)], Okv(vec![F64(2.0)])),
            s("near", vec![F64(3.5)], Okv(vec![F64(4.0)])),
            s("floor", vec![F32(-1.5)], Okv(vec![F32(-2.0)])),
            s("ceil", vec![F64(1.2)], Okv(vec![F64(2.0)])),
            s("trunc", vec![F64(-1.7)], Okv(vec![F64(-1.0)])),
            s("sqrt", vec![F64(2.0)], Okv(vec![F64(2.0f64.sqrt())])),
            s("abs", vec![F64(-3.0)], Okv(vec![F64(3.0)])),
            s("neg", vec![F64(0.0)], Okv(vec![F64(-0.0)])),
            s("div", vec![F64(1.0), F64(0.0)], Okv(vec![F64(f64::INFINITY)])),
            s("lt", vec![F64(f64::NAN), F64(1.0)], Okv(vec![I32(0)])),
            s("ne", vec![F64(f64::NAN), F64(f64::NAN)], Okv(vec![I32(1)])),
            s("sel", vec![I32(1), F64(1.0), F64(2.0)], Okv(vec![F64(1.0)])),
            s("sel", vec![I32(0), F64(1.0), F64(2.0)], Okv(vec![F64(2.0)])),
        ],
        false,
    );
}

#[test]
fn p11_loops_and_switch() {
    run(
        "p11",
        r#"oxiir 1
module "p11"
export func @sum(%n: i32) -> i64 {
entry:
  %i0: i32 = const 0
  %a0: i64 = const 0
  br loop(%i0, %a0)
loop(%i: i32, %acc: i64):
  %done: bool = ge %i, %n
  br_if %done, exit(%acc), body()
body:
  %w: i64 = conv %i
  %acc2: i64 = add %acc, %w
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br loop(%i2, %acc2)
exit(%res: i64):
  ret %res
}
export func @dense(%v: i32) -> i32 {
entry:
  switch %v, [0: a(), 1: b(), 2: c(), 3: d(), 5: f()], default z()
a:
  %x1: i32 = const 100
  ret %x1
b:
  %x2: i32 = const 101
  ret %x2
c:
  %x3: i32 = const 102
  ret %x3
d:
  %x4: i32 = const 103
  ret %x4
f:
  %x5: i32 = const 105
  ret %x5
z:
  %x6: i32 = const -1
  ret %x6
}
export func @sparse(%v: i64) -> i32 {
entry:
  switch %v, [1099511627776: a(), -3: b(), 7: c()], default z()
a:
  %x7: i32 = const 1
  ret %x7
b:
  %x8: i32 = const 2
  ret %x8
c:
  %x9: i32 = const 3
  ret %x9
z:
  %x10: i32 = const 0
  ret %x10
}
export func @shared(%v: i32) -> i32 {
entry:
  switch %v, [0: m(), 1: m(), 2: n(), 3: m()], default n()
m:
  %x: i32 = const 10
  br join(%x)
n:
  %y: i32 = const 20
  br join(%y)
join(%r: i32):
  %one: i32 = const 1
  %s: i32 = add %r, %one
  ret %s
}"#,
        vec![
            s("sum", vec![I32(0)], Okv(vec![I64(0)])),
            s("sum", vec![I32(100)], Okv(vec![I64(4950)])),
            s("dense", vec![I32(0)], Okv(vec![I32(100)])),
            s("dense", vec![I32(3)], Okv(vec![I32(103)])),
            s("dense", vec![I32(4)], Okv(vec![I32(-1)])),
            s("dense", vec![I32(5)], Okv(vec![I32(105)])),
            s("dense", vec![I32(-7)], Okv(vec![I32(-1)])),
            s("dense", vec![I32(1000)], Okv(vec![I32(-1)])),
            s("sparse", vec![I64(1 << 40)], Okv(vec![I32(1)])),
            s("sparse", vec![I64(-3)], Okv(vec![I32(2)])),
            s("sparse", vec![I64(7)], Okv(vec![I32(3)])),
            s("sparse", vec![I64(8)], Okv(vec![I32(0)])),
            s("shared", vec![I32(0)], Okv(vec![I32(11)])),
            s("shared", vec![I32(2)], Okv(vec![I32(21)])),
            s("shared", vec![I32(3)], Okv(vec![I32(11)])),
            s("shared", vec![I32(9)], Okv(vec![I32(21)])),
        ],
        false,
    );
}

#[test]
fn p12_parallel_moves_and_calls() {
    run(
        "p12",
        r#"oxiir 1
module "p12"
export func @fib(%n: i32) -> i64 {
entry:
  %i0: i32 = const 0
  %a0: i64 = const 0
  %b0: i64 = const 1
  br loop(%i0, %a0, %b0)
loop(%i: i32, %a: i64, %b: i64):
  %done: bool = ge %i, %n
  br_if %done, exit(%a), body()
body:
  %s: i64 = add %a, %b
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br loop(%i2, %b, %s)
exit(%r: i64):
  ret %r
}
export func @swapn(%n: i32) -> i32 {
entry:
  %i0: i32 = const 0
  %x0: i32 = const 1
  %y0: i32 = const 2
  br loop(%i0, %x0, %y0)
loop(%i: i32, %x: i32, %y: i32):
  %done: bool = ge %i, %n
  br_if %done, exit(%x, %y), body()
body:
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br loop(%i2, %y, %x)
exit(%p: i32, %q: i32):
  %t: i32 = const 10
  %pp: i32 = mul %p, %t
  %r: i32 = add %pp, %q
  ret %r
}
func @pair(%a: i32, %b: i32) -> (i32, i32) {
e:
  ret %b, %a
}
export func @usepair(%a: i32, %b: i32) -> i32 {
e:
  %x: i32, %y: i32 = call @pair(%a, %b)
  %k: i32 = const 100
  %xx: i32 = mul %x, %k
  %r: i32 = add %xx, %y
  ret %r
}
export func @fact(%n: i32) -> i32 {
entry:
  %one: i32 = const 1
  %le: bool = le %n, %one
  br_if %le, base(), rec()
base:
  ret %one
rec:
  %m: i32 = sub %n, %one
  %f: i32 = call @fact(%m)
  %r: i32 = mul %n, %f
  ret %r
}"#,
        vec![
            s("fib", vec![I32(10)], Okv(vec![I64(55)])),
            s("fib", vec![I32(90)], Okv(vec![I64(2880067194370816120)])),
            s("swapn", vec![I32(0)], Okv(vec![I32(12)])),
            s("swapn", vec![I32(1)], Okv(vec![I32(21)])),
            s("swapn", vec![I32(7)], Okv(vec![I32(21)])),
            s("usepair", vec![I32(3), I32(4)], Okv(vec![I32(403)])),
            s("fact", vec![I32(10)], Okv(vec![I32(3628800)])),
            s("fact", vec![I32(13)], Tr(TrapKind::Overflow)),
        ],
        false,
    );
}

#[test]
fn p13_trap_terminators() {
    run(
        "p13",
        r#"oxiir 1
module "p13"
export func @t(%k: i32) -> i32 {
entry:
  switch %k, [0: a(), 1: b(), 2: c(), 3: d(), 4: e(), 5: f(), 6: g(), 7: h(), 8: i(), 9: j()], default ok()
a:
  trap unreachable
b:
  trap overflow
c:
  trap div_by_zero
d:
  trap bounds
e:
  trap bad_cast
f:
  trap null_deref
g:
  trap out_of_memory
h:
  trap assert
i:
  trap bad_utf8
j:
  trap panic
ok:
  %x: i32 = const 42
  ret %x
}"#,
        vec![
            s("t", vec![I32(0)], Tr(TrapKind::Unreachable)),
            s("t", vec![I32(1)], Tr(TrapKind::Overflow)),
            s("t", vec![I32(2)], Tr(TrapKind::DivByZero)),
            s("t", vec![I32(3)], Tr(TrapKind::Bounds)),
            s("t", vec![I32(4)], Tr(TrapKind::BadCast)),
            s("t", vec![I32(5)], Tr(TrapKind::NullDeref)),
            s("t", vec![I32(6)], Tr(TrapKind::OutOfMemory)),
            s("t", vec![I32(7)], Tr(TrapKind::Assert)),
            s("t", vec![I32(8)], Tr(TrapKind::BadUtf8)),
            s("t", vec![I32(9)], Tr(TrapKind::Panic)),
            s("t", vec![I32(10)], Okv(vec![I32(42)])),
        ],
        false,
    );
}

#[test]
fn p14_narrow_arith() {
    run(
        "p14",
        r#"oxiir 1
module "p14"
export func @add8(%a: i8, %b: i8) -> i8 {
e:
  %c: i8 = add %a, %b
  ret %c
}
export func @add8w(%a: i8, %b: i8) -> i8 {
e:
  %c: i8 = add.wrap %a, %b
  ret %c
}
export func @add8s(%a: i8, %b: i8) -> i8 {
e:
  %c: i8 = add.sat %a, %b
  ret %c
}
export func @mul16(%a: i16, %b: i16) -> i16 {
e:
  %c: i16 = mul %a, %b
  ret %c
}
export func @mulu16(%a: u16, %b: u16) -> u16 {
e:
  %c: u16 = mul %a, %b
  ret %c
}
export func @mulu32(%a: u32, %b: u32) -> u32 {
e:
  %c: u32 = mul %a, %b
  ret %c
}
export func @mulu32s(%a: u32, %b: u32) -> u32 {
e:
  %c: u32 = mul.sat %a, %b
  ret %c
}
export func @muli32(%a: i32, %b: i32) -> i32 {
e:
  %c: i32 = mul %a, %b
  ret %c
}
export func @addu64(%a: u64, %b: u64) -> u64 {
e:
  %c: u64 = add %a, %b
  ret %c
}
export func @adds64(%a: i64, %b: i64) -> i64 {
e:
  %c: i64 = add.sat %a, %b
  ret %c
}
export func @subs64(%a: i64, %b: i64) -> i64 {
e:
  %c: i64 = sub.sat %a, %b
  ret %c
}
export func @subu64(%a: u64, %b: u64) -> u64 {
e:
  %c: u64 = sub %a, %b
  ret %c
}"#,
        vec![
            s("add8", vec![I32(127), I32(1)], Tr(TrapKind::Overflow)),
            s("add8", vec![I32(-128), I32(-1)], Tr(TrapKind::Overflow)),
            s("add8", vec![I32(100), I32(27)], Okv(vec![I32(127)])),
            s("add8w", vec![I32(127), I32(1)], Okv(vec![I32(-128)])),
            s("add8s", vec![I32(100), I32(100)], Okv(vec![I32(127)])),
            s("add8s", vec![I32(-100), I32(-100)], Okv(vec![I32(-128)])),
            s("mul16", vec![I32(200), I32(200)], Tr(TrapKind::Overflow)),
            s("mul16", vec![I32(-181), I32(181)], Okv(vec![I32(-32761)])),
            s("mulu16", vec![I32(256), I32(256)], Tr(TrapKind::Overflow)),
            s("mulu16", vec![I32(255), I32(257)], Okv(vec![I32(65535)])),
            s("mulu32", vec![I32(65536), I32(65536)], Tr(TrapKind::Overflow)),
            s("mulu32", vec![I32(65535), I32(65537)], Okv(vec![I32(-1)])),
            s("mulu32s", vec![I32(65536), I32(65536)], Okv(vec![I32(-1)])),
            s("muli32", vec![I32(65536), I32(32768)], Tr(TrapKind::Overflow)),
            s("muli32", vec![I32(-65536), I32(32768)], Okv(vec![I32(i32::MIN)])),
            s("addu64", vec![I64(-1), I64(1)], Tr(TrapKind::Overflow)),
            s("adds64", vec![I64(i64::MAX), I64(1)], Okv(vec![I64(i64::MAX)])),
            s("adds64", vec![I64(i64::MIN), I64(-1)], Okv(vec![I64(i64::MIN)])),
            s("subs64", vec![I64(i64::MIN), I64(1)], Okv(vec![I64(i64::MIN)])),
            s("subs64", vec![I64(i64::MAX), I64(-1)], Okv(vec![I64(i64::MAX)])),
            s("subu64", vec![I64(1), I64(2)], Tr(TrapKind::Overflow)),
        ],
        false,
    );
}

#[test]
fn p15_bool_ops() {
    run(
        "p15",
        r#"oxiir 1
module "p15"
export func @logic(%a: bool, %b: bool) -> i32 {
e:
  %x: bool = and %a, %b
  %y: bool = or %a, %b
  %z: bool = xor %a, %b
  %n: bool = not %a
  %xi: i32 = select %x, %c8, %c0
  %c8: i32 = const 8
  %c0: i32 = const 0
  ret %xi
}"#
        .replace(
            "%xi: i32 = select %x, %c8, %c0\n  %c8: i32 = const 8\n  %c0: i32 = const 0\n  ret %xi",
            "%c8: i32 = const 8\n  %c4: i32 = const 4\n  %c2: i32 = const 2\n  %c1: i32 = const 1\n  %c0: i32 = const 0\n  %xi: i32 = select %x, %c8, %c0\n  %yi: i32 = select %y, %c4, %c0\n  %zi: i32 = select %z, %c2, %c0\n  %ni: i32 = select %n, %c1, %c0\n  %s1: i32 = add %xi, %yi\n  %s2: i32 = add %s1, %zi\n  %s3: i32 = add %s2, %ni\n  ret %s3",
        )
        .as_str(),
        vec![
            s("logic", vec![I32(1), I32(1)], Okv(vec![I32(8 + 4)])),
            s("logic", vec![I32(1), I32(0)], Okv(vec![I32(4 + 2)])),
            s("logic", vec![I32(0), I32(1)], Okv(vec![I32(4 + 2 + 1)])),
            s("logic", vec![I32(0), I32(0)], Okv(vec![I32(1)])),
            // non-canonical host booleans are canonicalised by the export wrapper
            s("logic", vec![I32(2), I32(7)], Okv(vec![I32(8 + 4)])),
        ],
        false,
    );
}
