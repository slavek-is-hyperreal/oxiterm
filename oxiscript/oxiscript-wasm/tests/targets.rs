//! Compilation targets: same code on every target (only the manifest differs),
//! per-target validation, and host refusal of a module that exceeds the host's `max_target`.
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_abi::manifest::{Manifest, MANIFEST_SECTION};
use oxiscript_abi::target::{Feature, Target};
use oxiscript_ir::parse_module;
use oxiscript_runtime::{Grants, Host, HostOptions, Limits};
use oxiscript_wasm::{compile, features_for, validate_for, CompileOptions};

const SRC: &str = include_str!("../../oxiscript-ir/tests/programs/loop.oxir");

fn build_for(t: Target) -> Vec<u8> {
    let m = parse_module(SRC).unwrap();
    compile(&m, &CompileOptions { target: t, allow_raw_exports: true, ..opts() }).unwrap()
}

/// (manifest text, the module bytes with the manifest section's payload removed).
fn split(wasm: &[u8]) -> (String, Vec<u8>) {
    use wasmparser::{Parser, Payload};
    let mut text = String::new();
    let mut rest = Vec::new();
    for p in Parser::new(0).parse_all(wasm) {
        match p.unwrap() {
            Payload::CustomSection(c) if c.name() == MANIFEST_SECTION => {
                text = std::str::from_utf8(c.data()).unwrap().to_string();
            }
            Payload::CodeSectionEntry(b) => rest.extend_from_slice(&wasm[b.range().start as usize..b.range().end as usize]),
            Payload::DataSection(r) => rest.extend_from_slice(&wasm[r.range().start as usize..r.range().end as usize]),
            _ => {}
        }
    }
    (text, rest)
}

#[test]
fn all_targets_emit_the_same_code() {
    let dev = build_for(Target::Device);
    let (dm, dcode) = split(&dev);
    assert!(!dm.contains("target"), "a device module has no target line: {dm}");
    for t in [Target::Web, Target::Server] {
        let w = build_for(t);
        let (m, code) = split(&w);
        assert_eq!(code, dcode, "{t}: code and data identical");
        let mf = Manifest::parse(&m).unwrap();
        assert_eq!(mf.target, t);
        assert!(mf.features.is_empty(), "no feature is emitted today");
        // only the manifest section differs in length
        assert_eq!(w.len() - m.len(), dev.len() - dm.len(), "{t}: identical apart from the manifest text");
    }
    // the default is device
    assert_eq!(CompileOptions::default().target, Target::Device);
}

fn wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).unwrap()
}

#[test]
fn validator_uses_the_target_feature_set() {
    let simd = wat("(module (func (result v128) v128.const i64x2 0 0))");
    let tail = wat("(module (func $f return_call $f))");
    let eh = wat("(module (tag $t) (func try_table (catch_all 0) end))");
    let ext = wat("(module (global i32 (i32.add (i32.const 1) (i32.const 2))))");
    let mem64 = wat("(module (memory i64 1))");
    let two_mem = wat("(module (memory 1) (memory 1))");
    let sat = wat("(module (func (param f32) (result i32) local.get 0 i32.trunc_sat_f32_s))");
    let gc = wat("(module (type (struct)))");
    let case = |name: &str, wasm: &[u8], dev: bool, web: bool, srv: bool| {
        for (t, want) in [(Target::Device, dev), (Target::Web, web), (Target::Server, srv)] {
            assert_eq!(validate_for(wasm, t).is_ok(), want, "{name} on {t}: {:?}", validate_for(wasm, t));
        }
    };
    case("simd", &simd, false, true, true);
    case("tail-call", &tail, false, true, true);
    case("exceptions", &eh, false, true, true);
    case("extended-const", &ext, false, true, true);
    case("memory64", &mem64, false, false, true);
    // one memory only in every target (ABI 1.0; BAD ACTOR F2)
    case("multi-memory", &two_mem, false, false, false);
    // never in any target: trunc_sat (ABI rule) and GC types
    case("trunc_sat", &sat, false, false, false);
    case("gc", &gc, false, false, false);
    assert!(features_for(Target::Server).contains(wasmparser::WasmFeatures::MEMORY64));
    assert!(!features_for(Target::Server).contains(wasmparser::WasmFeatures::RELAXED_SIMD));
    assert!(!features_for(Target::Server).contains(wasmparser::WasmFeatures::THREADS));
    assert!(!features_for(Target::Server).contains(wasmparser::WasmFeatures::GC));
}

fn host_for(max: Target) -> Host {
    Host::with_options(Limits::default(), HostOptions { max_target: max, ..HostOptions::default() })
}

fn code(r: Result<oxiscript_runtime::Runtime, oxiscript_runtime::Error>) -> String {
    match r {
        Ok(_) => "ok".into(),
        Err(e) => e.code().to_string(),
    }
}

