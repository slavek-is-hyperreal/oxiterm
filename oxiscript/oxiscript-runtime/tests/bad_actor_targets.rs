//! BAD ACTOR attacks on the compilation-targets stage. Tests that FAIL today are findings
//! (named `fN_...`); passing tests document what held up. Layer attribution is read from the
//! rejection message (pre-scan / check_target / engine / load::check).
#![allow(clippy::pedantic, dead_code)]
mod common;

use common::append_custom;
use oxiscript_abi::target::Target;
use oxiscript_runtime::{Grants, Host, HostOptions, Limits};

fn host_for(max: Target) -> Host {
    Host::with_options(Limits::default(), HostOptions { max_target: max, ..HostOptions::default() })
}

/// Module = wat body + raw manifest text (None = no manifest section).
fn module(body: &str, manifest: Option<&str>) -> Vec<u8> {
    let mut b = wat::parse_str(format!("(module {body})")).unwrap_or_else(|e| panic!("wat: {e}\n{body}"));
    if let Some(m) = manifest {
        append_custom(&mut b, "oxi.manifest", m.as_bytes());
    }
    b
}

fn mf(extra: &str) -> String {
    format!("oxi-manifest 1\nabi 1.0\nedition 2026\nprofile default\n{extra}")
}

/// (code, message) of a load, or ("loaded", "") if the engine + load::check accepted.
/// A module that is only a body has no ABI exports, so passing the engine/target layers shows
/// up as an E04xx code other than E0408/E0411/E0402 (E0406 = missing exports) = "reached load::check".
fn load(wasm: &[u8], max: Target) -> (String, String) {
    match host_for(max).instantiate(wasm, Grants::all()) {
        Ok(_) => ("loaded".into(), String::new()),
        Err(e) => (e.code().to_string(), e.to_string()),
    }
}

/// true when the module got past pre-scan, check_target and the engine (reached `load::check`).
fn engine_accepted(c: &str) -> bool {
    !matches!(c, "OXS-E0408" | "OXS-E0411" | "OXS-E0402" | "OXS-E0409")
}

struct Probe {
    name: &'static str,
    body: &'static str,
    /// The wasm features the code needs beyond the Device base (None = never allowed anywhere).
    needs: Option<&'static [&'static str]>,
}

const PROBES: &[Probe] = &[
    Probe { name: "base", body: "", needs: Some(&[]) },
    Probe { name: "simd", body: "(func (result i32) v128.const i64x2 0 0 i32x4.extract_lane 0)", needs: Some(&["simd128"]) },
    Probe { name: "tail-call", body: "(func $f return_call $f)", needs: Some(&["tail-call"]) },
    Probe { name: "extended-const", body: "(global i32 (i32.add (i32.const 1) (i32.const 2)))", needs: Some(&["extended-const"]) },
    Probe { name: "exceptions", body: "(tag $t) (func try_table (catch_all 0) end)", needs: Some(&["exceptions"]) },
    Probe { name: "multi-memory", body: "(memory 1) (memory 1)", needs: Some(&["multi-memory"]) },
    Probe { name: "memory64", body: "(memory i64 1)", needs: Some(&["memory64"]) },
    Probe { name: "table64", body: "(table i64 1 funcref)", needs: Some(&["memory64"]) },
    Probe { name: "reftypes-extern", body: "(table 1 externref)", needs: None },
    Probe { name: "reftypes-2tables", body: "(table 1 funcref) (table 1 funcref)", needs: None },
    Probe { name: "ref-null-func", body: "(func (result funcref) ref.null func)", needs: None },
    Probe { name: "trunc_sat", body: "(func (param f32) (result i32) local.get 0 i32.trunc_sat_f32_s)", needs: None },
    Probe { name: "gc-struct", body: "(type (struct))", needs: None },
    Probe { name: "atomics", body: "(memory 1) (func (param i32) (result i32) local.get 0 i32.atomic.load)", needs: None },
    Probe { name: "shared-mem", body: "(memory 1 1 shared)", needs: None },
    Probe { name: "relaxed-simd", body: "(func (result v128) v128.const i64x2 0 0 i32x4.relaxed_trunc_f32x4_s)", needs: None },
    Probe { name: "wide-arith", body: "(func (param i64 i64 i64 i64) (result i64 i64) local.get 0 local.get 1 local.get 2 local.get 3 i64.add128)", needs: None },
    Probe { name: "custom-page", body: "(memory 1 (pagesize 1))", needs: None },
    Probe { name: "typed-funcref", body: "(type $t (func)) (func (param (ref null $t)))", needs: None },
    Probe { name: "legacy-eh", body: "(tag $t) (func try catch_all end)", needs: None },
    Probe { name: "return-call-indirect", body: "(table 1 funcref) (type $t (func)) (func i32.const 0 return_call_indirect (type $t))", needs: Some(&["tail-call"]) },
];

