//! OxiIR: OxiScript's typed SSA intermediate representation and its text form
//! (Phase 8, slice 8.C). Spec: `docs/oxiscript/ir.md`.
//!
//! * [`types`]: the data model (module, functions, blocks with parameters,
//!   instructions including explicit `retain`/`release`, host imports, memory
//!   ops, traps). `Ty`, `Op` and `Terminator` are `#[non_exhaustive]` so new
//!   kinds (SIMD, device buffers, ...) can be added without breaking
//!   downstream matches; consumers must treat unknown kinds as "unsupported".
//! * [`print`] / [`parse`]: the stable text form, round-trip tested.
//! * [`verify`]: typing and SSA verifier.
//! * [`build`]: a small builder for programmatic construction.
//!
//! Dependency direction: this crate depends only on `oxiscript-abi` (for
//! `TrapKind` and the runtime export table). It does not depend on the AST.
#![allow(
    clippy::doc_markdown,
    clippy::too_many_lines,
    clippy::wildcard_imports,
    clippy::similar_names,
    clippy::many_single_char_names,
    clippy::match_same_arms,
    clippy::cast_lossless,
    clippy::cast_possible_wrap
)]

pub mod build;
pub mod host;
pub mod parse;
pub mod print;
pub mod types;
pub mod verify;

pub use parse::{parse_module, ParseError};
pub use print::print_module;
pub use types::*;
pub use verify::{verify, verify_ok, verify_ok_with, verify_with, VerifyError, VerifyOptions};
