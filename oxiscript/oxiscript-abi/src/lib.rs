//! OxiScript ABI contracts (Phase 8, slice 8.C).
//!
//! This crate is a **leaf**: it depends on nothing. It fixes, as Rust
//! constants and small pure functions, everything that independently built
//! components must agree on:
//!
//! * [`layout`]: object header, strings, arrays, closures, unions, type
//!   descriptors and the field-layout function (shared by compiler and
//!   runtime).
//! * [`exports`]: the Wasm module conventions (export names and signatures of
//!   the runtime support functions and of the compiler-generated entry points).
//! * [`idl`]: the host interface definition language, its parser and its
//!   mapping to plain Wasm imports.
//! * [`manifest`]: the `oxi.manifest` custom section.
//! * [`target`]: compilation targets (`device|web|server`) and their Wasm features.
//! * [`trap`]: trap kinds.
//! * [`codes`]: the reason-code registry (diagnostics, placement, laws).
//!
//! Human-readable spec: `docs/oxiscript/abi.md`.
#![allow(clippy::doc_markdown, clippy::too_many_lines)]

pub mod codes;
pub mod exports;
pub mod idl;
pub mod layout;
pub mod manifest;
pub mod target;
pub mod trap;

/// The Core Wasm value types the ABI uses. `V128` (SIMD) is reserved: adding
/// it is a minor ABI version bump (see `docs/oxiscript/contracts.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum WasmTy {
    I32,
    I64,
    F32,
    F64,
}

impl WasmTy {
    pub fn name(self) -> &'static str {
        match self {
            WasmTy::I32 => "i32",
            WasmTy::I64 => "i64",
            WasmTy::F32 => "f32",
            WasmTy::F64 => "f64",
        }
    }
}
