#![allow(dead_code, clippy::pedantic)]
//! Test harness: compile `.oxs` source with the full pipeline and run it in the
//! strict wasmi runtime with a captured `host:log`.

use oxiscript_abi::idl::{self, IdlFn, Interface};
use oxiscript_lower::{compile_program, compile_source, CompileOutput, Options};
use oxiscript_runtime::caps::log::{LogLimits, LogProvider, LogSink};
use oxiscript_runtime::caps::state::{StateLimits, StateProvider, StateStore};
use oxiscript_runtime::{
    CallbackEvent, Capability, CapabilityProvider, Grants, Host, HostError, Limits, Runtime, Value,
};
use std::sync::{Arc, Mutex};

pub const TICKER_IDL: &str = r#"
interface Ticker "host:ticker" @1 {
    callback OnTick(n: i32, label: str);
    callback OnDone();
    fn register(cb: OnTick);
    fn on_done(cb: OnDone);
    fn fire(n: i32) -> i32;
    fn drop_all();
}
"#;

pub const UI_IDL: &str = include_str!("../../../oxiscript-check/tests/idl/ui.idl");
pub const CANVAS_IDL: &str = include_str!("../../../oxiscript-check/tests/idl/canvas.idl");

pub fn interfaces() -> Vec<Interface> {
    let mut v = Vec::new();
    for src in [idl::EXAMPLE_LOG_IDL, idl::EXAMPLE_STATE_IDL, TICKER_IDL, UI_IDL, CANVAS_IDL] {
        v.extend(idl::parse(src).expect("idl"));
    }
    v
}

pub fn show_diags(d: &oxiscript_lower::Diagnostics) -> String {
    d.to_string()
}

pub fn compile(src: &str) -> Vec<u8> {
    compile_source(src, &interfaces(), &Options::default()).unwrap_or_else(|e| panic!("compile failed:\n{e}\n--- source ---\n{src}"))
}

pub fn compile_full(src: &str) -> CompileOutput {
    compile_program("", src, &mut oxiscript_check::NoLoader, &interfaces(), &Options::default())
        .unwrap_or_else(|e| panic!("compile failed:\n{e}\n--- source ---\n{src}"))
}

/// Shared state of the scripted `host:ticker` test capability.
#[derive(Default)]
pub struct TickerState {
    /// closures registered for OnTick / OnDone (guest pointers)
    pub tick_cbs: Vec<u32>,
    pub done_cbs: Vec<u32>,
    /// events to deliver on the next pump
    pub queue: Vec<CallbackEvent>,
    /// closures whose registration was dropped (to be released by the runtime)
    pub releases: Vec<u32>,
}

struct TickerProvider {
    itf: Interface,
    st: Arc<Mutex<TickerState>>,
}

struct TickerCap {
    st: Arc<Mutex<TickerState>>,
}

impl CapabilityProvider for TickerProvider {
    fn interface(&self) -> &Interface {
        &self.itf
    }
    fn instantiate(&self, _ctx: &oxiscript_runtime::capability::InstanceCtx) -> Box<dyn Capability> {
        Box::new(TickerCap { st: Arc::clone(&self.st) })
    }
}

impl Capability for TickerCap {
    fn call(&mut self, func: &IdlFn, args: &[Value]) -> Result<Option<Value>, HostError> {
        let mut st = self.st.lock().unwrap();
        match (func.name.as_str(), args) {
            ("register", [Value::I32(p)]) => {
                st.tick_cbs.push(*p as u32);
                Ok(None)
            }
            ("on_done", [Value::I32(p)]) => {
                st.done_cbs.push(*p as u32);
                Ok(None)
            }
            ("fire", [Value::I32(n)]) => {
                let cbs = st.tick_cbs.clone();
                for (i, c) in cbs.iter().enumerate() {
                    st.queue.push(CallbackEvent {
                        callback: "OnTick".into(),
                        closure: *c,
                        args: vec![Value::I32(*n + i as i32), Value::Str(format!("tick{n}"))],
                    });
                }
                Ok(Some(Value::I32(cbs.len() as i32)))
            }
            ("drop_all", []) => {
                let mut a: Vec<u32> = std::mem::take(&mut st.tick_cbs);
                a.extend(std::mem::take(&mut st.done_cbs));
                st.releases.extend(a);
                Ok(None)
            }
            _ => Err(HostError::Trap(format!("ticker: unexpected {}", func.name))),
        }
    }
    fn take_events(&mut self, max: usize) -> Vec<CallbackEvent> {
        let mut st = self.st.lock().unwrap();
        let n = max.min(st.queue.len());
        st.queue.drain(..n).collect()
    }
    fn take_releases(&mut self, max: usize) -> Vec<u32> {
        let mut st = self.st.lock().unwrap();
        let n = max.min(st.releases.len());
        st.releases.drain(..n).collect()
    }
}

