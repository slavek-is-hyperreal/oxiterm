//! Stage 8.Fix regression tests for oxiscript-check:
//! - `static constructor` / `mutating constructor` -> OXS-E0123 ([R3-53, R2-106])
//! - Non-constant switch case labels -> OXS-E0116 ([R3-53, R2-106])
//! - Excluded union switch case -> OXS-W0005 ([R3-53, R2-106])
//! - `this` captured in closure inside `mutating` struct method -> OXS-E0126 ([R3-53, R2-106])
//! - Nullable arithmetic / unary / compound assign / ++ -> OXS-E0105 ([R2-105])
//! - Unused binding -> OXS-W0001 and expression with no effect -> OXS-W0004 ([R2-105])
//! - Closure returning narrowed-null local widens to `T | null` ([R2-111])
//! - `.length` on string and `s[i]` on string carry fixes directly from checker ([R3-249, R3-89, R3-235])
//! - Budget exhaustion `OXS-E0113` names the offending sub-expression ([R3-252])
//! - Diagnostic spans stay within source bounds ([R3-147])
//! - Array invariance, function parameter contravariance, narrowing invalidation ([R3-140, R3-209])
//! - `readonly` / `private` / `static` on struct value copies ([R3-210])

mod common;
use common::*;

#[test]
fn static_and_mutating_constructors_rejected_with_e0123() {
    for src in [
        "class C { static constructor() {} }\n",
        "struct S { x: i32; mutating constructor() { this.x = 1; } }\n",
    ] {
        let c = oxiscript_check::check_source(src, &hosts());
        assert!(
            c.errors().any(|d| d.code == "OXS-E0123"),
            "expected OXS-E0123 for {src:?}, got: {:?}",
            c.diagnostics
        );
    }
}

#[test]
fn non_constant_case_labels_rejected_with_e0116() {
    for src in [
        "type U = { kind: \"a\" } | { kind: \"b\" };\nfunction f(u: U, k: string): i32 { switch (u.kind) { case k: return 1; default: return 0; } }\n",
        "enum E { A, B }\nfunction f(e: E, other: E): i32 { switch (e) { case other: return 1; default: return 0; } }\n",
        "function f(b: bool, c: bool): i32 { switch (b) { case c: return 1; default: return 0; } }\n",
        "function f(x: i32 | null): i32 { switch (x) { case null: return 0; case 1: return 1; case 1: return 2; default: return 3; } }\n",
    ] {
        let c = oxiscript_check::check_source(src, &hosts());
        assert!(
            c.errors().any(|d| d.code == "OXS-E0116"),
            "expected OXS-E0116 for {src:?}, got: {:?}",
            c.diagnostics
        );
    }
}

#[test]
fn excluded_union_switch_case_emits_w0005() {
    let src = r#"
type U = { kind: "a"; x: i32 } | { kind: "b"; y: i32 } | { kind: "c"; z: i32 };
export function f(u: U): i32 {
  if (u.kind === "a") {
    return u.x;
  }
  switch (u.kind) {
    case "a":
      return 0;
    case "b":
      return u.y;
    case "c":
      return u.z;
  }
}
"#;
    let c = oxiscript_check::check_source(src, &hosts());
    assert!(c.ok(), "unexpected errors: {:?}", c.errors().collect::<Vec<_>>());
    assert!(
        c.diagnostics.iter().any(|d| d.code == "OXS-W0005"),
        "expected OXS-W0005 on excluded case, got: {:?}",
        c.diagnostics
    );
}

#[test]
fn this_captured_in_closure_inside_mutating_struct_method_cannot_mutate() {
    // Reading `this` in a closure inside a `mutating` struct method captures a
    // value snapshot (pinned by `oxiscript-lower/tests/programs_hard.rs::p43_closures_capturing_this`),
    // while attempting to mutate `this.n` through that closure is rejected.
    let read_ok = r"
struct Counter {
  n: i32;
  mutating inc(): () => i32 {
    this.n = this.n + 1;
    return () => this.n;
  }
}
";
    let c_ok = oxiscript_check::check_source(read_ok, &hosts());
    assert!(c_ok.ok(), "value snapshot capture should check: {:?}", c_ok.diagnostics);

    let mutate_err = r"
struct Counter {
  n: i32;
  mutating inc(): () => void {
    return () => { this.n = this.n + 1; };
  }
}
";
    let c_err = oxiscript_check::check_source(mutate_err, &hosts());
    assert!(
        !c_err.ok(),
        "mutating struct `this` inside a closure must be rejected: {:?}",
        c_err.diagnostics
    );
}

