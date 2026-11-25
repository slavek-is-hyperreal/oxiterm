//! Wasm module conventions: names and signatures of the exports a conforming
//! OxiScript module provides. The final `.wasm` is one module: the compiler
//! output *linked statically* with the runtime-support code (slice 8.M), so
//! "runtime" exports are ordinary functions inside it; they are exported so
//! the host, tests and tools can call them.
//!
//! **Re-entrancy rule (MVP).** While a host import is running the host MUST
//! NOT call back into the instance (no export, not even `oxi_alloc`). Host
//! to guest data therefore travels through the exported scratch region
//! (`oxi_scratch_ptr`/`oxi_scratch_cap`) or through guest-provided out
//! buffers. This keeps the engine contract trivial (wasmi, WAMR and wasm2c all
//! behave the same) and removes a class of sandbox bugs.

use crate::WasmTy::{self, F64, I32, I64};

/// Who provides an export.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    /// The runtime-support module (slice 8.M, `oxiscript-rc`).
    Runtime,
    /// The compiler-generated part of the module (slice 8.G / 8.L).
    Compiler,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportKind {
    Func,
    /// Immutable global.
    Global,
    /// Mutable global (host may read; only the module writes).
    MutGlobal,
    Memory,
}

/// One row of the export table.
#[derive(Clone, Copy, Debug)]
pub struct ExportSpec {
    pub name: &'static str,
    pub kind: ExportKind,
    pub provider: Provider,
    /// Parameters (functions) or `[ty]` (globals); empty for memory.
    pub params: &'static [WasmTy],
    pub results: &'static [WasmTy],
    pub doc: &'static str,
}

// ---- names (use these constants, never string literals) -----------------

pub const MEMORY: &str = "memory";
pub const ABI_VERSION: &str = "oxi_abi_version";
pub const TRAP_KIND: &str = "oxi_trap_kind";
pub const TYPEDESC_BASE: &str = "oxi_typedesc_base";
pub const TYPEDESC_COUNT: &str = "oxi_typedesc_count";
pub const SCRATCH_PTR: &str = "oxi_scratch_ptr";
pub const SCRATCH_CAP: &str = "oxi_scratch_cap";
pub const INIT: &str = "oxi_init";
pub const DEINIT: &str = "oxi_deinit";
pub const TICK: &str = "oxi_tick";

pub const ALLOC: &str = "oxi_alloc";
pub const ALLOC_BYTES: &str = "oxi_alloc_bytes";
pub const FREE_BYTES: &str = "oxi_free_bytes";
pub const RETAIN: &str = "oxi_retain";
pub const RELEASE: &str = "oxi_release";
pub const CLONE: &str = "oxi_clone";
pub const IS_UNIQUE: &str = "oxi_is_unique";
pub const COLLECT_CYCLES: &str = "oxi_collect_cycles";
pub const LIVE_BYTES: &str = "oxi_live_bytes";
pub const LIVE_OBJECTS: &str = "oxi_live_objects";
pub const STR_NEW: &str = "oxi_str_new";
pub const STR_CONCAT: &str = "oxi_str_concat";
pub const STR_EQ: &str = "oxi_str_eq";
pub const STR_CMP: &str = "oxi_str_cmp";
pub const STR_SLICE: &str = "oxi_str_slice";
pub const STR_FROM_I64: &str = "oxi_str_from_i64";
pub const STR_FROM_F64: &str = "oxi_str_from_f64";
pub const ARRAY_NEW: &str = "oxi_array_new";
pub const ARRAY_GROW: &str = "oxi_array_grow";
pub const TRAP: &str = "oxi_trap";

/// Prefix of the callback trampolines the module exports for host-declared
/// callbacks: `cb:<module>@<version>.<Interface>.<Callback>` (module and
/// version included so two interfaces or two versions never collide), signature
/// `(closure: i32, flattened params...) -> flattened result`.
pub const CALLBACK_PREFIX: &str = "cb:";

/// The name of the callback trampoline export, e.g.
/// `cb:host:state@1.State.OnChange`.
pub fn callback_export_name(module: &str, version: u32, interface: &str, callback: &str) -> String {
    format!("{CALLBACK_PREFIX}{module}@{version}.{interface}.{callback}")
}