/// Print the whole matrix (probe x declared manifest x host) with the refusing layer.
#[test]
fn matrix() {
    let hosts = Target::ALL;
    for p in PROBES {
        let manifests: Vec<(String, String)> = vec![
            ("device".into(), mf("")),
            ("web".into(), mf("target web\n")),
            ("web+tail".into(), mf("target web\nfeatures tail-call\n")),
            ("server+all".into(), mf("target server\nfeatures tail-call extended-const multi-memory\n")),
        ];
        for (mname, m) in manifests {
            let w = module(p.body, Some(&m));
            let row: Vec<String> = hosts
                .iter()
                .map(|h| {
                    let (c, msg) = load(&w, *h);
                    format!("{h}:{}{}", c.trim_start_matches("OXS-"), if c == "OXS-E0408" { format!("[{}]", msg[..msg.len().min(60)].replace('\n', " ")) } else { String::new() })
                })
                .collect();
            println!("{:<22} {:<11} {}", p.name, mname, row.join("  "));
        }
    }
}

/// CORE: no module may get past the target/engine layers on a host unless every feature its code
/// needs is (declared in its manifest) AND (allowed by the host's max target) AND (in the engine).
#[test]
fn f1_nothing_undeclared_reaches_the_engine() {
    let engine = ["tail-call", "extended-const", "multi-memory"];
    let allowed_by_host = |f: &str, h: Target| match f {
        "simd128" | "exceptions" | "tail-call" | "extended-const" => h >= Target::Web,
        "memory64" | "multi-memory" => h >= Target::Server,
        _ => false,
    };
    let manifests: Vec<(&str, String, Target, Vec<&str>)> = vec![
        ("device", mf(""), Target::Device, vec![]),
        ("web", mf("target web\n"), Target::Web, vec![]),
        ("web+tail", mf("target web\nfeatures tail-call\n"), Target::Web, vec!["tail-call"]),
        ("server+all", mf("target server\nfeatures tail-call extended-const multi-memory\n"), Target::Server, vec!["tail-call", "extended-const", "multi-memory"]),
        ("server+mem64", mf("target server\nfeatures memory64\n"), Target::Server, vec!["memory64"]),
        ("web+simd+eh", mf("target web\nfeatures simd128 exceptions\n"), Target::Web, vec!["simd128", "exceptions"]),
    ];
    let mut bad = vec![];
    for p in PROBES {
        for (mname, m, mt, declared) in &manifests {
            let w = module(p.body, Some(m));
            for h in Target::ALL {
                let (c, msg) = load(&w, h);
                let ok_expected = match p.needs {
                    None => false,
                    Some(n) => n.iter().all(|f| declared.contains(f) && allowed_by_host(f, h) && engine.contains(f)) && *mt <= h,
                };
                // the ABI 1.0 rule also forbids memory64 / 2nd memory etc. at load::check; only judge the policy layers
                if !ok_expected && engine_accepted(&c) {
                    bad.push(format!("{} / manifest {mname} / host {h}: {c} {}", p.name, &msg[..msg.len().min(80)]));
                }
            }
        }
    }
    assert!(bad.is_empty(), "modules passed the policy layers with features outside policy:\n{}", bad.join("\n"));
}

