//! BAD ACTOR round 2026-11-27-1, target 1 (8.Fix lowering): the software IEEE
//! float `%` helpers (`@lw.fmod_f64`, `@lw.fmod_f32`) against Rust `%` (libm
//! fmod) over random bit patterns and the worst-case exponent gap, plus `%=`.
//! Failing tests are findings (R1-nn in docs/stages/rounds/2026-11-27-1.md).
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::*;
use oxiscript_runtime::{Grants, Host, Limits, Value};

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

const SRC: &str = r#"
@host export function rem64(a: f64, b: f64): f64 {
  return a % b;
}
@host export function rem32(a: f32, b: f32): f32 {
  return a % b;
}
@host export function remeq64(a: f64, b: f64): f64 {
  let x = a;
  x %= b;
  return x;
}
"#;

fn interesting64(r: &mut Rng) -> f64 {
    match r.next() % 6 {
        0 => f64::from_bits(r.next()),
        1 => f64::from_bits(r.next() & 0x800f_ffff_ffff_ffff), // subnormal / zero
        2 => f64::from_bits((r.next() & 0x800f_ffff_ffff_ffff) | 0x7fe0_0000_0000_0000), // near max
        3 => (r.next() % 1000) as f64 - 500.0,
        4 => f64::from_bits(r.next() | 0x7ff0_0000_0000_0000), // inf / nan
        _ => f64::from_bits(r.next() & 0x3fff_ffff_ffff_ffff),
    }
}

fn interesting32(r: &mut Rng) -> f32 {
    let v = r.next() as u32;
    match r.next() % 5 {
        0 => f32::from_bits(v),
        1 => f32::from_bits(v & 0x807f_ffff),
        2 => f32::from_bits((v & 0x807f_ffff) | 0x7f00_0000),
        3 => (v % 1000) as f32 - 500.0,
        _ => f32::from_bits(v | 0x7f80_0000),
    }
}

/// Probe: random bit patterns, both widths, and `%=`, must be bit-exact with Rust `%`
/// (NaN: any NaN).
#[test]
fn fmod_matches_rust_rem_on_random_bits() {
    let wasm = compile(SRC);
    let host = Host::with_options(Limits::default(), test_host_options());
    let mut rt = host.instantiate(&wasm, Grants::none()).expect("instantiate");
    let mut r = Rng(0x9e37_79b9_7f4a_7c15);
    let mut bad = Vec::new();
    for i in 0..20_000 {
        let (a, b) = (interesting64(&mut r), interesting64(&mut r));
        let f = if i % 2 == 0 { "rem64" } else { "remeq64" };
        let out = rt.call_export(f, &[Value::F64(a), Value::F64(b)]).expect("call");
        let Some(&Value::F64(got)) = out.first() else { panic!("{out:?}") };
        let want = a % b;
        let ok = if want.is_nan() { got.is_nan() } else { got.to_bits() == want.to_bits() };
        if !ok && bad.len() < 10 {
            bad.push(format!("{f}({a:e} [{:016x}], {b:e} [{:016x}]) = {got:e}, want {want:e}", a.to_bits(), b.to_bits()));
        }
    }
    for _ in 0..20_000 {
        let (a, b) = (interesting32(&mut r), interesting32(&mut r));
        let out = rt.call_export("rem32", &[Value::F32(a), Value::F32(b)]).expect("call");
        let Some(&Value::F32(got)) = out.first() else { panic!("{out:?}") };
        let want = a % b;
        let ok = if want.is_nan() { got.is_nan() } else { got.to_bits() == want.to_bits() };
        if !ok && bad.len() < 20 {
            bad.push(format!("rem32({a:e} [{:08x}], {b:e} [{:08x}]) = {got:e}, want {want:e}", a.to_bits(), b.to_bits()));
        }
    }
    assert!(bad.is_empty(), "fmod mismatches:\n{}", bad.join("\n"));
}

/// Probe: the worst case of the bit-by-bit loop (largest finite % smallest subnormal) runs
/// ~2100 iterations; the fuel it costs must stay small relative to `fuel_per_call`.
#[test]
fn fmod_worst_case_fuel() {
    let wasm = compile(SRC);
    let host = Host::with_options(Limits::default(), test_host_options());
    let mut rt = host.instantiate(&wasm, Grants::none()).expect("instantiate");
    let out = rt.call_export("rem64", &[Value::F64(f64::MAX), Value::F64(f64::from_bits(1))]).unwrap();
    assert_eq!(out, vec![Value::F64(f64::MAX % f64::from_bits(1))]);
    let fuel = rt.last_fuel_used();
    eprintln!("f64::MAX % min subnormal: fuel {fuel}");
    assert!(fuel < 100_000, "one `%` costs {fuel} fuel");
}

const TWO_INST: &str = r#"
function id<T>(x: T): T {
  return x;
}
@host export function run(): i32 {
  const a = id(1);
  const b = id(true);
  return b ? a : 0;
}
"#;

/// Probe: `Options::max_instantiations` is honoured by both entry points, and a budget of 0 does
/// not fire on a program without generics.
#[test]
fn probe_max_instantiations_both_entry_points() {
    use oxiscript_lower::{compile_program, compile_source, Options};
    let opts = Options { target: test_target(), max_instantiations: 1, ..Options::default() };
    let e1 = compile_source(TWO_INST, &interfaces(), &opts).expect_err("budget 1, two instantiations");
    assert!(e1.to_string().contains("OXS-E0701"), "{e1}");
    let e2 = compile_program("", TWO_INST, &mut oxiscript_check::NoLoader, &interfaces(), &opts).err().expect("budget 1");
    assert!(e2.to_string().contains("OXS-E0701"), "{e2}");
    let zero = Options { target: test_target(), max_instantiations: 0, ..Options::default() };
    compile_source("@host export function f(): i32 {\n  return 1;\n}\n", &interfaces(), &zero).expect("no generics, budget 0");
    let ok = compile_program("", TWO_INST, &mut oxiscript_check::NoLoader, &interfaces(), &Options { target: test_target(), ..Options::default() }).expect("default");
    eprintln!("instantiations reported: {}", ok.instantiations);
    assert!(ok.instantiations >= 2);
}
