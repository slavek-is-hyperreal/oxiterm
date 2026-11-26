#![allow(clippy::format_push_string)]
//! Programs TypeScript accepts (or would accept with `any`/assertions) that are
//! unsound; the checker must reject every one of them.
mod common;
use common::*;

const SHAPES: &str = "type Shape = { kind: \"circle\"; r: f64 } | { kind: \"rect\"; w: f64 };\nstruct Node { value: i32; next: Node | null; }\n";

fn with_shapes(body: &str) -> String {
    format!("{SHAPES}{body}")
}

#[test]
fn any_and_assertions_do_not_exist() {
    // `any`, `as any`, `unknown`, `undefined`, `typeof` are reserved words (syntax error E0900)
    for src in [
        "function f(x: any): void {}",
        "function f(x: unknown): void {}",
        "function f(): void { const a = undefined; }",
        "function f(x: i32): void { const t = typeof x; }",
    ] {
        let c = codes(src);
        assert!(c.iter().any(|(c, _)| c == "OXS-E0900"), "{src}: {c:?}");
    }
}

#[test]
fn unsound_casts_are_rejected() {
    let cases: Vec<(&str, String)> = vec![
        ("assert_union_to_variant", with_shapes("function f(s: Shape): f64 {\n  const c = s as { kind: \"circle\"; r: f64 };\n  return 0.0;\n}")),
        ("string_to_number", "function f(s: string): i32 {\n  return s as i32; //! E0111\n}".into()),
        ("number_to_bool", "function f(n: i32): bool {\n  return n as bool; //! E0111\n}".into()),
        ("struct_to_struct", "struct A { x: i32; }\nstruct B { x: i32; }\nfunction f(a: A): B {\n  return a as B; //! E0111\n}".into()),
        ("nullable_to_non_null", "function f(n: i32 | null): i32 {\n  return n as i32; //! E0111\n}".into()),
    ];
    let mut bad = String::new();
    for (name, src) in &cases {
        let c = codes(src);
        if c.is_empty() {
            bad.push_str(&format!("{name}: accepted\n"));
        }
    }
    assert_eq!(bad.len(), 0, "{bad}");
    run_cases(&[(
        "string_to_number",
        "function f(s: string): i32 {\n  return s as i32; //! E0111\n}",
    )]);
}

#[test]
fn missing_narrowing_is_rejected() {
    let cases = [
        ("union_field", "type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 };\nfunction f(s: S): i32 {\n  return s.x; //! E0108\n}"),
        ("null_member", "struct N { v: i32; }\nfunction f(n: N | null): i32 {\n  return n.v; //! E0105\n}"),
        ("null_call", "function f(g: (() => i32) | null): i32 {\n  return g(); //! E0105\n}"),
        ("null_index", "function f(a: i32[] | null): i32 {\n  return a[0]; //! E0105\n}"),
        ("null_arith", "function f(a: i32 | null): i32 {\n  return a * 2; //! E0105\n}"),
        ("null_to_param", "function g(a: i32): i32 { return a; }\nfunction f(a: i32 | null): i32 {\n  return g(a); //! E0100\n}"),
        ("wrong_branch", "type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 };\nfunction f(s: S): i32 {\n  if (s.kind === \"a\") {\n    return s.y; //! E0108\n  }\n  return 0;\n}"),
        ("else_branch", "type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 };\nfunction f(s: S): i32 {\n  if (s.kind === \"a\") {\n    return s.x;\n  } else {\n    return s.x; //! E0108\n  }\n}"),
        ("after_merge", "type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 };\nfunction f(s: S, c: bool): i32 {\n  if (c) {\n    if (s.kind !== \"a\") { return 0; }\n  }\n  return s.x; //! E0108\n}"),
        ("or_condition", "function f(a: i32 | null, b: i32 | null): i32 {\n  if (a !== null || b !== null) {\n    return a + 1; //! E0105\n  }\n  return 0;\n}"),
    ];
    run_cases(&cases);
}

