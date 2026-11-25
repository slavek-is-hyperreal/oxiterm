#![allow(clippy::format_push_string)]
//! Programs that must be accepted (and what the output tables say about them).
mod common;
use common::*;
use oxiscript_check::*;

#[test]
fn non_exported_functions_may_take_many_strings() {
    assert_ok("function f(a: string, b: string): void {}");
}

#[test]
fn closure_captures_report_mutation() {
    let src = "function counter(): () => i32 {\n  let n = 0;\n  const k = 5;\n  return () => { n += 1; return n + k; };\n}\n";
    let c = check_source(src, &hosts());
    assert!(c.ok(), "{}", show(src));
    let m = &c.modules[0];
    let info = m.closures.values().next().expect("closure recorded");
    assert_eq!(info.captures.len(), 2);
    for cap in &info.captures {
        let name = &m.locals[cap.local.0 as usize].name;
        assert_eq!(cap.mutated, name == "n", "capture {name}");
    }
}

#[test]
fn definite_assignment_rules() {
    assert_ok("function f(c: bool): i32 {\n  let x: i32;\n  if (c) { x = 1; } else { x = 2; }\n  return x;\n}");
    assert_ok("function f(): i32 {\n  let x: i32;\n  x = 1;\n  x += 2;\n  return x;\n}");
    assert_eq!(codes("function f(c: bool): i32 {\n  let x: i32;\n  if (c) { x = 1; }\n  return x;\n}"), vec![("OXS-E0118".to_string(), 4)]);
    assert_eq!(codes("function f(): void {\n  let i: i32;\n  i++;\n}"), vec![("OXS-E0118".to_string(), 3)]);
    assert_eq!(codes("function f(): void {\n  let s: string;\n  s += \"a\";\n}"), vec![("OXS-E0118".to_string(), 3)]);
    assert_eq!(
        codes("function f(): void {\n  let x: i32;\n  const g = () => x;\n}"),
        vec![("OXS-E0118".to_string(), 3)]
    );
}

#[test]
fn constructors_track_fields() {
    assert_ok("class C {\n  n: i32;\n  s: string;\n  constructor(a: i32) { this.n = a; this.s = \"x\"; this.n += 1; }\n  get(): i32 { return this.n; }\n}");
    assert_eq!(
        codes("class C {\n  n: i32;\n  constructor() {\n    this.n += 1;\n    this.n = 0;\n  }\n}"),
        vec![("OXS-E0118".to_string(), 4)]
    );
}

#[test]
fn void_result_can_be_returned_with_ok() {
    assert_ok("function f(): Result<void, string> {\n  return Ok();\n}\nfunction g(): Result<i32, string> {\n  f()?;\n  return Ok(1);\n}");
}

#[test]
fn host_results_work_with_try() {
    assert_ok(
        "import { State } from \"host:state\";\nfunction f(s: State): Result<i64, i32> {\n  const slot = s.open(\"k\")?;\n  s.set_i64(slot, 3)?;\n  return s.get_i64(slot);\n}",
    );
}

#[test]
fn narrowing_survives_method_calls_on_other_receivers() {
    assert_ok(
        "function f(xs: i32[] | null): i32 {\n  if (xs !== null) {\n    for (let i = 0; i < 3; i++) { xs.push(i); }\n    return xs.length;\n  }\n  return 0;\n}",
    );
    assert_ok(
        "import { Log } from \"host:log\";\nstruct N { v: i32; }\nfunction f(log: Log, n: N | null): i32 {\n  const g = () => { log.write(1, \"x\"); };\n  if (n !== null) { g(); return n.v; }\n  return 0;\n}",
    );
}

#[test]
fn narrowing_forms() {
    assert_ok("struct N { v: i32; next: N | null; }\nfunction f(n: N | null): i32 {\n  if (n === null) { return 0; }\n  return n.v;\n}");
    assert_ok("struct N { v: i32; next: N | null; }\nfunction f(n: N): i32 {\n  if (n.next !== null) { return n.next.v; }\n  return 0;\n}");
    assert_ok("function f(a: i32 | null, b: i32 | null): i32 {\n  if (a !== null && b !== null) { return a + b; }\n  return 0;\n}");
    assert_ok("function f(a: i32 | null): i32 {\n  return a !== null ? a + 1 : 0;\n}");
    assert_ok("function f(a: i32 | null): i32 {\n  while (a === null) { a = 3; }\n  return a;\n}");
    assert_ok("type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 } | { kind: \"c\" };\nfunction f(s: S): i32 {\n  if (s.kind === \"c\") { return 0; }\n  if (s.kind === \"a\") { return s.x; }\n  return s.y;\n}");
    assert_ok("type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 };\nfunction f(s: S): i32 {\n  return switch (s.kind) { case \"a\": s.x, case \"b\": s.y, };\n}");
    assert_ok("type S = { kind: \"a\"; x: i32 } | { kind: \"b\"; y: i32 };\nfunction f(s: S): i32 {\n  if (s.kind !== \"a\") { return s.y; } else { return s.x; }\n}");
}

