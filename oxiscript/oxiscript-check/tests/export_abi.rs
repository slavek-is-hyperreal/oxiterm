//! Host export rule (owner decision 2026-11-26): `export` is module visibility only (no ABI restriction);
//! a `@host export function` of the entry module is host-callable and must fit ABI 1.0 (`OXS-E0121`).
mod common;
use common::*;
use oxiscript_check::{check_program, MapLoader};

fn count(src: &str, code: &str) -> usize {
    codes(src).iter().filter(|(c, _)| c == code).count()
}

fn e0121(src: &str) -> usize {
    count(src, "OXS-E0121")
}

const MAIN: &str = "\nexport function main(): void {}\n";

#[test]
fn unfit_host_functions_are_rejected() {
    for decl in [
        "struct P { x: i32; }\n@host export function f(p: P): i32 { return p.x; }",
        "struct P { x: i32; }\n@host export function f(x: i32): P { return { x: x }; }",
        "@host export function f(a: i32[]): i32 { return a.length; }",
        "@host export function f(a: i32 | null): i32 { return 1; }",
        "@host export function f(c: () => i32): i32 { return c(); }",
        "@host export function f(x: i32): string { return \"a\"; }",
        "@host export function f(a: string, b: string): void {}",
        "type U = { kind: \"a\" } | { kind: \"b\" };\n@host export function f(u: U): i32 { return 1; }",
    ] {
        assert_eq!(e0121(&format!("{decl}{MAIN}")), 1, "{decl}");
    }
}

#[test]
fn plain_exports_have_no_abi_restriction() {
    // `export` alone is module visibility, exactly like TypeScript: structs, unions, generics, strings, closures.
    for decl in [
        "struct P { x: i32; }\nexport function f(p: P): i32 { return p.x; }",
        "struct P { x: i32; }\nexport function f(x: i32): P { return { x: x }; }",
        "export function f(a: i32[]): i32 { return a.length; }",
        "export function f(a: i32 | null): i32 { return 1; }",
        "export function f(c: () => i32): i32 { return c(); }",
        "export function f(x: i32): string { return \"a\"; }",
        "export function f(a: string, b: string): void {}",
        "export function f<T>(x: T): i32 { return 1; }",
    ] {
        let src = format!("{decl}{MAIN}");
        assert_eq!(codes(&src).len(), 0, "{decl}\n{}", show(&src));
    }
}

#[test]
fn host_generic_gets_one_error() {
    assert_eq!(e0121(&format!("@host export function f<T>(x: T): i32 {{ return 1; }}{MAIN}")), 1);
}

#[test]
fn fitting_host_functions_are_accepted() {
    let src = format!(
        "enum Color {{ Red, Green }}\n@host export function a(x: i32, y: f64, b: bool): i64 {{ return 1i64; }}\n@host export function b(s: string): void {{}}\n@host export function c(d: u8[], e: Color): Color {{ return e; }}\n@host @export_name(\"init\") export function d(): void {{}}{MAIN}"
    );
    assert_eq!(codes(&src).len(), 0, "{}", show(&src));
}

#[test]
fn non_exported_functions_are_unconstrained() {
    let src = format!("struct P {{ x: i32; }}\nfunction f(p: P): P {{ return p; }}{MAIN}");
    assert_eq!(codes(&src).len(), 0, "{}", show(&src));
}

// ---- `@host` misuse: exactly one E0121 each ----

#[test]
fn host_misuse_is_one_error() {
    for (what, decl) in [
        ("not exported", "@host function f(): void {}"),
        ("nested", "export function g(): void { @host function h(): void {} }"),
        ("method", "struct S { x: i32; @host get(): i32 { return 1; } }"),
        ("constructor-less struct", "@host struct S { x: i32; }"),
        ("variable", "@host export const k: i32 = 1;"),
        ("on main", "@host export function main(): void {}"),
    ] {
        let src = if what == "on main" { decl.to_string() } else { format!("{decl}{MAIN}") };
        assert_eq!(e0121(&src), 1, "{what}: {}", show(&src));
    }
}

#[test]
fn host_statement_attribute_is_rejected() {
    let src = format!("export function g(): void {{ @host let x: i32 = 1; }}{MAIN}");
    assert_eq!(e0121(&src), 1, "{}", show(&src));
}

#[test]
fn host_with_arguments_is_e0123() {
    let src = format!("@host(1) export function f(): void {{}}{MAIN}");
    assert_eq!(count(&src, "OXS-E0123"), 1, "{}", show(&src));
}

#[test]
fn export_name_requires_host() {
    let src = format!("@export_name(\"x\") export function f(): void {{}}{MAIN}");
    assert_eq!(count(&src, "OXS-E0123"), 1, "{}", show(&src));
    let ok = format!("@host @export_name(\"x\") export function f(): void {{}}{MAIN}");
    assert_eq!(codes(&ok).len(), 0, "{}", show(&ok));
}

#[test]
fn host_in_a_non_entry_module_is_rejected() {
    let mut l = MapLoader::default();
    l.0.insert("util".into(), "@host export function a(): i32 { return 1; }\n".into());
    let c = check_program("main", "import { a } from \"util\";\nexport function main(): void { a(); }\n", &mut l, &hosts());
    let n = c.diagnostics.iter().filter(|d| d.code == "OXS-E0121").count();
    assert_eq!(n, 1, "{:?}", c.diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>());
}

#[test]
fn reserved_export_names_are_rejected() {
    for n in ["main", "memory", "__heap_base", "__data_end", "oxi_init", "oxi_abi", "oxi_x"] {
        let src = format!("@host @export_name(\"{n}\") export function f(): void {{}}{MAIN}");
        assert_eq!(count(&src, "OXS-E0123"), 1, "{n}: {}", show(&src));
    }
    for n in ["step", "init", "oxi", "heap_base", "mainx"] {
        let src = format!("@host @export_name(\"{n}\") export function f(): void {{}}{MAIN}");
        assert_eq!(codes(&src).len(), 0, "{n}: {}", show(&src));
    }
}