#[test]
fn nullable_arithmetic_and_unary_and_compound_assign_emit_e0105() {
    let cases = [
        "function f(a: i32 | null, b: i32 | null): i32 { return a + b; }\n",
        "function f(a: i32): i32 { return a + null; }\n",
        "function f(a: i32): i32 { return null + a; }\n",
        "function f(a: i32 | null): i32 { return -a; }\n",
        "function f(a: i32 | null): i32 { return ~a; }\n",
        "function f(a: bool | null): bool { return !a; }\n",
        "function f(a: i32 | null): void { let x: i32 | null = a; x += 1; }\n",
        "function f(a: i32 | null): void { let x: i32 = 1; x += a; }\n",
        "function f(a: i32 | null): void { let x: i32 | null = a; x++; }\n",
    ];
    for src in cases {
        let c = oxiscript_check::check_source(src, &hosts());
        assert!(
            c.errors().any(|d| d.code == "OXS-E0105"),
            "expected OXS-E0105 for {src:?}, got: {:?}",
            c.diagnostics
        );
    }
}

#[test]
fn unused_binding_emits_w0001_and_underscore_suppresses_it() {
    let src = r"
export function f(): i32 {
  const dead: i32 = 1;
  const _ok: i32 = 2;
  const used: i32 = 3;
  return used;
}
";
    let c = oxiscript_check::check_source(src, &hosts());
    assert!(c.ok(), "unexpected errors: {:?}", c.errors().collect::<Vec<_>>());
    let w1: Vec<_> = c.diagnostics.iter().filter(|d| d.code == "OXS-W0001").collect();
    assert_eq!(w1.len(), 1, "expected exactly 1 OXS-W0001, got: {w1:?}");
    assert!(w1[0].message.contains("dead"), "{:?}", w1[0]);
}

#[test]
fn expression_statement_with_no_effect_emits_w0004() {
    let src = "export function f(x: i32): i32 { x + 1; return x; }\n";
    let c = oxiscript_check::check_source(src, &hosts());
    assert!(c.ok(), "unexpected errors: {:?}", c.errors().collect::<Vec<_>>());
    assert!(
        c.diagnostics.iter().any(|d| d.code == "OXS-W0004"),
        "expected OXS-W0004, got: {:?}",
        c.diagnostics
    );
}

#[test]
fn closure_returning_narrowed_null_local_widens_to_declared_nullable() {
    let src = r#"
export function f(): string {
  const s: string | null = null;
  const g = () => s;
  const h = () => { return s; };
  return (g() ?? "a") + (h() ?? "b");
}
"#;
    let c = oxiscript_check::check_source(src, &hosts());
    assert!(c.ok(), "expected clean check, got: {:?}", c.diagnostics);
}

#[test]
fn string_length_and_indexing_carry_fixes_directly_in_checker() {
    let c_len = oxiscript_check::check_source(
        "export function f(s: string): i32 { return s.length; }\n",
        &hosts(),
    );
    let d_len = c_len
        .errors()
        .find(|d| d.code == "OXS-E0108")
        .expect("expected OXS-E0108");
    assert!(
        d_len
            .fixes
            .iter()
            .any(|fx| fx.edits.iter().any(|e| e.replacement == "byteLength")),
        "expected byteLength replacement fix directly from checker: {d_len:?}"
    );
    assert!(
        d_len.fixes.iter().any(|fx| fx.title.contains("chars()")),
        "expected chars() advice fix directly from checker: {d_len:?}"
    );

    let c_idx = oxiscript_check::check_source(
        "export function f(s: string): u8 { return s[0]; }\n",
        &hosts(),
    );
    let d_idx = c_idx
        .errors()
        .find(|d| d.code == "OXS-E0128")
        .expect("expected OXS-E0128");
    assert!(
        d_idx.fixes.iter().any(|fx| fx.title.contains("s.bytes()")),
        "expected s.bytes()/s.chars() advice directly from checker: {d_idx:?}"
    );
}

