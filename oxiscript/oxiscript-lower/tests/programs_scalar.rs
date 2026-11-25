#![allow(clippy::pedantic, clippy::nursery)]
//! Closed programs: integers, floats, strings, templates.
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::Value;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn prog(body: &str) -> String {
    format!("{HDR}{body}")
}

#[test]
fn p01_integer_arithmetic() {
    run_log(
        &prog(
            r#"
function f(a: i32, b: i32): i32 { return (a * b + 7) / 3 - a % 4; }
export function main(log: Log): void {
  log.write(2, `${f(10, 20)}`);
  log.write(2, `${7 / 2} ${-7 / 2} ${7 % 3} ${-7 % 3}`);
  log.write(2, `${1 << 4} ${256 >> 2} ${-16 >> 2} ${5 & 3} ${5 | 3} ${5 ^ 3} ${~5}`);
  const big: i64 = 4000000000i64;
  log.write(2, `${big * 3i64} ${-big}`);
  log.write(2, `${3 < 4} ${3 >= 4} ${3 === 3} ${3 !== 3}`);
}
"#,
        ),
        &["67", "3 -3 1 -1", "16 64 -4 1 7 6 -6", "12000000000 -4000000000", "true false true false"],
    );
}

#[test]
fn p02_unsigned_and_narrow() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  const a: u8 = 200u8;
  const b: u32 = 4000000000u32;
  const c: i8 = -128i8;
  const d: u64 = 18446744073709551615u64;
  log.write(2, `${a} ${b} ${c} ${d}`);
  log.write(2, `${(a as i32) + 100} ${b >> 28} ${c as i32}`);
  const e: i16 = 32767i16;
  log.write(2, `${e} ${(e as i32) + 1}`);
}
"#,
        ),
        &["200 4000000000 -128 18446744073709551615", "300 14 -128", "32767 32768"],
    );
}

#[test]
fn p03_overflow_traps() {
    let mut p = Prog::new(&prog(
        r#"
@host export function add_max(): i32 { let x: i32 = 2147483647; x += 1; return x; }
@host export function neg_min(): i32 { const m: i32 = -2147483647 - 1; return -m; }
@host export function u8_sub(): u8 { const z: u8 = 0u8; const o: u8 = 1u8; return z - o; }
@host export function mul_big(): i64 { const a: i64 = 9223372036854775807i64; return a * 2i64; }
@host export function cast_bad(): u8 { const x: i32 = 300; return x as u8; }
@host export function div_zero(): i32 { const z: i32 = 0; return 5 / z; }
@host export function rem_zero(): i32 { const z: i32 = 0; return 5 % z; }
@host export function shift_big(): i32 { const s: i32 = 40; return 1 << s; }
export function main(log: Log): void { log.write(2, "up"); }
"#,
    ));
    for (name, kind) in [
        ("add_max", TrapKind::Overflow),
        ("neg_min", TrapKind::Overflow),
        ("u8_sub", TrapKind::Overflow),
        ("mul_big", TrapKind::Overflow),
        ("cast_bad", TrapKind::Overflow),
        ("div_zero", TrapKind::DivByZero),
        ("rem_zero", TrapKind::DivByZero),
        ("shift_big", TrapKind::Bounds),
    ] {
        let e = p.call_err(name, &[]);
        assert_eq!(trap_kind(&e), Some(kind), "{name}");
        // a trapped instance is poisoned: restart for the next probe
        p = Prog::from_wasm(p.wasm.clone());
    }
}

#[test]
fn p04_wrap_and_saturate() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  const a: u8 = 200u8;
  const b: u8 = 100u8;
  log.write(2, `${a +% b} ${a +| b} ${b -% a} ${b -| a} ${a *% b} ${a *| b}`);
  const x: i32 = 2147483647;
  log.write(2, `${x +% 1} ${x +| 1} ${(0 - x - 1) -| 5}`);
  const big: i32 = 1000;
  log.write(2, `${big.wrapping<u8>()} ${big.saturating<u8>()} ${(-5).saturating<u8>()} ${300.wrapping<i8>()}`);
  const f: f64 = 1.0e30;
  log.write(2, `${f.saturating<i32>()} ${(-f).saturating<i32>()}`);
}
"#,
        ),
        &["44 255 156 0 32 255", "-2147483648 2147483647 -2147483648", "232 255 0 44", "2147483647 -2147483648"],
    );
}

