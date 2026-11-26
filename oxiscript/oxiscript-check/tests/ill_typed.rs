#![allow(clippy::format_push_string)]
//! Ill-typed programs with expected diagnostics. A line that ends in `//! E0100 E0110`
//! must produce exactly those codes (`OXS-` implied); lines without a marker produce none.
mod common;
use common::*;
use std::collections::BTreeMap;

fn expected(src: &str) -> Vec<(String, u32)> {
    let mut v = Vec::new();
    for (i, l) in src.lines().enumerate() {
        if let Some(at) = l.find("//! ") {
            for c in l[at + 4..].split_whitespace() {
                v.push((format!("OXS-{c}"), (i + 1) as u32));
            }
        }
    }
    v.sort();
    v
}

const CASES: &[(&str, &str)] = &[
    ("assign_string_to_int", r#"function f(): void {
  let x: i32 = "a"; //! E0100
}"#),
    ("return_type_mismatch", r#"function f(): i32 {
  return "a"; //! E0100
}"#),
    ("missing_return_on_a_path", r"function f(a: bool): i32 { //! E0114
  if (a) { return 1; }
}"),
    ("unknown_name", r"function f(): i32 {
  return y; //! E0300
}"),
    ("duplicate_function", r"function f(): void {}
function f(): void {} //! E0301"),
    ("duplicate_local_in_scope", r"function f(): void {
  let a = 1;
  let a = 2; //! E0301
}"),
    ("mixed_numeric_types", r"function f(a: i32, b: i64): i64 {
  return a + b; //! E0110
}"),
    ("literal_out_of_range", r"function f(): void {
  let x: u8 = 256; //! E0102
}"),
    ("negative_unsigned_literal", r"function f(): void {
  let x: u8 = -1; //! E0128
}"),
    ("float_literal_to_int", r"function f(): void {
  let x: i32 = 1.5; //! E0100
}"),
    ("string_plus_number", r#"function f(): string {
  return "a" + 1; //! E0110
}"#),
    ("non_bool_condition", r"function f(): void {
  if (1) { return; } //! E0100
}"),
    ("too_many_arguments", r"function g(a: i32): i32 { return a; }
function f(): i32 {
  return g(1, 2); //! E0107
}"),
    ("too_few_arguments", r"function g(a: i32, b: i32): i32 { return a; }
function f(): i32 {
  return g(1); //! E0107
}"),
    ("wrong_argument_type", r#"function g(a: i32): i32 { return a; }
function f(): i32 {
  return g("x"); //! E0100
}"#),
    ("unknown_field", r"struct P { x: i32; }
function f(p: P): i32 {
  return p.z; //! E0108
}"),
    ("possibly_null_member", r"struct Node { value: i32; }
function f(n: Node | null): i32 {
  return n.value; //! E0105
}"),
    ("assign_to_const", r"function f(): void {
  const x = 1;
  x = 2; //! E0109
}"),
    ("const_struct_field_assignment", r"struct P { x: i32; }
function f(): void {
  const p: P = { x: 1 };
  p.x = 2; //! E0109
}"),
    ("switch_not_exhaustive_union", r#"type S = { kind: "a"; x: i32 } | { kind: "b" } | { kind: "c" };
function f(s: S): i32 {
  switch (s.kind) { //! E0101
    case "a": return 1;
    case "b": return 2;
  }
  return 0;
}"#),
    ("switch_not_exhaustive_enum", r"enum Color { Red, Green, Blue }
function f(c: Color): i32 {
  switch (c) { //! E0101
    case Color.Red: return 1;
    case Color.Green: return 2;
  }
  return 0;
}"),
    ("switch_expression_needs_default", r"function f(n: i32): i32 {
  return switch (n) { //! E0101
    case 1: 10,
    case 2: 20,
  };
}"),
    ("duplicate_case_label", r#"type S = { kind: "a" } | { kind: "b" };
function f(s: S): i32 {
  switch (s.kind) {
    case "a": return 1;
    case "a": return 2; //! E0116
    case "b": return 3;
  }
}"#),
    ("unknown_discriminant_label", r#"type S = { kind: "a" } | { kind: "b" };
function f(s: S): i32 {
  switch (s.kind) {
    case "a": return 1;
    case "z": return 2; //! E0116
    case "b": return 3;
  }
}"#),
    ("union_field_without_narrowing", r#"type S = { kind: "c"; r: f64 } | { kind: "d"; w: f64 };
function f(s: S): f64 {
  return s.r; //! E0108
}"#),
    ("try_outside_result_function", r"function g(): Result<i32, string> { return Ok(1); }
function f(): i32 {
  const v = g()?; //! E0104
  return v;
}"),
    ("try_on_non_result", r"function f(): Result<i32, string> {
  const v = 1?; //! E0100
  return Ok(v);
}"),
    ("try_with_different_error_type", r"function g(): Result<i32, string> { return Ok(1); }
function f(): Result<i32, i32> {
  const v = g()?; //! E0104
  return Ok(v);
}"),
    ("optional_chain_on_result", r"struct R { v: i32; }
function g(): Result<R, string> { return Ok({ v: 1 }); }
function f(): Result<i32, string> {
  const v = g()?.v; //! E0106
  return Ok(1);
}"),
    ("optional_chain_on_non_null", r"struct P { x: i32; }
function f(p: P): i32 {
  return p?.x ?? 0; //! E0127
}"),
    ("coalesce_on_non_null", r"function f(a: i32): i32 {
  return a ?? 1; //! E0127
}"),
    ("break_outside_loop", r"function f(): void {
  break; //! E0122
}"),
    ("unknown_label", r"function f(): void {
  while (true) {
    break nowhere; //! E0122
  }
}"),
    ("use_before_assignment", r"function f(): i32 {
  let x: i32;
  return x; //! E0118
}"),
    ("compound_assignment_before_assignment", r"function f(): i32 {
  let x: i32;
  x += 1; //! E0118
  return 0;
}"),
    ("call_a_number", r"function f(): void {
  const x = 1;
  x(); //! E0112
}"),
    ("cast_string_to_number", r#"function f(): i32 {
  return "a" as i32; //! E0111
}"#),
    ("cast_struct", r"struct P { x: i32; }
function f(p: P): i32 {
  return p as i32; //! E0111
}"),
    ("new_on_struct", r"struct P { x: i32; }
function f(): P {
  return new P(); //! E0119
}"),
    ("new_on_host_capability", r#"import { Log } from "host:log";
function f(): Log {
  return new Log(); //! E0119
}"#),
    ("equality_on_structs", r"struct P { x: i32; }
function f(a: P, b: P): bool {
  return a === b; //! E0110
}"),
    ("equality_on_arrays", r"function f(a: i32[], b: i32[]): bool {
  return a === b; //! E0110
}"),
    ("equality_mixed_numbers", r"function f(a: i32, b: i64): bool {
  return a === b; //! E0110
}"),
    ("object_literal_missing_field", r"struct P { x: i32; y: i32; }
function f(): P {
  return { x: 1 }; //! E0120
}"),
    ("object_literal_extra_field", r"struct P { x: i32; }
function f(): P {
  return { x: 1, z: 2 }; //! E0120
}"),
    ("object_literal_without_context", r"function f(): void {
  const o = { x: 1 }; //! E0103
}"),
    ("empty_array_without_context", r"function f(): void {
  const a = []; //! E0103
}"),
    ("lambda_parameter_without_type", r"function f(): void {
  const g = (x) => x; //! E0103
}"),
    ("interface_as_value_type", r"interface Shape { area(): f64; }
function f(s: Shape): f64 { //! E0115
  return 0.0;
}"),
    ("wrong_type_argument_count", r"function f(a: Array<i32, i32>): void {} //! E0117"),
    ("bound_not_satisfied", r"interface Shape { area(): f64; }
function total<T extends Shape>(x: T): f64 { return x.area(); }
function f(): f64 {
  return total(1); //! E0117
}"),
    ("type_argument_cannot_be_inferred", r"function mk<T>(): T[] { return []; }
function f(): void {
  const a = mk(); //! E0103
}"),
    ("unknown_attribute", r"@nosuch //! E0124
function f(): void {}"),
    ("allow_with_unregistered_code", r#"@allow("OXS-P9999", "typo") //! E0123
function f(): void {}"#),
    ("exported_function_without_return_type", r"export function f(a: i32) { //! E0103
  return a;
}"),
    ("exported_function_with_two_strings", r"@host export function f(a: string, b: string): void {} //! E0121"),
    ("main_with_non_capability_parameter", r"export function main(x: i32): void {} //! E0121"),
    ("main_not_exported", r"function main(): void {} //! E0121"),
    ("main_returning_a_value", r"export function main(): i32 { return 1; } //! E0121"),
    ("private_member_access", r"class C {
  private n: i32 = 0;
}
function f(c: C): i32 {
  return c.n; //! E0119
}"),
    ("readonly_field_assignment", r"class C {
  readonly n: i32 = 0;
}
function f(c: C): void {
  c.n = 1; //! E0109
}"),
    ("mutating_call_on_const_struct", r"struct V {
  x: i32;
  mutating bump(): void { this.x += 1; }
}
function f(): void {
  const v: V = { x: 1 };
  v.bump(); //! E0109
}"),
    ("this_assignment_without_mutating", r"struct V {
  x: i32;
  bump(): void { this.x = 1; } //! E0119
}"),
    ("unknown_host_module", r#"import { Net } from "host:net"; //! E0400"#),
    ("unknown_host_interface", r#"import { Nope } from "host:log"; //! E0401"#),
    ("law_body_not_bool", r"law l(a: i32) { a + 1 } //! E0125"),
    ("law_uses_capability", r#"import { Log } from "host:log";
law l(log: Log) { log.write(1, "x"); true } //! E0125 E0125"#),
    ("duplicate_struct_field", r"struct P { x: i32; x: i32; } //! E0301"),
    ("for_of_over_number", r"function f(): void {
  for (const x of 5) {} //! E0128
}"),
    ("negate_bool", r"function f(a: bool): bool {
  return -a; //! E0128
}"),
    ("not_on_integer", r"function f(a: i32): bool {
  return !a; //! E0128
}"),
    ("index_with_wrong_type", r"function f(a: i32[]): i32 {
  const i: i64 = 1;
  return a[i]; //! E0100
}"),
    ("string_index", r"function f(s: string): string {
  return s[0]; //! E0128
}"),
    ("variable_without_type_or_initializer", r"function f(): void {
  let x; //! E0103
}"),
    ("const_without_initializer", r"function f(): void {
  const x: i32; //! E0123
}"),
    ("template_with_struct", r"struct P { x: i32; }
function f(p: P): string {
  return `${p}`; //! E0100
}"),
    ("assign_to_union_member_field", r#"type S = { kind: "a"; x: i32 } | { kind: "b" };
function f(s: S): void {
  if (s.kind === "a") {
    s.x = 1; //! E0109
  }
}"#),
    ("assign_to_discriminant", r#"type S = { kind: "a" } | { kind: "b" };
function f(s: S): void {
  s.kind = "b"; //! E0109
}"#),
    ("class_constructor_leaves_field_unassigned", r"class C {
  n: i32;
  constructor() {} //! E0123
}"),
    ("this_used_before_fields_assigned", r"class C {
  n: i32;
  constructor() {
    this.touch(); //! E0119
    this.n = 1;
  }
  touch(): void {}
}"),
    ("circular_type_alias", r"type A = A | null; //! E0123"),
    ("plain_union_of_primitives", r"function f(a: i32 | string): void {} //! E0115"),
    ("enum_member_out_of_range", r"enum E: u8 { A = 300 } //! E0102"),
    ("shift_float", r"function f(a: f64): f64 {
  return a << 1; //! E0110
}"),
    ("switch_on_struct", r"struct P { x: i32; }
function f(p: P): void {
  switch (p) {} //! E0123
}"),
    ("nested_function_captures_local", r"function f(): i32 {
  const a = 1;
  function g(): i32 {
    return a; //! E0126
  }
  return g();
}"),
    ("closure_return_mismatch", r#"function f(): void {
  const g: (x: i32) => i32 = (x) => "s"; //! E0100
}"#),
    ("required_parameter_after_default", r"function f(a: i32 = 1, b: i32): i32 { return a; } //! E0123"),
    ("return_value_in_void_function", r"function f(): void {
  return 1; //! E0100
}"),
    ("bare_return_in_value_function", r"function f(): i32 {
  return; //! E0100
}"),
    ("expression_without_effect_warns", r"function f(a: i32): void {
  a + 1; //! W0004
}"),
    ("import_of_missing_module", r#"import { a } from "./nowhere"; //! E0302"#),
    ("generic_equality_is_not_available", r"function f<T>(a: T, b: T): bool {
  return a === b; //! E0110
}"),
    ("null_to_non_nullable", r"function f(): void {
  const s: string = null; //! E0100
}"),
    ("arithmetic_on_nullable", r"function f(a: i32 | null): i32 {
  return a + 1; //! E0105
}"),
    ("variant_construction_with_unknown_kind", r#"type S = { kind: "a" } | { kind: "b" };
function f(): S {
  return { kind: "nope" }; //! E0120
}"#),
    ("anonymous_object_type_outside_union", r#"type S = { kind: "a" } | { kind: "b" };
function f(xs: S[]): void {}
function g(ys: { kind: "a" }[]): void {} //! E0115"#),
];

#[test]
fn ill_typed_programs_report_the_expected_diagnostics() {
    assert!(CASES.len() >= 60, "need at least 60 ill-typed programs, have {}", CASES.len());
    let mut names = std::collections::HashSet::new();
    let mut bad = String::new();
    let mut seen_codes: BTreeMap<String, usize> = BTreeMap::new();
    for (name, src) in CASES {
        assert!(names.insert(*name), "duplicate case name {name}");
        let want = expected(src);
        assert_ne!(want.len(), 0, "{name}: no expected diagnostics");
        let got = codes(src);
        for (c, _) in &got {
            *seen_codes.entry(c.clone()).or_default() += 1;
        }
        if got != want {
            bad.push_str(&format!("--- {name}\nwant {want:?}\n got {got:?}\n{}\n", show(src)));
        }
    }
    assert_eq!(bad.len(), 0, "\n{bad}");
    assert!(seen_codes.len() >= 25, "the corpus should cover many codes: {seen_codes:?}");
}

#[test]
fn messages_are_complete_and_carry_fixes_where_mechanical() {
    for (name, src) in CASES {
        let c = oxiscript_check::check_source(src, &hosts());
        for d in &c.diagnostics {
            let m = &d.message;
            assert!(!m.contains("``"), "{name}: empty backticks in {m:?}");
            let bytes = m.as_bytes();
            // an unfilled `{placeholder}` is a lowercase identifier in braces
            for (i, b) in bytes.iter().enumerate() {
                if *b == b'{' {
                    let rest = &m[i + 1..];
                    if let Some(end) = rest.find('}') {
                        let inner = &rest[..end];
                        assert!(
                            inner.is_empty() || !inner.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                            "{name}: unfilled placeholder in {m:?}"
                        );
                    }
                }
            }
            assert_ne!(m.len(), 0, "{name}: empty message for {}", d.code);
        }
    }
    // mechanical fixes
    let src = "function f(a: i32, b: i64): i64 {\n  return a + b;\n}\n";
    let c = oxiscript_check::check_source(src, &hosts());
    let d = c.errors().next().unwrap();
    let fix = &d.fixes[0];
    let fixed = oxiscript_syntax::apply_edits(src, &fix.edits).unwrap();
    assert!(fixed.contains("(b as i32)"), "{fixed}");
    let src = "type S = { kind: \"a\" } | { kind: \"b\" };\nfunction f(s: S): i32 {\n  switch (s.kind) {\n    case \"a\": return 1;\n  }\n  return 0;\n}\n";
    let c = oxiscript_check::check_source(src, &hosts());
    let d = c.errors().next().unwrap();
    let fixed = oxiscript_syntax::apply_edits(src, &d.fixes[0].edits).unwrap();
    assert!(fixed.contains("case \"b\""), "{fixed}");
    assert!(oxiscript_check::check_source(&fixed, &hosts()).ok(), "{fixed}");
    let src = "function f(): void {\n  const x = 1;\n  x = 2;\n}\n";
    let c = oxiscript_check::check_source(src, &hosts());
    let fixed = oxiscript_syntax::apply_edits(src, &c.errors().next().unwrap().fixes[0].edits).unwrap();
    assert!(fixed.contains("let x = 1"), "{fixed}");
    assert!(oxiscript_check::check_source(&fixed, &hosts()).ok());
}