#[test]
fn narrowing_is_invalidated_soundly() {
    let cases = [
        (
            "assignment",
            "struct N { v: i32; }\nfunction f(n: N | null): i32 {\n  if (n !== null) {\n    n = null;\n    return n.v; //! E0105\n  }\n  return 0;\n}",
        ),
        (
            "assigned_in_closure",
            "struct N { v: i32; }\nfunction f(n: N | null): i32 {\n  if (n !== null) {\n    const clear = () => { n = null; };\n    clear();\n    return n.v; //! E0105\n  }\n  return 0;\n}",
        ),
        (
            "assigned_in_loop",
            "struct N { v: i32; next: N | null; }\nfunction f(n: N | null, c: bool): i32 {\n  if (n !== null) {\n    while (c) {\n      const x = n.v; //! E0105\n      n = n.next; //! E0105\n    }\n  }\n  return 0;\n}",
        ),
        (
            "class_field_not_narrowed",
            "class C {\n  next: C | null = null;\n  clear(): void { this.next = null; }\n  f(): i32 {\n    if (this.next !== null) {\n      this.clear();\n      return this.next.f(); //! E0105\n    }\n    return 0;\n  }\n}",
        ),
        (
            "module_let_not_narrowed",
            "struct N { v: i32; }\nlet g: N | null = null;\nfunction clear(): void { g = null; }\nfunction f(): i32 {\n  if (g !== null) {\n    clear();\n    return g.v; //! E0105\n  }\n  return 0;\n}",
        ),
        (
            "mutating_call_kills_field_facts",
            "struct N { v: i32; }\nstruct W {\n  n: N | null;\n  mutating drop(): void { this.n = null; }\n}\nfunction f(w: W): i32 {\n  if (w.n !== null) {\n    w.drop();\n    return w.n.v; //! E0105\n  }\n  return 0;\n}",
        ),
        (
            "array_element_not_narrowed",
            "struct N { v: i32; }\nfunction f(xs: (N | null)[]): i32 {\n  if (xs[0] !== null) {\n    return xs[0].v; //! E0105\n  }\n  return 0;\n}",
        ),
    ];
    run_cases(&cases);
}

#[test]
fn struct_value_semantics_and_aliasing() {
    let cases = [
        ("const_binding_field", "struct P { x: i32; }\nfunction f(): void {\n  const a: P = { x: 1 };\n  a.x = 2; //! E0109\n}"),
        ("const_alias", "struct P { x: i32; }\nfunction f(): void {\n  const a: P = { x: 1 };\n  const b = a;\n  b.x = 2; //! E0109\n}"),
        ("for_of_const_copy", "struct P { x: i32; }\nfunction f(xs: P[]): void {\n  for (const p of xs) {\n    p.x += 1; //! E0109\n  }\n}"),
        ("rvalue_field_assign", "struct P { x: i32; }\nfunction mk(): P { return { x: 1 }; }\nfunction f(): void {\n  mk().x = 2; //! E0109\n}"),
        ("readonly_struct_field", "struct P { readonly x: i32; }\nfunction f(): void {\n  let a: P = { x: 1 };\n  a.x = 2; //! E0109\n}"),
        ("mutating_on_param_ok_but_on_rvalue_not", "struct V { x: i32; mutating bump(): void { this.x += 1; } }\nfunction mk(): V { return { x: 1 }; }\nfunction f(): void {\n  mk().bump(); //! E0109\n}"),
    ];
    run_cases(&cases);
    // a copy is independent: this is fine
    assert_ok("struct P { x: i32; }\nfunction f(): i32 {\n  const a: P = { x: 1 };\n  let b = a;\n  b.x = 2;\n  return a.x + b.x;\n}");
}

#[test]
fn variance_is_sound() {
    let cases = [
        (
            "arrays_are_invariant",
            "struct P { x: i32; }\nfunction h(a: P[]): void {\n  const b: (P | null)[] = a; //! E0100\n}",
        ),
        (
            "array_of_variants_is_not_array_of_union",
            "type A = { kind: \"a\" }; type B = { kind: \"b\" }; type S = A | B;\nfunction h(xs: A[]): void {\n  const ys: S[] = xs; //! E0100\n}",
        ),
        (
            "function_result_widening_changes_representation",
            "function h(): void {\n  const a: () => i32 = () => 1;\n  const b: () => i32 | null = a; //! E0100\n}",
        ),
        (
            "function_parameter_must_be_contravariant",
            "type A = { kind: \"a\" }; type B = { kind: \"b\" }; type S = A | B;\nfunction k(f: (s: A) => void): void {\n  const g: (s: S) => void = f; //! E0100\n}",
        ),
    ];
    run_cases(&cases);
    // the sound direction is accepted
    assert_ok("type A = { kind: \"a\" }; type B = { kind: \"b\" }; type S = A | B;\nfunction h(f: (s: S) => void): void {\n  const g: (s: A) => void = f;\n}");
}