/// A capability that accepts everything (for modules that only need to load).
struct NullProvider(Interface);
struct NullCap;
impl CapabilityProvider for NullProvider {
    fn interface(&self) -> &Interface {
        &self.0
    }
    fn instantiate(&self, _ctx: &oxiscript_runtime::capability::InstanceCtx) -> Box<dyn Capability> {
        Box::new(NullCap)
    }
}
impl Capability for NullCap {
    fn call(&mut self, _f: &IdlFn, _a: &[Value]) -> Result<Option<Value>, HostError> {
        Ok(None)
    }
}

pub struct Env {
    pub host: Host,
    pub sink: Arc<LogSink>,
    pub store: StateStore,
    pub ticker: Arc<Mutex<TickerState>>,
}

pub fn env() -> Env {
    let host = Host::new(Limits::default());
    let sink = LogSink::new(4096);
    let store = StateStore::new(StateLimits::default());
    let ticker = Arc::new(Mutex::new(TickerState::default()));
    let limits = LogLimits {
        max_lines_per_turn: 10_000,
        max_bytes_per_turn: 4 * 1024 * 1024,
        ..LogLimits::default()
    };
    host.register(Arc::new(LogProvider::new(Arc::clone(&sink), limits))).unwrap();
    host.register(Arc::new(StateProvider::new(store.clone()))).unwrap();
    let all = interfaces();
    let t = all.iter().find(|i| i.name == "Ticker").unwrap().clone();
    host.register(Arc::new(TickerProvider { itf: t, st: Arc::clone(&ticker) })).unwrap();
    for n in ["Ui", "Canvas"] {
        let i = all.iter().find(|i| i.name == n).unwrap().clone();
        host.register(Arc::new(NullProvider(i))).unwrap();
    }
    Env { host, sink, store, ticker }
}

/// A compiled and initialised program.
pub struct Prog {
    pub env: Env,
    pub rt: Runtime,
    pub wasm: Vec<u8>,
}

impl Prog {
    pub fn new(src: &str) -> Prog {
        let wasm = compile(src);
        Prog::from_wasm(wasm)
    }

    pub fn from_wasm(wasm: Vec<u8>) -> Prog {
        let env = env();
        let rt = env.host.instantiate(&wasm, Grants::all()).unwrap_or_else(|e| panic!("instantiate: {e}"));
        Prog { env, rt, wasm }
    }

    /// Log messages written so far (oldest first).
    pub fn log(&self) -> Vec<String> {
        self.env.sink.lines().into_iter().map(|l| l.msg).collect()
    }

    pub fn call(&mut self, name: &str, args: &[Value]) -> Vec<Value> {
        self.rt.call_export(name, args).unwrap_or_else(|e| panic!("call {name}: {e}"))
    }

    pub fn call_err(&mut self, name: &str, args: &[Value]) -> oxiscript_runtime::Error {
        self.rt.call_export(name, args).expect_err("expected a trap")
    }

    /// Deinit and return the number of live objects (0 = no leaks).
    pub fn live_after_deinit(&mut self) -> i32 {
        self.rt.deinit().unwrap_or_else(|e| panic!("deinit: {e}"))
    }

    pub fn finish(&mut self) {
        let n = self.live_after_deinit();
        assert_eq!(n, 0, "leaked {n} objects");
    }
}

/// Compile `src`, run `main`, check the log and that nothing leaks.
pub fn run_log(src: &str, want: &[&str]) {
    let mut p = Prog::new(src);
    let got = p.log();
    assert_eq!(got, want.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "log of:\n{src}");
    p.finish();
}

pub fn trap_kind(e: &oxiscript_runtime::Error) -> Option<oxiscript_abi::trap::TrapKind> {
    match e {
        oxiscript_runtime::Error::Trap(t) => t.guest_kind,
        _ => None,
    }
}
