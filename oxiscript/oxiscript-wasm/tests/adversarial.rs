//! Adversarial IR shapes: deep nesting, huge functions, tiny functions. The
//! compiler must answer with a diagnostic or valid Wasm, never crash.
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_ir::parse_module;
use oxiscript_runtime::{Grants, Value};
use oxiscript_wasm::{compile, E_LIMIT};
use std::fmt::Write as _;

/// `n` properly nested natural loops.
fn nested_loops(n: usize) -> String {
    let mut s = String::from("oxiir 1\nmodule \"nl\"\nexport func @f(%c: bool) -> i32 {\nentry:\n  br h0()\n");
    for i in 0..n {
        let _ = write!(s, "h{i}:\n  br h{}()\n", i + 1);
    }
    let _ = write!(s, "h{n}:\n  br_if %c, h{n}(), x{n}()\n");
    for i in (1..=n).rev() {
        let _ = write!(s, "x{i}:\n  br_if %c, h{}(), x{}()\n", i - 1, i - 1);
    }
    let _ = write!(s, "x0:\n  %r: i32 = const 7\n  ret %r\n}}");
    s
}

/// A dominator chain whose inner levels are merge children (`j`/`k`) of the
/// level above; `nest_k` picks which of the two carries the next level.
fn merge_chain(n: usize, nest_k: bool) -> String {
    let mut s = String::from("oxiir 1\nmodule \"mc\"\nexport func @f(%c: bool) -> i32 {\nentry:\n  br x0()\n");
    for i in 0..n {
        let (a, b) = if nest_k { (format!("j{i}"), format!("k{i}")) } else { (format!("k{i}"), format!("j{i}")) };
        let _ = write!(
            s,
            "x{i}:\n  br_if %c, p{i}(), q{i}()\np{i}:\n  br_if %c, {a}(), {b}()\nq{i}:\n  br_if %c, {a}(), {b}()\nj{i}:\n  %r{i}: i32 = const {i}\n  ret %r{i}\n"
        );
        if i + 1 < n {
            let _ = write!(s, "k{i}:\n  br x{}()\n", i + 1);
        } else {
            let _ = write!(s, "k{i}:\n  %z: i32 = const -1\n  ret %z\n");
        }
    }
    s.push('}');
    s
}

fn compile_in_small_stack(src: String, stack: usize) -> Result<Vec<u8>, String> {
    let m = parse_module(&src).expect("parse");
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || compile(&m, &opts()).map_err(|e| format!("{}", e.0[0].code)))
        .unwrap()
        .join()
        .expect("compiler must not overflow the stack")
}

#[test]
fn deep_nesting_is_a_diagnostic_or_valid_wasm_in_a_small_stack() {
    // 2 MiB: the stack of a default Rust test thread.
    for n in [50, 200, 900, 1200, 3000] {
        for src in [nested_loops(n), merge_chain(n, true), merge_chain(n, false)] {
            match compile_in_small_stack(src, 2 * 1024 * 1024) {
                Ok(w) => oxiscript_wasm::validate(&w).unwrap(),
                Err(code) => assert_eq!(code, E_LIMIT, "n={n}"),
            }
        }
    }
}

#[test]
fn nesting_just_under_the_limit_loads_and_runs() {
    let h = host();
    for src in [nested_loops(40), merge_chain(40, true), merge_chain(40, false)] {
        let m = parse_module(&src).unwrap();
        let wasm = build(&m);
        let mut rt = h.instantiate(&wasm, Grants::all()).unwrap();
        let r = rt.call_export("f", &[Value::I32(0)]).unwrap();
        assert_eq!(r.len(), 1);
    }
}

