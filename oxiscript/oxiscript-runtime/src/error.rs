//! Structured errors. Every variant that corresponds to a registry entry
//! carries its `OXS-` code ([`Error::code`]).

use oxiscript_abi::trap::TrapKind;
use std::fmt;

/// Why a module was refused before it ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    /// `OXS-E04xx` registry code.
    pub code: &'static str,
    pub message: String,
}

/// Why a running instance stopped. After any of these the instance is
/// *poisoned* (see [`crate::Runtime`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrapCause {
    /// The per-call fuel budget ran out.
    OutOfFuel,
    /// Recursion or value stack limit.
    StackOverflow,
    /// A Wasm load/store/bulk op outside linear memory.
    MemoryOutOfBounds,
    /// `unreachable` executed (guest traps use this plus `guest_kind`).
    Unreachable,
    /// An import rejected the guest's arguments (bad pointer, bad UTF-8,
    /// non-canonical value, payload too large, ...).
    HostBadArgument(String),
    /// A host function without a status result was called with an invalid or
    /// revoked capability/handle.
    HostDenied { interface: String, func: String },
    /// A capability implementation reported an unrecoverable condition.
    HostFailure(String),
    /// Any other engine trap (division by zero, bad table access, ...).
    Engine(String),
}

/// A guest or engine trap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrapInfo {
    pub cause: TrapCause,
    /// The code the guest stored in `oxi_trap_kind` just before trapping
    /// (`None` = 0, an engine trap). Unknown non-zero values are kept raw.
    pub guest_kind_raw: u32,
    /// `guest_kind_raw` as a [`TrapKind`] when it is one.
    pub guest_kind: Option<TrapKind>,
    /// Fuel the call had consumed when it stopped.
    pub fuel_used: u64,
}

impl TrapInfo {
    pub fn code(&self) -> &'static str {
        match &self.cause {
            TrapCause::OutOfFuel => "OXS-E0420",
            TrapCause::StackOverflow => "OXS-E0421",
            TrapCause::MemoryOutOfBounds => "OXS-E0422",
            TrapCause::HostBadArgument(_) => "OXS-E0423",
            TrapCause::HostDenied { .. } => "OXS-E0424",
            TrapCause::HostFailure(_) => "OXS-E0428",
            TrapCause::Unreachable => "OXS-E0425",
            TrapCause::Engine(_) => "OXS-E0428",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// Load-time rejection (nothing was instantiated).
    Rejected(Rejection),
    /// The guest or the engine trapped; the instance is now poisoned.
    Trap(TrapInfo),
    /// The instance trapped earlier; `restart` or `reload` it.
    Poisoned,
    /// A host resource limit (instance count) stopped the operation.
    Limit(String),
    /// The call itself was malformed (unknown export, wrong argument types).
    /// Never poisons the instance.
    BadCall(String),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Error::Rejected(r) => r.code,
            Error::Trap(t) => t.code(),
            Error::Poisoned => "OXS-E0426",
            Error::Limit(_) => "OXS-E0427",
            Error::BadCall(_) => "OXS-E0423",
        }
    }

    pub(crate) fn reject(code: &'static str, message: impl Into<String>) -> Error {
        Error::Rejected(Rejection {
            code,
            message: message.into(),
        })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Rejected(r) => write!(f, "{}: module rejected: {}", r.code, r.message),
            Error::Trap(t) => {
                write!(f, "{}: trap {:?}", t.code(), t.cause)?;
                if t.guest_kind_raw != 0 {
                    match t.guest_kind {
                        Some(k) => write!(f, " (guest kind {})", k.name())?,
                        None => write!(f, " (guest kind unknown({}))", t.guest_kind_raw)?,
                    }
                }
                Ok(())
            }
            Error::Poisoned => write!(f, "OXS-E0426: instance poisoned by an earlier trap"),
            Error::Limit(m) => write!(f, "OXS-E0427: {m}"),
            Error::BadCall(m) => write!(f, "bad call: {m}"),
        }
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;
