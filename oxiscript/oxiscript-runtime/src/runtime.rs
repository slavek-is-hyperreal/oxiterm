//! [`Host`] (engine, limits, registered capabilities) and [`Runtime`] (one
//! running script: a wasmi instance plus its capability objects).
//!
//! # Lifecycle and policies
//!
//! * **Build** (`Runtime::new`, `reload`): size check, engine validation,
//!   ABI check ([`crate::load`]), capability objects created, instantiated
//!   (no start function), ABI globals checked, then `oxi_init(caps...)` with
//!   a fresh fuel budget. Nothing is visible to the embedder until all of it
//!   succeeded.
//! * **Trap policy.** Any trap (fuel, stack, bounds, `unreachable`, host trap,
//!   guest `oxi_trap`) *poisons* the instance: every later call returns
//!   [`Error::Poisoned`] (abi.md 3.4: a trapped instance is only discarded).
//!   The runtime object stays usable: [`Runtime::restart`] rebuilds the same
//!   module from scratch (module state resets, host-held state survives) and
//!   [`Runtime::reload`] swaps in new code; both work on a poisoned runtime.
//!   Malformed *calls* (unknown export, wrong argument types) are plain
//!   errors and do not poison.
//! * **Reload rule.** The new instance is built and `oxi_init` is run to
//!   completion *before* the swap; on any failure the old instance is kept
//!   untouched (even if poisoned) and the new one is dropped, which drops its
//!   capability objects (and their watcher registrations). Module-local
//!   state resets; state held by capability providers (the `state` store)
//!   survives. The new instance's `oxi_init` already ran against the live
//!   host state, so its host-side writes persist even if a later step of the
//!   reload fails (documented, not staged). Callbacks registered by the old
//!   instance die with it (its memory is discarded: no `oxi_release`); the
//!   new instance registers its own during `oxi_init`/`main`, and events for
//!   those registrations are only delivered by `pump` after the swap.
//! * **No re-entrancy.** Guest entry points are only reachable from `&mut
//!   Runtime` methods; capability code receives plain values, never the
//!   engine, so a host import cannot call the guest.

use crate::capability::{CallbackEvent, CapabilityProvider, Grants, Value};
use crate::error::{Error, Result, TrapCause, TrapInfo};
use crate::handle::HandleTable;
use crate::host::{make_import, CapSlot, Data, HostTrap};
use crate::limits::Limits;
use crate::load::{self, val_ty};
use oxiscript_abi::exports;
use oxiscript_abi::idl::{IdlTy, Interface};
use oxiscript_abi::layout::{ABI_VERSION, RESERVED_LOW_BYTES};
use oxiscript_abi::trap::TrapKind;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use wasmi::{
    CompilationMode, Config, EnforcedLimits, Engine, ExternType, Func, FuncType, Global, Linker,
    Memory, Module, Store, StoreLimitsBuilder, TrapCode, Val, ValType,
};

const PAGE: usize = 65_536;

struct HostInner {
    /// One `Engine` per instance (built from this config): wasmi's engine
    /// keeps the compiled code of every module it ever translated, so a
    /// shared engine would grow with each reload.
    config: Config,
    limits: Arc<Limits>,
    providers: Mutex<Vec<Arc<dyn CapabilityProvider>>>,
    live: AtomicUsize,
    epoch: AtomicU32,
}

/// The shared, long-lived part: engine configuration, limits and the
/// registered capability providers. Cheap to clone.
#[derive(Clone)]
pub struct Host {
    inner: Arc<HostInner>,
}

