#![allow(clippy::format_push_string)]
mod common;
use common::*;
use oxiscript_check::*;

fn check(entry: &str, mods: &[(&str, &str)]) -> Checked {
    let mut l = MapLoader::default();
    for (n, s) in mods {
        l.0.insert((*n).into(), (*s).into());
    }
    check_program("main", entry, &mut l, &hosts())
}

fn errs(c: &Checked) -> Vec<String> {
    c.errors().map(|d| format!("{}: {}", d.code, d.message)).collect()
}

#[test]
fn imports_exports_and_namespaces() {
    let c = check(
        "import { add as plus, Pt } from \"./math\";\nimport * as m from \"./math\";\nexport function f(): i32 {\n  const p: Pt = { x: 1, y: 2 };\n  return plus(p.x, p.y) + m.add(1, 2) + m.ONE;\n}\n",
        &[("math", "export function add(a: i32, b: i32): i32 { return a + b; }\nexport struct Pt { x: i32; y: i32; }\nexport const ONE: i32 = 1;\n")],
    );
    assert!(c.ok(), "{:?}", errs(&c));
    assert_eq!(c.modules.len(), 3); // main, math, prelude
    assert_eq!(c.prelude, Some(2));
}

#[test]
fn importing_a_non_exported_name_is_an_error() {
    let c = check("import { hidden } from \"./lib\";\n", &[("lib", "function hidden(): void {}\n")]);
    assert_eq!(errs(&c).len(), 1, "{:?}", errs(&c));
    assert!(errs(&c)[0].starts_with("OXS-E0302"));
}

#[test]
fn cyclic_imports_are_rejected() {
    let c = check(
        "import { a } from \"./a\";\n",
        &[("a", "import { b } from \"./b\";\nexport function a(): void {}\n"), ("b", "import { a } from \"./a\";\nexport function b(): void {}\n")],
    );
    assert!(errs(&c).iter().any(|e| e.starts_with("OXS-E0302") && e.contains("cyclic")), "{:?}", errs(&c));
}

#[test]
fn errors_in_imported_modules_are_reported() {
    let c = check("import { a } from \"./a\";\n", &[("a", "export function a(): i32 { return \"x\"; }\n")]);
    assert!(errs(&c).iter().any(|e| e.starts_with("OXS-E0100")), "{:?}", errs(&c));
    assert!(c.errors().any(|d| d.span.file.0 == 1));
}

#[test]
fn syntax_errors_stop_checking() {
    let c = check("function f( {", &[]);
    assert!(!c.ok());
    assert_eq!(c.modules.len(), 0, "a program with syntax errors is not checked");
}

#[test]
fn inferred_return_types_do_not_see_the_callers_locals() {
    // `helper` is checked lazily from inside `f` while `f`'s locals are in scope; it must not see `secret`
    let c = check(
        "import { helper } from \"./lib\";\nexport function f(): i32 {\n  const secret = 5;\n  return helper();\n}\n",
        &[("lib", "let state = 3;\nexport function helper() { return secret; }\n")],
    );
    let e = errs(&c);
    assert!(e.iter().any(|x| x.starts_with("OXS-E0300")), "{e:?}");
}

#[test]
fn return_types_are_inferred_across_modules() {
    let c = check(
        "import { make } from \"./lib\";\nfunction f(): string {\n  return make().name;\n}\n",
        &[("lib", "export struct U { name: string; }\nexport function make(): U { return { name: \"a\" }; }\n")],
    );
    assert!(c.ok(), "{:?}", errs(&c));
    // a non-exported function without a return type is inferred in its own module
    let c = check(
        "function helper() { return 1 + 2; }\nexport function f(): i32 { return helper(); }\n",
        &[],
    );
    assert!(c.ok(), "{:?}", errs(&c));
}

#[test]
fn recursion_needs_a_return_type() {
    assert_eq!(codes("function f(n: i32) {\n  if (n === 0) { return 0; }\n  return f(n - 1); //! E0103\n}").len(), 1);
    assert_ok("function f(n: i32): i32 {\n  if (n === 0) { return 0; }\n  return f(n - 1);\n}");
}

#[test]
fn use_before_declaration_is_fine_at_module_level() {
    assert_ok("function a(): i32 { return b(); }\nfunction b(): i32 { return c; }\nconst c: i32 = 3;\n");
}
