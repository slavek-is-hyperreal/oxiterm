//! Host-construction self-probe (BAD ACTOR F1d): whatever wasmi Cargo features the build ended up
//! with (per package, `--features wasmi/simd,wasmi/memory64`, workspace unification), every host
//! must REFUSE modules using SIMD, relaxed-SIMD, table64, memory64, threads, GC and non-function
//! imports, with any manifest, as full ABI modules that would otherwise load and run.
#![allow(clippy::pedantic)]
mod common;

use common::Fixture;
use oxiscript_abi::target::{Feature, Target};
use oxiscript_runtime::{Grants, Host, HostOptions, Limits};

const PROBES: &[(&str, &str)] = &[
    ("simd", r#"(func (export "probe") (result i32) v128.const i64x2 7 9 i32x4.extract_lane 0)"#),
    ("relaxed-simd", r#"(func (export "probe") (result i32) v128.const i64x2 0 0 i32x4.relaxed_trunc_f32x4_s i32x4.extract_lane 0)"#),
    (
        "table64",
        r#"(table $t i64 1 funcref) (elem (table $t) (i64.const 0) func $seven) (type $v (func (result i32))) (func $seven (result i32) i32.const 7)
           (func (export "probe") (result i32) i64.const 0 call_indirect $t (type $v))"#,
    ),
    ("table64-unused", "(table i64 1 funcref)"),
    ("memory64", "(memory i64 1)"),
    ("multi-memory", "(memory 1)"),
    ("shared-memory+atomics", r#"(memory 1 1 shared) (func (export "probe") (param i32) (result i32) local.get 0 i32.atomic.load)"#),
    ("gc", "(type (struct (field i32)))"),
];

const IMPORT_PROBES: &[(&str, &str)] = &[
    ("memory-import", r#"(import "env" "m" (memory 1))"#),
    ("table-import", r#"(import "env" "t" (table 1 funcref))"#),
    ("global-import", r#"(import "env" "g" (global i32))"#),
];

fn manifests() -> Vec<(String, String)> {
    let mut v = vec![("none-declared".to_string(), String::new())];
    for t in Target::ALL {
        v.push((format!("{t}"), format!("target {t}\n")));
        let feats: Vec<&str> = Feature::ALL.iter().filter(|f| t.allows(**f)).map(|f| f.name()).collect();
        if !feats.is_empty() {
            v.push((format!("{t}+all"), format!("target {t}\nfeatures {}\n", feats.join(" "))));
        }
    }
    v
}

#[test]
fn no_host_runs_a_feature_outside_abi_1_0() {
    let mut bad = vec![];
    let all = PROBES.iter().map(|(n, b)| (*n, "", *b)).chain(IMPORT_PROBES.iter().map(|(n, i)| (*n, *i, "")));
    for (name, imports, body) in all {
        for (mname, extra) in manifests() {
            let mut f = Fixture::new(&[]).imports(imports).body(body);
            f.manifest = Some(format!("{}{extra}", f.manifest_text()));
            let wasm = f.bytes();
            for h in Target::ALL {
                let host = Host::with_options(Limits::default(), HostOptions { max_target: h, ..HostOptions::default() });
                if host.instantiate(&wasm, Grants::all()).is_ok() {
                    bad.push(format!("{name} / manifest {mname} / host {h}: LOADED"));
                }
            }
        }
    }
    assert_eq!(bad.len(), 0, "hosts loaded modules outside ABI 1.0:\n{}", bad.join("\n"));
}

/// Even a Server host (the most permissive) refuses SIMD / relaxed-SIMD / table64 end to end. The
/// ENGINE layer alone is tested in `runtime.rs` (`engine_config_tests`), bypassing `check_target`.
#[test]
fn server_host_refuses_simd_and_table64_end_to_end() {
    for (name, body) in &PROBES[..3] {
        let f = Fixture::new(&[]).body(body);
        let wasm = f.bytes();
        let host = Host::with_options(Limits::default(), HostOptions { max_target: Target::Server, ..HostOptions::default() });
        let e = host.instantiate(&wasm, Grants::all()).err().unwrap_or_else(|| panic!("{name} loaded"));
        assert!(matches!(e.code(), "OXS-E0408" | "OXS-E0411" | "OXS-E0402"), "{name}: {e}");
    }
}

/// F5: the manifest cap is applied before any parsing (a huge section is cheap and E0402).
#[test]
fn oversized_manifest_is_refused_before_parsing() {
    let mut f = Fixture::new(&[]);
    let mut m = f.manifest_text();
    m.push_str(&"\n".repeat(3_000_000));
    f.manifest = Some(m);
    let wasm = f.bytes();
    let t = std::time::Instant::now();
    let e = Host::new(Limits::default()).instantiate(&wasm, Grants::all()).err().expect("refused");
    assert_eq!(e.code(), "OXS-E0402", "{e}");
    assert!(t.elapsed().as_millis() < 500, "took {:?}", t.elapsed());
}