impl Host {
    pub fn new(limits: Limits) -> Host {
        let mut c = Config::default();
        // Exactly the ABI's feature set: MVP + multi-value + bulk-memory +
        // sign-extension, plus mutable globals (needed for the exported
        // `oxi_trap_kind`). Everything else is off.
        c.consume_fuel(true)
            .floats(true)
            .wasm_mutable_global(true)
            .wasm_sign_extension(true)
            .wasm_multi_value(true)
            .wasm_bulk_memory(true)
            .wasm_saturating_float_to_int(false)
            .wasm_reference_types(false)
            .wasm_tail_call(false)
            .wasm_extended_const(false)
            .wasm_multi_memory(false)
            .wasm_custom_page_sizes(false)
            .wasm_wide_arithmetic(false)
            .allow_start_fn(false)
            .ignore_custom_sections(false)
            .compilation_mode(CompilationMode::Eager)
            .enforced_limits(EnforcedLimits::strict())
            .set_max_recursion_depth(limits.max_recursion_depth)
            .set_min_stack_height(limits.max_stack_height.min(1000))
            .set_max_stack_height(limits.max_stack_height);
        Host {
            inner: Arc::new(HostInner {
                config: c,
                limits: Arc::new(limits),
                providers: Mutex::new(Vec::new()),
                live: AtomicUsize::new(0),
                epoch: AtomicU32::new(1),
            }),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.inner.limits
    }

    /// Register a capability provider. Fails if the same `module@version`
    /// is already registered.
    pub fn register(
        &self,
        provider: Arc<dyn CapabilityProvider>,
    ) -> std::result::Result<(), String> {
        let mut v = self.inner.providers.lock().unwrap();
        let key = provider.interface().import_module();
        if v.iter().any(|p| p.interface().import_module() == key) {
            return Err(format!("`{key}` is already registered"));
        }
        v.push(provider);
        Ok(())
    }

    /// Instances currently alive (including a reload's transient one).
    pub fn live_instances(&self) -> usize {
        self.inner.live.load(Ordering::SeqCst)
    }

    /// Build a runtime from a `.wasm` module.
    pub fn instantiate(&self, wasm: &[u8], grants: Grants) -> Result<Runtime> {
        Runtime::new(self, wasm, grants)
    }
}

struct Guard(Arc<HostInner>);

impl Guard {
    fn acquire(h: &Arc<HostInner>) -> Result<Guard> {
        let max = h.limits.max_instances;
        h.live
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
                (n < max).then_some(n + 1)
            })
            .map_err(|_| Error::Limit(format!("instance limit of {max} reached")))?;
        Ok(Guard(Arc::clone(h)))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Instance {
    store: Store<Data>,
    instance: wasmi::Instance,
    memory: Memory,
    trap_kind: Global,
    scratch_ptr: u32,
    scratch_cap: u32,
    cap_handles: Vec<u32>,
    poisoned: bool,
    last_fuel_used: u64,
    dropped_events: u64,
    /// Closure pointers the host dropped, waiting for `oxi_release`.
    pending_releases: std::collections::VecDeque<u32>,
    // Dropped last (fields drop in order): the live-instance count falls only
    // after the store and its capability objects are gone.
    _guard: Guard,
}

/// Counters from one [`Runtime::pump`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PumpStats {
    pub delivered: usize,
    /// Events dropped because they broke the delivery rules (more than one
    /// payload, payload above `HOST_PAYLOAD_MAX`, wrong argument types, unknown
    /// callback).
    pub dropped: usize,
    pub released: usize,
}

/// What a successful reload cost.
#[derive(Clone, Copy, Debug)]
pub struct ReloadStats {
    /// Validate + translate + instantiate + `oxi_init`.
    pub build: Duration,
}

/// One running script.
pub struct Runtime {
    host: Arc<HostInner>,
    grants: Grants,
    wasm: Vec<u8>,
    inst: Instance,
}

fn to_val(v: &Value) -> Option<Val> {
    Some(match v {
        Value::I32(x) => Val::I32(*x),
        Value::I64(x) => Val::I64(*x),
        Value::F32(x) => Val::F32((*x).into()),
        Value::F64(x) => Val::F64((*x).into()),
        _ => return None,
    })
}

fn from_val(v: &Val) -> Value {
    match v {
        Val::I32(x) => Value::I32(*x),
        Val::I64(x) => Value::I64(*x),
        Val::F32(x) => Value::F32(f32::from(*x)),
        Val::F64(x) => Value::F64(f64::from(*x)),
        _ => Value::I32(0),
    }
}

