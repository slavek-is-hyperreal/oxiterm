#![allow(clippy::all, clippy::pedantic, clippy::nursery)]
//! Verification tests for 8.M-link BAD ACTOR fixes:
//! - BL04: no builder paths or rustc commit hash in any compiled Linked module
//! - BL10: debug runtime (`debug_runtime: true`) catches double release / UAF with trap 65
mod common;
use common::*;
use oxiscript_ir::parse_module;
use oxiscript_wasm::{compile, CompileOptions, RuntimeMode};

fn scan_forbidden_strings(label: &str, bytes: &[u8]) {
    let forbidden: &[&[u8]] = &[
        b"/home",
        b"/Users",
        b".cargo/registry",
        b"rustc/",
    ];
    for needle in forbidden {
        if let Some(pos) = bytes.windows(needle.len()).position(|w| w == *needle) {
            let start = pos.saturating_sub(16);
            let end = (pos + needle.len() + 64).min(bytes.len());
            let snippet = String::from_utf8_lossy(&bytes[start..end]);
            panic!(
                "BL04: {label} contains forbidden string {:?} at offset {pos}: {snippet:?}",
                String::from_utf8_lossy(needle)
            );
        }
    }
}

#[test]
fn linked_modules_contain_no_builder_paths_or_rustc_hash() {
    let ir = r#"
oxiir 1
module "clean"
data @msg = "hello"
export func @hi() -> i32 {
entry:
  %s: str = str.const @msg
  %n: i32 = str.len %s
  release %s
  ret %n
}
"#;
    let m = parse_module(ir).unwrap();
    let linked = compile(
        &m,
        &CompileOptions {
            runtime: RuntimeMode::Linked,
            allow_raw_exports: true,
            ..opts()
        },
    )
    .unwrap();
    scan_forbidden_strings("linked release module", &linked);

    let stub = compile(
        &m,
        &CompileOptions {
            runtime: RuntimeMode::Stub,
            allow_raw_exports: true,
            ..opts()
        },
    )
    .unwrap();
    scan_forbidden_strings("stub module", &stub);

    #[cfg(feature = "debug-runtime")]
    {
        let debug_linked = compile(
            &m,
            &CompileOptions {
                runtime: RuntimeMode::Linked,
                debug_runtime: true,
                allow_raw_exports: true,
                ..opts()
            },
        )
        .unwrap();
        scan_forbidden_strings("linked debug module", &debug_linked);
    }
}

#[cfg(feature = "debug-runtime")]
#[test]
fn debug_runtime_traps_65_on_double_release() {
    use wasmi::{Engine, Linker, Module, Store, Val};

    let ir = r#"
oxiir 1
module "uaf"
type Box = struct { v: i32 }
export func @mk() -> i32 {
entry:
  %b: ref Box = alloc Box
  %v: i32 = const 42
  setfield.init Box.v %b, %v
  %r: i32 = getfield Box.v %b
  release %b
  ret %r
}
"#;
    let m = parse_module(ir).unwrap();
    let wasm = compile(
        &m,
        &CompileOptions {
            runtime: RuntimeMode::Linked,
            debug_runtime: true,
            allow_raw_exports: true,
            ..opts()
        },
    )
    .unwrap();
    oxiscript_wasm::validate(&wasm).unwrap();

    let engine = Engine::default();
    let module = Module::new(&engine, &wasm).unwrap();
    let mut store = Store::new(&engine, ());
    let linker = <Linker<()>>::new(&engine);
    let inst = linker.instantiate_and_start(&mut store, &module).unwrap();
    inst.get_func(&store, "oxi_init")
        .unwrap()
        .call(&mut store, &[], &mut [])
        .unwrap();

    // Normal well-formed function succeeds with 0 leaks.
    let mut out = [Val::I32(0)];
    inst.get_func(&store, "mk")
        .unwrap()
        .call(&mut store, &[], &mut out)
        .unwrap();
    assert_eq!(out[0].i32(), Some(42));

    // Double release via oxi_alloc + 2x oxi_release MUST trap with stale_object (65).
    let alloc = inst.get_func(&store, "oxi_alloc").unwrap();
    let release = inst.get_func(&store, "oxi_release").unwrap();
    alloc.call(&mut store, &[Val::I32(16)], &mut out).unwrap();
    let ptr = out[0].i32().unwrap();
    assert_ne!(ptr, 0);
    release.call(&mut store, &[Val::I32(ptr)], &mut []).unwrap();
    let err = release.call(&mut store, &[Val::I32(ptr)], &mut []);
    assert!(err.is_err(), "debug runtime must trap on double release");
    let kind = inst
        .get_global(&store, "oxi_trap_kind")
        .unwrap()
        .get(&store)
        .i32()
        .unwrap();
    assert_eq!(kind, 65, "expected stale_object trap 65, got {kind}");
}
