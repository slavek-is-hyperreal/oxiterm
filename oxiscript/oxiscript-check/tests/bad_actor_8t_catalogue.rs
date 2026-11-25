//! Known-attack catalogue (docs/research/oxiscript-known-attacks.md, section 1 and the 8.T list).
//! Passing = held, failing = finding. Test names start with the catalogue row id.
mod common;
use common::*;
use oxiscript_check::check_source;

fn rejected(src: &str) -> bool {
    !check_source(src, &hosts()).ok()
}
fn all_rejected(v: &[&str]) {
    for s in v {
        assert!(rejected(s), "ACCEPTED:\n{s}");
    }
}

#[test]
fn r1_1_array_covariance() {
    all_rejected(&[
        "function g(a: (i32 | null)[]): void { a[0] = null; }\nfunction f(): i32 { const a: i32[] = [1]; g(a); return a[0]; }",
        "type Aa = { kind: \"a\" };\ntype Bb = { kind: \"b\" };\ntype U = Aa | Bb;\nfunction g(u: U[]): void { u[0] = { kind: \"b\" }; }\nfunction f(a: Aa[]): void { g(a); }",
        "struct B<T> { v: T; }\nfunction f(a: B<i32>): B<i32 | null> { return a; }",
    ]);
}
#[test]
fn r1_2_method_param_contravariance() {
    all_rejected(&[
        "interface I { m(a: i32): i32; }\nstruct S implements I { x: i32; m(a: string): i32 { return 1; } }",
        "interface I { m(a: i32 | null): i32; }\nstruct S implements I { x: i32; m(a: i32): i32 { return 1; } }",
        "function f(): void { const g: (x: i32 | null) => void = (x: i32 | null) => {}; const h: (x: i32) => void = g; const k: (x: i32 | null) => void = h; }",
    ]);
}
#[test]
fn r1_3_unchecked_as() {
    all_rejected(&[
        "type S = { kind: \"a\" } | { kind: \"b\" };\ntype Aa = { kind: \"a\" };\nfunction f(s: S): Aa { return s as Aa; }",
        "function f(s: string): i32 { return s as i32; }",
        "function g<T, U>(a: T): U { return a as U; }",
        "class A { constructor() {} }\nclass B { constructor() {} }\nfunction f(a: A): B | null { return a.downcast<B>(); }",
    ]);
}
#[test]
fn r1_4_any_leak() {
    let c = codes("function f(x: any): void {}");
    assert!(c.iter().any(|(c, _)| c == "OXS-E0900"));
}
#[test]
fn r1_5_index_oob() {
    // result type is the element type (OOB is a defined `bounds` trap per grammar.md 19), index must be i32
    all_rejected(&["function f(a: i32[]): i32 | null { const x: i32 = a[0]; return a[1.0]; }", "function f(a: i32[], i: i64): i32 { return a[i]; }"]);
    assert_ok("function f(a: i32[]): i32 { return a[5]; }");
}
#[test]
fn r1_6_optional_properties() {
    assert!(rejected("struct S { a?: i32; }"));
    assert!(rejected("function f(a: { x?: i32 }): void {}"));
}
#[test]
fn r1_7_narrowing_invalidated_by_callbacks() {
    all_rejected(&[
        "function f(a: i32 | null): i32 { let x = a; if (x !== null) { const c = () => { x = null; }; c(); return x + 1; } return 0; }",
        "function f(a: i32 | null): i32 { let x = a; const c = () => { x = null; }; if (x !== null) { c(); return x + 1; } return 0; }",
        "struct N { v: i32; }\nfunction f(n: N | null): i32 { if (n !== null) { function clr(): void { n = null; } clr(); return n.v; } return 0; }",
        "let a: i32 | null = 1;\nfunction clr(): void { a = null; }\nfunction f(): i32 { if (a !== null) { clr(); return a + 1; } return 0; }",
    ]);
}
#[test]
fn r1_8_narrowing_through_aliases() {
    all_rejected(&[
        "class C { n: i32 | null = null; constructor() {} }\nfunction f(c: C): i32 { const d = c; if (c.n !== null) { d.n = null; return c.n + 1; } return 0; }",
        "class C { n: i32 | null = null; constructor() {} clr(): void { this.n = null; } }\nfunction f(c: C): i32 { if (c.n !== null) { c.clr(); return c.n + 1; } return 0; }",
        "function f(a: (i32 | null)[], b: (i32 | null)[]): i32 { if (a[0] !== null) { b[0] = null; return a[0] + 1; } return 0; }",
    ]);
}
#[test]
fn r1_9_this_typing() {
    all_rejected(&[
        "class C { n: i32 = 0; constructor() {} m(): i32 { return this.n; } }\nfunction f(c: C): i32 { const g = c.m; return g(); }",
        "struct S { x: i32; static f(): i32 { return this.x; } }",
        "struct S { x: i32; f(): void { const c = () => { this.x = 2; }; c(); } }",
    ]);
}
#[test]
fn r1_10_class_fields_use_before_init() {
    all_rejected(&[
        "class C { a: string; b: string; constructor() { this.a = this.b; this.b = \"x\"; } }",
        "class C { a: string; constructor() { this.m(); this.a = \"x\"; } m(): void {} }",
        "class C { s: string; constructor() { leak(this); this.s = \"a\"; } }\nfunction leak(c: C): void {}",
    ]);
}
#[test]
fn r1_10b_static_uninit_is_broken() {
    // FINDING F14: statics bypass the non-null-managed initialisation rule
    assert!(rejected("class C { static s: string; constructor() {} }"));
}
#[test]
fn r1_11_closed_enums() {
    all_rejected(&[
        "enum C: u8 { A, B }\nfunction f(x: u8): C { return x as C; }",
        "enum C: u8 { A, B }\nfunction f(): C { return 5; }",
        "enum C: u8 { A, B }\nfunction f(c: C): i32 { switch (c) { case C.A: return 1; } }",
    ]);
}
#[test]
fn r1_11b_enum_duplicate_values_is_broken() {
    assert!(rejected("enum C: u8 { A = 1, B = 1 }\nfunction f(c: C): i32 { switch (c) { case C.A: return 1; case C.B: return 2; } }"));
}
#[test]
fn r1_12_overloads() {
    // no overloads exist: duplicate names are errors
    assert!(rejected("function f(a: string): string { return a; }\nfunction f(a: i32): i32 { return a; }"));
}
#[test]
fn r1_13_generic_constraints() {
    all_rejected(&[
        "struct S { x: i32; }\nfunction g<T extends S>(a: T): T { return a; }",
        "interface I { m(): i32; }\nstruct S { x: i32; }\nfunction g<T extends I>(t: T): i32 { return t.m(); }\nfunction f(s: S): i32 { return g(s); }",
        "function g<T>(a: T | null): T { return a; }",
        "function g<T extends T>(a: T): T { return a; }",
    ]);
}
#[test]
fn r1_14_readonly() {
    all_rejected(&[
        "struct S { readonly n: i32; }\nfunction f(): void { let s: S = { n: 1 }; s.n = 2; }",
        "struct S { readonly a: i32; }\nfunction f(s: S): void { let t = s; t.a = 1; }",
        "class C { readonly n: i32 = 0; constructor() {} set(): void { this.n = 1; } }",
    ]);
}
#[test]
fn r1_15_unforgeable_nominal_and_capabilities() {
    all_rejected(&[
        "import { Log } from \"host:log\";\nfunction f(): Log { return new Log(); }",
        "import { Log } from \"host:log\";\nfunction f(): Log { return {}; }",
        "import { Log } from \"host:log\";\nfunction f(n: i32): Log { return n as Log; }",
        "struct A { x: i32; }\nstruct B { x: i32; }\nfunction f(a: A): B { return a; }",
        "class C { private x: i32 = 0; constructor() {} }\nfunction f(c: C): i32 { return c.x; }",
    ]);
}
#[test]
fn r1_15b_private_field_via_literal_cross_module_is_broken() {
    let mut l = oxiscript_check::MapLoader::default();
    l.0.insert("a".into(), "export struct S { private x: i32; y: i32; }".into());
    let c = oxiscript_check::check_program("main", "import { S } from \"./a\";\nexport function f(): S { return { x: 1, y: 2 }; }", &mut l, &hosts());
    assert!(!c.ok());
}
#[test]
fn r1_16_arity_width_freshness() {
    all_rejected(&[
        "function g(a: i32, b: i32 = 1): i32 { return a; }\nfunction f(): void { const h: (a: i32) => i32 = g; }",
        "struct S { x: i32; }\nfunction f(): S { return { x: 1, y: 2 }; }",
        "function f(): void { const g: (a: i32) => void = (a: i32, b: i32) => {}; }",
    ]);
}
#[test]
fn r1_17_never() {
    all_rejected(&["function f(): i32 { const x: never = 1; return 0; }", "function f(a: never[]): i32 { return a[0]; }"]);
}
#[test]
fn r1_17b_never_function_fallthrough_is_broken() {
    assert!(rejected("function f(): never { }"));
}
#[test]
fn r1_18_spread_arity() {
    all_rejected(&["struct S { x: i32; }\nfunction f(a: S): S { return { ...a }; }", "function g(a: i32, b: i32): i32 { return a; }\nfunction f(a: i32[]): i32 { return g(...a); }"]);
}
#[test]
fn r1_19_checker_dos() {
    // 10^5-member union and deep instantiation: must finish with diagnostics on a 1 MB stack
    let big = format!("type U = {};", (0..100_000).map(|i| format!("{{ kind: {i}; v: i32 }}")).collect::<Vec<_>>().join(" | "));
    let r = std::thread::Builder::new().stack_size(1 << 20).spawn(move || oxiscript_check::check_source(&big, &hosts()).diagnostics.len()).unwrap().join();
    assert!(r.is_ok_and(|n| n > 0), "huge union must be refused with a diagnostic");
}
#[test]
fn r1_19b_polymorphic_recursion_is_broken() {
    assert!(rejected("function g<T>(a: T, n: i32): void { if (n > 0) { g<T[]>([a], n - 1); } }\nfunction f(): void { g(1, 3); }"));
}
#[test]
fn r1_20_void_callbacks_drop_value() {
    // `() => 1` where `() => void` is expected: the dropped value must be visible in the tables
    let src = "function ap(f: () => void): void { f(); }\nfunction f(): void { ap(() => 1); }";
    let c = check_source(src, &hosts());
    if c.ok() {
        let m = &c.modules[0];
        assert!(m.coercions.values().any(|co| co.to == oxiscript_check::ty::VOID), "closure body value is silently discarded: no coercion to void recorded");
    }
}
#[test]
fn r1_21_definite_assignment() {
    all_rejected(&[
        "function f(c: bool): i32 { let a: i32; if (c) { a = 1; } return a; }",
        "function f(c: bool): i32 { let a: i32; const g = () => a; a = 1; return g(); }",
        "function f(): i32 { const a = a + 1; return a; }",
        "function f(): void { const c = (n: i32): i32 => c(n); }",
        "let s: string;",
    ]);
}
#[test]
fn r1_21b_global_forward_read_is_broken() {
    assert!(rejected("let a: string = b;\nlet b: string = \"x\";"));
}
#[test]
fn r1_22_nan_equality_spec() {
    assert_ok("function f(a: f64): bool { return a === a; }");
    all_rejected(&["struct S { x: i32; }\nfunction f(a: S, b: S): bool { return a === b; }", "function f(a: i32[], b: i32[]): bool { return a === b; }"]);
}
#[test]
fn r2_7_float_to_int_conversion_recorded() {
    let src = "function f(a: f64): i32 { return a as i32; }";
    let c = check_source(src, &hosts());
    assert!(c.ok());
    assert!(c.modules[0].casts.values().any(|k| k.from == oxiscript_check::ty::F64 && k.to == oxiscript_check::ty::I32));
}
#[test]
fn r4_4_deep_nesting_is_bounded() {
    // the full matrix is in bad_actor_8t_stack.rs; spot check here
    let src = format!("function f(): i32 {{ return {}1{}; }}", "(".repeat(100_000), ")".repeat(100_000));
    let r = std::thread::Builder::new().stack_size(1 << 20).spawn(move || oxiscript_check::check_source(&src, &hosts()).ok()).unwrap().join();
    assert!(r.is_ok());
}
#[test]
fn r4_13_capability_confusion() {
    all_rejected(&[
        "import { Log } from \"host:log\";\nfunction f(): void { Log.write(1, \"x\"); }",
        "import { Nope } from \"host:log\";",
        "import { X } from \"host:nope\";",
        "import { Log } from \"host:log\";\nexport function main(x: i32, l: Log): void {}",
    ]);
}