/// Full ABI modules that use a forbidden feature in an *exported, called* function.
/// Under the default per-package build the engine lacks simd/memory64 and refuses (this passes);
/// when Cargo feature unification turns wasmi's `simd`/`memory64` on (workspace-wide build/test,
/// `cargo test -p oxiscript-runtime -p oxiscript-rc`, `--features wasmi/simd,wasmi/memory64`) a Device
/// host runs them, because `Host::with_options` never calls `wasm_simd(false)`/`wasm_memory64(false)`.
#[test]
fn f1_device_host_runs_simd_and_table64_under_feature_unification() {
    use common::{env, Fixture};
    use oxiscript_runtime::Value;
    let e = env();
    let _ = &e;
    let mut ran = vec![];
    for (name, body, call) in [
        ("simd", r#"(func (export "probe") (result i32) v128.const i64x2 7 9 i32x4.extract_lane 0)"#, "probe"),
        ("relaxed-simd", r#"(func (export "probe") (result i32) v128.const i64x2 0 0 i32x4.relaxed_trunc_f32x4_s i32x4.extract_lane 0)"#, "probe"),
        ("table64", r#"(table $t i64 1 funcref) (elem (table $t) (i64.const 0) func $seven) (type $v (func (result i32))) (func $seven (result i32) i32.const 7)
                       (func (export "probe") (result i32) i64.const 0 call_indirect $t (type $v))"#, "probe"),
    ] {
        let f = Fixture::new(&[]).body(body);
        let host = host_for(Target::Device);
        if let Ok(mut rt) = host.instantiate(&f.bytes(), Grants::all()) {
            let r = rt.call_export(call, &[]);
            ran.push(format!("{name}: device host loaded and ran it -> {r:?}"));
        }
    }
    let _ = Value::I32(0);
    assert!(ran.is_empty(), "a Device host ran code outside the Device feature set:\n{}", ran.join("\n"));
}

/// F2 (fixed by REMOVING multi-memory: ABI 1.0 has one memory, abi.md 3.6): the policy table and the
/// behaviour must agree. A module declaring `multi-memory` is refused by the manifest/policy layer
/// (E0402/E0411), never advertised-then-refused by the engine (E0408 "exceeds the limit of 1 memories").
/// The original test asserted acceptance; with the feature gone from the table that is no longer the
/// safe behaviour, the assertion now is "refused before the engine, with a policy code".
#[test]
fn f2_server_multi_memory_is_advertised_but_unusable() {
    use common::Fixture;
    let mut f = Fixture::new(&[]).body("(memory $m2 1)");
    f.manifest = Some(format!("{}target server\nfeatures multi-memory\n", mf("")));
    let r = host_for(Target::Server).instantiate(&f.bytes(), Grants::all());
    let code = match &r { Ok(_) => "ok".to_string(), Err(e) => format!("{} {}", e.code(), e) };
    assert!(
        matches!(&r, Err(e) if matches!(e.code(), "OXS-E0402" | "OXS-E0411")),
        "multi-memory must be refused by the policy layer, not advertised: {code}"
    );
}

/// Differential fuzz: wasm-smith modules (all proposals) x random honest/dishonest manifests x hosts.
/// Oracle: if a module gets past pre-scan, check_target and the engine (any code other than
/// E0407/E0408/E0409/E0402/E0411), an independent wasmparser validation against
/// (Device base + declared features that the host allows and the engine has) must succeed.
#[test]
fn f3_smith_differential_policy_oracle() {
    use arbitrary::{Arbitrary, Unstructured};
    use oxiscript_abi::manifest::Manifest;
    use oxiscript_abi::target::Feature;
    use wasmparser::{Validator, WasmFeatures};
    let n: u64 = std::env::var("BA_SMITH_CASES").ok().and_then(|v| v.parse().ok()).unwrap_or(if cfg!(debug_assertions) { 3_000 } else { 60_000 });
    let engine = [Feature::TailCall, Feature::ExtendedConst];
    let flags = |f: Feature| match f {
        Feature::Simd128 => WasmFeatures::SIMD,
        Feature::Exceptions => WasmFeatures::EXCEPTIONS | WasmFeatures::REFERENCE_TYPES,
        Feature::TailCall => WasmFeatures::TAIL_CALL,
        Feature::ExtendedConst => WasmFeatures::EXTENDED_CONST,
        Feature::Memory64 => WasmFeatures::MEMORY64,
    };
    let mut rng = 0x1234_5678_9ABC_DEF1u64;
    let mut next = move || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let hosts: Vec<Host> = Target::ALL.iter().map(|t| host_for(*t)).collect();
    let (mut gen, mut passed, mut bad) = (0u64, 0u64, vec![]);
    let mut codes = std::collections::BTreeMap::<String, u64>::new();
    for seed in 0..n {
        let raw: Vec<u8> = (0..4096).map(|_| (next() >> 24) as u8).collect();
        let mut u = Unstructured::new(&raw);
        let mut cfg = wasm_smith::Config::arbitrary(&mut u).unwrap_or_default();
        cfg.allow_invalid_funcs = false;
        cfg.max_imports = 4;
        cfg.min_exports = 0;
        let Ok(m) = wasm_smith::Module::new(cfg, &mut u) else { continue };
        let mut wasm = m.to_bytes();
        gen += 1;
        let t = Target::ALL[(next() % 3) as usize];
        let mut man = Manifest::new(2026, "default");
        man.target = t;
        man.features = Feature::ALL.iter().copied().filter(|f| t.allows(*f) && next() % 2 == 0).collect();
        append_custom(&mut wasm, "oxi.manifest", man.print().as_bytes());
        for (hi, h) in hosts.iter().enumerate() {
            let host_t = Target::ALL[hi];
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.instantiate(&wasm, Grants::all())));
            let c = match r {
                Err(_) => {
                    bad.push(format!("seed {seed}: PANIC on {host_t}"));
                    continue;
                }
                Ok(Ok(_)) => "loaded".to_string(),
                Ok(Err(e)) => e.code().to_string(),
            };
            *codes.entry(c.clone()).or_default() += 1;
            if matches!(c.as_str(), "OXS-E0407" | "OXS-E0408" | "OXS-E0409" | "OXS-E0402" | "OXS-E0411") {
                continue;
            }
            passed += 1;
            let mut allowed = WasmFeatures::WASM1 | WasmFeatures::MULTI_VALUE | WasmFeatures::BULK_MEMORY | WasmFeatures::SIGN_EXTENSION;
            for f in &man.features {
                if host_t.allows(*f) && engine.contains(f) && t <= host_t {
                    allowed |= flags(*f);
                }
            }
            if Validator::new_with_features(allowed).validate_all(&wasm).is_err() {
                bad.push(format!("seed {seed}: {t} {:?} on {host_t}: {c} but code needs more than declared+host+engine", man.features));
            }
        }
    }
    println!("smith generated={gen} reached-load::check={passed} codes={codes:?}");
    assert!(bad.is_empty(), "{} violations, first: {:#?}", bad.len(), &bad[..bad.len().min(5)]);
}

/// Honesty inversion: an honest module must never fare worse than a lying one. Fails only when the
/// engine has SIMD (Cargo unification, `--features wasmi/simd`): `target web` + `features simd128` is
/// refused (E0411, ENGINE_FEATURES lacks simd) while the same code with an undeclared manifest loads.
#[test]
fn f1c_honest_declaration_is_not_punished_relative_to_lying() {
    let body = r#"(func (export "probe") (result i32) v128.const i64x2 7 9 i32x4.extract_lane 0)"#;
    let honest = {
        let mut f = common::Fixture::new(&[]).body(body);
        f.manifest = Some(mf("target web\nfeatures simd128\n"));
        f.bytes()
    };
    let lying = {
        let mut f = common::Fixture::new(&[]).body(body);
        f.manifest = Some(mf("target web\n"));
        f.bytes()
    };
    let h = host_for(Target::Web).instantiate(&honest, Grants::all()).is_ok();
    let l = host_for(Target::Web).instantiate(&lying, Grants::all()).is_ok();
    assert!(!(l && !h), "lying module (no `features`) loads on a Web host, the honest one (`features simd128`) is refused");
}