#[test]
fn host_refuses_a_module_above_its_max_target() {
    for module_t in Target::ALL {
        let wasm = build_main(module_t);
        for host_t in Target::ALL {
            let r = code(host_for(host_t).instantiate(&wasm, Grants::all()));
            if module_t <= host_t {
                assert_eq!(r, "ok", "module {module_t} on host {host_t}");
            } else {
                assert_eq!(r, "OXS-E0411", "module {module_t} on host {host_t}");
            }
        }
    }
}

/// A complete loadable module (stub runtime) compiled for `t`.
fn build_main(t: Target) -> Vec<u8> {
    let m = parse_module(
        "oxiir 1\nmodule \"t\"\nexport func @main() -> void {\nentry:\n  ret\n}\n",
    );
    let m = m.unwrap_or_else(|e| panic!("{e}"));
    compile(&m, &CompileOptions { target: t, ..opts() }).unwrap()
}

#[test]
fn declared_features_must_fit_target_and_engine() {
    let base = build_main(Target::Web);
    // rewrite the manifest text of a Web module to claim a feature
    let with = |features: &str| -> Vec<u8> { patch_manifest(&base, |t| format!("{t}features {features}\n")) };
    // tail-call: Web allows it and this wasmi build has it
    assert_eq!(code(host_for(Target::Web).instantiate(&with("tail-call"), Grants::all())), "ok");
    // refused by a Device host
    assert_eq!(code(host_for(Target::Device).instantiate(&with("tail-call"), Grants::all())), "OXS-E0411");
    // simd128: the Web target has it but this engine build does not: refused, not misrun
    assert_eq!(code(host_for(Target::Web).instantiate(&with("simd128"), Grants::all())), "OXS-E0411");
    // honesty: a `device` module whose code uses `return_call` is refused even by a Web host
    let dev = build_main(Target::Device);
    let tail_code = patch_code_with_tail_call(&dev);
    assert_eq!(code(host_for(Target::Web).instantiate(&tail_code, Grants::all())), "OXS-E0411");
    // ... and accepted when it declares the feature
    let web = build_main(Target::Web);
    let tail_web = patch_manifest(&patch_code_with_tail_call(&web), |t| format!("{t}features tail-call\n"));
    assert_eq!(code(host_for(Target::Web).instantiate(&tail_web, Grants::all())), "ok");
    // memory64 in a Web manifest is a malformed manifest
    assert_eq!(code(host_for(Target::Server).instantiate(&with("memory64"), Grants::all())), "OXS-E0402");
}

/// Replace the manifest custom section's text.
fn patch_manifest(wasm: &[u8], f: impl Fn(&str) -> String) -> Vec<u8> {
    use wasm_encoder::{CustomSection, RawSection};
    use wasmparser::{Parser, Payload};
    let mut out = wasm_encoder::Module::new();
    for p in Parser::new(0).parse_all(wasm) {
        match p.unwrap() {
            Payload::CustomSection(c) if c.name() == MANIFEST_SECTION => {
                let t = f(std::str::from_utf8(c.data()).unwrap());
                out.section(&CustomSection { name: MANIFEST_SECTION.into(), data: t.as_bytes().into() });
            }
            Payload::Version { .. } | Payload::End(_) => {}
            other => {
                if let Some((id, range)) = other.as_section() {
                    out.section(&RawSection { id, data: &wasm[range.start as usize..range.end as usize] });
                }
            }
        }
    }
    out.finish()
}

#[test]
fn feature_names_cover_the_target_table() {
    assert_eq!(Target::Web.features(), vec![Feature::Simd128, Feature::Exceptions, Feature::TailCall, Feature::ExtendedConst]);
}

/// Append an extra function using `return_call` (type section/func section/code section edited via text round trip).
fn patch_code_with_tail_call(wasm: &[u8]) -> Vec<u8> {
    let mut text = wasmprinter::print_bytes(wasm).unwrap();
    // add a function before the final closing paren
    let end = text.rfind(')').unwrap();
    text.insert_str(end, "\n  (func $tc_extra return_call $tc_extra)\n");
    // wasmprinter does not print custom sections' contents faithfully: restore the manifest afterwards
    let mut m = wat::parse_str(&text).unwrap();
    use wasmparser::{Parser, Payload};
    let mut mf = String::new();
    for p in Parser::new(0).parse_all(wasm) {
        if let Payload::CustomSection(c) = p.unwrap() {
            if c.name() == MANIFEST_SECTION {
                mf = std::str::from_utf8(c.data()).unwrap().to_string();
            }
        }
    }
    m = patch_manifest(&m, |_| mf.clone());
    m
}
