#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.L: misuse of the IR ops added by 8.L (str.concat/eq/cmp/slice/byte/from_f64, array.empty, array.pop).
//! The verifier must reject every ill-typed use; accepted modules must compile to Wasm without panicking.
use oxiscript_ir::*;
use std::panic::catch_unwind;

fn outcome(body: &str) -> String {
    let src = format!("oxiir 1\nmodule \"m\"\n{body}");
    let r = catch_unwind(|| {
        let m = match parse_module(&src) { Ok(m) => m, Err(e) => return format!("parse-reject: {e}") };
        match verify_ok(&m) {
            Err(e) => format!("verify-reject: {e}"),
            Ok(()) => match oxiscript_wasm::compile(&m, &oxiscript_wasm::CompileOptions::default()) {
                Ok(_) => "ACCEPTED".to_string(),
                Err(e) => format!("codegen-reject: {e}"),
            },
        }
    });
    r.unwrap_or_else(|_| "PANIC".to_string())
}

const BAD: &[(&str, &str)] = &[
    ("concat_int", "func @f(%a: str, %b: i32) -> str {\nentry:\n  %c: str = str.concat %a, %b\n  ret %c\n}\n"),
    ("concat_result_int", "func @f(%a: str, %b: str) -> i32 {\nentry:\n  %c: i32 = str.concat %a, %b\n  ret %c\n}\n"),
    ("eq_wrong_result", "func @f(%a: str, %b: str) -> i32 {\nentry:\n  %c: i32 = str.eq %a, %b\n  ret %c\n}\n"),
    ("eq_array", "func @f(%a: [i32], %b: [i32]) -> bool {\nentry:\n  %c: bool = str.eq %a, %b\n  ret %c\n}\n"),
    ("cmp_bool_result", "func @f(%a: str, %b: str) -> bool {\nentry:\n  %c: bool = str.cmp %a, %b\n  ret %c\n}\n"),
    ("slice_u32_idx", "func @f(%a: str, %i: u32, %j: i32) -> str {\nentry:\n  %c: str = str.slice %a, %i, %j\n  ret %c\n}\n"),
    ("slice_i64_idx", "func @f(%a: str, %i: i64, %j: i32) -> str {\nentry:\n  %c: str = str.slice %a, %i, %j\n  ret %c\n}\n"),
    ("slice_on_array", "func @f(%a: [u8], %i: i32, %j: i32) -> str {\nentry:\n  %c: str = str.slice %a, %i, %j\n  ret %c\n}\n"),
    ("slice_result_array", "func @f(%a: str, %i: i32, %j: i32) -> [u8] {\nentry:\n  %c: [u8] = str.slice %a, %i, %j\n  ret %c\n}\n"),
    ("byte_result_i32", "func @f(%a: str, %i: i32) -> i32 {\nentry:\n  %c: i32 = str.byte %a, %i\n  ret %c\n}\n"),
    ("byte_idx_u8", "func @f(%a: str, %i: u8) -> u8 {\nentry:\n  %c: u8 = str.byte %a, %i\n  ret %c\n}\n"),
    ("from_f64_f32", "func @f(%a: f32) -> str {\nentry:\n  %c: str = str.from_f64 %a\n  ret %c\n}\n"),
    ("from_f64_int", "func @f(%a: i64) -> str {\nentry:\n  %c: str = str.from_f64 %a\n  ret %c\n}\n"),
    ("from_f64_wrong_res", "func @f(%a: f64) -> i32 {\nentry:\n  %c: i32 = str.from_f64 %a\n  ret %c\n}\n"),
    ("array_empty_scalar", "func @f() -> i32 {\nentry:\n  %c: i32 = array.empty\n  ret %c\n}\n"),
    ("array_empty_str", "func @f() -> str {\nentry:\n  %c: str = array.empty\n  ret %c\n}\n"),
    ("pop_on_str", "func @f(%a: str) -> u8 {\nentry:\n  %c: u8 = array.pop %a\n  ret %c\n}\n"),
    ("pop_elem_mismatch", "func @f(%a: [i32]) -> i64 {\nentry:\n  %c: i64 = array.pop %a\n  ret %c\n}\n"),
    ("pop_nullable_mismatch", "func @f(%a: [str]) -> str? {\nentry:\n  %c: str? = array.pop %a\n  ret %c\n}\n"),
    ("pop_result_arrayof", "func @f(%a: [i32]) -> [i32] {\nentry:\n  %c: [i32] = array.pop %a\n  ret %c\n}\n"),
    ("pop_on_opt_array", "func @f(%a: [i32]?) -> i32 {\nentry:\n  %c: i32 = array.pop %a\n  ret %c\n}\n"),
    ("concat_two_results", "func @f(%a: str, %b: str) -> str {\nentry:\n  %c: str, %d: str = str.concat %a, %b\n  ret %c\n}\n"),
    ("concat_no_result", "func @f(%a: str, %b: str) -> void {\nentry:\n  str.concat %a, %b\n  ret\n}\n"),
    ("pop_no_result", "func @f(%a: [i32]) -> void {\nentry:\n  array.pop %a\n  ret\n}\n"),
    ("concat_opt_operand", "func @f(%a: str?, %b: str) -> str {\nentry:\n  %c: str = str.concat %a, %b\n  ret %c\n}\n"),
];