impl Instance {
    fn build(host: &Arc<HostInner>, wasm: &[u8], grants: &Grants) -> Result<Instance> {
        let limits = &host.limits;
        if wasm.len() > limits.max_module_bytes {
            return Err(Error::reject(
                "OXS-E0409",
                format!(
                    "module is {} bytes, the limit is {}",
                    wasm.len(),
                    limits.max_module_bytes
                ),
            ));
        }
        let guard = Guard::acquire(host)?;
        let engine = Engine::new(&host.config);
        // wasmi can still panic on some inputs it does not model (e.g. a tag
        // import that validation should have refused); a panic is a rejection.
        let module = catch_unwind(AssertUnwindSafe(|| Module::new(&engine, wasm)))
            .map_err(|_| Error::reject("OXS-E0408", "the engine panicked on this module"))?
            .map_err(|e| Error::reject("OXS-E0408", e.to_string()))?;
        let providers = host.providers.lock().unwrap().clone();
        let plan = load::check(&module, limits, &providers, grants)?;

        // Capability objects, one per `needs` line; handles seeded per
        // instance so a handle from another instance never validates.
        let seed = host.epoch.fetch_add(1, Ordering::SeqCst).wrapping_mul(2654);
        let mut handles = HandleTable::new(oxiscript_abi::manifest::MAX_NEEDS, seed);
        let mut caps = Vec::new();
        let mut cap_handles = Vec::new();
        for (i, p) in plan.providers.iter().enumerate() {
            caps.push(CapSlot {
                interface: Arc::new(p.interface().clone()),
                imp: Some(p.instantiate()),
            });
            cap_handles.push(handles.insert(i).expect("MAX_NEEDS entries fit"));
        }
        let store_limits = StoreLimitsBuilder::new()
            .memory_size(limits.max_memory_pages as usize * PAGE)
            .table_elements(limits.max_table_elements as usize)
            .instances(1)
            .memories(1)
            .tables(1)
            .trap_on_grow_failure(false)
            .build();
        let mut store = Store::new(
            &engine,
            Data {
                limits: store_limits,
                cfg: Arc::clone(limits),
                caps,
                handles,
                memory: None,
                host_calls: 0,
            },
        );
        store.limiter(|d| &mut d.limits);
        let _ = store.set_fuel(limits.fuel_per_call);

        let mut linker = <Linker<Data>>::new(&engine);
        for imp in module.imports() {
            let (ci, itf) = store
                .data()
                .caps
                .iter()
                .enumerate()
                .find(|(_, c)| c.interface.import_module() == imp.module())
                .map(|(i, c)| (i, Arc::clone(&c.interface)))
                .expect("checked by load::check");
            let fi = itf
                .fns
                .iter()
                .position(|f| f.name == imp.name())
                .expect("checked by load::check");
            let w = itf
                .wasm_imports()
                .into_iter()
                .find(|w| w.name == imp.name())
                .expect("checked by load::check");
            let ft = FuncType::new(
                w.params.iter().map(|t| val_ty(*t)),
                w.results.iter().map(|t| val_ty(*t)),
            );
            linker
                .func_new(imp.module(), imp.name(), ft, make_import(ci, itf, fi))
                .map_err(|e| Error::reject("OXS-E0408", e.to_string()))?;
        }
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|e| Error::reject("OXS-E0409", format!("instantiation failed: {e}")))?;

        let g = |name: &str| -> Result<Global> {
            instance
                .get_global(&store, name)
                .ok_or_else(|| Error::reject("OXS-E0406", format!("export `{name}`: missing")))
        };
        let memory = instance
            .get_memory(&store, exports::MEMORY)
            .ok_or_else(|| Error::reject("OXS-E0406", "export `memory`: missing"))?;
        let trap_kind = g(exports::TRAP_KIND)?;
        let geti = |gl: Global, store: &Store<Data>| match gl.get(store) {
            Val::I32(x) => x as u32,
            _ => 0,
        };
        let version = geti(g(exports::ABI_VERSION)?, &store);
        let want = (u32::from(plan.manifest.abi_major) << 16) | u32::from(plan.manifest.abi_minor);
        let host_major = ABI_VERSION >> 16;
        if version != want
            || version >> 16 != host_major
            || (version & 0xFFFF) > (ABI_VERSION & 0xFFFF)
        {
            return Err(Error::reject(
                "OXS-E0403",
                format!(
                    "`oxi_abi_version` is {}.{}, the manifest says {}.{}",
                    version >> 16,
                    version & 0xFFFF,
                    plan.manifest.abi_major,
                    plan.manifest.abi_minor
                ),
            ));
        }
        let scratch_ptr = geti(g(exports::SCRATCH_PTR)?, &store);
        let scratch_cap = geti(g(exports::SCRATCH_CAP)?, &store);
        let mem_len = memory.data_size(&store);
        let end = u64::from(scratch_ptr) + u64::from(scratch_cap);
        if scratch_ptr < RESERVED_LOW_BYTES
            || scratch_ptr % 8 != 0
            || scratch_cap < exports::SCRATCH_MIN_CAP
            || end > mem_len as u64
        {
            return Err(Error::reject(
                "OXS-E0410",
                format!(
                    "scratch region ({scratch_ptr}, {scratch_cap}) is invalid for a memory of {mem_len} bytes"
                ),
            ));
        }
        store.data_mut().memory = Some(memory);

