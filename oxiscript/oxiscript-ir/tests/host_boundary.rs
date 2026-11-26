//! The typed host boundary: imports must match the IDL, and the IR typed
//! signature must flatten to exactly the ABI crate's Wasm imports.
#![allow(clippy::doc_markdown)]

use oxiscript_abi::idl::{self, Interface};
use oxiscript_abi::WasmTy;
use oxiscript_ir::host::{
    callback_sig, flatten, import_sig, verify_imports, verify_with_interfaces,
};
use oxiscript_ir::*;

fn interfaces() -> Vec<Interface> {
    let mut v = idl::parse(idl::EXAMPLE_LOG_IDL).unwrap();
    v.extend(idl::parse(idl::EXAMPLE_STATE_IDL).unwrap());
    v
}

fn module_with_import(line: &str) -> Module {
    parse_module(&format!("oxiir 1\nmodule \"m\"\n\n{line}\n")).unwrap()
}

#[test]
fn host_program_imports_match_the_idl() {
    let m = parse_module(include_str!("programs/host.oxir")).unwrap();
    let errs = verify_with_interfaces(&m, VerifyOptions::default(), &interfaces());
    assert_eq!(errs.len(), 0, "{errs:#?}");
}

#[test]
fn forged_flat_signatures_verify_plainly_but_fail_the_boundary_check() {
    let forged = [
        // log.write with raw (ptr, len) integers
        "import @w = \"host:log@1\" \"write\" (i32, u8, i32, i32) -> void",
        // read_str with a forged out_ptr/out_cap
        "import @r = \"host:state@1\" \"read_str\" (i32, i32, i32, i32) -> (i32, i32)",
        // watch with a raw closure pointer
        "import @c = \"host:state@1\" \"watch\" (i32, i32, i32) -> void",
        // wrong callback type
        "import @c = \"host:state@1\" \"watch\" (i32, i32, fn() -> void) -> void",
    ];
    for line in forged {
        let m = module_with_import(line);
        // Plain verification cannot know the interface: legal parameter kinds pass.
        assert_eq!(verify(&m).len(), 0, "{line}: {:?}", verify(&m));
        // The boundary check rejects every one of them.
        let errs = verify_imports(&m, &interfaces());
        assert_eq!(errs.len(), 1, "{line}: {errs:?}");
        assert!(
            errs[0].msg.contains("does not match the interface"),
            "{line}: {errs:?}"
        );
    }
}

#[test]
fn unknown_modules_and_functions_are_rejected_with_codes() {
    let m = module_with_import("import @x = \"host:fs@1\" \"open\" (i32, str) -> i32");
    let errs = verify_imports(&m, &interfaces());
    assert!(errs[0].msg.contains("OXS-E0400"), "{errs:?}");
    let m = module_with_import("import @x = \"host:log@1\" \"nope\" (i32) -> void");
    assert!(verify_imports(&m, &interfaces())[0]
        .msg
        .contains("OXS-E0401"));
    let m = module_with_import("import @x = \"host:log@2\" \"write\" (i32, u8, str) -> void");
    assert!(verify_imports(&m, &interfaces())[0]
        .msg
        .contains("OXS-E0400"));
}

/// The IR typed signature flattens to exactly what `Interface::wasm_imports()` says,
/// and callback closure types flatten to the `cb:` trampoline signatures.
#[test]
fn typed_signatures_flatten_to_the_abi_imports() {
    for itf in interfaces() {
        let wasm = itf.wasm_imports();
        assert_eq!(wasm.len(), itf.fns.len());
        for (f, w) in itf.fns.iter().zip(&wasm) {
            let sig = import_sig(f, &itf);
            let (mut params, mut results) = (Vec::new(), Vec::new());
            sig.params.iter().for_each(|t| flatten(t, &mut params));
            sig.rets.iter().for_each(|t| flatten(t, &mut results));
            assert_eq!(params, w.params, "{}.{} params", itf.name, f.name);
            assert_eq!(results, w.results, "{}.{} results", itf.name, f.name);
        }
        for (c, w) in itf.callbacks.iter().zip(itf.wasm_callback_exports()) {
            let sig = callback_sig(&itf, &c.name);
            let mut params = vec![WasmTy::I32]; // the closure itself
            sig.params.iter().for_each(|t| flatten(t, &mut params));
            let mut results = Vec::new();
            sig.rets.iter().for_each(|t| flatten(t, &mut results));
            assert_eq!(params, w.params, "cb {}", c.name);
            assert_eq!(results, w.results, "cb {}", c.name);
        }
    }
}

/// A module with no interfaces granted has no imports it may keep.
#[test]
fn nothing_is_granted_by_default() {
    let m = parse_module(include_str!("programs/host.oxir")).unwrap();
    let errs = verify_imports(&m, &[]);
    assert_eq!(errs.len(), m.imports.len());
}
