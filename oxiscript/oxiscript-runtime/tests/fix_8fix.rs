//! Stage 8.Fix regression tests for oxiscript-runtime:
//! - Relaxed lazy-compilation-only 40 avg-bytes/function heuristic so modules
//!   with 1000 tiny functions load and execute cleanly ([R2-109, R3-61]), while
//!   structural caps (max params, max globals, max functions) remain enforced.
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::Fixture;
use oxiscript_runtime::{Grants, Host, Limits, Value};

#[test]
fn thousand_tiny_exported_functions_load_and_run() {
    // 1000 tiny exported functions (well below 40 bytes/func average) previously
    // failed with OXS-E0408 under wasmi::EnforcedLimits::strict().
    let mut extra = String::new();
    for i in 0..1000 {
        extra.push_str(&format!(
            "  (func (export \"f{i}\") (result i32) (i32.const {i}))\n"
        ));
    }
    let bytes = Fixture::new(&[]).body(&extra).bytes();
    let host = Host::new(Limits::default());
    let mut rt = host
        .instantiate(&bytes, Grants::none())
        .expect("1000 tiny functions should load under eager compilation");
    let v = rt.call_export("f999", &[]).expect("call f999");
    assert_eq!(v, vec![Value::I32(999)]);
}

#[test]
fn structural_caps_still_reject_excessive_params_and_globals() {
    // >32 params must still be rejected with OXS-E0408.
    let params = " (param i32)".repeat(33);
    let extra_params = format!("  (func (export \"wide\"){params})\n");
    let b_params = Fixture::new(&[]).body(&extra_params).bytes();
    let host = Host::new(Limits::default());
    let err = host
        .instantiate(&b_params, Grants::none())
        .err()
        .expect("33 params must be rejected");
    assert_eq!(err.code(), "OXS-E0408", "got: {err:?}");

    // >1000 globals must still be rejected with OXS-E0408.
    let mut extra_globals = String::new();
    for i in 0..1001 {
        extra_globals.push_str(&format!("  (global $g{i} i32 (i32.const 0))\n"));
    }
    let b_globals = Fixture::new(&[]).body(&extra_globals).bytes();
    let err_g = host
        .instantiate(&b_globals, Grants::none())
        .err()
        .expect("1001 globals must be rejected");
    assert_eq!(err_g.code(), "OXS-E0408", "got: {err_g:?}");
}
