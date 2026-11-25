//! Every `.oxir` program of `oxiscript-ir` compiles, validates with exactly
//! the ABI feature set and (where it uses the ABI) loads in `oxiscript-runtime`.
#![allow(clippy::pedantic)]

use oxiscript_abi::idl;
use oxiscript_ir::{parse_module, VerifyOptions};
use oxiscript_wasm::{compile, validate, CompileOptions, RuntimeMode};

pub const PROGRAMS: &[(&str, &str)] = &[
    ("arith", include_str!("../../oxiscript-ir/tests/programs/arith.oxir")),
    ("loop", include_str!("../../oxiscript-ir/tests/programs/loop.oxir")),
    ("structs", include_str!("../../oxiscript-ir/tests/programs/structs.oxir")),
    ("closure", include_str!("../../oxiscript-ir/tests/programs/closure.oxir")),
    ("host", include_str!("../../oxiscript-ir/tests/programs/host.oxir")),
    ("rc", include_str!("../../oxiscript-ir/tests/programs/rc.oxir")),
    ("raw", include_str!("../../oxiscript-ir/tests/programs/raw.oxir")),
];

pub fn interfaces() -> Vec<idl::Interface> {
    let mut v = idl::parse(idl::EXAMPLE_LOG_IDL).unwrap();
    v.extend(idl::parse(idl::EXAMPLE_STATE_IDL).unwrap());
    v
}

fn opts(name: &str) -> CompileOptions {
    CompileOptions {
        interfaces: interfaces(),
        verify: VerifyOptions {
            allow_raw_memory: name == "raw",
        },
        runtime: if name == "raw" {
            RuntimeMode::Import
        } else {
            RuntimeMode::Stub
        },
        // the IR corpus exports raw `ref`/`[T]` signatures (ABI 1.0 default rejects them, see below)
        allow_raw_exports: true,
        ..CompileOptions::default()
    }
}

/// Every ABI-conforming program of the corpus loads in `oxiscript-runtime`
/// (all export rows, manifest, imports and callbacks are checked there).
#[test]
fn corpus_loads_in_the_runtime() {
    use oxiscript_runtime::caps::log::{LogLimits, LogProvider, LogSink};
    use oxiscript_runtime::caps::state::{StateLimits, StateProvider, StateStore};
    use oxiscript_runtime::{Grants, Host, Limits};
    use std::sync::Arc;
    let host = Host::new(Limits::default());
    host.register(Arc::new(LogProvider::new(LogSink::new(16), LogLimits::default())))
        .unwrap();
    host.register(Arc::new(StateProvider::new(StateStore::new(StateLimits::default()))))
        .unwrap();
    for (name, text) in PROGRAMS {
        if *name == "raw" {
            continue;
        }
        let m = parse_module(text).unwrap();
        let wasm = compile(&m, &opts(name)).unwrap();
        let mut rt = host
            .instantiate(&wasm, Grants::all())
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(rt.deinit().unwrap(), 0, "{name}: leak");
    }
}

#[test]
fn corpus_compiles_and_validates() {
    for (name, text) in PROGRAMS {
        let m = parse_module(text).unwrap();
        let wasm = compile(&m, &opts(name)).unwrap_or_else(|e| panic!("{name}: {e}"));
        validate(&wasm).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// Default options enforce the ABI export boundary: the corpus programs that
/// export raw managed signatures are rejected with E0601.
#[test]
fn corpus_raw_exports_rejected_by_default() {
    for name in ["structs", "rc", "host"] {
        let text = PROGRAMS.iter().find(|(n, _)| *n == name).unwrap().1;
        let m = parse_module(text).unwrap();
        let o = CompileOptions { allow_raw_exports: false, ..opts(name) };
        let e = compile(&m, &o).expect_err(name);
        assert_eq!(e.0[0].code, oxiscript_wasm::E_UNSUPPORTED, "{name}: {e}");
    }
}
