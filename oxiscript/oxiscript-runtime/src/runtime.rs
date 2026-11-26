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

use crate::capability::{CallbackEvent, CapabilityProvider, Grants, InstanceCtx, Value};
use crate::error::{Error, Result, TrapCause, TrapInfo};
use crate::handle::{mix, HandleTable};
use crate::host::{make_import, CapSlot, Data, HostTrap};
use crate::limits::Limits;
use crate::load::{self, val_ty};
use oxiscript_abi::exports;
use oxiscript_abi::idl::{IdlTy, Interface};
use oxiscript_abi::layout::{ABI_VERSION, RESERVED_LOW_BYTES};
use oxiscript_abi::target::{Feature, Target};
use oxiscript_abi::trap::TrapKind;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
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
    /// Counter for automatic namespace names.
    ns_counter: AtomicU64,
    /// Embedder-supplied entropy for opaque handle values (`None` =
    /// deterministic mode: handle values depend on nothing but the module).
    entropy: Option<u64>,
    /// Monotonic microseconds; `None` disables every wall-clock feature.
    clock: Option<Clock>,
    /// The largest compilation target this host runs (docs/oxiscript/abi.md 3.6).
    max_target: Target,
}

/// A monotonic clock in microseconds, supplied by the embedder.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Optional host services. The defaults are fully deterministic except the
/// clock, which defaults to `std::time::Instant` where it exists (not on
/// `wasm32-unknown-unknown`, where it would panic: supply one there).
#[derive(Clone, Default)]
pub struct HostOptions {
    /// Secret entropy (any 16 bytes from the embedder's CSPRNG) mixed into
    /// every instance's handle seed, so handle values are unpredictable and
    /// differ per instance. `None` = deterministic mode (tests, replays).
    pub entropy: Option<[u8; 16]>,
    /// Overrides the default clock.
    pub clock: Option<Clock>,
    /// Run with no clock at all (no wall-time pump cap, zero reload timing).
    pub no_clock: bool,
    /// The largest compilation target this host accepts and configures its
    /// engine for. Default `Device`: a module whose manifest says `web` or
    /// `server` is refused with `OXS-E0411`.
    pub max_target: Target,
}

#[allow(clippy::unnecessary_wraps)]
fn default_clock() -> Option<Clock> {
    #[cfg(not(all(target_arch = "wasm32", target_os = "unknown")))]
    {
        let t0 = std::time::Instant::now();
        let c: Clock = Arc::new(move || t0.elapsed().as_micros() as u64);
        Some(c)
    }
    #[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
    {
        None
    }
}

/// The shared, long-lived part: engine configuration, limits and the
/// registered capability providers. Cheap to clone.
#[derive(Clone)]
pub struct Host {
    inner: Arc<HostInner>,
}

impl Host {
    pub fn new(limits: Limits) -> Host {
        Host::with_options(limits, HostOptions::default())
    }

    /// Like [`Host::new`] with secret entropy for unpredictable handles.
    pub fn with_entropy(limits: Limits, entropy: [u8; 16]) -> Host {
        Host::with_options(
            limits,
            HostOptions {
                entropy: Some(entropy),
                ..HostOptions::default()
            },
        )
    }