#[test]
fn struct_values_are_copies() {
    assert_ok("struct P { x: i32; }\nfunction f(a: P): i32 {\n  let b = a;\n  b.x = 5;\n  return a.x;\n}");
}

#[test]
fn generics_and_instances() {
    let src = "function id<T>(x: T): T { return x; }\nstruct Box<T> { v: T; get(): T { return this.v; } }\nfunction f(b: Box<string>): string {\n  const n = id(3);\n  const s = id<string>(\"a\");\n  return b.get();\n}\n";
    let c = check_source(src, &hosts());
    assert!(c.ok(), "{}", show(src));
    // id<i32>, id<string>, Box<string>.get
    assert!(c.instances.len() >= 3, "{:?}", c.instances);
}

#[test]
fn interface_bounds() {
    assert_ok("interface Shape { area(): f64; }\nstruct Sq implements Shape { s: f64; area(): f64 { return this.s * this.s; } }\nfunction total<T extends Shape>(xs: T[]): f64 {\n  let t = 0.0;\n  for (const x of xs) { t += x.area(); }\n  return t;\n}\nfunction g(): f64 {\n  const a: Sq = { s: 2.0 };\n  return total([a]);\n}");
}

#[test]
fn laws_may_call_pure_functions_only() {
    assert_ok("function sq(a: i32): i32 { return a * a; }\nlaw sq_nonneg(a: i32) when a > -100 { sq(a) >= 0 }");
    let src = "import { Log } from \"host:log\";\nfunction noisy(log: Log): bool { log.write(1, \"x\"); return true; }\nlet counter = 0;\nfunction bump(): bool { counter += 1; return true; }\nlaw bad(a: i32) { bump() }\n";
    let c = check_source(src, &hosts());
    assert!(c.errors().any(|d| d.code == "OXS-E0125"), "{}", show(src));
    let eff = c.effects.iter().find(|(d, _)| c.defs[d.0 as usize].name == "bump").map(|(_, e)| *e).unwrap();
    assert!(eff.writes_globals && !eff.uses_host);
    let eff = c.effects.iter().find(|(d, _)| c.defs[d.0 as usize].name == "noisy").map(|(_, e)| *e).unwrap();
    assert!(eff.uses_host);
}

#[test]
fn numeric_literal_inference() {
    assert_ok("function f(a: u16, b: f32): f32 {\n  const x = a + 1;\n  const y = 1 + a;\n  const z: u8 = 200;\n  const w: f64 = 1;\n  const v = 2.5;\n  return b * 2.0;\n}");
    assert_ok("function f(a: i8): i8 { return -128; }");
}

#[test]
fn wrapping_saturating_and_downcast() {
    assert_ok("type S = { kind: \"a\" } | { kind: \"b\" };\ntype A = S;\nfunction f(x: i64, y: f64): u8 {\n  const a = x.wrapping<u8>();\n  const b = y.saturating<i32>();\n  return a;\n}");
}

#[test]
fn enums_and_switch() {
    assert_ok("enum Color: u8 { Red, Green = 5, Blue }\nfunction f(c: Color): string {\n  switch (c) {\n    case Color.Red: return \"r\";\n    case Color.Green, Color.Blue: return \"gb\";\n  }\n}");
    assert_ok("function f(b: bool): i32 {\n  switch (b) {\n    case true: return 1;\n    case false: return 0;\n  }\n}");
    assert_ok("function f(n: i32 | null): i32 {\n  switch (n) {\n    case null: return 0;\n    default: return 1;\n  }\n}");
}

#[test]
fn namespaces_and_static_members() {
    assert_ok("class C {\n  static n: i32 = 0;\n  static make(): C { return new C(); }\n}\nfunction f(): C {\n  C.n = 3;\n  return C.make();\n}");
}

#[test]
fn deep_but_reasonable_programs() {
    // 400 chained additions are folded iteratively
    let mut s = String::from("function f(a: i32): i32 {\n  return a");
    for _ in 0..400 {
        s.push_str(" + a");
    }
    s.push_str(";\n}\n");
    assert_ok(&s);
}