const GOOD: &[(&str, &str)] = &[
    ("empty_arrays_all_kinds", "func @f() -> i32 {\nentry:\n  %a: [i32] = array.empty\n  %b: [str] = array.empty\n  %c: [[u8]] = array.empty\n  %n: i32 = array.len %a\n  ret %n\n}\n"),
    ("pop_managed_owned", "func @f(%a: [str]) -> str {\nentry:\n  %c: str = array.pop %a\n  ret %c\n}\n"),
];

#[test]
fn ir_new_ops_misuse_is_rejected() {
    let mut bad = Vec::new();
    for (name, body) in BAD {
        let o = outcome(body);
        eprintln!("{name}: {}", o.chars().take(110).collect::<String>());
        if o == "ACCEPTED" || o == "PANIC" || o.starts_with("codegen-reject") {
            bad.push(format!("{name}: {o}"));
        }
    }
    for (name, body) in GOOD {
        let o = outcome(body);
        eprintln!("GOOD {name}: {}", o.chars().take(110).collect::<String>());
        if o == "PANIC" { bad.push(format!("{name}: PANIC")); }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// array.pop on an empty array and str.byte/str.slice out of range at IR level (no lowering guard): must trap, not read freed/garbage memory.
#[test]
fn ir_pop_empty_and_str_byte_oob_trap() {
    use oxiscript_runtime::{Grants, Host, Limits};
    let cases = [
        ("pop_empty_i32", "export func @f() -> i32 {\nentry:\n  %a: [i32] = array.empty\n  %c: i32 = array.pop %a\n  release %a\n  ret %c\n}\n"),
        ("pop_empty_str", "export func @f() -> i32 {\nentry:\n  %a: [str] = array.empty\n  %c: str = array.pop %a\n  %n: i32 = str.len %c\n  release %c\n  release %a\n  ret %n\n}\n"),
        ("byte_oob", "export func @f() -> i32 {\nentry:\n  %s: str = str.const @d\n  %i: i32 = const 5\n  %b: u8 = str.byte %s, %i\n  %r: i32 = conv %b\n  ret %r\n}\n"),
        ("byte_neg", "export func @f() -> i32 {\nentry:\n  %s: str = str.const @d\n  %i: i32 = const -1\n  %b: u8 = str.byte %s, %i\n  %r: i32 = conv %b\n  ret %r\n}\n"),
    ];
    for (name, body) in cases {
        let src = format!("oxiir 1\nmodule \"m\"\ndata @d = \"abc\"\n{body}");
        let m = match parse_module(&src) { Ok(m) => m, Err(e) => { eprintln!("{name}: parse {e}"); continue; } };
        if let Err(e) = verify_ok(&m) { eprintln!("{name}: verify {e}"); continue; }
        let wasm = match oxiscript_wasm::compile(&m, &oxiscript_wasm::CompileOptions::default()) { Ok(w) => w, Err(e) => { eprintln!("{name}: codegen {e}"); continue; } };
        let host = Host::new(Limits::default());
        match host.instantiate(&wasm, Grants::all()) {
            Ok(mut rt) => {
                let r = rt.call_export("f", &[]);
                eprintln!("{name}: {:?}", r.as_ref().map(|v| format!("{v:?}")).map_err(|e| e.to_string()));
                assert!(r.is_err(), "{name}: out-of-range access produced a value instead of trapping");
            }
            Err(e) => eprintln!("{name}: instantiate {e}"),
        }
    }
}
