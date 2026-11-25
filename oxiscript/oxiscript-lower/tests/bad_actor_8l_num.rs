#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.L numerics end to end (differential against Rust semantics).
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::Value;
use Value::{F64, I32, I64};

const SRC: &str = r#"import { Log } from "host:log";
export function addI(a: i32, b: i32): i32 { return a + b; }
export function subI(a: i32, b: i32): i32 { return a - b; }
export function mulI(a: i32, b: i32): i32 { return a * b; }
export function divI(a: i32, b: i32): i32 { return a / b; }
export function remI(a: i32, b: i32): i32 { return a % b; }
export function negI(a: i32): i32 { return -a; }
export function shlI(a: i32, b: i32): i32 { return a << b; }
export function shrI(a: i32, b: i32): i32 { return a >> b; }
export function addW(a: i32, b: i32): i32 { return a +% b; }
export function mulW(a: i32, b: i32): i32 { return a *% b; }
export function addS(a: i32, b: i32): i32 { return a +| b; }
export function subS(a: i32, b: i32): i32 { return a -| b; }
export function divL(a: i64, b: i64): i64 { return a / b; }
export function remL(a: i64, b: i64): i64 { return a % b; }
export function mulL(a: i64, b: i64): i64 { return a * b; }
export function addL(a: i64, b: i64): i64 { return a + b; }
export function negL(a: i64): i64 { return -a; }
export function u8w(a: u8, b: u8): u8 { return a +% b; }
export function u8sub(a: u8, b: u8): i32 { return (a -% b) as i32; }
export function u8mul(a: u8, b: u8): u8 { return a * b; }
export function u8mulw(a: u8, b: u8): i32 { return (a *% b) as i32; }
export function u8shl(a: u8, b: u8): i32 { return (a << b) as i32; }
export function u8not(a: u8): i32 { return (~a) as i32; }
export function u8lt(a: u8, b: u8): bool { return (a +% b) < a; }
export function u8sats(a: u8, b: u8): i32 { return (a +| b) as i32; }
export function u8inc(a: u8): u8 { let x = a; x += 1; return x; }
export function i8w(a: i8, b: i8): i32 { return (a *% b) as i32; }
export function i8neg(a: i8): i8 { return -a; }
export function i8divm(a: i8, b: i8): i8 { return a / b; }
export function i8not(a: i8): i32 { return (~a) as i32; }
export function i8sat(a: i8, b: i8): i32 { return (a -| b) as i32; }
export function i16w(a: i16, b: i16): i32 { return (a +% b) as i32; }
export function u16shl(a: u16, b: u16): i32 { return (a << b) as i32; }
export function u32div(a: u32, b: u32): u32 { return a / b; }
export function u32rem(a: u32, b: u32): u32 { return a % b; }
export function u32shr(a: u32, b: u32): u32 { return a >> b; }
export function u32lt(a: u32, b: u32): bool { return a < b; }
export function u32addw(a: u32, b: u32): u32 { return a +% b; }
export function u32add(a: u32, b: u32): u32 { return a + b; }
export function u32to64(a: u32): u64 { return a as u64; }
export function i32to64(a: i32): i64 { return a as i64; }
export function u32toi(a: u32): i32 { return a as i32; }
export function itou(a: i32): u32 { return a as u32; }
export function u8toi8(a: u8): i8 { return a as i8; }
export function l2i(a: i64): i32 { return a as i32; }
export function u64lt(a: u64, b: u64): bool { return a < b; }
export function u64div(a: u64, b: u64): u64 { return a / b; }
export function u64shr(a: u64, b: u64): u64 { return a >> b; }
export function u64max(): u64 { return 18446744073709551615; }
export function i64min(): i64 { return -9223372036854775808; }
export function u64toi64(a: u64): i64 { return a as i64; }
export function f2i(x: f64): i32 { return x as i32; }
export function f2u8(x: f64): u8 { return x as u8; }
export function f2u32(x: f64): u32 { return x as u32; }
export function f2l(x: f64): i64 { return x as i64; }
export function f2ul(x: f64): u64 { return x as u64; }
export function i2f(x: i32): f64 { return x as f64; }
export function l2f(x: i64): f64 { return x as f64; }
export function ul2f(x: u64): f64 { return x as f64; }
export function nanOut(): f64 { const z = 0.0; return z / z; }
export function fdiv(a: f64, b: f64): f64 { return a / b; }
export function fmin(a: f64, b: f64): f64 { return a < b ? a : b; }
export function nanlt(a: f64): i32 { return (a < 1.0 ? 1 : 0) + (a >= 1.0 ? 2 : 0) + (a === a ? 4 : 0) + (a !== a ? 8 : 0); }
export function negzero(a: f64): f64 { return -a; }
export function fsub(a: f64, b: f64): f64 { return a - b; }
export function main(log: Log): void { log.write(2, "up"); }
"#;

