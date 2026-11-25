//! 8.L fix L08: an `export function` of the entry module whose signature does not fit ABI 1.0 is `OXS-E0121`.
mod common;
use common::*;

fn e0121(src: &str) -> usize {
    codes(src).iter().filter(|(c, _)| c == "OXS-E0121").count()
}

const MAIN: &str = "\nexport function main(): void {}\n";

#[test]
fn unfit_exports_are_rejected() {
    for decl in [
        "struct P { x: i32; }\nexport function f(p: P): i32 { return p.x; }",
        "struct P { x: i32; }\nexport function f(x: i32): P { return { x: x }; }",
        "export function f(a: i32[]): i32 { return a.length; }",
        "export function f(a: i32 | null): i32 { return 1; }",
        "export function f(c: () => i32): i32 { return c(); }",
        "export function f(x: i32): string { return \"a\"; }",
        "type U = { kind: \"a\" } | { kind: \"b\" };\nexport function f(u: U): i32 { return 1; }",
    ] {
        assert_eq!(e0121(&format!("{decl}{MAIN}")), 1, "{decl}");
    }
}

#[test]
fn generic_export_gets_one_error() {
    assert_eq!(e0121(&format!("export function f<T>(x: T): i32 {{ return 1; }}{MAIN}")), 1);
}

#[test]
fn fitting_exports_are_accepted() {
    let src = format!(
        "enum Color {{ Red, Green }}\nexport function a(x: i32, y: f64, b: bool): i64 {{ return 1i64; }}\nexport function b(s: string): void {{}}\nexport function c(d: u8[], e: Color): Color {{ return e; }}{MAIN}"
    );
    assert!(codes(&src).is_empty(), "{}", show(&src));
}

#[test]
fn non_exported_functions_are_unconstrained() {
    let src = format!("struct P {{ x: i32; }}\nfunction f(p: P): P {{ return p; }}{MAIN}");
    assert!(codes(&src).is_empty(), "{}", show(&src));
}
