#![allow(
    clippy::doc_markdown,
    clippy::format_push_string,
    clippy::too_many_lines
)]
//! BAD ACTOR tests for slice 8.C (OxiIR). Each test FAILS today; see the report.
use oxiscript_ir::*;
use std::panic::catch_unwind;

const HDR: &str = "oxiir 1\nmodule \"m\"\n";
fn p(body: &str) -> Result<Module, ParseError> {
    parse_module(&format!("{HDR}{body}"))
}
fn pv(body: &str) -> Result<(), String> {
    let m = p(body).map_err(|e| format!("parse: {e}"))?;
    verify_ok(&m).map_err(|e| format!("verify: {e}"))
}

#[test]
fn ba01_documented_effects_attr_syntax_parses() {
    // ir.md section 1: #effects(alloc, panic, loops, recursion, host:log, ...)
    p("func @f() -> void #effects(alloc, host:log) {\nentry:\n  ret\n}\n")
        .expect("documented syntax rejected");
}

#[test]
fn ba02_verified_module_must_print_to_parseable_text() {
    let mut m = p("func @f() -> i32 {\nentry:\n  br b2()\nb2:\n  %x: i32 = const 1\n  br b1()\nb1:\n  ret %x\n}\n").unwrap();
    let f = &mut m.funcs[0];
    f.blocks.swap(1, 2);
    if let Terminator::Br(t) = &mut f.blocks[0].term {
        t.block = BlockId(2);
    }
    if let Terminator::Br(t) = &mut f.blocks[2].term {
        t.block = BlockId(1);
    }
    verify_ok(&m).expect("dominance-valid module");
    parse_module(&print_module(&m))
        .expect("verified module prints to text the parser rejects (non-RPO block order)");
}

#[test]
fn ba03_verify_must_not_panic_on_bad_value_id() {
    let mut m = p("func @f() -> i32 {\nentry:\n  %x: i32 = const 1\n  ret %x\n}\n").unwrap();
    m.funcs[0].blocks[0].term = Terminator::Ret(vec![ValueId(999)]);
    let r = catch_unwind(|| verify(&m));
    assert!(r.is_ok(), "verify panicked");
}

#[test]
fn ba04_bitcast_ptr_f32_not_allowed() {
    assert!(
        pv("func @f(%a: ptr) -> f32 {\nentry:\n  %b: f32 = bitcast %a\n  ret %b\n}\n").is_err()
    );
}

#[test]
fn ba05_forging_ptr_outside_raw_memory() {
    // ir.md: `ptr` is runtime modules only
    assert!(
        pv("func @f(%a: i32) -> ptr {\nentry:\n  %p: ptr = bitcast %a\n  ret %p\n}\n").is_err()
    );
}

#[test]
fn ba06_closure_new_may_trap() {
    let op = Op::ClosureNew {
        env: "E".into(),
        func: "f".into(),
        captures: vec![],
    };
    assert!(
        op.may_trap(),
        "closure.new allocates (out_of_memory) but may_trap() is false"
    );
}

#[test]
fn ba07_reserved_export_name_rejected() {
    for n in ["memory", "oxi_release", "cb:X.Y", "oxi_init"] {
        let s = format!("export func @{n}() -> void {{\nentry:\n  ret\n}}\n");
        // `cb:` cannot lex as a name; only the lexable ones matter
        if let Ok(m) = p(&s) {
            assert!(verify_ok(&m).is_err(), "export @{n} accepted");
        }
    }
}

#[test]
fn ba08_export_name_attr_reserved() {
    assert!(pv("export func @g() -> void #export_name(memory) {\nentry:\n  ret\n}\n").is_err());
}

#[test]
fn ba09_nan_payload_truncation() {
    assert!(
        p("global @g: f32 = nan:0x1ffffffff\n").is_err(),
        "f32 nan bits silently truncated"
    );
}

#[test]
fn ba10_nan_literal_must_be_nan() {
    assert!(
        p("global @g: f32 = nan:0x3f800000\n").is_err(),
        "nan: accepts non-NaN bits (1.0)"
    );
}

#[test]
fn ba11_dotted_names_round_trip() {
    pv("type A.B = struct { f: i32 }\nfunc @g(%o: ref A.B) -> i32 {\nentry:\n  %v: i32 = getfield A.B.f %o\n  ret %v\n}\n")
        .expect("parser accepts `type A.B` but getfield A.B.f splits at the first dot");
}

#[test]
fn ba12_unprintable_names_rejected_by_verify() {
    let mut m = p("func @f(%a: i32) -> i32 {\nentry:\n  ret %a\n}\n").unwrap();
    m.funcs[0].values[0].name = "a b".into();
    assert!(
        verify_ok(&m).is_err() || parse_module(&print_module(&m)).as_ref() == Ok(&m),
        "verified module whose names do not survive print/parse"
    );
}

#[test]
fn ba13_own_on_scalar_param() {
    assert!(pv("func @f(%a: own i32) -> void {\nentry:\n  ret\n}\n").is_err());
}

#[test]
fn ba14_acyclic_on_cyclic_type() {
    assert!(
        pv("type A #acyclic = struct { a: ref A? }\n").is_err(),
        "#acyclic on a self-referential type -> green leak"
    );
    assert!(
        pv("type B #acyclic #cyclic = struct { }\n").is_err(),
        "contradictory colour hints"
    );
}

#[test]
fn ba15_switch_case_must_fit_scrutinee() {
    assert!(pv("func @f(%a: u8) -> void {\nentry:\n  switch %a, [300: e(), -1: e()], default e()\ne:\n  ret\n}\n").is_err());
}

#[test]
fn ba16_callback_closure_cannot_reach_host_import() {
    // abi.md 5.1: callback params are an i32 closure pointer passed to the host.
    pv("import @watch = \"host:state@1\" \"watch\" (i32, i32, fn() -> void) -> void\n")
        .expect("IR has no way to hand a closure to a host import");
}

#[test]
fn ba17_is_unique_on_nullable() {
    // null is only defined for retain/release; is_unique(null) would read address -8.
    assert!(
        pv("func @f(%a: str?) -> bool {\nentry:\n  %b: bool = is_unique %a\n  ret %b\n}\n")
            .is_err()
    );
}

#[test]
fn ba18_import_module_name_checked() {
    assert!(
        pv("import @e = \"env\" \"x\" () -> void\n").is_err(),
        "abi.md 3.2: no env/WASI imports"
    );
}

#[test]
fn ba19_deep_type_nesting() {
    let n = 400_000;
    let s = format!("global @g: {}i32{}\n", "[".repeat(n), "]".repeat(n));
    let _ = p(&s);
}

#[test]
fn ba20_many_blocks_quadratic() {
    let n = 30_000;
    let mut s = String::from("func @f() -> void {\n");
    for i in 0..n {
        s.push_str(&format!("b{i}:\n  br b{}()\n", i + 1));
    }
    s.push_str(&format!("b{n}:\n  ret\n}}\n"));
    let m = p(&s).unwrap();
    let _ = verify(&m);
}
