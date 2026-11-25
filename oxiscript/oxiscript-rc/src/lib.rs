//! OxiScript memory runtime (Phase 8, slice 8.M): reference counting with a
//! cycle collector, the allocator, and the strings/arrays support functions of
//! the runtime export table (`docs/oxiscript/abi.md`).
//!
//! # Shape of the crate
//!
//! * [`rt`]: the algorithms (`Rt<H: Heap>`): allocation, retain/release with
//!   sticky counts, the recursion-free destruction queue, Bacon-Rajan trial
//!   deletion with a work limit, `clone`, arrays, strings.
//! * [`heap`]: the memory abstraction `Rt` is generic over.
//! * `wasm` (wasm32 only): the guest build. `#![no_std]`, raw linear memory,
//!   **dlmalloc** (MIT OR Apache-2.0, the allocator Rust's own wasm32 std
//!   uses) as the block allocator under our object header, `#[no_mangle]`
//!   exports with the exact names and signatures of `exports::EXPORTS`.
//! * `sim` (native only): a checked arena heap (also dlmalloc) so the same
//!   algorithms are unit-tested and fuzzed natively with use-after-free,
//!   double-free and out-of-bounds detection.
//!
//! # Linking model (what 8.G / 8.R must know)
//!
//! The ABI says one module = compiler output statically linked with this
//! runtime. Stable Rust cannot define or write an arbitrary Wasm *global*, so:
//!
//! * `oxi_scratch_ptr`: the exported static *is* the 4096-byte scratch buffer,
//!   so the exported global's value (an address) is exactly the ABI value.
//! * `oxi_scratch_cap`: not a global here; function `oxi_rt_scratch_cap()`
//!   returns it and the linker materialises the global (value 4096).
//! * `oxi_trap_kind`: `oxi_trap` stores the kind in the memory cell
//!   `oxi_trap_cell` (exported address) before `unreachable`; the linking
//!   step either mirrors it into the mutable global or replaces `oxi_trap`
//!   by its own three-instruction version.
//! * `oxi_typedesc_base/count` are compiler globals the runtime cannot read:
//!   the generated `oxi_init` must call the extension export
//!   `oxi_rt_init(base, count)` before anything else.
//!
//! # Extension exports (additive; to be promoted into `exports::EXPORTS`)
//!
//! `oxi_rt_init(base, count)`, `oxi_rt_scratch_cap() -> i32`,
//! `oxi_gc_step(budget) -> i32` (bits: 1 more work, 2 deferred candidates need
//! a full `oxi_collect_cycles`), `oxi_gc_stat(key) -> i32`,
//! `oxi_gc_config(key, value)`.
//!
//! # Semantics beyond the ABI text
//!
//! * `release` to zero never recurses: the object joins an intrusive queue (its
//!   rc word is the link) and every `release`/`alloc` call drains at most
//!   `drain_budget` (default 256) work units; an array is resumable per
//!   element. `oxi_live_*` and `oxi_collect_cycles` drain completely.
//! * Trial deletion never runs inside `alloc`/`release`. `oxi_collect_cycles`
//!   is unbounded; `oxi_gc_step(budget)` is the bounded form (see `rt`).
//! * Colours 5 (PENDING) and 6 (ZOMBIE) are runtime-internal and appear only
//!   on dead objects.
//! * `oxi_alloc_bytes(0)` returns 0 and `oxi_free_bytes(0, _)` is a no-op.
//!
//! # What IR pass 8.L may elide (ir.md section 4) and what is guaranteed
//!
//! The runtime treats `retain`/`release` as exact, independent operations, so
//! any pair removal that keeps every object's count correct at every call
//! boundary is sound:
//!
//! * `retain x ... release x` with no `call`, `call_closure`, host call,
//!   `setfield`, `array.set`, `array.push`, `global.set`, `clone` or
//!   `release` of a possible alias between them: removable (nothing can drop
//!   the other reference meanwhile).
//! * Owned-to-borrowed handoff: passing an owned value to a borrowed
//!   parameter and releasing it afterwards may be replaced by letting the
//!   callee borrow and releasing once; the runtime only sees the final count.
//! * `clone` becomes in-place mutation only when `oxi_is_unique` is proven
//!   (rc == 1 and not sticky). A sticky object is *never* unique, so a
//!   saturated count degrades to copying, never to aliasing.
//! * Borrowed (`c = 0`) values: the runtime gives them no protection. A borrow
//!   loaded from a global/field/array element/closure environment dies when
//!   the container drops its reference, which any call or store may do; it
//!   must be `retain`ed first. Borrows from parameters stay valid for the whole
//!   call. An owned value (`c >= 1`) is protected exactly until its own release.
//! * Retain of `null` and of sticky objects is a no-op, so elision need not
//!   special-case them. **There is no double-release / use-after-free
//!   detection in release builds**: a stale `release` of a dead object reads
//!   whatever the allocator left in its header. Corruption stays inside the
//!   sandbox, but only the checked native heap (`sim`), the debug build's
//!   poisoning and the RC-balance checker detect it; the compiler/elision pass
//!   must emit balanced code.
//! * A retained object is recoloured black (not a candidate), so
//!   `retain`/`release` pairs elided around a still-live object cost the
//!   collector nothing; conversely, every *non-final* release of a cyclic
//!   type buffers a candidate, so removing such releases is the elision pass's
//!   main lever against collector load. Types marked `#acyclic` never buffer.
#![cfg_attr(target_arch = "wasm32", no_std)]
#![allow(
    clippy::missing_safety_doc,
    // low-level pointer code: single-letter names (p, c, n, m, s) are the idiom here,
    // casts between u32 addresses/lengths are range-checked by the ABI
    clippy::many_single_char_names,
    clippy::too_many_lines,
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_lossless,
    clippy::similar_names,
    clippy::match_same_arms,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::needless_pass_by_value,
    clippy::struct_field_names,
    clippy::items_after_statements,
    clippy::should_implement_trait
)]

pub mod heap;
pub mod rt;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub mod sim;
#[cfg(not(target_arch = "wasm32"))]
pub mod testkit;

/// Extension exports provided by this runtime beyond `exports::EXPORTS`:
/// `(name, params, results)` with `i32` for every value.
pub const EXTENSION_EXPORTS: &[(&str, usize, usize)] = &[
    ("oxi_rt_init", 2, 0),
    ("oxi_rt_scratch_cap", 0, 1),
    ("oxi_gc_step", 1, 1),
    ("oxi_gc_stat", 1, 1),
    ("oxi_gc_config", 2, 0),
];
