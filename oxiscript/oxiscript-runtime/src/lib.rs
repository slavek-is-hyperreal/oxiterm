//! OxiScript execution runtime (Phase 8, slice 8.R): a wasmi host that loads
//! ABI-conforming modules, grants them capabilities, bounds everything they
//! can consume, and supports hot reload by instance swap.
//!
//! * [`Host`]: engine + [`Limits`] + registered [`CapabilityProvider`]s.
//! * [`Runtime`]: one running module (`new`, `call_export`, `tick`, `pump`,
//!   `reload`, `restart`, `revoke`). See `runtime.rs` for the trap and reload
//!   policies.
//! * [`caps`]: the reference `log` and `state` capabilities.
//!
//! Spec: `docs/oxiscript/abi.md`, `docs/oxiscript/contracts.md` (8.R).
#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    // Guest values are i32 bit patterns of u32 pointers/handles/lengths; the
    // conversions are intentional and bounds-checked where it matters.
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::struct_field_names,
    clippy::match_same_arms,
    clippy::semicolon_if_nothing_returned,
    clippy::needless_pass_by_value
)]

pub mod capability;
pub mod caps;
pub mod error;
pub mod handle;
mod host;
pub mod limits;
mod load;
pub mod runtime;

pub use capability::{CallbackEvent, Capability, CapabilityProvider, Grants, HostError, Value};
pub use error::{Error, Rejection, Result, TrapCause, TrapInfo};
pub use handle::HandleTable;
pub use limits::Limits;
pub use runtime::{Host, PumpStats, ReloadStats, Runtime};
