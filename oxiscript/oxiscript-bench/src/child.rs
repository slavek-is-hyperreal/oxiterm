//! The in-process executors (wasmi, Wasmtime, native Rust) and the measuring loop shared with the
//! out-of-process ones (Node, C): every executor runs in its own child process and prints one JSON
//! line (checksum, cold time, per-iteration nanoseconds, peak RSS, ...).

use serde_json::{json, Value};
use std::time::Instant;

/// One executor of one workload. `prepare` builds whatever a timed call needs and is NOT timed
/// (for the wasm engines: a fresh instance, because the stub runtime never reuses memory).
pub trait Runner {
    fn prepare(&mut self) -> Result<(), String>;
    fn call(&mut self) -> Result<u32, String>;
    /// `(fuel, linear-memory pages)` of the last call, where the engine reports them.
    fn metrics(&mut self) -> Option<(u64, u64)> {
        None
    }
}

/// Peak resident set of this process in KiB (`VmHWM`), 0 when unreadable.
pub fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1).and_then(|v| v.parse().ok())))
        .unwrap_or(0)
}

fn timed_call(r: &mut dyn Runner) -> Result<(u32, f64), String> {
    r.prepare()?;
    let t = Instant::now();
    let c = r.call()?;
    Ok((c, t.elapsed().as_nanos() as f64))
}

/// The measuring protocol (methodology R08, R09, R15): a cold first call, `warmup` discarded calls,
/// then `iters` timed iterations of `inner_reps` calls each (`inner_reps` chosen so one iteration
/// lasts at least `min_iter_ms`). Reports nanoseconds per single call.
pub fn measure(r: &mut dyn Runner, warmup: usize, iters: usize, min_iter_ms: f64) -> Result<Value, String> {
    let t = Instant::now();
    r.prepare()?;
    let load_ns = t.elapsed().as_nanos() as f64;
    let t = Instant::now();
    let expect = r.call()?;
    let cold_ns = t.elapsed().as_nanos() as f64;
    let first_metrics = r.metrics();
    let mut stable = true;
    let mut fuel_stable = true;
    let mut last = cold_ns;
    for _ in 0..warmup {
        let (c, ns) = timed_call(r)?;
        stable &= c == expect;
        fuel_stable &= r.metrics() == first_metrics;
        last = ns;
    }
    let reps = ((min_iter_ms * 1e6 / last.max(1.0)) as usize + 1).max(1);
    let mut ns = Vec::with_capacity(iters);
    for _ in 0..iters {
        let mut total = 0.0;
        for _ in 0..reps {
            let (c, t) = timed_call(r)?;
            stable &= c == expect;
            fuel_stable &= r.metrics() == first_metrics;
            total += t;
        }
        ns.push((total / reps as f64 * 10.0).round() / 10.0);
    }
    let mut out = json!({
        "checksum": expect,
        "checksum_stable": stable,
        "load_ns": load_ns,
        "cold_ns": cold_ns,
        "warmup": warmup,
        "inner_reps": reps,
        "peak_rss_kib": peak_rss_kib(),
        "iter_ns": ns,
    });
    if let Some((fuel, pages)) = first_metrics {
        if fuel > 0 {
            // only wasmi meters fuel (Wasmtime reports 0 here: no metering enabled)
            out["fuel"] = json!(fuel);
        }
        out["fuel_stable"] = json!(fuel_stable);
        out["wasm_pages"] = json!(pages);
    }
    Ok(out)
}

// ---- native Rust ----

pub struct Native {
    pub id: String,
    pub n: i32,
}

impl Runner for Native {
    fn prepare(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn call(&mut self) -> Result<u32, String> {
        crate::native::run(&self.id, std::hint::black_box(self.n)).map(std::hint::black_box).ok_or_else(|| format!("no native twin for {}", self.id))
    }
}

// ---- wasmi (the OxiScript runtime: fuel, limits, capabilities) ----

pub struct Wasmi {
    host: oxiscript_runtime::Host,
    wasm: Vec<u8>,
    n: i32,
    rt: Option<oxiscript_runtime::Runtime>,
}

impl Wasmi {
    pub fn new(wasm: Vec<u8>, n: i32) -> Wasmi {
        let limits = oxiscript_runtime::Limits { fuel_per_call: u64::MAX / 4, max_memory_pages: 8192, max_module_bytes: 16 << 20, ..Default::default() };
        Wasmi { host: oxiscript_runtime::Host::new(limits), wasm, n, rt: None }
    }
}

impl Runner for Wasmi {
    fn prepare(&mut self) -> Result<(), String> {
        self.rt = None; // drop the old instance first (the live-instance cap)
        self.rt = Some(self.host.instantiate(&self.wasm, oxiscript_runtime::Grants::all()).map_err(|e| format!("instantiate: {e}"))?);
        Ok(())
    }
    fn call(&mut self) -> Result<u32, String> {
        let rt = self.rt.as_mut().ok_or("not prepared")?;
        match rt.call_export("run", &[oxiscript_runtime::Value::I32(self.n)]).map_err(|e| format!("run: {e}"))?.first() {
            Some(oxiscript_runtime::Value::I32(v)) => Ok(*v as u32),
            other => Err(format!("unexpected result {other:?}")),
        }
    }
    fn metrics(&mut self) -> Option<(u64, u64)> {
        self.rt.as_ref().map(|r| (r.last_fuel_used(), r.memory_pages() as u64))
    }
}

// ---- Wasmtime ----

#[cfg(feature = "wasmtime")]
pub struct Wt {
    engine: wasmtime::Engine,
    module: wasmtime::Module,
    n: i32,
    cur: Option<(wasmtime::Store<()>, wasmtime::Instance)>,
    pub compile_ns: f64,
}

#[cfg(feature = "wasmtime")]
impl Wt {
    pub fn new(wasm: &[u8], n: i32) -> Result<Wt, String> {
        let engine = wasmtime::Engine::default();
        let t = Instant::now();
        let module = wasmtime::Module::new(&engine, wasm).map_err(|e| format!("wasmtime compile: {e}"))?;
        Ok(Wt { engine, module, n, cur: None, compile_ns: t.elapsed().as_nanos() as f64 })
    }
}

#[cfg(feature = "wasmtime")]
impl Runner for Wt {
    fn prepare(&mut self) -> Result<(), String> {
        let mut store = wasmtime::Store::new(&self.engine, ());
        let instance = wasmtime::Instance::new(&mut store, &self.module, &[]).map_err(|e| format!("instantiate: {e}"))?;
        let init_fn = instance.get_typed_func::<(), ()>(&mut store, "oxi_init").map_err(|e| format!("oxi_init: {e}"))?;
        init_fn.call(&mut store, ()).map_err(|e| format!("oxi_init call: {e}"))?;
        self.cur = Some((store, instance));
        Ok(())
    }
    fn call(&mut self) -> Result<u32, String> {
        let (store, inst) = self.cur.as_mut().ok_or("not prepared")?;
        let f = inst.get_typed_func::<i32, i32>(&mut *store, "run").map_err(|e| format!("run export: {e}"))?;
        f.call(&mut *store, self.n).map(|v| v as u32).map_err(|e| format!("run: {e}"))
    }
    fn metrics(&mut self) -> Option<(u64, u64)> {
        let (store, inst) = self.cur.as_mut()?;
        let mem = inst.get_memory(&mut *store, "memory")?;
        Some((0, mem.size(&*store)))
    }
}