        let mut inst = Instance {
            store,
            instance,
            memory,
            trap_kind,
            scratch_ptr,
            scratch_cap,
            cap_handles,
            poisoned: false,
            last_fuel_used: 0,
            dropped_events: 0,
            pending_releases: std::collections::VecDeque::new(),
            _guard: guard,
        };
        let init = inst
            .instance
            .get_func(&inst.store, exports::INIT)
            .expect("checked by load::check");
        let args: Vec<Val> = inst
            .cap_handles
            .iter()
            .map(|h| Val::I32(*h as i32))
            .collect();
        inst.invoke(init, &args, None)?;
        Ok(inst)
    }

    fn zero_results(ft: &FuncType) -> Vec<Val> {
        ft.results()
            .iter()
            .map(|t| Val::default_for_ty(*t))
            .collect()
    }

    /// Enter the guest. Resets fuel, runs `begin_turn` on every capability,
    /// writes the optional scratch payload, calls, and converts any failure
    /// into a structured trap that poisons the instance.
    fn invoke(&mut self, f: Func, args: &[Val], payload: Option<&[u8]>) -> Result<Vec<Val>> {
        if self.poisoned {
            return Err(Error::Poisoned);
        }
        let fuel = self.store.data().cfg.fuel_per_call;
        let _ = self.store.set_fuel(fuel);
        for c in &mut self.store.data_mut().caps {
            if let Some(imp) = c.imp.as_mut() {
                imp.begin_turn();
            }
        }
        if let Some(p) = payload {
            // Validated at build time: scratch lies inside memory, which only grows.
            self.memory
                .write(&mut self.store, self.scratch_ptr as usize, p)
                .map_err(|e| Error::BadCall(format!("scratch write failed: {e}")))?;
        }
        let ft = f.ty(&self.store);
        let mut results = Self::zero_results(&ft);
        let r = f.call(&mut self.store, args, &mut results);
        let left = self.store.get_fuel().unwrap_or(0);
        self.last_fuel_used = fuel.saturating_sub(left);
        match r {
            Ok(()) => Ok(results),
            Err(e) => {
                self.poisoned = true;
                Err(Error::Trap(self.trap_info(&e)))
            }
        }
    }

    fn trap_info(&self, e: &wasmi::Error) -> TrapInfo {
        let raw = match self.trap_kind.get(&self.store) {
            Val::I32(x) => x as u32,
            _ => 0,
        };
        let cause = if let Some(h) = e.downcast_ref::<HostTrap>() {
            h.0.clone()
        } else {
            match e.as_trap_code() {
                Some(TrapCode::OutOfFuel) => TrapCause::OutOfFuel,
                Some(TrapCode::StackOverflow) => TrapCause::StackOverflow,
                Some(TrapCode::MemoryOutOfBounds) => TrapCause::MemoryOutOfBounds,
                Some(TrapCode::UnreachableCodeReached) => TrapCause::Unreachable,
                Some(c) => TrapCause::Engine(format!("{c:?}")),
                None => TrapCause::Engine(e.to_string()),
            }
        };
        // `oxi_trap_kind` is guest-writable: believe it only when the guest
        // actually stopped with `unreachable` (what `oxi_trap` does).
        let raw = if cause == TrapCause::Unreachable {
            raw
        } else {
            0
        };
        TrapInfo {
            cause,
            guest_kind_raw: raw,
            guest_kind: TrapKind::from_code(raw),
            fuel_used: self.last_fuel_used,
        }
    }

    /// Turn host-side arguments into Wasm values; at most one `Str`/`Bytes`
    /// is allowed, delivered through the scratch region as `(ptr, len)`.
    fn expand(&self, ft: &FuncType, args: &[Value]) -> Result<(Vec<Val>, Option<Vec<u8>>)> {
        let mut out: Vec<Val> = Vec::new();
        let mut payload: Option<Vec<u8>> = None;
        for a in args {
            match a {
                Value::Str(_) | Value::Bytes(_) => {
                    if payload.is_some() {
                        return Err(Error::BadCall(
                            "at most one str/bytes argument (abi.md 3.1)".into(),
                        ));
                    }
                    let b: Vec<u8> = match a {
                        Value::Str(s) => s.clone().into_bytes(),
                        Value::Bytes(b) => b.clone(),
                        _ => unreachable!(),
                    };
                    if b.len() > exports::HOST_PAYLOAD_MAX as usize
                        || b.len() > self.scratch_cap as usize
                    {
                        return Err(Error::BadCall(format!(
                            "payload of {} bytes exceeds HOST_PAYLOAD_MAX",
                            b.len()
                        )));
                    }
                    out.push(Val::I32(self.scratch_ptr as i32));
                    out.push(Val::I32(b.len() as i32));
                    payload = Some(b);
                }
                v => out.push(to_val(v).expect("scalar")),
            }
        }
        if out.len() != ft.params().len() || out.iter().zip(ft.params()).any(|(v, t)| v.ty() != *t)
        {
            return Err(Error::BadCall(format!(
                "argument types do not match the signature {:?}",
                ft.params()
            )));
        }
        Ok((out, payload))
    }
}

