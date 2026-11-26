//! Compile and run (feature `run`): the one execution path shared by the browser (the runner bundle in
//! a Web Worker) and `oxi run`. Same lowering, same `oxiscript-runtime` (wasmi) with fuel, ABI checks and
//! the `host:log` / `host:state` providers, so a program behaves identically in both places.
//!
//! Nothing here reads a clock: the wall-clock limit of the page lives in JavaScript (the worker is
//! terminated); the deterministic limit is fuel.

use crate::{check_json, host_interfaces, utf16_at};
use oxiscript_abi::target::Target;
use oxiscript_lower::{compile_source_with_notes, Options};
use oxiscript_runtime::caps::log::{LogLimits, LogProvider, LogSink};
use oxiscript_runtime::caps::state::{StateLimits, StateProvider, StateStore};
use oxiscript_runtime::{Error, Grants, Host, HostOptions, Limits, TrapCause};
use oxiscript_syntax::LineIndex;
use serde_json::{json, Value};
use std::sync::Arc;

/// Fuel the playground and `oxi run` grant when nothing else is asked for (the runtime default).
pub const DEFAULT_FUEL: u64 = 10_000_000;
/// Most log lines and bytes one run may produce; more are dropped and counted.
pub const MAX_LINES: usize = 1000;
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// A log line longer than this many bytes is cut and marked (`LINE_CUT_MARK`).
pub const MAX_LINE_BYTES: usize = 1024;
/// Appended to a cut log line.
pub const LINE_CUT_MARK: &str = " ... (line truncated at 1024 bytes)";

/// One line written through `host:log` (level 0 trace .. 4 error).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    pub level: u8,
    pub msg: String,
}

/// Why the program stopped before `main` returned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StopReason {
    /// Stable id the page localises: `overflow`, `div_by_zero`, `bounds`, `bad_cast`, `null_deref`,
    /// `out_of_memory`, `assert`, `bad_utf8`, `panic`, `unreachable`, `out_of_fuel`, `stack_overflow`,
    /// `memory_bounds`, `host_denied`, `host_bad_argument`, `host_error`, `rejected`, `limit`, `engine`.
    pub reason: &'static str,
    /// Registry code (`OXS-E04xx`).
    pub code: String,
    /// The runtime's own wording (English, for the details line).
    pub detail: String,
}

/// The result of running a module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunReport {
    pub lines: Vec<Line>,
    /// Lines dropped by the output cap.
    pub dropped: u64,
    pub fuel_used: u64,
    pub stop: Option<StopReason>,
}

fn classify(e: &Error) -> StopReason {
    let reason = match e {
        Error::Trap(t) => match (t.guest_kind, &t.cause) {
            (Some(k), _) => k.name(),
            (None, TrapCause::OutOfFuel) => "out_of_fuel",
            (None, TrapCause::StackOverflow) => "stack_overflow",
            (None, TrapCause::MemoryOutOfBounds) => "memory_bounds",
            (None, TrapCause::Unreachable) => "unreachable",
            (None, TrapCause::HostDenied { .. }) => "host_denied",
            (None, TrapCause::HostBadArgument(_)) => "host_bad_argument",
            (None, TrapCause::HostFailure(_) | TrapCause::QueueFull(_)) => "host_error",
            (None, TrapCause::Engine(_)) => "engine",
        },
        Error::Rejected(_) => "rejected",
        Error::Limit(_) => "limit",
        Error::Poisoned | Error::BadCall(_) => "engine",
    };
    StopReason { reason, code: e.code().to_string(), detail: e.to_string() }
}

/// Run a compiled module: `main` executes inside `oxi_init` with `fuel` instructions of budget.
/// Capabilities: `host:log@1` and `host:state@1` (in memory, private to this run).
pub fn run_wasm(wasm: &[u8], fuel: u64) -> RunReport {
    run_wasm_on(wasm, fuel, PLAYGROUND_TARGET)
}

/// The playground compiles for `web` and runs on a Web-configured runtime (the browser is its host).
pub const PLAYGROUND_TARGET: Target = Target::Web;

