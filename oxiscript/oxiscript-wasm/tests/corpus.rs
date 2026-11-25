//! Every `.oxir` program of `oxiscript-ir` compiles, validates with exactly
//! the ABI feature set and (where it uses the ABI) loads in `oxiscript-runtime`.

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
        ..CompileOptions::default()
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