impl Runtime {
    pub fn new(host: &Host, wasm: &[u8], grants: Grants) -> Result<Runtime> {
        let inst = Instance::build(&host.inner, wasm, &grants)?;
        Ok(Runtime {
            host: Arc::clone(&host.inner),
            grants,
            wasm: wasm.to_vec(),
            inst,
        })
    }

    /// Replace the running code with `wasm`: build and `oxi_init` the new
    /// instance first, then swap. On error the old instance is untouched.
    pub fn reload(&mut self, wasm: &[u8]) -> Result<ReloadStats> {
        let t = Instant::now();
        let new = Instance::build(&self.host, wasm, &self.grants)?;
        let build = t.elapsed();
        self.inst = new;
        self.wasm = wasm.to_vec();
        Ok(ReloadStats { build })
    }

    /// Rebuild the current module from scratch (recovery after a trap).
    pub fn restart(&mut self) -> Result<ReloadStats> {
        let wasm = std::mem::take(&mut self.wasm);
        let r = self.reload(&wasm);
        if r.is_err() {
            self.wasm = wasm;
        }
        r
    }

    pub fn is_poisoned(&self) -> bool {
        self.inst.poisoned
    }

    /// Fuel the most recent guest entry consumed.
    pub fn last_fuel_used(&self) -> u64 {
        self.inst.last_fuel_used
    }

    /// Number of import calls the current instance made.
    pub fn host_calls(&self) -> u64 {
        self.inst.store.data().host_calls
    }

    /// Current linear memory size in pages.
    pub fn memory_pages(&self) -> usize {
        self.inst.memory.data_size(&self.inst.store) / PAGE
    }

    /// Callback events dropped for breaking the delivery rules.
    pub fn dropped_events(&self) -> u64 {
        self.inst.dropped_events
    }

    /// Revoke a capability (by `host:name@N`): its object is dropped and every
    /// later call through its handle is denied (status 2, or a trap for
    /// functions without a status result). Returns whether it existed.
    pub fn revoke(&mut self, module_at_version: &str) -> bool {
        let d = self.inst.store.data_mut();
        let Some(ci) = d
            .caps
            .iter()
            .position(|c| c.interface.import_module() == module_at_version)
        else {
            return false;
        };
        let h = self.inst.cap_handles[ci];
        d.handles.remove(h);
        d.caps[ci].imp = None;
        true
    }

