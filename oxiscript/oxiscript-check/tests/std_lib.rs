//! Tests for Step 1 (std module resolution) and Step 2 (`@intrinsic` gating and std/math).

use oxiscript_check::{check_program, check_source, CheckOptions, MapLoader};

fn opts() -> CheckOptions {
    CheckOptions::default()
}

#[test]
fn std_modules_resolve_with_check_source_and_all_std_files_check_cleanly() {
    let src = r#"
import * as m from "std/math";
import * as s from "std/string";
import * as a from "std/array";
import * as r from "std/result";
import * as c from "std/collections";

export function test(x: f64): f64 {
  const _a = s.isEmpty("");
  const _b = a.isEmpty([1, 2]);
  const _c = r.isOk(Ok<i32, string>(1));
  return m.sqrt(m.abs(x)) + m.PI;
}
"#;
    let ck = check_source(src, &opts());
    assert!(ck.ok(), "diagnostics: {:?}", ck.diagnostics);
}

#[test]
fn unknown_std_module_gives_clear_diagnostic() {
    let ck = check_source("import { foo } from \"std/nope\";\n", &opts());
    assert!(!ck.ok());
    let d = ck.errors().next().expect("expected error");
    assert_eq!(d.code, "OXS-E0302");
    assert!(
        d.message.contains("unknown standard library module") && d.message.contains("std/math"),
        "unexpected message: {}",
        d.message
    );
}

#[test]
fn user_cannot_shadow_std_module_via_loader() {
    let mut loader = MapLoader::default();
    loader.0.insert(
        "std/math".to_string(),
        "export function hacked(): i32 { return 1; }\n".to_string(),
    );
    let ck = check_program(
        "main",
        "import { hacked } from \"std/math\";\n",
        &mut loader,
        &opts(),
    );
    assert!(!ck.ok(), "loader must not shadow std/math");
    assert!(ck.errors().any(|d| d.code == "OXS-E0302"));
}

#[test]
fn intrinsic_attribute_is_rejected_in_user_code() {
    let src = "@intrinsic(\"sqrt\")\nexport function mySqrt(x: f64): f64 { return x; }\n";
    let ck = check_source(src, &opts());
    assert!(!ck.ok());
    let d = ck.errors().next().expect("expected error");
    assert_eq!(d.code, "OXS-E0123");
    assert!(
        d.message.contains("@intrinsic") && d.message.contains("standard library"),
        "unexpected message: {}",
        d.message
    );
}

#[test]
fn intrinsic_attribute_cannot_be_forged_by_entry_name() {
    let src = "@intrinsic(\"sqrt\")\nexport function mySqrt(x: f64): f64 { return x; }\n";
    let mut loader = MapLoader::default();
    let ck = check_program("std/math", src, &mut loader, &opts());
    assert!(!ck.ok(), "entry module named std/math must not be trusted for @intrinsic");
    assert!(ck.errors().any(|d| d.code == "OXS-E0123"));
}
