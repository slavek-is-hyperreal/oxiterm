//! BAD ACTOR findings for slice 8.T. Every test asserts that the checker REJECTS an ill-typed
//! (or ABI-unsound) program; each one currently FAILS (the program is accepted).
mod common;
use common::*;

fn rejects(src: &str) {
    let c = oxiscript_check::check_source(src, &hosts());
    assert!(!c.ok(), "ACCEPTED an ill-typed program:\n{src}");
}

/// Rejected with (at least) the given registry code, so an unrelated rejection cannot turn a test green.
fn rejects_with(src: &str, code: &str) {
    let c = oxiscript_check::check_source(src, &hosts());
    assert!(c.errors().any(|d| d.code == code), "expected {code}, got {:?} for:\n{src}", c.errors().map(|d| d.code).collect::<Vec<_>>());
}

// F1 never-returning function that returns (unsound: callers treat the call as diverging)
#[test]
fn f01_never_function_may_fall_off_the_end() {
    rejects_with("function f(): never { }\nfunction g(): i32 { f(); }", "OXS-E0114");
}
#[test]
fn f01b_never_function_loop_that_exits() {
    rejects_with("function f(): never { while (false) { } }", "OXS-E0114");
}

// F2 module `let` initialiser reads a later (not yet initialised) managed global: null in a non-null type
#[test]
fn f02_global_forward_read_before_init() {
    rejects("let a: string = b;\nlet b: string = \"x\";");
}
#[test]
fn f02b_global_read_through_function_before_init() {
    rejects("let s: string = f();\nfunction f(): string { return s; }");
}

// F3 builtin read-only members are assignable
#[test]
fn f03_array_length_assignable() {
    rejects_with("function f(a: i32[]): void { a.length = 0; }", "OXS-E0109");
}
#[test]
fn f03b_array_length_increment() {
    rejects_with("function f(a: i32[]): void { a.length++; }", "OXS-E0109");
}
#[test]
fn f03c_string_byte_length_assignable() {
    rejects_with("function f(a: string): void { a.byteLength = 1; }", "OXS-E0109");
}

// F4 a private method satisfies an interface (and is then callable through the bound)
#[test]
fn f04_private_method_satisfies_interface() {
    rejects("interface I { m(): i32; }\nstruct S { x: i32; private m(): i32 { return 1; } }\nfunction g<T extends I>(t: T): i32 { return t.m(); }\nfunction f(s: S): i32 { return g(s); }");
}
#[test]
fn f04b_implements_private_method() {
    rejects("interface I { m(): i32; }\nstruct S implements I { x: i32; private m(): i32 { return 1; } }");
}

// F5 law purity bypass through an interface bound (effects of T::m are not known at the generic call)
#[test]
fn f05_law_host_effect_through_bound() {
    rejects_with("import { Log } from \"host:log\";\ninterface I { m(): bool; }\nstruct S { log: Log; m(): bool { this.log.write(1, \"x\"); return true; } }\nfunction g<T extends I>(t: T): bool { return t.m(); }\nlaw l(s: S) { g(s) }", "OXS-E0125");
}
#[test]
fn f05b_law_global_write_through_bound() {
    rejects_with("let n: i32 = 0;\ninterface I { m(): bool; }\nstruct S { x: i32; m(): bool { n = n + 1; return true; } }\nfunction g<T extends I>(t: T): bool { return t.m(); }\nlaw l(s: S) { g(s) }", "OXS-E0125");
}
// F6 law mutates its (shared, reference) array input
#[test]
fn f06_law_mutates_array_argument() {
    rejects_with("function m(a: i32[]): bool { a.push(1); return true; }\nlaw l(a: i32[]) { m(a) }", "OXS-E0125");
}

// F7 polymorphic recursion: infinitely many instances for monomorphisation
#[test]
fn f07_polymorphic_recursion() {
    rejects("function g<T>(a: T, n: i32): void { if (n > 0) { g<T[]>([a], n - 1); } }\nfunction f(): void { g(1, 3); }");
}
#[test]
fn f07b_polymorphic_recursion_struct_growth() {
    rejects("struct B<T> { v: T; }\nfunction g<T>(a: T): void { g<B<T>>({ v: a }); }");
}

// F8 `void`/`never` are rejected as element/field/param types but slip through generic instantiation
#[test]
fn f08_array_of_void_via_generic() {
    rejects("function g<T>(): T[] { return []; }\nfunction f(): void { const a = g<void>(); }");
}
#[test]
fn f08b_void_field_via_generic() {
    rejects("struct B<T> { v: T; }\nfunction f(b: B<void>): void { }");
}
#[test]
fn f08c_never_field_via_generic() {
    rejects("struct B<T> { v: T; }\nfunction f(b: B<never>): never { return b.v; }");
}

// F9 shift amount must have the operand type (ir.md: shl/shr T,T -> T)
#[test]
fn f09_shift_mixed_widths() {
    rejects("function f(a: u8, b: i32): u8 { return a << b; }");
}
#[test]
fn f09b_shift_assign_mixed_widths() {
    rejects("function f(a: u8, b: i32): void { let x = a; x <<= b; }");
}
#[test]
fn f09c_shift_constant_out_of_range() {
    rejects("function f(a: i32): i32 { return a << 40; }");
}