    pub fn with_options(limits: Limits, opts: HostOptions) -> Host {
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
            // Target-dependent features: only those the engine can run
            // (`load::ENGINE_FEATURES`: tail-call, extended-const) follow `max_target`.
            .wasm_tail_call(opts.max_target.allows(Feature::TailCall))
            .wasm_extended_const(opts.max_target.allows(Feature::ExtendedConst))
            // Everything else is OFF explicitly, whatever wasmi's Cargo features (and so its
            // defaults) are after feature unification (BAD ACTOR F1). The wasmi `simd` and
            // `memory64` features are compiled in only so these setters exist.
            .wasm_multi_memory(false)
            .wasm_simd(false)
            .wasm_relaxed_simd(false)
            .wasm_memory64(false)
            .wasm_custom_page_sizes(false)
            .wasm_wide_arithmetic(false)
            .allow_start_fn(false)
            .ignore_custom_sections(false)
            .compilation_mode(CompilationMode::Eager)
            // Use default engine limits instead of `EnforcedLimits::strict()`:
            // `strict()` adds a 40 avg-bytes/function rule that only defends lazy
            // compilation (we use `CompilationMode::Eager`) and rejects valid
            // modules with ~1000 tiny functions ([R2-109, R3-61]). Every structural
            // cap from `strict()` (functions, globals, tables, segments, params,
            // results, memories) is enforced directly in `load::reject_reexports`.
            .enforced_limits(EnforcedLimits::default())
            .set_max_recursion_depth(limits.max_recursion_depth)
            .set_min_stack_height(limits.max_stack_height.min(1000))
            .set_max_stack_height(limits.max_stack_height);
        Host {
            inner: Arc::new(HostInner {
                config: c,
                max_target: opts.max_target,
                limits: Arc::new(limits),
                providers: Mutex::new(Vec::new()),
                live: AtomicUsize::new(0),
                epoch: AtomicU32::new(1),
                ns_counter: AtomicU64::new(1),
                entropy: opts.entropy.map(|e| {
                    mix(u64::from_le_bytes(e[..8].try_into().unwrap()))
                        ^ mix(u64::from_le_bytes(e[8..].try_into().unwrap()) ^ 0xA5A5)
                }),
                clock: if opts.no_clock {
                    None
                } else {
                    opts.clock.or_else(default_clock)
                },
            }),
        }
    }