/// Like [`run_wasm`] on a host that accepts modules up to `max_target`.
pub fn run_wasm_on(wasm: &[u8], fuel: u64, max_target: Target) -> RunReport {
    let limits = Limits { fuel_per_call: fuel.max(1), ..Limits::default() };
    let host = Host::with_options(limits, HostOptions { no_clock: true, max_target, ..HostOptions::default() });
    let sink = LogSink::new(MAX_LINES);
    let log_limits = LogLimits { max_line: MAX_LINE_BYTES + 4, max_lines_per_turn: MAX_LINES, max_bytes_per_turn: MAX_OUTPUT_BYTES };
    let store = StateStore::new(StateLimits::default());
    host.register(Arc::new(LogProvider::new(Arc::clone(&sink), log_limits))).expect("log registers once");
    host.register(Arc::new(StateProvider::new(store))).expect("state registers once");
    let (stop, fuel_used) = match host.instantiate(wasm, Grants::all()) {
        Ok(mut rt) => {
            let used = rt.last_fuel_used();
            let _ = rt.deinit();
            (None, used)
        }
        Err(e) => {
            let used = match &e {
                Error::Trap(t) => t.fuel_used,
                _ => 0,
            };
            (Some(classify(&e)), used)
        }
    };
    // The provider cuts at `MAX_LINE_BYTES + 4`, so a line over `MAX_LINE_BYTES` was (or may have been) cut: mark it.
    let lines = sink.lines().into_iter().map(|l| Line { level: l.level, msg: mark_cut(l.msg) }).collect();
    RunReport { lines, dropped: sink.dropped(), fuel_used, stop }
}

fn mark_cut(mut msg: String) -> String {
    if msg.len() > MAX_LINE_BYTES {
        let mut at = MAX_LINE_BYTES;
        while !msg.is_char_boundary(at) {
            at -= 1;
        }
        msg.truncate(at);
        msg.push_str(LINE_CUT_MARK);
    }
    msg
}

/// Compile one source file with the playground's host interfaces. `Err` holds diagnostics in the JSON
/// shape of [`check_json`] (`{ ok: false, stage, diagnostics }`).
pub fn compile(src: &str) -> Result<Vec<u8>, String> {
    let checked: Value = serde_json::from_str(&check_json(src)).unwrap_or(Value::Null);
    if checked["ok"] != Value::Bool(true) {
        let mut v = checked;
        v["stage"] = json!("check");
        return Err(v.to_string());
    }
    match compile_source_with_notes(src, &host_interfaces(), &Options { target: PLAYGROUND_TARGET, ..Options::default() }) {
        Ok((wasm, _)) => Ok(wasm),
        Err(ds) => {
            let li = LineIndex::new(src);
            let list: Vec<Value> = ds
                .0
                .iter()
                .map(|d| {
                    let (lo, hi) = d.span.map_or((0, 0), |s| (s.lo, s.hi.max(s.lo)));
                    let (line, col) = li.line_col(src, lo);
                    let (el, ec) = li.line_col(src, hi);
                    let title = oxiscript_abi::codes::lookup(&d.code).map_or_else(Default::default, |c| c.title);
                    json!({ "code": d.code, "severity": "error", "title": title, "message": d.message,
                            "start": utf16_at(src, lo as usize), "end": utf16_at(src, hi as usize),
                            "line": line, "col": col, "endLine": el, "endCol": ec, "fixes": [] })
                })
                .collect();
            Err(json!({ "ok": false, "stage": "compile", "diagnostics": list }).to_string())
        }
    }
}

/// Compile and run `src`. `{ ok: false, stage, diagnostics }` when it does not compile (exactly the
/// `check` shape); otherwise `{ ok: true, lines: [{level, msg}], dropped, fuelUsed, fuel, stop }` where
/// `stop` is `null` or `{ reason, code, detail }`.
pub fn run_json(src: &str, fuel: u64) -> String {
    let wasm = match compile(src) {
        Ok(w) => w,
        Err(j) => return j,
    };
    let r = run_wasm(&wasm, fuel);
    json!({
        "ok": true,
        "lines": r.lines.iter().map(|l| json!({ "level": l.level, "msg": l.msg })).collect::<Vec<_>>(),
        "dropped": r.dropped,
        "fuelUsed": r.fuel_used,
        "fuel": fuel,
        "stop": r.stop.map(|s| json!({ "reason": s.reason, "code": s.code, "detail": s.detail })),
    })
    .to_string()
}