// F10/F11 duplicate switch labels
#[test]
fn f10_enum_duplicate_values() {
    rejects("enum C: u8 { A = 1, B = 1 }\nfunction f(c: C): i32 { switch (c) { case C.A: return 1; case C.B: return 2; } }");
}
#[test]
fn f11_bool_duplicate_case() {
    rejects("function f(a: bool): i32 { switch (a) { case true: return 1; case true: return 2; case false: return 3; } }");
}

// F12 types of infinite size
#[test]
fn f12_struct_contains_itself() {
    rejects("struct A { a: A; }\nfunction f(): void {}");
}
#[test]
fn f12b_mutual_struct_by_value() {
    rejects("struct A { b: B; }\nstruct B { a: A; }\nfunction f(): void {}");
}
#[test]
fn f12c_generic_struct_infinite_instantiation() {
    rejects("struct A<T> { a: A<A<T>>; }\nfunction f(): void {}");
}

// F13 `?` inside a default parameter expression (evaluated at the call site, in the CALLER's frame)
#[test]
fn f13_try_in_default_parameter() {
    rejects("function g(): Result<i32, string> { return Ok(1); }\nfunction h(a: i32 = g()?): Result<i32, string> { return Ok(a); }\nfunction k(): i32 { return h().kind === \"ok\" ? 1 : 0; }");
}

// F14 non-null managed `static` without initialiser (module `let` is rejected with E0123, statics are not)
#[test]
fn f14_static_non_null_class_without_initialiser() {
    rejects_with("class D { constructor() {} }\nclass C { static d: D; constructor() {} }", "OXS-E0123");
}
#[test]
fn f14b_static_non_null_string_without_initialiser() {
    rejects_with("class C { static s: string; constructor() {} }", "OXS-E0123");
}
#[test]
fn f14c_static_forward_read_before_init() {
    rejects("class C { static a: string = C.b; static b: string = \"x\"; constructor() {} }");
}
#[test]
fn f14d_const_initialiser_reads_later_const_through_function() {
    rejects("const a: string = mk();\nconst b: string = \"x\";\nfunction mk(): string { return b; }");
}

// F15 an enum member (a constant) is an assignable place
#[test]
fn f15_assign_to_enum_member() {
    rejects_with("enum E { A }\nfunction f(): void { E.A = E.A; }", "OXS-E0109");
}

// F16 imported bindings are assignable from the importing module (module-level state is module-local)
fn rejects_prog(entry: &str, mods: &[(&str, &str)]) {
    let mut l = oxiscript_check::MapLoader::default();
    for (n, s) in mods {
        l.0.insert((*n).into(), (*s).into());
    }
    let c = oxiscript_check::check_program("main", entry, &mut l, &hosts());
    assert!(!c.ok(), "ACCEPTED an ill-typed program:\n{entry}");
}
#[test]
fn f16_assign_imported_let() {
    rejects_prog("import { counter } from \"./a\";\nexport function f(): void { counter = 1; }", &[("a", "export let counter: i32 = 0;")]);
}
#[test]
fn f16b_assign_through_namespace() {
    rejects_prog("import * as a from \"./a\";\nexport function f(): void { a.x = 1; }", &[("a", "export let x: i32 = 0;")]);
}

// F17 private field initialised through an object literal in another module
#[test]
fn f17_private_field_in_object_literal_cross_module() {
    rejects_prog("import { S } from \"./a\";\nexport function f(): S { return { x: 1, y: 2 }; }", &[("a", "export struct S { private x: i32; y: i32; }")]);
}

// F18 a local declaration inside a generic function mentions the function's type parameter:
// `records[S].fields[0].ty` is `Param(T)` of a different owner, so `S` cannot be monomorphised per instance.
#[test]
fn f18_local_struct_captures_outer_type_parameter() {
    rejects("function f<T>(a: T): T { struct S { v: T; } const s: S = { v: a }; return s.v; }\nfunction g(): i32 { return f(1); }");
}

// F19 quadratic work in the number of parameters / type parameters / enum members
fn time(src: String) -> f64 {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let h = hosts();
            let t = std::time::Instant::now();
            std::hint::black_box(oxiscript_check::check_source(&src, &h));
            t.elapsed().as_secs_f64()
        })
        .unwrap()
        .join()
        .unwrap()
}
fn assert_linear(gen: impl Fn(usize) -> String) {
    let best = |n: usize| (0..3).map(|_| time(gen(n))).fold(f64::MAX, f64::min);
    let (a, b) = (best(6_000), best(24_000));
    // linear => ratio ~4, quadratic => ~16; generous margin for a loaded machine
    assert!(b / a.max(1e-4) < 9.0, "super-linear growth: {a:.4}s -> {b:.4}s for 4x the input");
}
#[test]
fn f19_params_quadratic() {
    assert_linear(|n| format!("function f({}): void {{}}", (0..n).map(|i| format!("a{i}: i32")).collect::<Vec<_>>().join(", ")));
}
#[test]
fn f19b_type_params_quadratic() {
    assert_linear(|n| format!("function f<{}>(): void {{}}", (0..n).map(|i| format!("T{i}")).collect::<Vec<_>>().join(", ")));
}
#[test]
fn f19c_enum_members_quadratic() {
    assert_linear(|n| format!("enum E: u32 {{ {} }}", (0..n).map(|i| format!("M{i}")).collect::<Vec<_>>().join(", ")));
}
