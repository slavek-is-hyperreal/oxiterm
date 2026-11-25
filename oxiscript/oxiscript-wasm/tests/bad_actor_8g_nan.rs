//! BAD ACTOR 8.G: NaN payloads must not reach the host (ir.md section 6).
#![allow(clippy::pedantic)]
mod common;
use common::*;
use oxiscript_ir::parse_module;
use oxiscript_runtime::Value;

fn run(src: &str, f: &str, args: &[Value]) -> Vec<Value> {
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    rt.call_export(f, args).unwrap()
}

#[test]
fn export_returns_canonical_nan_f32() {
    // 0.0/0.0 is a NaN produced by the engine; its bits are hardware dependent
    let r = run(
        "oxiir 1\nmodule \"n\"\nexport func @nan() -> f32 {\ne:\n  %z: f32 = const 0.0\n  %n: f32 = div %z, %z\n  ret %n\n}",
        "nan",
        &[],
    );
    let Value::F32(x) = r[0] else { panic!() };
    assert_eq!(x.to_bits(), 0x7fc0_0000, "NaN handed to the host must be the canonical NaN, got {:#x}", x.to_bits());
}

#[test]
fn export_returns_canonical_nan_f64() {
    let r = run(
        "oxiir 1\nmodule \"n\"\nexport func @nan() -> f64 {\ne:\n  %z: f64 = const 0.0\n  %n: f64 = div %z, %z\n  ret %n\n}",
        "nan",
        &[],
    );
    let Value::F64(x) = r[0] else { panic!() };
    assert_eq!(x.to_bits(), 0x7ff8_0000_0000_0000, "got {:#x}", x.to_bits());
}

#[test]
fn host_nan_payload_does_not_round_trip() {
    let r = run(
        "oxiir 1\nmodule \"n\"\nexport func @id(%a: f32) -> f32 {\ne:\n  ret %a\n}",
        "id",
        &[Value::F32(f32::from_bits(0x7fc0_0001))],
    );
    let Value::F32(x) = r[0] else { panic!() };
    assert_eq!(x.to_bits(), 0x7fc0_0000, "payload leaked: {:#x}", x.to_bits());
}

#[test]
fn sqrt_neg_nan_bits() {
    let r = run(
        "oxiir 1\nmodule \"n\"\nexport func @s(%a: f64) -> f64 {\ne:\n  %n: f64 = sqrt %a\n  ret %n\n}",
        "s",
        &[Value::F64(-1.0)],
    );
    let Value::F64(x) = r[0] else { panic!() };
    assert_eq!(x.to_bits(), 0x7ff8_0000_0000_0000, "got {:#x}", x.to_bits());
}