fn run(wasm: &[u8], name: &str, args: &[Value]) -> Result<Vec<Value>, Option<TrapKind>> {
    let mut p = Prog::from_wasm(wasm.to_vec());
    match p.rt.call_export(name, args) {
        Ok(v) => Ok(v),
        Err(e) => Err(trap_kind(&e)),
    }
}

fn i(x: i32) -> Value { I32(x) }

#[test]
fn numerics_table() {
    let wasm = compile(SRC);
    let mut bad: Vec<String> = Vec::new();
    let mut n = 0;
    macro_rules! ok { ($f:expr, [$($a:expr),*], $want:expr) => {{ let args: Vec<Value> = vec![$($a),*];
        n += 1;
        let got = run(&wasm, $f, &args);
        let want: Result<Vec<Value>, Option<TrapKind>> = Ok(vec![$want]);
        if got != want { bad.push(format!("{}({:?}) = {:?}, want {:?}", $f, args, got, want)); }
    }}; }
    macro_rules! trap { ($f:expr, [$($a:expr),*], $k:expr) => {{ let args: Vec<Value> = vec![$($a),*];
        n += 1;
        let got = run(&wasm, $f, &args);
        if got != Err(Some($k)) { bad.push(format!("{}({:?}) = {:?}, want trap {:?}", $f, args, got, $k)); }
    }}; }
    let (mx, mn) = (i32::MAX, i32::MIN);
    ok!("addI", [i(1), i(2)], i(3));
    trap!("addI", [i(mx), i(1)], TrapKind::Overflow);
    trap!("subI", [i(mn), i(1)], TrapKind::Overflow);
    trap!("mulI", [i(65536), i(32768)], TrapKind::Overflow);
    ok!("mulI", [i(-65536), i(32768)], i(mn));
    trap!("divI", [i(1), i(0)], TrapKind::DivByZero);
    trap!("divI", [i(mn), i(-1)], TrapKind::Overflow);
    ok!("divI", [i(-7), i(2)], i(-3));
    trap!("remI", [i(5), i(0)], TrapKind::DivByZero);
    trap!("remI", [i(mn), i(-1)], TrapKind::Overflow);
    ok!("remI", [i(-7), i(2)], i(-1));
    trap!("negI", [i(mn)], TrapKind::Overflow);
    trap!("shlI", [i(1), i(32)], TrapKind::Bounds);
    trap!("shlI", [i(1), i(-1)], TrapKind::Bounds);
    ok!("shlI", [i(1), i(31)], i(mn));
    ok!("shrI", [i(mn), i(31)], i(-1));
    trap!("shrI", [i(1), i(32)], TrapKind::Bounds);
    ok!("addW", [i(mx), i(1)], i(mn));
    ok!("mulW", [i(mx), i(2)], i(-2));
    ok!("addS", [i(mx), i(5)], i(mx));
    ok!("subS", [i(mn), i(5)], i(mn));
    trap!("divL", [I64(i64::MIN), I64(-1)], TrapKind::Overflow);
    trap!("remL", [I64(i64::MIN), I64(-1)], TrapKind::Overflow);
    trap!("divL", [I64(1), I64(0)], TrapKind::DivByZero);
    trap!("mulL", [I64(i64::MAX), I64(2)], TrapKind::Overflow);
    trap!("addL", [I64(i64::MAX), I64(1)], TrapKind::Overflow);
    trap!("negL", [I64(i64::MIN)], TrapKind::Overflow);
    ok!("addL", [I64(i64::MAX), I64(i64::MIN)], I64(-1));
    ok!("remL", [I64(-7), I64(2)], I64(-1));
    // narrow ints: re-canonicalisation
    ok!("u8w", [i(200), i(100)], i(44));
    ok!("u8sub", [i(1), i(2)], i(255));
    trap!("u8mul", [i(16), i(16)], TrapKind::Overflow);
    ok!("u8mul", [i(15), i(17)], i(255));
    ok!("u8mulw", [i(16), i(17)], i(16));
    ok!("u8shl", [i(200), i(1)], i(144));
    ok!("u8shl", [i(1), i(7)], i(128));
    trap!("u8shl", [i(1), i(8)], TrapKind::Bounds);
    ok!("u8not", [i(5)], i(250));
    ok!("u8not", [i(0)], i(255));
    ok!("u8lt", [i(250), i(10)], i(1));
    ok!("u8sats", [i(250), i(10)], i(255));
    trap!("u8inc", [i(255)], TrapKind::Overflow);
    ok!("i8w", [i(127), i(2)], i(-2));
    ok!("i8w", [i(-128), i(-1)], i(-128));
    trap!("i8neg", [i(-128)], TrapKind::Overflow);
    trap!("i8divm", [i(-128), i(-1)], TrapKind::Overflow);
    ok!("i8not", [i(5)], i(-6));
    ok!("i8not", [i(-128)], i(127));
    ok!("i8sat", [i(-100), i(100)], i(-128));
    ok!("i16w", [i(32767), i(1)], i(-32768));
    ok!("u16shl", [i(0xFFFF), i(4)], i(0xFFF0));
    ok!("u32div", [i(-1), i(3)], i(1431655765));
    ok!("u32rem", [i(-1), i(10)], i(5));
    ok!("u32shr", [i(mn), i(31)], i(1));
    ok!("u32lt", [i(-1), i(1)], i(0));
    ok!("u32lt", [i(1), i(-1)], i(1));
    ok!("u32addw", [i(-1), i(2)], i(1));
    trap!("u32add", [i(-1), i(2)], TrapKind::Overflow);
    ok!("u32to64", [i(-1)], I64(4294967295));
    ok!("i32to64", [i(-1)], I64(-1));
    trap!("u32toi", [i(-1)], TrapKind::Overflow);
    trap!("itou", [i(-1)], TrapKind::Overflow);
    ok!("itou", [i(5)], i(5));
    trap!("u8toi8", [i(200)], TrapKind::Overflow);
    ok!("u8toi8", [i(100)], i(100));
    trap!("l2i", [I64(1 << 32)], TrapKind::Overflow);
    ok!("l2i", [I64(-5)], i(-5));
    ok!("u64lt", [I64(-1), I64(1)], i(0));
    ok!("u64div", [I64(-1), I64(2)], I64(i64::MAX));
    ok!("u64shr", [I64(i64::MIN), I64(63)], I64(1));
    ok!("u64max", [], I64(-1));
    ok!("i64min", [], I64(i64::MIN));
    trap!("u64toi64", [I64(-1)], TrapKind::Overflow);
    // float -> int
    trap!("f2i", [F64(f64::NAN)], TrapKind::Overflow);
    trap!("f2i", [F64(2147483648.0)], TrapKind::Overflow);
    ok!("f2i", [F64(2147483647.9)], i(2147483647));
    ok!("f2i", [F64(-2147483648.9)], i(mn));
    trap!("f2i", [F64(-2147483649.0)], TrapKind::Overflow);
    trap!("f2i", [F64(f64::INFINITY)], TrapKind::Overflow);
    ok!("f2u8", [F64(255.9)], i(255));
    ok!("f2u8", [F64(-0.9)], i(0));
    trap!("f2u8", [F64(256.0)], TrapKind::Overflow);
    trap!("f2u8", [F64(-1.0)], TrapKind::Overflow);
    ok!("f2u32", [F64(4294967295.5)], i(-1));
    trap!("f2u32", [F64(4294967296.0)], TrapKind::Overflow);
    trap!("f2l", [F64(9223372036854775808.0)], TrapKind::Overflow);
    ok!("f2l", [F64(-9223372036854775808.0)], I64(i64::MIN));
    trap!("f2ul", [F64(-1.0)], TrapKind::Overflow);
    ok!("f2ul", [F64(18446744073709549568.0)], I64(-2048));
    trap!("f2ul", [F64(18446744073709551616.0)], TrapKind::Overflow);
    ok!("i2f", [i(mn)], F64(-2147483648.0));
    ok!("l2f", [I64(i64::MAX)], F64(9223372036854775807i64 as f64));
    ok!("ul2f", [I64(-1)], F64(u64::MAX as f64));
    ok!("fdiv", [F64(1.0), F64(0.0)], F64(f64::INFINITY));
    ok!("negzero", [F64(0.0)], F64(-0.0));
    ok!("nanlt", [F64(f64::NAN)], i(8));
    ok!("nanlt", [F64(2.0)], i(6));
    // NaN determinism at the export boundary
    for (name, args) in [("nanOut", vec![]), ("fdiv", vec![F64(0.0), F64(0.0)]), ("fsub", vec![F64(f64::INFINITY), F64(f64::INFINITY)]), ("fdiv", vec![F64(f64::NAN), F64(1.0)])] {
        n += 1;
        match run(&wasm, name, &args) {
            Ok(v) => match v.as_slice() {
                [F64(x)] if x.is_nan() && x.to_bits() == 0x7ff8_0000_0000_0000 => {}
                o => bad.push(format!("{name}: NaN not canonical: {:?}", o.iter().map(|v| if let F64(x) = v { format!("{:#x}", x.to_bits()) } else { "?".into() }).collect::<Vec<_>>())),
            },
            Err(e) => bad.push(format!("{name}: trap {e:?}")),
        }
    }
    eprintln!("numeric cases: {n}, mismatches: {}", bad.len());
    assert!(bad.is_empty(), "{} of {n} numeric cases differ:\n{}", bad.len(), bad.join("\n"));
}
