//! The capability model.
//!
//! A *capability* is a host object implementing one IDL [`Interface`]. The
//! embedder registers a [`CapabilityProvider`] per interface on the
//! [`crate::Host`]; each [`crate::Runtime`] instance gets its own
//! [`Capability`] object per `needs` line of its manifest (created by the
//! provider, so per-instance tables such as slot handles are naturally
//! isolated, while state *shared across reloads* lives behind the provider).
//!
//! What an implementation can and cannot do:
//! * it receives **host-owned copies** of the arguments ([`Value`]): strings
//!   are already validated UTF-8, narrow integers are range-checked, copy
//!   sizes are capped by [`crate::Limits::max_host_payload`];
//! * it never sees guest memory, the engine, the store or the instance, so it
//!   cannot call back into the guest (the no-re-entrancy rule is structural);
//! * results come back as [`Value`]s; the runtime does all pointer work
//!   (scratch/out-buffer protocol, `(status, T)` flattening, bounds checks);
//! * it is never called with an invalid or revoked *capability* handle (the
//!   runtime validates that first). Handles *it* hands out (IDL `handle`
//!   types such as `Slot`) are its own business: use [`crate::HandleTable`]
//!   and answer `Err(HostError::Denied)` for a bad one.

use oxiscript_abi::idl::{IdlFn, Interface};

/// A value crossing the host boundary (also used for exported-function
/// arguments/results). `Handle` and callback parameters travel as `I32`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
}

/// How a capability call fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// Bad/revoked capability or handle: status `2` for functions that have a
    /// status result, otherwise the instance traps (abi.md 5.1).
    Denied,
    /// A host-defined status (must be `>= 16`) for functions with a status
    /// result; for functions without one the instance traps.
    Status(i32),
    /// Unconditionally trap the instance (resource abuse, host bug).
    Trap(String),
}

/// A host-to-guest callback ready for delivery (between calls only).
#[derive(Clone, Debug, PartialEq)]
pub struct CallbackEvent {
    /// IDL callback name, e.g. `OnChange`.
    pub callback: String,
    /// Closure pointer the guest passed when registering.
    pub closure: u32,
    /// The callback's parameters (at most one `Str`/`Bytes` of at most
    /// `HOST_PAYLOAD_MAX` bytes; others are dropped and counted).
    pub args: Vec<Value>,
}

/// One live capability object, owned by one instance.
pub trait Capability: Send {
    /// Execute interface function `func` with its IDL parameters (without the
    /// implicit capability handle and without the out-buffer). `Ok(None)` for
    /// `void` and `result<void>`; `Ok(Some(v))` for the value of `-> T` and
    /// `result<T>` (`Str`/`Bytes` for the out-buffer forms).
    fn call(&mut self, func: &IdlFn, args: &[Value]) -> Result<Option<Value>, HostError>;

    /// Called before each guest entry (new "turn"): reset per-turn budgets
    /// such as log rate limits. There is no clock; a turn is the unit of rate.
    fn begin_turn(&mut self) {}

    /// Up to `max` callbacks to deliver now (the rest stay queued). Polled by
    /// [`crate::Runtime::pump`], never while the guest runs.
    fn take_events(&mut self, max: usize) -> Vec<CallbackEvent> {
        let _ = max;
        Vec::new()
    }

    /// Closure pointers whose registration the host dropped; the runtime
    /// calls `oxi_release` on each at the next quiescent point (abi.md 5.3).
    fn take_releases(&mut self) -> Vec<u32> {
        Vec::new()
    }
}

/// Factory for [`Capability`] objects of one interface.
pub trait CapabilityProvider: Send + Sync {
    fn interface(&self) -> &Interface;
    /// A fresh capability for a new instance. Called while an instance is
    /// being built; if the build fails the object is dropped (it must undo
    /// any registration it made in shared state, i.e. clean up in `Drop`).
    fn instantiate(&self) -> Box<dyn Capability>;
}

/// Which capabilities a runtime may be granted: a module whose manifest asks
/// for anything else is rejected with `OXS-E0400`.
#[derive(Clone, Debug)]
pub struct Grants(Option<Vec<String>>);

impl Grants {
    /// Everything registered on the host.
    pub fn all() -> Grants {
        Grants(None)
    }

    /// Only these import modules, e.g. `["host:log@1", "host:state@1"]`.
    pub fn only<I, S>(modules: I) -> Grants
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Grants(Some(modules.into_iter().map(Into::into).collect()))
    }

    pub fn none() -> Grants {
        Grants(Some(Vec::new()))
    }

    pub(crate) fn allows(&self, module_at_version: &str) -> bool {
        match &self.0 {
            None => true,
            Some(v) => v.iter().any(|m| m == module_at_version),
        }
    }
}