#[test]
fn budget_exhaustion_e0113_names_offending_subexpression() {
    // A long `??` chain parses iteratively in oxiscript-syntax and recurses in
    // oxiscript-check::expr::coalesce until MAX_EXPR_DEPTH, emitting OXS-E0113
    // with the offending sub-expression snippet.
    let mut expr = String::from("x");
    for _ in 0..140 {
        expr = format!("{expr} ?? x");
    }
    let src = format!("export function f(x: i32 | null): i32 | null {{ return {expr}; }}\n");
    let c = oxiscript_check::check_source(&src, &hosts());
    let d = c
        .errors()
        .find(|d| d.code == "OXS-E0113")
        .expect("expected OXS-E0113");
    assert!(
        d.message.contains("at `"),
        "expected E0113 to name offending sub-expression, got: {:?}",
        d.message
    );
    // [R3-147] Diagnostic span is within source bounds
    assert!(d.span.lo <= d.span.hi && (d.span.hi as usize) <= src.len());
}

#[test]
fn soundness_probes_invariance_contravariance_and_narrowing_invalidation() {
    // Array invariance: (i32)[] is not assignable to (i32 | null)[]
    let c_arr = oxiscript_check::check_source(
        "function g(xs: (i32 | null)[]): void { xs.push(null); }\nexport function f(): void { const a: i32[] = [1]; g(a); }\n",
        &hosts(),
    );
    assert!(!c_arr.ok(), "array must be invariant: {c_arr:?}");

    // Function parameter contravariance: (x: i32) => void is NOT assignable to (x: i32 | null) => void
    let c_fn = oxiscript_check::check_source(
        "function apply(cb: (x: i32 | null) => void): void { cb(null); }\nexport function f(): void { const g = (x: i32): void => { const _y = x + 1; }; apply(g); }\n",
        &hosts(),
    );
    assert!(!c_fn.ok(), "callback param must be contravariant: {c_fn:?}");

    // Narrowing invalidation across mutation by a closure
    let c_nar = oxiscript_check::check_source(
        "export function f(): i32 {\n  let x: i32 | null = 1;\n  const reset = (): void => { x = null; };\n  if (x !== null) {\n    reset();\n    return x;\n  }\n  return 0;\n}\n",
        &hosts(),
    );
    assert!(!c_nar.ok(), "narrowing must be invalidated by closure mutation");
}

#[test]
fn struct_readonly_private_and_static_enforced_on_value_copies() {
    // Writing a readonly field on a struct copy is rejected (OXS-E0109).
    let c_ro = oxiscript_check::check_source(
        "struct S { readonly x: i32; }\nexport function f(a: S): void { let b = a; b.x = 2; }\n",
        &hosts(),
    );
    assert!(c_ro.errors().any(|d| d.code == "OXS-E0109"), "{:?}", c_ro.diagnostics);

    // Reading a private field on a struct copy outside its methods is rejected (OXS-E0119).
    let c_priv = oxiscript_check::check_source(
        "struct S { private x: i32; constructor(x: i32) { this.x = x; } }\nexport function f(a: S): i32 { const b = a; return b.x; }\n",
        &hosts(),
    );
    assert!(c_priv.errors().any(|d| d.code == "OXS-E0119"), "{:?}", c_priv.diagnostics);

    // Accessing a static field through a struct value instance is rejected (OXS-E0108).
    let c_stat = oxiscript_check::check_source(
        "struct S { static k: i32 = 7; x: i32; }\nexport function f(a: S): i32 { return a.k; }\n",
        &hosts(),
    );
    assert!(c_stat.errors().any(|d| d.code == "OXS-E0108"), "{:?}", c_stat.diagnostics);
}