/// Minimum size of the scratch region (`oxi_scratch_cap >= SCRATCH_MIN_CAP`).
pub const SCRATCH_MIN_CAP: u32 = 4096;
/// Largest `str`/`bytes` payload a host may deliver to the guest in one call
/// (callback parameter or exported-function parameter). A host that has more
/// data must not deliver it (it fails the originating operation); there is no
/// chunking protocol in ABI 1.0.
pub const HOST_PAYLOAD_MAX: u32 = SCRATCH_MIN_CAP;

/// The fixed export table. User `export function`s are additional exports
/// under their own names (flattened per the IDL/ABI rules); names starting
/// with `oxi_` or `cb:` are reserved and a user export with such a name is a
/// compile error.
pub const EXPORTS: &[ExportSpec] = &[
    ExportSpec { name: MEMORY, kind: ExportKind::Memory, provider: Provider::Compiler, params: &[], results: &[],
        doc: "The single linear memory (wasm32, min size from the data layout, max unset)." },
    ExportSpec { name: ABI_VERSION, kind: ExportKind::Global, provider: Provider::Compiler, params: &[], results: &[I32],
        doc: "ABI_VERSION (major << 16 | minor)." },
    ExportSpec { name: TRAP_KIND, kind: ExportKind::MutGlobal, provider: Provider::Compiler, params: &[], results: &[I32],
        doc: "TrapKind code of the last trap, written by oxi_trap just before `unreachable`; 0 = none." },
    ExportSpec { name: TYPEDESC_BASE, kind: ExportKind::Global, provider: Provider::Compiler, params: &[], results: &[I32],
        doc: "Address of the type descriptor table (see layout::TypeDesc)." },
    ExportSpec { name: TYPEDESC_COUNT, kind: ExportKind::Global, provider: Provider::Compiler, params: &[], results: &[I32],
        doc: "Number of descriptors (highest type id + 1)." },
    ExportSpec { name: SCRATCH_PTR, kind: ExportKind::Global, provider: Provider::Runtime, params: &[], results: &[I32],
        doc: "Start of the scratch region: the one region the host may write without having been handed its address, and only before calling a guest entry point (8-aligned, never moves)." },
    ExportSpec { name: SCRATCH_CAP, kind: ExportKind::Global, provider: Provider::Runtime, params: &[], results: &[I32],
        doc: "Size of the scratch region in bytes (at least 4096)." },
    ExportSpec { name: INIT, kind: ExportKind::Func, provider: Provider::Compiler, params: &[], results: &[],
        doc: "oxi_init(cap_0, ..., cap_n-1): one i32 capability handle per manifest `needs` line, in order; runs module initialisers. Exactly once per instance; the signature depends on the manifest (all params i32, no results)." },
    ExportSpec { name: DEINIT, kind: ExportKind::Func, provider: Provider::Compiler, params: &[], results: &[],
        doc: "Drops module-level state (releases globals) so leak checks see a clean heap." },
    ExportSpec { name: TICK, kind: ExportKind::Func, provider: Provider::Compiler, params: &[I64], results: &[I64],
        doc: "oxi_tick(now_ms) -> next_deadline_ms or -1: run due timers; the host schedules the next call (matches OxiUI's next_deadline step API)." },
    ExportSpec { name: ALLOC, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[I32],
        doc: "oxi_alloc(type_id) -> ptr: zeroed record/closure/union-variant object, rc = 1, meta from the descriptor; traps OutOfMemory." },
    ExportSpec { name: ALLOC_BYTES, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[I32],
        doc: "oxi_alloc_bytes(size) -> ptr: zeroed header-less 8-aligned block (array buffers, scratch)." },
    ExportSpec { name: FREE_BYTES, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[],
        doc: "oxi_free_bytes(ptr, size): frees an oxi_alloc_bytes block." },
    ExportSpec { name: RETAIN, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[],
        doc: "oxi_retain(ptr): rc += 1 unless null or sticky; saturates into sticky; a non-Green object becomes Black (a retained object is not a cycle candidate); a Green object keeps its colour." },
    ExportSpec { name: RELEASE, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[],
        doc: "oxi_release(ptr): rc -= 1 unless null or sticky; at 0 frees (children released via the descriptor pointer map, iteratively, bounded work per call); at nonzero buffers the object as a Purple cycle candidate unless Green." },
    ExportSpec { name: CLONE, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[I32],
        doc: "oxi_clone(ptr) -> ptr: shallow copy of a record/union variant/array, new object rc = 1, every child reference retained." },
    ExportSpec { name: IS_UNIQUE, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[I32],
        doc: "oxi_is_unique(ptr) -> 0/1: rc == 1 and not sticky (copy-on-write test)." },
    ExportSpec { name: COLLECT_CYCLES, kind: ExportKind::Func, provider: Provider::Runtime, params: &[], results: &[I32],
        doc: "Run the cycle collector over buffered candidates; returns the number of objects freed. Called by the host between events, never mid-call." },
    ExportSpec { name: LIVE_BYTES, kind: ExportKind::Func, provider: Provider::Runtime, params: &[], results: &[I32],
        doc: "Bytes currently allocated (objects and blocks), for leak tests and budgets." },
    ExportSpec { name: LIVE_OBJECTS, kind: ExportKind::Func, provider: Provider::Runtime, params: &[], results: &[I32],
        doc: "Number of live managed objects." },
    ExportSpec { name: STR_NEW, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[I32],
        doc: "oxi_str_new(ptr, len) -> string (rc 1): copies and validates UTF-8; traps BadUtf8." },
    ExportSpec { name: STR_CONCAT, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[I32],
        doc: "Borrowed a, b -> new string (rc 1)." },
    ExportSpec { name: STR_EQ, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[I32],
        doc: "Bytewise equality -> 0/1." },
    ExportSpec { name: STR_CMP, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[I32],
        doc: "Bytewise (= code point) order -> -1/0/1." },
    ExportSpec { name: STR_SLICE, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32, I32], results: &[I32],
        doc: "oxi_str_slice(s, start, end) byte offsets -> new string; traps Bounds, BadUtf8 if not on boundaries." },
    ExportSpec { name: STR_FROM_I64, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I64], results: &[I32],
        doc: "Decimal text of a signed integer (u64 uses oxi_str_from_u64 added in a minor version)." },
    ExportSpec { name: STR_FROM_F64, kind: ExportKind::Func, provider: Provider::Runtime, params: &[F64], results: &[I32],
        doc: "Shortest round-trip decimal text; NaN prints `NaN`, infinities `Infinity`/`-Infinity`; deterministic across targets." },
    ExportSpec { name: ARRAY_NEW, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[I32],
        doc: "oxi_array_new(array_type_id, len) -> array (rc 1), len elements zeroed; traps Bounds if len < 0." },
    ExportSpec { name: ARRAY_GROW, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32, I32], results: &[],
        doc: "oxi_array_grow(array, min_cap): ensure cap >= min_cap (amortised doubling); may move the data buffer, never the array object." },
    ExportSpec { name: TRAP, kind: ExportKind::Func, provider: Provider::Runtime, params: &[I32], results: &[],
        doc: "oxi_trap(kind): store kind in oxi_trap_kind, then `unreachable`. Never returns." },
];

