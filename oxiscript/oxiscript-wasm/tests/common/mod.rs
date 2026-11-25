#![allow(dead_code, clippy::pedantic)]
pub mod gen;
pub mod interp;

use oxiscript_abi::idl;
use oxiscript_ir::Module;
use oxiscript_runtime::{Grants, Host, Limits, Runtime};
use oxiscript_wasm::{compile, CompileOptions};

pub fn interfaces() -> Vec<idl::Interface> {
    let mut v = idl::parse(idl::EXAMPLE_LOG_IDL).unwrap();
    v.extend(idl::parse(idl::EXAMPLE_STATE_IDL).unwrap());
    v
}

pub fn opts() -> CompileOptions {
    CompileOptions {
        interfaces: interfaces(),
        ..CompileOptions::default()
    }
}

pub fn build(m: &Module) -> Vec<u8> {
    compile(m, &opts()).unwrap_or_else(|e| panic!("compile failed: {e}"))
}

pub fn host() -> Host {
    Host::new(Limits::default())
}

pub fn instantiate(host: &Host, wasm: &[u8]) -> Runtime {
    host.instantiate(wasm, Grants::all())
        .unwrap_or_else(|e| panic!("load failed: {e}"))
}

/// Full-size fuzz runs in release or with OXI_FULL_FUZZ=1.
pub fn fuzz_scale(full: usize, debug: usize) -> usize {
    if cfg!(debug_assertions) && std::env::var("OXI_FULL_FUZZ").is_err() {
        debug
    } else {
        full
    }
}
