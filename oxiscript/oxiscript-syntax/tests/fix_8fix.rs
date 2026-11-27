//! Stage 8.Fix regression tests for oxiscript-syntax:
//! - Reserved verification keywords `decreases`, `ghost`, `pure` ([R3-169])
//! - Constructor modifiers preserved on AST (`static constructor`, etc.) ([R3-53, R2-106])
//! - `throw`/`try`/`catch` and continuation-hazard `OXS-E0004` directly in parser ([R2-122, R2-121])
//! - Formatter F8 comment re-attachment (`[1, // one\n 2, // two\n]`, `a /* b */ + c`) ([R2-133])
//! - Lexer/parser edge cases (`1e999`, `\u{110000}`, `\u{d800}`, 10^6-digit literal, huge ident) ([R3-207, R3-208])

use oxiscript_syntax::ast::{ItemKind, MemberKind};
use oxiscript_syntax::{format, parse_module};

#[test]
fn reserved_verification_keywords_rejected_with_e0900() {
    for kw in ["decreases", "ghost", "pure"] {
        let src = format!("const {kw}: i32 = 1;\n");
        let (_, diags) = parse_module(&src);
        assert!(
            diags.iter().any(|d| d.code == "OXS-E0900"),
            "expected OXS-E0900 for `{kw}`, got: {diags:?}"
        );
    }
}

#[test]
fn constructor_preserves_static_and_mutating_modifiers_in_ast() {
    let src = "class C {\n  static constructor() {}\n}\n";
    let (m, diags) = parse_module(src);
    assert!(diags.is_empty(), "parse diags: {diags:?}");
    let ItemKind::Record(r) = &m.items[0].kind else {
        panic!("expected record");
    };
    let MemberKind::Constructor(f) = &r.members[0].kind else {
        panic!("expected constructor");
    };
    assert!(f.mods.is_static, "static modifier must be preserved on constructor");
    // Formatting preserves the modifier too (round-trip safe).
    let formatted = format(src).expect("format ok");
    assert!(formatted.contains("static constructor"), "formatted: {formatted:?}");
}

#[test]
fn throw_try_catch_diagnosed_without_semicolon_fix() {
    for src in [
        "function f(): void { throw \"boom\" }\n",
        "function f(): void { try { let x: i32 = 1; } catch (e) {} }\n",
    ] {
        let (_, diags) = parse_module(src);
        let e4: Vec<_> = diags.iter().filter(|d| d.code == "OXS-E0004").collect();
        assert!(!e4.is_empty(), "expected OXS-E0004 for {src:?}, got {diags:?}");
        for d in &e4 {
            assert!(
                d.message.contains("has no exceptions"),
                "expected exception hint in parser diagnostic: {d:?}"
            );
            assert!(
                d.fixes.is_empty(),
                "must not suggest bogus `;` insert fix on exception syntax: {d:?}"
            );
        }
    }
}

#[test]
fn continuation_hazard_points_to_previous_line_without_semicolon_fix() {
    let src = "const c: i32 = 1\n[1]\n";
    let (_, diags) = parse_module(src);
    let d = diags
        .iter()
        .find(|d| d.code == "OXS-E0004")
        .expect("expected OXS-E0004");
    let line1_end = "const c: i32 = 1".len() as u32;
    assert_eq!(
        d.span.lo, line1_end,
        "continuation hazard should point at end of previous line ({line1_end}), got: {d:?}"
    );
    assert!(
        d.fixes.is_empty(),
        "continuation hazard must not attach a machine-applicable `;` fix: {d:?}"
    );
}

#[test]
fn formatter_f8_binary_and_array_comments_stay_attached_and_idempotent() {
    let bin = "const x: i32 = a /* b */ + c;\n";
    let out_bin = format(bin).expect("format binary");
    let c_pos = out_bin.find("/* b */").expect("comment kept");
    let plus_pos = out_bin.find('+').expect("+ kept");
    assert!(
        c_pos < plus_pos,
        "block comment before `+` must stay before `+`: {out_bin:?}"
    );
    assert_eq!(format(&out_bin).unwrap(), out_bin, "binary format not idempotent");

    let arr = "const xs = [1, // one\n  2, // two\n];\n";
    let out_arr = format(arr).expect("format array");
    let two_pos = out_arr.find("// two").expect("// two kept");
    let close_pos = out_arr.rfind(']').expect("] kept");
    let semi_pos = out_arr.rfind(';').expect("; kept");
    assert!(
        two_pos < close_pos && close_pos < semi_pos,
        "trailing comment inside array must stay before `]`: {out_arr:?}"
    );
    assert_eq!(format(&out_arr).unwrap(), out_arr, "array format not idempotent");

    let obj = "const o = {\n  /* k */ a: 1, // one\n  b: 2, // two\n};\n";
    let out_obj = format(obj).expect("format object");
    assert!(out_obj.find("/* k */").unwrap() < out_obj.find("a:").unwrap(), "{out_obj:?}");
    assert!(out_obj.find("// two").unwrap() < out_obj.rfind('}').unwrap(), "{out_obj:?}");
    assert_eq!(format(&out_obj).unwrap(), out_obj, "object format not idempotent");
}

#[test]
fn lexer_edge_cases_do_not_panic_and_reject_invalid_literals() {
    // Out-of-range unicode escape and lone surrogate -> OXS-E0013
    for bad in ["const a = \"\\u{110000}\";\n", "const b = \"\\u{d800}\";\n"] {
        let (_, diags) = parse_module(bad);
        assert!(
            diags.iter().any(|d| d.code == "OXS-E0013"),
            "expected OXS-E0013 for {bad:?}, got {diags:?}"
        );
    }

    // Float overflow literal `1e999` lexes cleanly (checked in oxiscript-check).
    let (_, diags) = parse_module("const x: f64 = 1e999;\n");
    assert!(diags.is_empty(), "1e999 lex/parse diags: {diags:?}");

    // 10^6-digit integer literal overflows u128 cleanly with OXS-E0003 and no panic.
    let big_int = format!("const n: i32 = {};\n", "9".repeat(1_000_000));
    let (_, diags) = parse_module(&big_int);
    assert!(
        diags.iter().any(|d| d.code == "OXS-E0003"),
        "expected OXS-E0003 for 10^6-digit int, got {diags:?}"
    );

    // Very long identifier parses cleanly without panic.
    let big_ident = format!("const {}: i32 = 1;\n", "a".repeat(100_000));
    let (_, diags) = parse_module(&big_ident);
    assert!(diags.is_empty(), "big ident diags: {diags:?}");
}