pub fn find(name: &str) -> Option<&'static ExportSpec> {
    EXPORTS.iter().find(|e| e.name == name)
}

/// Wasm import namespace prefix for host capabilities.
pub const HOST_NAMESPACE_PREFIX: &str = "host:";

/// Reserved: a name is reserved for the ABI if user code may not export it.
pub fn is_reserved_export_name(name: &str) -> bool {
    name == MEMORY || name.starts_with("oxi_") || name.starts_with(CALLBACK_PREFIX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_unique_and_reserved() {
        let mut names: Vec<_> = EXPORTS.iter().map(|e| e.name).collect();
        names.sort_unstable();
        let n = names.len();
        names.dedup();
        assert_eq!(n, names.len());
        for e in EXPORTS {
            assert!(is_reserved_export_name(e.name), "{}", e.name);
        }
        assert!(!is_reserved_export_name("main"));
    }

    #[test]
    fn callback_names() {
        let n = callback_export_name("host:state", 1, "State", "OnChange");
        assert_eq!(n, "cb:host:state@1.State.OnChange");
        assert!(is_reserved_export_name(&n));
        assert_ne!(
            n,
            callback_export_name("host:state", 2, "State", "OnChange")
        );
    }

    #[test]
    fn lookup() {
        assert_eq!(find(RETAIN).unwrap().params, &[I32]);
        assert!(find("nope").is_none());
    }
}
