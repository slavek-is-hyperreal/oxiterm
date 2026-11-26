#![allow(dead_code, clippy::pedantic)]
pub mod gen;
pub mod interp;

use oxiscript_abi::idl;
use oxiscript_ir::Module;
use oxiscript_runtime::{Grants, Host, Limits, Runtime};
use oxiscript_wasm::{compile, CompileOptions};

/// The target the program test suites compile for and run on: `OXI_TARGET=device|web|server`
/// (default device). `scripts/oxiscript-gate.sh` runs the suites for device and web.
pub fn test_target() -> oxiscript_abi::target::Target {
    match std::env::var("OXI_TARGET") {
        Ok(s) => oxiscript_abi::target::Target::parse(&s).unwrap_or_else(|| panic!("OXI_TARGET={s}: expected device, web or server")),
        Err(_) => oxiscript_abi::target::Target::Device,
    }
}

pub fn test_host_options() -> oxiscript_runtime::HostOptions {
    oxiscript_runtime::HostOptions { max_target: test_target(), ..Default::default() }
}

pub fn interfaces() -> Vec<idl::Interface> {
    let mut v = idl::parse(idl::EXAMPLE_LOG_IDL).unwrap();
    v.extend(idl::parse(idl::EXAMPLE_STATE_IDL).unwrap());
    v
}

pub fn opts() -> CompileOptions {
    CompileOptions {
        interfaces: interfaces(),
        target: test_target(),
        ..CompileOptions::default()
    }
}

pub fn build(m: &Module) -> Vec<u8> {
    compile(m, &opts()).unwrap_or_else(|e| panic!("compile failed: {e}"))
}

pub fn host() -> Host {
    Host::with_options(Limits::default(), test_host_options())
}

pub fn instantiate(host: &Host, wasm: &[u8]) -> Runtime {
    host.instantiate(wasm, Grants::all())
        .unwrap_or_else(|e| panic!("load failed: {e}"))
}

pub struct Env {
    pub host: Host,
    pub sink: std::sync::Arc<oxiscript_runtime::caps::log::LogSink>,
    pub store: oxiscript_runtime::caps::state::StateStore,
}

pub fn env() -> Env {
    use oxiscript_runtime::caps::log::{LogLimits, LogProvider, LogSink};
    use oxiscript_runtime::caps::state::{StateLimits, StateProvider, StateStore};
    use std::sync::Arc;
    let host = Host::with_options(Limits::default(), test_host_options());
    let sink = LogSink::new(256);
    let store = StateStore::new(StateLimits::default());
    host.register(Arc::new(LogProvider::new(
        Arc::clone(&sink),
        LogLimits::default(),
    )))
    .unwrap();
    host.register(Arc::new(StateProvider::new(store.clone())))
        .unwrap();
    Env { host, sink, store }
}

/// Full-size fuzz runs in release or with OXI_FULL_FUZZ=1.
pub fn fuzz_scale(full: usize, debug: usize) -> usize {
    if let Some(n) = std::env::var("OXI_FUZZ_N").ok().and_then(|v| v.parse().ok()) {
        return n;
    }
    if cfg!(debug_assertions) && std::env::var("OXI_FULL_FUZZ").is_err() {
        debug
    } else {
        full
    }
}