#[test]
fn huge_function_is_a_diagnostic_or_loads() {
    // ~70k SSA values: one wasm local each
    let mut s = String::from("oxiir 1\nmodule \"big\"\nexport func @f(%a: i32) -> i32 {\ne:\n");
    let n = 70_000;
    let mut prev = "%a".to_string();
    for i in 0..n {
        let _ = write!(s, "  %v{i}: i32 = add.wrap {prev}, %a\n");
        prev = format!("%v{i}");
    }
    let _ = write!(s, "  ret {prev}\n}}");
    let m = parse_module(&s).unwrap();
    match compile(&m, &opts()) {
        Ok(w) => oxiscript_wasm::validate(&w).unwrap(),
        Err(e) => assert_eq!(e.0[0].code, E_LIMIT, "{e}"),
    }
}

#[test]
fn many_tiny_functions_load_under_the_strict_engine_limits() {
    let mut s = String::from("oxiir 1\nmodule \"tiny\"\nglobal @g: i32\n");
    for i in 0..1000 {
        let _ = write!(s, "export func @get{i}() -> i32 {{\ne:\n  %v: i32 = global.get @g\n  ret %v\n}}\n");
    }
    let m = parse_module(&s).unwrap();
    let wasm = build(&m);
    let h = host();
    match h.instantiate(&wasm, Grants::all()) {
        Ok(mut rt) => {
            assert_eq!(rt.call_export("get999", &[]).unwrap(), vec![Value::I32(0)]);
        }
        // wasmi's `EnforcedLimits::strict()` demands >= 40 bytes per function on
        // average (once a module exceeds 1 KiB): recorded as a cross-slice finding.
        Err(e) => assert!(e.to_string().contains("minimum average bytes per function"), "{e}"),
    }
}

#[test]
fn empty_array_and_empty_string_cross_the_host_boundary() {
    // (ptr, len) = (0, 0) for a zero-capacity out-buffer and an empty string
    let src = r#"oxiir 1
module "emptyhost"
import @state_open = "host:state@1" "open" (i32, str) -> (i32, i32)
import @write_str = "host:state@1" "write_str" (i32, i32, str) -> i32
import @read_str = "host:state@1" "read_str" (i32, i32, [u8]) -> (i32, i32)
data @key = "k"
data @empty = ""
global @st: i32
global @slot: i32
export func @main(%st: i32) -> void {
e:
  global.set @st, %st
  %k: str = str.const @key
  %s: i32, %h: i32 = call @state_open(%st, %k)
  global.set @slot, %h
  ret
}
export func @probe() -> i32 {
e:
  %st: i32 = global.get @st
  %h: i32 = global.get @slot
  %e: str = str.const @empty
  %w: i32 = call @write_str(%st, %h, %e)
  %z: i32 = const 0
  %buf: [u8] = array.new %z
  %rs: i32, %len: i32 = call @read_str(%st, %h, %buf)
  release %buf
  %k: i32 = const 100
  %a: i32 = mul %rs, %k
  %r: i32 = add %a, %len
  ret %r
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let e = env();
    let mut rt = e.host.instantiate(&wasm, Grants::all()).unwrap();
    // empty value fits a zero-capacity buffer: status 0, length 0
    assert_eq!(rt.call_export("probe", &[]).unwrap(), vec![Value::I32(0)]);
    assert_eq!(rt.deinit().unwrap(), 0);
}

#[test]
fn empty_arrays_and_zero_length_boundary_values() {
    // `array.new 0`, `str.from_utf8` of an empty array, a zero-capacity out buffer
    let src = r#"oxiir 1
module "empty"
export func @f() -> i32 {
e:
  %z: i32 = const 0
  %a: [u8] = array.new %z
  %s: str = str.from_utf8 %a, %z
  %n: i32 = str.len %s
  %l: i32 = array.len %a
  release %s
  release %a
  %r: i32 = add %n, %l
  ret %r
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = h.instantiate(&wasm, Grants::all()).unwrap();
    assert_eq!(rt.call_export("f", &[]).unwrap(), vec![Value::I32(0)]);
    assert_eq!(rt.deinit().unwrap(), 0);
}
