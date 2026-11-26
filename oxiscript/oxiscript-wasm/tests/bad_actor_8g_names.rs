//! BAD ACTOR 8.G: export-name attacks. `#export_name` must not be a way to
//! shadow ABI exports or to produce a module the host cannot load.
#![allow(clippy::pedantic)]
mod common;
use common::*;
use oxiscript_ir::{parse_module, verify};
use oxiscript_wasm::{compile, validate};

fn try_compile(name_attr: &str, second: &str) -> Result<(), String> {
    let src = format!(
        "oxiir 1\nmodule \"n\"\nexport func @a() -> i32 #export_name({name_attr}) {{\ne:\n  %z: i32 = const 1\n  ret %z\n}}\n{second}"
    );
    let m = parse_module(&src).map_err(|e| format!("parse {e}"))?;
    let errs = verify(&m);
    let r = compile(&m, &opts());
    match r {
        Ok(w) => {
            validate(&w).map_err(|e| format!("INVALID WASM: {e}"))?;
            let h = host();
            h.instantiate(&w, oxiscript_runtime::Grants::all())
                .map(|_| ())
                .map_err(|e| format!("host rejects accepted module (verify errs: {}): {e}", errs.len()))
        }
        Err(e) => {
            let _ = errs;
            Err(format!("diag {}", e.0[0].code))
        }
    }
}

#[test]
fn export_name_cannot_shadow_abi_exports() {
    for n in ["memory", "oxi_init", "oxi_tick", "oxi_deinit", "oxi_trap_kind", "oxi_alloc", "oxi_scratch_cap", "oxi_live_objects"] {
        let r = try_compile(n, "");
        eprintln!("export_name({n}) -> {r:?}");
        assert!(r.is_err(), "export_name({n}) was accepted and loaded");
        assert!(!r.as_ref().unwrap_err().starts_with("INVALID") && !r.as_ref().unwrap_err().starts_with("host rejects"), "{n}: {r:?}");
    }
}

#[test]
fn export_name_cannot_collide() {
    let second = "export func @b() -> i32 #export_name(x) {\ne:\n  %z: i32 = const 2\n  ret %z\n}\n";
    let r = try_compile("x", second);
    eprintln!("duplicate export_name -> {r:?}");
    assert!(r.is_err());
    assert!(!r.as_ref().unwrap_err().starts_with("INVALID") && !r.as_ref().unwrap_err().starts_with("host rejects"), "{r:?}");
    // a plain name equal to another function's export_name
    let second = "export func @x() -> i32 {\ne:\n  %z: i32 = const 2\n  ret %z\n}\n";
    let r = try_compile("x", second);
    eprintln!("name vs export_name -> {r:?}");
    assert!(r.is_err());
    assert!(!r.as_ref().unwrap_err().starts_with("INVALID") && !r.as_ref().unwrap_err().starts_with("host rejects"), "{r:?}");
}

#[test]
fn export_name_cb_prefix_and_odd_names() {
    for n in ["\"cb:host:state@1.State.OnChange\"", "\"\"", "\"a b\"", "\"memory \"", "\"oxi_\"", "\"\\u{0}\""] {
        let r = try_compile(n, "");
        eprintln!("export_name({n}) -> {r:?}");
        if let Err(e) = &r {
            assert!(!e.starts_with("INVALID") && !e.starts_with("host rejects"), "{n}: {e}");
        }
    }
}

/// Exports whose parameters/results are managed objects other than str/[u8]
/// (or whose result is str/[u8]) would hand the host raw guest pointers:
/// codegen must reject them (E0601) unless the trusted `allow_raw_exports` is set.
#[test]
fn export_with_raw_object_params() {
    for (params, rets, body) in [
        ("%a: ref T", "i32", "  %v: i32 = getfield T.x %a\n  ret %v"),
        ("%a: [i32]", "i32", "  %z: i32 = const 0\n  %v: i32 = array.get %a, %z\n  ret %v"),
        ("%a: i32", "ref T", "  %o: ref T = alloc T\n  %z: i32 = const 0\n  setfield.init T.x %o, %z\n  ret %o"),
        ("%a: str?", "i32", "  %z: i32 = const 0\n  ret %z"),
        ("%a: i32", "str", "  %s: str = str.const @d\n  ret %s"),
        ("%a: fn(i32) -> i32", "i32", "  %z: i32 = const 0\n  ret %z"),
    ] {
        let src = format!("oxiir 1\nmodule \"e\"\ntype T = struct {{ x: i32 }}\ndata @d = \"x\"\nexport func @f({params}) -> {rets} {{\ne:\n{body}\n}}\n");
        let m = parse_module(&src).unwrap();
        let errs = verify(&m);
        assert_eq!(errs.len(), 0, "{params}: {errs:?}");
        let e = compile(&m, &opts()).expect_err(params);
        assert_eq!(e.0[0].code, oxiscript_wasm::E_UNSUPPORTED, "{params} -> {rets}: {e}");
        // the trusted option still compiles it
        let o = oxiscript_wasm::CompileOptions { allow_raw_exports: true, ..opts() };
        assert!(compile(&m, &o).is_ok(), "{params} -> {rets} with allow_raw_exports");
    }
}