    /// The largest compilation target this host runs.
    pub fn max_target(&self) -> Target {
        self.inner.max_target
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

    /// Build a runtime from a `.wasm` module in a fresh anonymous state
    /// namespace (freed when the `Runtime` drops).
    pub fn instantiate(&self, wasm: &[u8], grants: Grants) -> Result<Runtime> {
        let ns = format!(
            "~rt{}",
            self.inner.ns_counter.fetch_add(1, Ordering::SeqCst)
        );
        Runtime::new(self, wasm, grants, ns, true)
    }

    /// Build a runtime whose persistent state lives in the named namespace
    /// (an app identity). The namespace is *not* freed on drop; the embedder
    /// decides with `StateStore::clear_ns`. Two runtimes with the same name
    /// share their state, so only the embedder may choose names.
    pub fn instantiate_in(&self, wasm: &[u8], grants: Grants, namespace: &str) -> Result<Runtime> {
        Runtime::new(self, wasm, grants, namespace.to_string(), false)
    }
}

struct Guard(Arc<HostInner>);

impl Guard {
    fn acquire(h: &Arc<HostInner>) -> Result<Guard> {
        let max = h.limits.max_instances;
        // A compare-exchange loop rather than `fetch_update`, which Rust 1.100
        // deprecates in favour of `try_update` (not available on older toolchains).
        let mut n = h.live.load(Ordering::SeqCst);
        loop {
            if n >= max {
                return Err(Error::Limit(format!("instance limit of {max} reached")));
            }
            match h.live.compare_exchange_weak(n, n + 1, Ordering::SeqCst, Ordering::SeqCst) {
                Ok(_) => break,
                Err(cur) => n = cur,
            }
        }
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
    /// Capability manifest keys (`host:name@N`) this instance declared in `needs`.
    needs: Vec<String>,
    poisoned: bool,
    last_fuel_used: u64,
    dropped_events: u64,
    /// Closure pointers the host dropped, waiting for `oxi_release`.
    pending_releases: std::collections::VecDeque<u32>,
    /// Events taken from capabilities but not yet delivered (bounded by
    /// `max_events_per_pump`); they belong to this instance's memory and die
    /// with it.
    pending_events: std::collections::VecDeque<(usize, CallbackEvent)>,
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
    /// Events still queued after this pump (carried to the next one).
    pub pending_events: usize,
    /// Releases still queued after this pump.
    pub pending_releases: usize,
}

/// What a successful reload cost.
#[derive(Clone, Copy, Debug)]
pub struct ReloadStats {
    /// Validate + translate + instantiate + `oxi_init`.
    /// Zero when the host has no clock.
    pub build: Duration,
}

/// One running script.
pub struct Runtime {
    host: Arc<HostInner>,
    grants: Grants,
    wasm: Vec<u8>,
    namespace: String,
    anonymous: bool,
    /// Sticky revocations (`host:name@N`): applied to every later build.
    revoked: Vec<String>,
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
    fn build(
        host: &Arc<HostInner>,
        wasm: &[u8],
        grants: &Grants,
        namespace: &str,
        revoked: &[String],
        allowed_needs: Option<&[String]>,
    ) -> Result<Instance> {
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
        // Own policy pre-scan first (re-exports, memory64/table64/shared memory, non-function imports), so the
        // ABI rules do not depend on which wasmi features are compiled in.
        // Target policy first (fail-closed validation of the whole module), then the ABI 1.0 pre-scan.
        load::check_target(wasm, host.max_target, &host.limits)?;
        load::reject_reexports(wasm)?;
        // wasmi can still panic on some inputs it does not model (e.g. a tag
        // import that validation should have refused); a panic is a rejection.
        let module = catch_unwind(AssertUnwindSafe(|| Module::new(&engine, wasm)))
            .map_err(|_| Error::reject("OXS-E0408", "the engine panicked on this module"))?
            .map_err(|e| Error::reject("OXS-E0408", e.to_string()))?;
        let providers = host.providers.lock().unwrap().clone();
        let plan = load::check(&module, limits, &providers, grants)?;
        let needs: Vec<String> = plan
            .manifest
            .needs
            .iter()
            .map(|n| format!("{}@{}", n.module, n.version))
            .collect();
        if let Some(allowed) = allowed_needs {
            for key in &needs {
                if !allowed.iter().any(|a| a == key) {
                    return Err(Error::reject(
                        "OXS-E0400",
                        format!(
                            "hot reload cannot widen capabilities: `{key}` was not in the previous manifest"
                        ),
                    ));
                }
            }
        }

        // Capability objects, one per `needs` line; handles seeded per
        // instance so a handle from another instance never validates.
        let counter = u64::from(host.epoch.fetch_add(1, Ordering::SeqCst));
        let seed = match host.entropy {
            Some(e) => mix(e ^ counter),
            None => 0x5EED_5EED,
        };
        let ctx = InstanceCtx {
            namespace: namespace.to_string(),
            seed,
        };
        let mut handles = HandleTable::new(oxiscript_abi::manifest::MAX_NEEDS, seed);
        let mut caps = Vec::new();
        let mut cap_handles = Vec::new();
        for (i, p) in plan.providers.iter().enumerate() {
            let itf = Arc::new(p.interface().clone());
            let h = handles.insert(i).expect("MAX_NEEDS entries fit");
            if revoked.contains(&itf.import_module()) {
                // Sticky revocation: no object is created (so nothing is
                // registered anywhere); the guest gets a dead handle, so every
                // call through it is denied.
                handles.remove(h);
                caps.push(CapSlot {
                    interface: itf,
                    imp: None,
                });
            } else {
                caps.push(CapSlot {
                    interface: itf,
                    imp: Some(p.instantiate(&ctx)),
                });
            }
            cap_handles.push(h);
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
                clock: host.clock.clone(),
                call_start_us: None,
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
            needs,
            poisoned: false,
            last_fuel_used: 0,
            dropped_events: 0,
            pending_releases: std::collections::VecDeque::new(),
            pending_events: std::collections::VecDeque::new(),
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
        let wall_cap = self.store.data().cfg.call_wall_micros;
        let t0 = if wall_cap > 0 {
            self.store.data().clock.as_ref().map(|c| c())
        } else {
            None
        };
        self.store.data_mut().call_start_us = t0;
        let r = f.call(&mut self.store, args, &mut results);
        self.store.data_mut().call_start_us = None;
        let left = self.store.get_fuel().unwrap_or(0);
        self.last_fuel_used = fuel.saturating_sub(left);
        match r {
            Ok(()) => {
                if wall_cap > 0 {
                    if let (Some(start), Some(clk)) = (t0, self.store.data().clock.as_ref()) {
                        if clk().saturating_sub(start) >= wall_cap {
                            self.poisoned = true;
                            return Err(Error::Trap(TrapInfo {
                                cause: TrapCause::HostFailure("wall-clock limit exceeded".into()),
                                guest_kind_raw: 0,
                                guest_kind: None,
                                fuel_used: self.last_fuel_used,
                            }));
                        }
                    }
                }
                Ok(results)
            }
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
    fn new(
        host: &Host,
        wasm: &[u8],
        grants: Grants,
        namespace: String,
        anonymous: bool,
    ) -> Result<Runtime> {
        let inst = match Instance::build(&host.inner, wasm, &grants, &namespace, &[], None) {
            Ok(i) => i,
            Err(e) => {
                if anonymous {
                    release_ns(&host.inner, &namespace);
                }
                return Err(e);
            }
        };
        Ok(Runtime {
            host: Arc::clone(&host.inner),
            grants,
            wasm: wasm.to_vec(),
            namespace,
            anonymous,
            revoked: Vec::new(),
            inst,
        })
    }

    /// The state namespace of this runtime (stable across `restart` and
    /// `reload`).
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Replace the running code with `wasm`: build and `oxi_init` the new
    /// instance first, then swap. On error the old instance is untouched.
    pub fn reload(&mut self, wasm: &[u8]) -> Result<ReloadStats> {
        let t = self.host.clock.as_ref().map(|c| c());
        let new = Instance::build(
            &self.host,
            wasm,
            &self.grants,
            &self.namespace,
            &self.revoked,
            Some(&self.inst.needs),
        )?;
        let build = match (t, self.host.clock.as_ref()) {
            (Some(t0), Some(c)) => Duration::from_micros(c().saturating_sub(t0)),
            _ => Duration::ZERO,
        };
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
        // Sticky: `restart` and `reload` keep it revoked (a guest can trap
        // itself to force a restart, so recovery must not hand authority back).
        if !self.revoked.iter().any(|m| m == module_at_version) {
            self.revoked.push(module_at_version.to_string());
        }
        true
    }

    /// Lift a revocation. Only the embedder can call this; it takes effect at
    /// the next `restart`/`reload` (the running instance's handle stays dead).
    pub fn regrant(&mut self, module_at_version: &str) -> bool {
        let n = self.revoked.len();
        self.revoked.retain(|m| m != module_at_version);
        n != self.revoked.len()
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
        let lim = Arc::clone(&self.host.limits);
        let clock = self.host.clock.clone();
        let t0 = clock.as_ref().map(|c| c());
        let mut stats = PumpStats::default();
        let release = self.rt_func(exports::RELEASE);
        let ncaps = self.inst.store.data().caps.len();
        // Refill the bounded runtime queues from the capabilities (never
        // beyond the bounds: what does not fit stays inside the capability).
        for ci in 0..ncaps {
            let room = lim
                .max_pending_releases
                .saturating_sub(self.inst.pending_releases.len());
            if room == 0 {
                break;
            }
            let rel = match self.inst.store.data_mut().caps[ci].imp.as_mut() {
                Some(imp) => imp.take_releases(room),
                None => Vec::new(),
            };
            self.inst.pending_releases.extend(rel);
        }
        for ci in 0..ncaps {
            let room = lim
                .max_events_per_pump
                .saturating_sub(self.inst.pending_events.len());
            if room == 0 {
                break;
            }
            let evs = match self.inst.store.data_mut().caps[ci].imp.as_mut() {
                Some(imp) => imp.take_events(room),
                None => Vec::new(),
            };
            self.inst
                .pending_events
                .extend(evs.into_iter().take(room).map(|e| (ci, e)));
        }
        // Fairness: releases and events alternate, so neither starves the
        // other; the aggregate fuel/wall budget caps the whole pump. The first
        // entry always runs, so every pump makes progress.
        let mut spent: u64 = 0;
        let mut first = true;
        let mut turn_release = true;
        let (mut rel_n, mut ev_n) = (0usize, 0usize);
        loop {
            if !first {
                if spent >= lim.pump_fuel {
                    break;
                }
                if lim.pump_wall_micros > 0 {
                    if let (Some(t), Some(c)) = (t0, clock.as_ref()) {
                        if c().saturating_sub(t) >= lim.pump_wall_micros {
                            break;
                        }
                    }
                }
            }
            let can_rel = rel_n < lim.max_events_per_pump && !self.inst.pending_releases.is_empty();
            let can_ev = ev_n < lim.max_events_per_pump && !self.inst.pending_events.is_empty();
            let do_release = match (can_rel, can_ev) {
                (false, false) => break,
                (true, false) => true,
                (false, true) => false,
                (true, true) => turn_release,
            };
            turn_release = !turn_release;
            first = false;
            if do_release {
                let ptr = self.inst.pending_releases.pop_front().expect("checked");
                rel_n += 1;
                self.inst.invoke(release, &[Val::I32(ptr as i32)], None)?;
                stats.released += 1;
            } else {
                let (ci, ev) = self.inst.pending_events.pop_front().expect("checked");
                ev_n += 1;
                let itf = Arc::clone(&self.inst.store.data().caps[ci].interface);
                if self.deliver(&itf, ev)? {
                    stats.delivered += 1;
                } else {
                    stats.dropped += 1;
                    self.inst.dropped_events += 1;
                }
            }
            spent = spent.saturating_add(self.inst.last_fuel_used);
        }
        stats.pending_events = self.inst.pending_events.len();
        stats.pending_releases = self.inst.pending_releases.len();
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

fn release_ns(host: &HostInner, ns: &str) {
    let ps = host.providers.lock().unwrap().clone();
    for p in ps {
        p.release_namespace(ns);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // An anonymous namespace can never be addressed again.
        if self.anonymous {
            release_ns(&self.host, &self.namespace);
        }
    }
}

#[cfg(test)]
mod engine_config_tests {
    //! The engine layer in isolation (BAD ACTOR F1d): `Module::new` against the config
    //! `Host::with_options` builds must refuse every proposal the target policy does not provide,
    //! whatever wasmi Cargo features are compiled in. Bypasses `load::check_target` on purpose.
    use super::*;

    fn refused(host: &Host, body: &str) -> bool {
        let wasm = wat::parse_str(format!("(module {body})")).expect("wat");
        let engine = Engine::new(&host.inner.config);
        Module::new(&engine, &wasm[..]).is_err()
    }

    #[test]
    fn engine_refuses_every_proposal_outside_the_target() {
        let probes: &[(&str, &str)] = &[
            ("simd", "(func (result i32) v128.const i64x2 0 0 i32x4.extract_lane 0)"),
            ("relaxed-simd", "(func (result v128) v128.const i64x2 0 0 i32x4.relaxed_trunc_f32x4_s)"),
            ("memory64", "(memory i64 1)"),
            ("table64", "(table i64 1 funcref)"),
            ("multi-memory", "(memory 1) (memory 1)"),
            ("shared+atomics", "(memory 1 1 shared) (func (param i32) (result i32) local.get 0 i32.atomic.load)"),
            ("externref", "(table 1 externref)"),
            ("trunc_sat", "(func (param f32) (result i32) local.get 0 i32.trunc_sat_f32_s)"),
            ("gc", "(type (struct (field i32)))"),
        ];
        for t in Target::ALL {
            let host = Host::with_options(Limits::default(), HostOptions { max_target: t, ..HostOptions::default() });
            for (name, body) in probes {
                assert!(refused(&host, body), "engine for max_target {t} accepted {name}");
            }
        }
    }

    #[test]
    fn engine_follows_max_target_for_tail_call() {
        let body = "(func $f return_call $f)";
        let dev = Host::with_options(Limits::default(), HostOptions::default());
        let web = Host::with_options(Limits::default(), HostOptions { max_target: Target::Web, ..HostOptions::default() });
        assert!(refused(&dev, body), "device engine accepted return_call");
        assert!(!refused(&web, body), "this test must reach the real config: web engine refused return_call");
    }
}
