#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! Host export rule: `export` is module visibility only; Wasm exports are `main` plus `@host` functions.
mod common;
use common::*;
use oxiscript_runtime::Value;

const HDR: &str = "import { Log } from \"host:log\";\n";

#[test]
fn only_host_functions_are_wasm_exports() {
    let src = format!(
        "{HDR}export function plain(x: i32): i32 {{ return x + 1; }}\n@host export function callable(x: i32): i32 {{ return plain(x) * 2; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n"
    );
    let mut p = Prog::new(&src);
    assert_eq!(p.rt.user_exports(), vec!["callable".to_string()], "exports of the program");
    assert_eq!(p.call("callable", &[Value::I32(4)]), vec![Value::I32(10)]);
    assert!(p.rt.call_export("plain", &[Value::I32(1)]).is_err());
    p.finish();
}

#[test]
fn unfit_and_generic_exports_are_ordinary_internal_functions() {
    run_log(
        &format!(
            "{HDR}struct P {{ x: i32; }}\nexport function mk(x: i32): P {{ return {{ x: x }}; }}\nexport function first<T>(a: T, b: T): T {{ return a; }}\nexport function describe(p: P): string {{ return `p${{p.x}}`; }}\nexport function main(log: Log): void {{ log.write(2, describe(mk(first(3, 4)))); log.write(2, first(\"a\", \"b\")); }}\n"
        ),
        &["p3", "a"],
    );
}

#[test]
fn export_name_is_the_wasm_name() {
    let src = format!(
        "{HDR}@host @export_name(\"step_it\") export function step(dt: f64): i32 {{ return 7; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n"
    );
    let mut p = Prog::new(&src);
    assert_eq!(p.rt.user_exports(), vec!["step_it".to_string()]);
    assert_eq!(p.call("step_it", &[Value::F64(1.0)]), vec![Value::I32(7)]);
    p.finish();
}

fn compile_err(src: &str) -> String {
    match oxiscript_lower::compile_source(src, &interfaces(), &oxiscript_lower::Options::default()) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("must be rejected:\n{src}"),
    }
}

#[test]
fn export_name_cannot_forge_reserved_or_empty_names() {
    for n in ["memory", "oxi_init", "oxi_alloc", "cb:x", "cb:host:log@1.Log.X", "", "oxi_tick"] {
        let src = format!(
            "{HDR}@host @export_name(\"{n}\") export function f(): i32 {{ return 1; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n"
        );
        let t = compile_err(&src);
        assert_ne!(t.len(), 0, "export_name({n:?})");
    }
}

#[test]
fn export_name_collisions_are_rejected() {
    let two = format!(
        "{HDR}@host @export_name(\"x\") export function a(): i32 {{ return 1; }}\n@host @export_name(\"x\") export function b(): i32 {{ return 2; }}\nexport function main(log: Log): void {{}}\n"
    );
    assert_ne!(compile_err(&two).len(), 0);
    let vs_name = format!(
        "{HDR}@host @export_name(\"b\") export function a(): i32 {{ return 1; }}\n@host export function b(): i32 {{ return 2; }}\nexport function main(log: Log): void {{}}\n"
    );
    assert_ne!(compile_err(&vs_name).len(), 0);
}

#[test]
fn unreachable_plain_export_is_not_lowered() {
    // a plain export that nothing calls costs nothing and cannot fail lowering
    let src = format!("{HDR}export function dead(x: i32): i32 {{ return x; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n");
    let mut p = Prog::new(&src);
    assert_eq!(p.rt.user_exports().len(), 0);
    p.finish();
}