    fn user_func(&self, name: &str) -> Result<Func> {
        if exports::is_reserved_export_name(name) {
            return Err(Error::BadCall(format!("`{name}` is not a user export")));
        }
        self.inst
            .instance
            .get_func(&self.inst.store, name)
            .ok_or_else(|| Error::BadCall(format!("no exported function `{name}`")))
    }

    /// Call a user `export function`. `Str`/`Bytes` arguments (at most one)
    /// are delivered through the scratch region.
    pub fn call_export(&mut self, name: &str, args: &[Value]) -> Result<Vec<Value>> {
        if self.inst.poisoned {
            return Err(Error::Poisoned);
        }
        let f = self.user_func(name)?;
        let ft = f.ty(&self.inst.store);
        let (vals, payload) = self.inst.expand(&ft, args)?;
        let res = self.inst.invoke(f, &vals, payload.as_deref())?;
        Ok(res.iter().map(from_val).collect())
    }

    fn rt_func(&self, name: &str) -> Func {
        self.inst
            .instance
            .get_func(&self.inst.store, name)
            .expect("checked by load::check")
    }

    /// `oxi_tick(now_ms)`: returns the next deadline in ms or `-1`.
    pub fn tick(&mut self, now_ms: i64) -> Result<i64> {
        let f = self.rt_func(exports::TICK);
        match self.inst.invoke(f, &[Val::I64(now_ms)], None)?.first() {
            Some(Val::I64(d)) => Ok(*d),
            _ => Ok(-1),
        }
    }

    /// `oxi_collect_cycles()`: objects freed.
    pub fn collect_cycles(&mut self) -> Result<i32> {
        let f = self.rt_func(exports::COLLECT_CYCLES);
        match self.inst.invoke(f, &[], None)?.first() {
            Some(Val::I32(n)) => Ok(*n),
            _ => Ok(0),
        }
    }

    /// `oxi_live_objects()` for leak checks.
    pub fn live_objects(&mut self) -> Result<i32> {
        let f = self.rt_func(exports::LIVE_OBJECTS);
        match self.inst.invoke(f, &[], None)?.first() {
            Some(Val::I32(n)) => Ok(*n),
            _ => Ok(0),
        }
    }

    /// Call `oxi_deinit()` and return `oxi_live_objects()` (0 = no leaks). The
    /// instance should be dropped afterwards.
    pub fn deinit(&mut self) -> Result<i32> {
        let f = self.rt_func(exports::DEINIT);
        self.inst.invoke(f, &[], None)?;
        self.live_objects()
    }

    /// The quiescent point: perform queued releases, then deliver up to
    /// `max_events_per_pump` capability callbacks, in capability order. Events
    /// raised while pumping wait for the next pump (so a watcher that writes
    /// what it watches cannot loop inside one pump).
    pub fn pump(&mut self) -> Result<PumpStats> {
        if self.inst.poisoned {
            return Err(Error::Poisoned);
        }
        let mut stats = PumpStats::default();
        let release = self.rt_func(exports::RELEASE);
        let ncaps = self.inst.store.data().caps.len();
        for ci in 0..ncaps {
            let rel = match self.inst.store.data_mut().caps[ci].imp.as_mut() {
                Some(imp) => imp.take_releases(),
                None => Vec::new(),
            };
            self.inst.pending_releases.extend(rel);
        }
        // Bounded like events: a guest that makes the host queue thousands of
        // releases must not turn one pump into thousands of guest entries.
        let mut rel_budget = self.host.limits.max_events_per_pump;
        while rel_budget > 0 {
            let Some(ptr) = self.inst.pending_releases.pop_front() else {
                break;
            };
            rel_budget -= 1;
            self.inst.invoke(release, &[Val::I32(ptr as i32)], None)?;
            stats.released += 1;
        }
        let mut budget = self.host.limits.max_events_per_pump;
        for ci in 0..ncaps {
            if budget == 0 {
                break;
            }
            let evs = match self.inst.store.data_mut().caps[ci].imp.as_mut() {
                Some(imp) => imp.take_events(budget),
                None => Vec::new(),
            };
            let itf = Arc::clone(&self.inst.store.data().caps[ci].interface);
            for ev in evs.into_iter().take(budget) {
                budget -= 1;
                if self.deliver(&itf, ev)? {
                    stats.delivered += 1;
                } else {
                    stats.dropped += 1;
                    self.inst.dropped_events += 1;
                }
            }
        }
        Ok(stats)
    }

