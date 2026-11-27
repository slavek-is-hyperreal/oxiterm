#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! BAD ACTOR 8.Std: module resolution, shadowing, capability purity and doc/code agreement.
use oxiscript_check::{check_program, check_source, CheckOptions, MapLoader};
use std::collections::BTreeSet;

fn opts() -> CheckOptions { CheckOptions::default() }

#[test]
fn std_sources_do_not_reach_host_modules() {
    for name in ["math", "string", "array", "result", "collections"] {
        let src = std::fs::read_to_string(format!("{}/../std/{name}.oxs", env!("CARGO_MANIFEST_DIR"))).unwrap();
        assert!(!src.contains("host:"), "std/{name} mentions host:");
    }
}

#[test]
fn path_traversal_and_odd_std_specs_are_rejected() {
    for spec in ["std/../x", "std/math/../string", "std//math", "std/math.oxs", "std/Math", "std/", "std", "std/math/", "./std/nope", "std/./math", "STD/math", "std\\math"] {
        let ck = check_source(&format!("import {{ sqrt }} from \"{spec}\";\n"), &opts());
        // each must either fail with a diagnostic or (for exactly the canonical form) succeed; none may panic
        eprintln!("spec {spec:?}: ok={} codes={:?}", ck.ok(), ck.diagnostics.iter().map(|d| d.code).collect::<Vec<_>>());
    }
    for spec in ["std/../x", "std/math/../string", "std//math", "std/math.oxs", "std/Math", "std/", "std", "std/math/", "std/./math"] {
        let ck = check_source(&format!("import {{ sqrt }} from \"{spec}\";\n"), &opts());
        assert!(!ck.ok(), "{spec:?} unexpectedly resolved");
    }
}

/// BS: a user module reached by a relative spec gets the canonical name `std/math`. Which sqrt does `import ... from "std/math"` see?
#[test]
fn relative_spec_user_module_must_not_shadow_std() {
    let mut l = MapLoader::default();
    l.0.insert("std/math".into(), "export function sqrt(): i32 { return 7; }\n".into());
    let ck = check_program("main", "import { sqrt as a } from \"./std/math\";\nimport { sqrt as b } from \"std/math\";\nexport function f(): f64 { return b(4.0) + (a() as f64); }\n", &mut l, &opts());
    assert!(!ck.ok(), "import from \"std/math\" silently resolved to the user's module (or both collided) with no diagnostic: {:?}", ck.diagnostics);
}

/// BS: can a user module smuggle @intrinsic by taking the canonical name std/math through a relative import?
#[test]
fn relative_spec_user_module_cannot_forge_intrinsic() {
    let mut l = MapLoader::default();
    l.0.insert("std/math".into(), "@intrinsic(\"sqrt\")\nexport function fake(x: f64): f64 { return x; }\n".into());
    let ck = check_program("main", "import { fake } from \"./std/math\";\nexport function f(): f64 { return fake(4.0); }\n", &mut l, &opts());
    assert!(ck.errors().any(|d| d.code == "OXS-E0123"), "user module named std/math accepted @intrinsic: {:?}", ck.diagnostics);
}

#[test]
fn entry_module_named_std_cannot_export_intrinsic_via_nested_std_name() {
    for name in ["std/string", "std/array", "std/collections", "std/x", "std"] {
        let src = "@intrinsic(\"abs\")\nexport function myAbs(x: f64): f64 { return x; }\n";
        let ck = check_program(name, src, &mut MapLoader::default(), &opts());
        assert!(ck.errors().any(|d| d.code == "OXS-E0123"), "entry {name} accepted @intrinsic");
    }
}

#[test]
fn intrinsic_misuse_in_user_code_variants() {
    for src in [
        "@intrinsic(\"nope\")\nexport function f(x: f64): f64 { return x; }\n",
        "@intrinsic\nexport function f(x: f64): f64 { return x; }\n",
        "@intrinsic(\"sqrt\")\nfunction f(x: f64): f64 { return x; }\n",
        "struct S { x: i32; @intrinsic(\"abs\") f(): i32 { return 1; } }\n",
    ] {
        let ck = check_source(src, &opts());
        assert!(!ck.ok(), "accepted: {src}");
    }
}