#[test]
fn generics_cannot_be_abused() {
    let cases = [
        ("no_equality_on_t", "function f<T>(a: T, b: T): bool {\n  return a === b; //! E0110\n}"),
        ("no_arithmetic_on_t", "function f<T>(a: T, b: T): T {\n  return a + b; //! E0110\n}"),
        ("no_members_on_t", "function f<T>(a: T): i32 {\n  return a.x; //! E0108\n}"),
        ("bound_violation", "interface I { m(): i32; }\nstruct S { x: i32; }\nfunction f<T extends I>(a: T): i32 { return a.m(); }\nfunction g(s: S): i32 {\n  return f(s); //! E0117\n}"),
        ("bound_signature_mismatch", "interface I { m(): i32; }\nstruct S { x: i32; m(): string { return \"\"; } }\nfunction f<T extends I>(a: T): i32 { return a.m(); }\nfunction g(s: S): i32 {\n  return f(s); //! E0117\n}"),
        ("returning_wrong_t", "function f<T, U>(a: T, b: U): T {\n  return b; //! E0100\n}"),
    ];
    run_cases(&cases);
}

#[test]
fn definite_assignment_and_capabilities() {
    let cases = [
        ("conditional_init", "function f(c: bool): i32 {\n  let x: i32;\n  if (c) { x = 1; }\n  return x; //! E0118\n}"),
        ("loop_init", "function f(c: bool): i32 {\n  let x: i32;\n  while (c) { x = 1; }\n  return x; //! E0118\n}"),
        ("closure_before_init", "function f(): void {\n  let x: i32;\n  const g = () => x + 1; //! E0118\n}"),
        ("forged_capability_literal", "import { Log } from \"host:log\";\nfunction f(): Log {\n  return { x: 1 }; //! E0120\n}"),
        ("forged_capability_cast", "import { Log } from \"host:log\";\nfunction f(): Log {\n  return 1 as Log; //! E0111\n}"),
        ("forged_handle", "import { State, Slot } from \"host:state\";\nfunction f(s: State): void {\n  const h: Slot = 1; //! E0100\n}"),
        ("capability_in_law", "import { Log } from \"host:log\";\nlaw l(log: Log) { true } //! E0125"),
    ];
    run_cases(&cases);
}

#[test]
fn constants_and_integers() {
    let cases = [
        ("i8_overflow_literal", "function f(): void {\n  const a: i8 = 128; //! E0102\n}"),
        ("u32_vs_i32", "function f(a: u32, b: i32): u32 {\n  return a + b; //! E0110\n}"),
        ("float_to_int_param", "function g(a: i32): void {}\nfunction f(): void {\n  g(1.0); //! E0100\n}"),
        ("int_div_by_float", "function f(a: i32): i32 {\n  return a / 2.0; //! E0110\n}"),
        ("bit_ops_on_float", "function f(a: f64): f64 {\n  return a & a; //! E0110\n}"),
        ("wrapping_op_on_float", "function f(a: f64): f64 {\n  return a +% a; //! E0110\n}"),
    ];
    run_cases(&cases);
}

#[test]
fn unions_must_be_discriminated() {
    let cases = [
        ("string_or_number", "function f(a: string | i32): void {} //! E0115"),
        ("missing_discriminant", "type S = { x: i32 } | { y: i32 }; //! E0116 E0116"),
        ("mixed_discriminant_kinds", "type S = { kind: \"a\" } | { kind: 1 }; //! E0116"),
        ("duplicate_discriminant", "type S = { kind: \"a\" } | { kind: \"a\" }; //! E0116"),
        ("union_with_primitive", "type S = { kind: \"a\" } | i32; //! E0115"),
    ];
    run_cases(&cases);
}

#[test]
fn host_result_values_are_typed() {
    // `result<T>` of the IDL is `Result<T, i32>`: it must not be silently accepted as anything else
    let pre = "import { State } from \"host:state\";\nexport function main(st: State): void {\n";
    assert_ok(&format!("{pre}  const r = st.open(\"k\");\n  switch (r.kind) {{ case \"ok\": st.set_i64(r.value, 1); case \"err\": {{}} }}\n}}\n"));
    let bad = codes(&format!("{pre}  const r: i32 = st.open(\"k\");\n}}\n"));
    assert!(bad.iter().any(|(c, _)| c == "OXS-E0100"), "{bad:?}");
    let bad = codes(&format!("{pre}  const r = st.open(\"k\");\n  st.set_i64(r, 1);\n}}\n"));
    assert_ne!(bad.len(), 0, "a Result is not a handle");
}