#[test]
fn p05_floats() {
    run_log(
        &prog(
            r#"
function hyp(a: f64, b: f64): f64 { return a * a + b * b; }
export function main(log: Log): void {
  const h = hyp(3.0, 4.0);
  log.write(2, `${h as i32} ${(h * 100.0) as i32} ${(1.0 / 3.0 * 3.0) as i32}`);
  const x: f32 = 1.5f32;
  const y: f32 = 2.25f32;
  log.write(2, `${(x * y * 1000.0f32) as i32} ${(x < y)} ${(x === 1.5f32)}`);
  const nan = 0.0 / 0.0;
  log.write(2, `${nan === nan} ${nan !== nan} ${(-0.0 === 0.0)}`);
  log.write(2, `${(7.9 as i32)} ${(-7.9 as i32)} ${(2.0 as f32 as f64) === 2.0}`);
}
"#,
        ),
        &["25 2500 1", "3375 true true", "false true true", "7 -7 true"],
    );
}

#[test]
fn p06_float_to_int_traps() {
    let mut p = Prog::new(&prog(
        r#"
@host export function nan_cast(): i32 { const n = 0.0 / 0.0; return n as i32; }
@host export function big_cast(): i32 { const b: f64 = 1.0e12; return b as i32; }
export function main(log: Log): void { log.write(2, "up"); }
"#,
    ));
    let e = p.call_err("nan_cast", &[]);
    assert_eq!(trap_kind(&e), Some(TrapKind::Overflow));
    p = Prog::from_wasm(p.wasm.clone());
    let e = p.call_err("big_cast", &[]);
    assert_eq!(trap_kind(&e), Some(TrapKind::Overflow));
}

#[test]
fn p07_strings() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  const a = "héllo";
  const b = " wörld";
  const c = a + b;
  log.write(2, c);
  log.write(2, `${c.byteLength} ${a === "héllo"} ${a !== b} ${a < b} ${"abc" < "abd"} ${"b" > "abc"}`);
  log.write(2, c.slice(0, 6) + "|" + c.slice(7, 13));
  log.write(2, `${c.startsWith("hé")} ${c.endsWith("rld")} ${c.includes("o w")} ${c.indexOf("wö")} ${c.indexOf("zz")}`);
  const bs = "abc".bytes();
  log.write(2, `${bs.length} ${bs[0]} ${bs[2]}`);
  const cs = "aé✓😀".chars();
  log.write(2, `${cs.length} ${cs[1]} ${cs[2]} ${cs[3]}`);
}
"#,
        ),
        &["héllo wörld", "13 true true false true true", "héllo|wörld", "true true true 7 -1", "3 97 99", "4 233 10003 128512"],
    );
}

#[test]
fn p08_templates_and_numbers() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  const i: i64 = -9223372036854775807i64 - 1i64;
  const u: u64 = 18446744073709551615u64;
  log.write(2, `${i}|${u}|${0}|${-1}|${true}|${false}`);
  log.write(2, `${`in${1 + 1}ner`} ${"s"}${"t"}`);
  log.write(2, `plain`);
  log.write(2, `${(5).toString()} ${true.toString()}`);
}
"#,
        ),
        &["-9223372036854775808|18446744073709551615|0|-1|true|false", "in2ner st", "plain", "5 true"],
    );
}

#[test]
fn p09_string_slice_traps() {
    let mut p = Prog::new(&prog(
        r#"
@host export function bad_slice(): i32 { const s = "héllo"; return s.slice(0, 2).byteLength; }
@host export function oob_slice(): i32 { const s = "abc"; return s.slice(0, 10).byteLength; }
export function main(log: Log): void { log.write(2, "up"); }
"#,
    ));
    let e = p.call_err("bad_slice", &[]);
    assert!(matches!(trap_kind(&e), Some(TrapKind::BadUtf8)), "{e:?}");
    p = Prog::from_wasm(p.wasm.clone());
    let e = p.call_err("oob_slice", &[]);
    assert_eq!(trap_kind(&e), Some(TrapKind::Bounds));
}

#[test]
fn p10_exports_with_scalars_and_strings() {
    let mut p = Prog::new(&prog(
        r#"
@host export function twice(x: i32): i32 { return x * 2; }
@host export function mix(a: i64, b: f64, c: bool): f64 { return c ? (a as f64) + b : 0.0; }
@host export function greet(name: string): i32 { return name.byteLength; }
export function main(log: Log): void { log.write(2, "up"); }
"#,
    ));
    assert_eq!(p.call("twice", &[Value::I32(21)]), vec![Value::I32(42)]);
    assert_eq!(p.call("mix", &[Value::I64(2), Value::F64(0.5), Value::I32(1)]), vec![Value::F64(2.5)]);
    assert_eq!(p.call("mix", &[Value::I64(2), Value::F64(0.5), Value::I32(0)]), vec![Value::F64(0.0)]);
    assert_eq!(p.call("greet", &[Value::Str("héllo".into())]), vec![Value::I32(6)]);
    p.finish();
}