fn exported_fns(src: &str) -> BTreeSet<String> {
    src.lines().filter_map(|l| {
        let l = l.trim_start();
        let l = l.strip_prefix("export function ").or_else(|| l.strip_prefix("export const "))?;
        Some(l.split(|c: char| !(c.is_alphanumeric() || c == '_')).next()?.to_string())
    }).collect()
}

/// Every backticked `name(` mentioned in a bullet of std.md must exist as an export of some std module
/// (or a documented built-in); every export must be documented.
#[test]
fn std_md_and_sources_agree() {
    let root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    let md = std::fs::read_to_string(format!("{root}/docs/oxiscript/std.md")).unwrap();
    let mut exports = BTreeSet::new();
    for name in ["math", "string", "array", "result", "collections"] {
        exports.extend(exported_fns(&std::fs::read_to_string(format!("{root}/oxiscript/std/{name}.oxs")).unwrap()));
    }
    let mut missing_in_md = vec![];
    for e in exports.iter().filter(|e| !(e.ends_with("F32") && md.contains("*F32"))) {
        if !md.contains(&format!("`{e}")) && !md.contains(&format!("{e}`")) && !md.contains(&format!("{e}(")) && !md.contains(&format!(" {e},")) && !md.contains(&format!("{e}, ")) {
            missing_in_md.push(e.clone());
        }
    }
    assert_eq!(missing_in_md.len(), 0, "exported but undocumented: {missing_in_md:?}");
    // names the doc lists as functions/constants must exist
    let mut missing_in_code = vec![];
    for l in md.lines().filter(|l| l.trim_start().starts_with('*') || l.starts_with('|')) {
        for part in l.split('`').skip(1).step_by(2) {
            let id: String = part.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
            let looks_fn = part.starts_with(&id) && part[id.len()..].starts_with(['(', '<']) ;
            if looks_fn && !id.is_empty() && !exports.contains(&id) && !["push", "pop", "map", "filter", "reduce", "forEach", "slice", "some", "every", "concat", "startsWith", "endsWith", "includes", "indexOf", "hash", "equals", "toString", "wrapping", "saturating", "downcast", "bytes", "chars", "range", "Ok", "Err", "Result", "Map", "Set", "has", "get", "getOr", "set", "remove", "clear", "keys", "values", "entries", "size", "isEmpty", "add", "Hash", "Eq", "HashEq", "MapEntry"].contains(&id.as_str()) {
                missing_in_code.push(id);
            }
        }
    }
    missing_in_code.sort(); missing_in_code.dedup();
    assert_eq!(missing_in_code.len(), 0, "documented but not exported: {missing_in_code:?}");
}

/// std.md claims `isFinite`/`isNaN`/`isInfinite` have `*F32` variants and tables list F32 constants: they must exist.
#[test]
fn std_md_f32_variants_exist() {
    let src = std::fs::read_to_string(format!("{}/../std/math.oxs", env!("CARGO_MANIFEST_DIR"))).unwrap();
    let ex = exported_fns(&src);
    for n in ["isNaNF32", "isFiniteF32", "isInfiniteF32", "F32_MAX", "F32_MIN_POSITIVE", "FRAC_PI_4", "F64_MIN_POSITIVE", "wrappingAbsI64", "saturatingMulU64"] {
        assert!(ex.contains(n), "{n} documented but missing");
    }
}

/// The documented `Map`/`Set` contract: constructor with a non-HashEq key must be rejected by the checker.
#[test]
fn map_key_without_hasheq_is_rejected() {
    let ck = check_source("import { Map } from \"std/collections\";\nstruct P { x: i32; }\nexport function f(): i32 { const m = new Map<P, i32>(); return m.size(); }\n", &opts());
    assert!(!ck.ok(), "Map<P,..> with P lacking hash/equals was accepted");
}
#[test]
fn map_key_f32_and_user_class_ok_and_array_key_rejected() {
    let ck = check_source("import { Map } from \"std/collections\";\nexport function f(): i32 { const m = new Map<i32[], i32>(); return m.size(); }\n", &opts());
    assert!(!ck.ok(), "Map<i32[],..> accepted although arrays have no hash/equals");
}