    /// Deliver one callback. `Ok(false)` = the event broke the rules and was dropped.
    fn deliver(&mut self, itf: &Interface, ev: CallbackEvent) -> Result<bool> {
        let Some(cb) = itf.callbacks.iter().find(|c| c.name == ev.callback) else {
            return Ok(false);
        };
        let name = exports::callback_export_name(&itf.module, itf.version, &itf.name, &cb.name);
        let Some(f) = self.inst.instance.get_func(&self.inst.store, &name) else {
            return Ok(false);
        };
        if ev.args.len() != cb.params.len() {
            return Ok(false);
        }
        let mut vals = vec![Val::I32(ev.closure as i32)];
        let mut payload: Option<Vec<u8>> = None;
        for (p, a) in cb.params.iter().zip(&ev.args) {
            match (&p.ty, a) {
                (IdlTy::Str, Value::Str(_)) | (IdlTy::Bytes, Value::Bytes(_))
                    if payload.is_none() =>
                {
                    let b = s_bytes(a);
                    if b.len() > exports::HOST_PAYLOAD_MAX as usize {
                        return Ok(false);
                    }
                    vals.push(Val::I32(self.inst.scratch_ptr as i32));
                    vals.push(Val::I32(b.len() as i32));
                    payload = Some(b);
                }
                (IdlTy::Prim(pr), Value::I32(x))
                    if matches!(pr.wasm(), oxiscript_abi::WasmTy::I32) =>
                {
                    vals.push(Val::I32(*x))
                }
                (IdlTy::Prim(pr), Value::I64(x))
                    if matches!(pr.wasm(), oxiscript_abi::WasmTy::I64) =>
                {
                    vals.push(Val::I64(*x))
                }
                (IdlTy::Prim(pr), Value::F32(x))
                    if matches!(pr.wasm(), oxiscript_abi::WasmTy::F32) =>
                {
                    vals.push(Val::F32((*x).into()))
                }
                (IdlTy::Prim(pr), Value::F64(x))
                    if matches!(pr.wasm(), oxiscript_abi::WasmTy::F64) =>
                {
                    vals.push(Val::F64((*x).into()))
                }
                (IdlTy::Handle(_), Value::I32(x)) => vals.push(Val::I32(*x)),
                _ => return Ok(false),
            }
        }
        self.inst.invoke(f, &vals, payload.as_deref())?;
        Ok(true)
    }

    /// Names of the user exports (everything not reserved by the ABI).
    pub fn user_exports(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .inst
            .instance
            .exports(&self.inst.store)
            .filter(|e| !exports::is_reserved_export_name(e.name()))
            .map(|e| e.name().to_string())
            .collect();
        v.sort();
        v
    }

    /// The signature of a user export (for embedders that bind by type).
    pub fn export_type(&self, name: &str) -> Option<(Vec<ValType>, Vec<ValType>)> {
        match self
            .inst
            .instance
            .get_export(&self.inst.store, name)?
            .ty(&self.inst.store)
        {
            ExternType::Func(ft) => Some((ft.params().to_vec(), ft.results().to_vec())),
            _ => None,
        }
    }

    /// Direct read of guest memory for tests and tooling (bounds-checked).
    pub fn read_memory(&self, ptr: u32, len: u32) -> Option<Vec<u8>> {
        let d = self.inst.memory.data(&self.inst.store);
        let end = (ptr as usize).checked_add(len as usize)?;
        d.get(ptr as usize..end).map(<[u8]>::to_vec)
    }

    pub fn scratch(&self) -> (u32, u32) {
        (self.inst.scratch_ptr, self.inst.scratch_cap)
    }
}

fn s_bytes(v: &Value) -> Vec<u8> {
    match v {
        Value::Str(s) => s.clone().into_bytes(),
        Value::Bytes(b) => b.clone(),
        _ => Vec::new(),
    }
}

/// Lookup helper kept for embedders that list what a host offers.
impl Host {
    pub fn interfaces(&self) -> Vec<Interface> {
        self.inner
            .providers
            .lock()
            .unwrap()
            .iter()
            .map(|p| p.interface().clone())
            .collect()
    }
}
