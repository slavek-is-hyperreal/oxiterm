//! BAD ACTOR round 2026-11-27-1, target 1 (8.Fix checker rules): E0105 nullable
//! arithmetic, W0001 unused bindings, W0005 excluded case labels, E0126 closure
//! mutation of a struct `this`. Failing tests are findings (R1-nn in
//! docs/stages/rounds/2026-11-27-1.md).
#![allow(clippy::pedantic)]

mod common;
use common::*;

fn diags(src: &str) -> Vec<(String, String)> {
    let c = oxiscript_check::check_source(src, &hosts());
    c.diagnostics.iter().map(|d| (d.code.to_string(), d.message.clone())).collect()
}

fn has(src: &str, code: &str) -> bool {
    diags(src).iter().any(|(c, _)| c == code)
}

fn errors(src: &str) -> Vec<(String, String)> {
    let c = oxiscript_check::check_source(src, &hosts());
    c.errors().map(|d| (d.code.to_string(), d.message.clone())).collect()
}

const COUNTER: &str = r"
struct Inner {
  v: i32;
}
struct Counter {
  n: i32;
  inner: Inner;
  mutating bump(): void {
    this.n = this.n + 1;
  }
";

/// Probe (passes): E0126 only guards assignment *places* that start at `this`; every other
/// way of mutating the snapshot from inside a closure must be rejected too.
#[test]
fn probe_closure_mutation_of_struct_this_bypasses() {
    let bodies = [
        ("mutating method call", "  mutating m(): () => void {\n    return () => { this.bump(); };\n  }\n"),
        ("++", "  mutating m(): () => void {\n    return () => { this.n++; };\n  }\n"),
        ("+=", "  mutating m(): () => void {\n    return () => { this.n += 1; };\n  }\n"),
        ("nested field", "  mutating m(): () => void {\n    return () => { this.inner.v = 1; };\n  }\n"),
        ("nested closure", "  mutating m(): () => () => void {\n    return () => () => { this.n = 1; };\n  }\n"),
    ];
    let mut bypass = Vec::new();
    for (what, body) in bodies {
        let src = format!("{COUNTER}{body}}}\n");
        let e = errors(&src);
        if e.is_empty() {
            bypass.push(what);
        }
        eprintln!("{what}: {e:?}");
    }
    assert!(bypass.is_empty(), "closure mutates struct `this` without an error: {bypass:?}");
}

/// R1-21: E0105 must fire on every arithmetic/comparison use of a nullable operand and
/// never on a narrowed one.
#[test]
fn r1_21_e0105_precision() {
    let must_err = [
        ("optional chain +", "struct P {\n  x: i32;\n}\nfunction f(p: P | null): i32 {\n  return p?.x + 1;\n}\n"),
        ("< nullable", "function f(a: i32 | null): bool {\n  return a < 1;\n}\n"),
        ("* nullable", "function f(a: f64 | null): f64 {\n  return 2.0 * a;\n}\n"),
        ("<< nullable", "function f(a: i32 | null): i32 {\n  return 1 << a;\n}\n"),
        ("as from nullable", "function f(a: i32 | null): f64 {\n  return a as f64;\n}\n"),
        ("-= nullable rhs", "function f(a: i32 | null): i32 {\n  let x: i32 = 1;\n  x -= a;\n  return x;\n}\n"),
        ("--", "function f(a: i32 | null): void {\n  let x: i32 | null = a;\n  x--;\n}\n"),
    ];
    let mut missed = Vec::new();
    for (what, src) in must_err {
        let e = errors(src);
        if e.is_empty() {
            missed.push(what);
        }
        eprintln!("{what}: {e:?}");
    }
    let must_ok = [
        ("narrowed +=", "export function f(a: i32 | null): i32 {\n  let x: i32 | null = a;\n  if (x !== null) {\n    x += 1;\n    return x;\n  }\n  return 0;\n}\n"),
        ("?? then +", "export function f(a: i32 | null): i32 {\n  return (a ?? 0) + 1;\n}\n"),
        ("narrowed ternary", "export function f(a: i32 | null): i32 {\n  return a !== null ? a + 1 : 0;\n}\n"),
        ("narrowed &&", "export function f(a: i32 | null): bool {\n  return a !== null && a > 1;\n}\n"),
        ("=== null", "export function f(a: i32 | null): bool {\n  return a === null;\n}\n"),
        ("narrowed ++", "export function f(a: i32 | null): i32 {\n  let x = a;\n  if (x === null) {\n    return 0;\n  }\n  x++;\n  return x;\n}\n"),
    ];
    let mut false_pos = Vec::new();
    for (what, src) in must_ok {
        let e = errors(src);
        if !e.is_empty() {
            false_pos.push((what, e));
        }
    }
    assert!(missed.is_empty() && false_pos.is_empty(), "E0105 missed: {missed:?}; false positives: {false_pos:?}");
}

/// W0001: no warning on a binding that is used; a warning on one that is not.
#[test]
fn probe_w0001_precision() {
    let used = [
        ("only in closure", "export function f(): i32 {\n  const k = 1;\n  const g = () => k;\n  return g();\n}\n"),
        ("only in template", "export function f(): string {\n  const k = 1;\n  return `${k}`;\n}\n"),
        ("only via ?", "function g(): Result<i32, i32> {\n  return Ok(1);\n}\nexport function f(): Result<i32, i32> {\n  const r = g();\n  const v = r?;\n  return Ok(v);\n}\n"),
        ("only as switch subject", "type S = { kind: \"a\" } | { kind: \"b\" };\nexport function f(s: S): i32 {\n  const t = s;\n  switch (t.kind) {\n    case \"a\":\n      return 1;\n    case \"b\":\n      return 2;\n  }\n}\n"),
        ("only compound-assigned then returned", "export function f(): i32 {\n  let k = 1;\n  k += 1;\n  return k;\n}\n"),
        ("for-of used", "export function f(xs: i32[]): i32 {\n  let s = 0;\n  for (const x of xs) {\n    s += x;\n  }\n  return s;\n}\n"),
        ("only in law", "export function id(a: i32): i32 {\n  return a;\n}\nlaw l(a: i32) {\n  id(a) === a\n}\n"),
    ];
    let mut fp = Vec::new();
    for (what, src) in used {
        let d = diags(src);
        if d.iter().any(|(c, _)| c == "OXS-W0001") {
            fp.push((what, d));
        }
    }
    let unused = [
        ("write-only let", "export function f(): i32 {\n  let k = 1;\n  k = 2;\n  return 0;\n}\n"),
        ("unused for-of", "export function f(xs: i32[]): i32 {\n  let s = 0;\n  for (const x of xs) {\n    s += 1;\n  }\n  return s;\n}\n"),
        ("unused in closure body", "export function f(): () => i32 {\n  return () => {\n    const dead = 1;\n    return 2;\n  };\n}\n"),
    ];
    let mut missed = Vec::new();
    for (what, src) in unused {
        if !has(src, "OXS-W0001") {
            missed.push((what, diags(src)));
        }
    }
    eprintln!("W0001 missed (informational): {missed:?}");
    assert!(fp.is_empty(), "W0001 false positives: {fp:?}");
}

/// W0005: a case label excluded by narrowing; must not fire on a reachable one.
#[test]
fn probe_w0005_precision() {
    let reachable = "type S = { kind: \"a\" } | { kind: \"b\" } | { kind: \"c\" };\nexport function f(s: S): i32 {\n  if (s.kind === \"a\") {\n    return 0;\n  }\n  switch (s.kind) {\n    case \"b\":\n      return 1;\n    case \"c\":\n      return 2;\n  }\n}\n";
    assert!(!has(reachable, "OXS-W0005"), "{:?}", diags(reachable));
    // Narrowing invalidated by reassignment: the label is reachable again.
    let reassigned = "type S = { kind: \"a\" } | { kind: \"b\" };\nexport function f(s: S, t: S): i32 {\n  let u = s;\n  if (u.kind === \"a\") {\n    u = t;\n    switch (u.kind) {\n      case \"a\":\n        return 1;\n      case \"b\":\n        return 2;\n    }\n  }\n  return 0;\n}\n";
    assert!(!has(reassigned, "OXS-W0005"), "W0005 after reassignment: {:?}", diags(reassigned));
    let excluded = "type S = { kind: \"a\" } | { kind: \"b\" };\nexport function f(s: S): i32 {\n  if (s.kind === \"a\") {\n    return 0;\n  }\n  switch (s.kind) {\n    case \"a\":\n      return 9;\n    case \"b\":\n      return 1;\n  }\n}\n";
    assert!(has(excluded, "OXS-W0005"), "{:?}", diags(excluded));
}
