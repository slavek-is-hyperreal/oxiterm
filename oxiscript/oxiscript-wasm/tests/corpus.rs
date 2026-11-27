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
fn corpus_loads_in_the_runtime_linked() {
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
        let o = CompileOptions {
            runtime: RuntimeMode::Linked,
            ..opts(name)
        };
        let wasm = compile(&m, &o).unwrap_or_else(|e| panic!("{name} (linked): {e}"));
        validate(&wasm).unwrap_or_else(|e| panic!("{name} (linked validate): {e}"));
        let mut rt = host
            .instantiate(&wasm, Grants::all())
            .unwrap_or_else(|e| panic!("{name} (linked load): {e}"));
        assert_eq!(rt.deinit().unwrap(), 0, "{name} (linked): leak");
    }
}

/// Runtime-raised traps (`str.from_utf8` -> `BadUtf8`, `oxi_alloc` -> `OutOfMemory`)
/// must write the single exported `oxi_trap_kind` global through `oxi_set_trap`.
#[test]
fn linked_str_from_utf8_and_oom_traps_set_oxi_trap_kind() {
    use oxiscript_abi::trap::TrapKind;
    use oxiscript_runtime::{Error, Grants, Host, Limits, Value};

    let ir = r#"
oxiir 1
module "rt_traps"
export func @bad_utf8() -> i32 {
entry:
  %n: i32 = const 2
  %buf: [u8] = array.new %n
  %i0: i32 = const 0
  %i1: i32 = const 1
  %b0: u8 = const 255
  %b1: u8 = const 254
  array.set %buf, %i0, %b0
  array.set %buf, %i1, %b1
  %s: str = str.from_utf8 %buf, %n
  %l: i32 = str.len %s
  release %s
  release %buf
  ret %l
}
export func @alloc_big(%n: i32) -> i32 {
entry:
  %buf: [u8] = array.new %n
  %l: i32 = array.len %buf
  release %buf
  ret %l
}
"#;
    let m = parse_module(ir).unwrap();
    let wasm = compile(
        &m,
        &CompileOptions {
            runtime: RuntimeMode::Linked,
            ..CompileOptions::default()
        },
    )
    .unwrap();
    validate(&wasm).unwrap();

    // 1. BadUtf8 from `str.from_utf8` (calls `oxi_str_new` in `oxiscript-rc`)
    let host = Host::new(Limits::default());
    let mut rt = host.instantiate(&wasm, Grants::all()).unwrap();
    let err = rt.call_export("bad_utf8", &[]).expect_err("expected BadUtf8");
    match err {
        Error::Trap(t) => assert_eq!(t.guest_kind, Some(TrapKind::BadUtf8), "{t:?}"),
        other => panic!("expected Trap(BadUtf8), got {other}"),
    }

    // 2. OutOfMemory from `array.new` (calls `oxi_array_new` -> `dlmalloc` in `oxiscript-rc`)
    let tight = Host::new(Limits {
        max_memory_pages: 4,
        ..Limits::default()
    });
    let mut rt2 = tight.instantiate(&wasm, Grants::all()).unwrap();
    let err2 = rt2
        .call_export("alloc_big", &[Value::I32(1_000_000)])
        .expect_err("expected OutOfMemory");
    match err2 {
        Error::Trap(t) => assert_eq!(t.guest_kind, Some(TrapKind::OutOfMemory), "{t:?}"),
        other => panic!("expected Trap(OutOfMemory), got {other}"),
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
