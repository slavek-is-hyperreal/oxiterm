# OxiScript Memory Management Performance Research

**Date:** 2026-11-26  
**Target Environment:** OxiScript (statically typed, TS-readable, deterministic, no UB) compiled to Core WebAssembly (`wasm32`, single linear memory, single-threaded per instance, no WasmGC) running in `wasmi` 2.0 (fuel-metered register-based interpreter), `w2c2`/`wasm2c` + GCC/Clang AOT (Raspberry Pi 1 ARMv6 700 MHz / 512 MB RAM, ESP32-S3), and browser engines.  
**Observed Baseline Gap (from 8.M-link measurements):** Linked `oxiscript-rc` (`dlmalloc` + Bacon-Rajan cycle collector) costs **6.5–10.5×** more `wasmi` fuel/iter than the bump-allocator stub (`new class`: 398 → 3,070 fuel/iter; array literal: 579 → 5,584; template string: 1,657 → 12,006), **~2.5×** wall time, and requires large heap headroom for cycle-collector work vectors.  
**Verification note:** Claims grounded in repository files (`oxiscript-rc`, `oxiscript-wasm`, `oxiscript-lower`) or `~/.cargo/registry` (`wasmi 2.0.3`, `dlmalloc 0.2.14`) are cited directly. External literature and project claims carry links; quantitative estimates for OxiScript that have not yet been benchmarked are marked **[estimate]** and unverified secondary claims are marked **[unverified]**.

> **Corrected by Claude (Opus), 2026-11-26:** this draft was verified claim by claim; the ledger, the measurements and corrected recommendations are in `oxiscript-memory-performance-verification.md`. The corrections below are inline; the most important one is that the fuel gap is mostly a **metering artifact of dlmalloc's code shape** (A1/A2 there), not call overhead or missing elision.


---

## Executive Summary & Clear Recommendations

OxiScript's 6.5–10.5× fuel penalty is **not** a fundamental limitation of reference counting in WebAssembly; it is the compound product of four multiplicative bottlenecks visible directly in the current codebase:

1. **General-purpose boundary-tag allocator (`dlmalloc`) on every tiny fixed-size object** (~40–50% of the excess fuel): Every `oxi_alloc` pays `drain()` entry checks, type-descriptor indirection, `dlmalloc::calloc` (which always runs `write_bytes` even on fresh memory or immediately overwritten payloads, adds a 4-byte chunk header + 8-byte alignment rounding—turning 16 B objects into 24 B and 24 B into 32 B—and walks doubly-linked bins/dv/top), followed on free by boundary-tag coalescing.
2. **Function-call overhead and zero inlining on hot memory operations** (~25–35% of excess fuel): In `wasmi 2.0.3`, *every* Wasm operator (except structured delimiters `block`/`loop`/`end`/`else`/`nop`/`drop`/`return`/`unreachable`) costs 1 fuel (`wasmi-2.0.3/src/engine/costs.rs:505–538`). Every `retain`, `release`, `is_unique`, and `alloc` currently crosses a Wasm `call` boundary into LLVM-compiled Rust code (prologue/epilogue, shadow-stack `$__stack_pointer` traffic, generic type-descriptor lookup, null/sticky/colour branching, and `q_head` push-then-immediate-pop on every `rc 1 → 0` release).

> **Corrected by Claude (Opus), 2026-11-26:** the percentage split in items 1-3 is wrong. Measured fuel per call (rtlink 0cf56d3, wasmi 2.0.3 eager): `oxi_retain` 41, `oxi_is_unique` 10, non-final `oxi_release` 120, `oxi_alloc` 1908, final release 1107. For `new class`, alloc + final release = 3015 of the 3070 fuel/iter, so the allocator and the destruction path are ~95% of that row; call overhead on retain/is_unique is a few percent. See `oxiscript-memory-performance-verification.md` A2.

3. **Zero RC elision and naive lowering in `oxiscript-lower`** (~20–40% of excess fuel, and the primary driver of array-literal 9.6× and template-string 7.2× blowups): Lowering follows "own everything" (retaining every field/array/global load, never reusing/cancelling `retain`/`release` pairs), lowers array literals `[a, b, c, d, e]` via `ArrayEmpty` + repeated `ArrayPush` (triggering 2–3 `dlmalloc` allocs/copies/frees per literal), and lowers template strings via pairwise `StrConcat` (quadratic copying + $O(k)$ temporary allocations).
4. **Vector-backed cycle collector requiring large headroom**: `oxiscript-rc` allocates three growable guest-heap `V32` vectors (`roots`, `stack`, `log`) and reserves `stack.cap >= 2 * work + roots + 8` words before Phase 2 (`rt.rs:1056`), forcing OOM aborts when linear memory is tight.

### Recommended Target Architecture (4 Pillars)

| Pillar | Concrete Recommendation | Expected Impact on Fuel / Memory |
|---|---|---|
| **1. Hybrid Slab + TLSF/Talc Allocator** | **Small objects ($\le 256$ B, 14–16 size classes)** served by a **single-threaded segregated-slab (sharded free-list) allocator** using 4 KiB slabs carved from a **TLSF (`rlsf`, MIT/Apache-2.0) or `talc` (MIT)** backing allocator for slabs and large blocks ($> 256$ B, arrays, large strings). Zero per-object allocator header for slab objects (size class derived from `ptr & !4095` slab header); `0` bytes extra overhead beyond OxiScript's 8-byte RC header. | **3–5×** lower alloc/free fuel; **25–33%** less RAM on small objects (24 B stays 24 B, not 32 B); deterministic $O(1)$ worst-case time; empty 4 KiB slabs returned to backing allocator immediately. |
| **2. Codegen Inline Fast Paths & Specialised Ops** | Emit **inline Wasm fast paths** in `oxiscript-wasm` for: (a) `is_unique` (5 ops, no call), (b) `retain` (9–11 ops for acyclic/known-non-null, no call), (c) `release` fast path (decrement & check `rc > 1` for acyclic types inline; call `oxi_release_slow` only on `rc == 1` or cyclic purple transition), (d) `alloc` slab free-list pop (10–14 ops inline; call `oxi_slab_refill` on empty free list), (e) `alloc_uninit` when `setfield.init` overwrites all fields, (f) direct-sized `array.from_elems` and `str.concat_n` (builder/2-pass length sum). | Cuts hot `retain`/`release`/`alloc` fuel from ~150–400 to **~8–20 fuel** **[estimate]**; eliminates 2–3× excess allocs on array literals and template strings immediately. |
| **3. OxiIR Ownership & Perceus-Lite Compiler Passes** | Implement on OxiIR (in order): (1) **Local & intraprocedural RC pair elision** + **borrowed-parameter inference** (Lean 4 / Perceus style), (2) **Sink/move parameter & last-use transfer** (Nim ARC style, eliminating retain+release on argument handoff), (3) **Scalar replacement / escape analysis** (unbox non-escaping structs/closures/boxed-optionals into Wasm locals), (4) **Perceus reuse analysis (`reset`/`reuse`)** for COW structs and functional updates. | **50–80%** of dynamic `retain`/`release` ops removed (matching Lean 4 / Perceus / Swift OSSA literature); **100%** heap elimination for non-escaping temporaries. |
| **4. Zero-Headroom Intrusive Cycle Collector** | Replace `roots`/`stack`/`log` dynamic `V32` vectors with **intrusive metadata**: (a) **inline slab bitmaps** (3–4 bits/slot inside the 4 KiB slab header: purple/buffered/gray/white/queued, freeing header bits) or **intrusive doubly-linked root list** for cyclic-only types (`!#acyclic`), (b) **bounded fixed-size static work stack** + **Mark-Stack Overflow fallback (pointer-scan over cyclic slabs)** so `oxi_collect_cycles` / `oxi_gc_step` **never allocates** and runs in **$O(1)$ extra memory** even at 99% heap utilisation. | **0 bytes** dynamic collector headroom; eliminates `oom_abort` under tight memory caps (Pi 1 / ESP32-S3); removes `ZOMBIE` deferral overhead. |

Combined, Pillars 1–3 should reduce per-operation fuel from the current **6.5–10.5×** stub baseline down to **~1.1–1.8×** of the bump stub (a **4–7× fuel reduction** **[estimate]**) while retaining exact deterministic RC reclamation, full cycle collection, and < 15–25 MB RSS on small hardware.

> **Corrected by Claude (Opus), 2026-11-26:** Pillar 2's "~150-400 fuel" baseline holds only for alloc and final release (retain is 41, is_unique 10). The 1.1-1.8x target is reachable for `new class` mainly through Pillar 1 (a scratch free-list front cut `oxi_alloc` from 1908 to 241 fuel) plus direct free of leaf objects. The 50-80% RC-removal figure of Pillar 3 has no source in the cited papers (ledger A8). Pillar 4 is not log-free as described (ledger A7).


---

## 0. Baseline Diagnosis: Why OxiScript's Linked RC Runtime Costs 6.5–10.5× Fuel Today

Inspecting `oxiscript/oxiscript-rc/src/{rt.rs,wasm.rs}`, `oxiscript/oxiscript-wasm/src/{func.rs,stubrt.rs}`, `oxiscript/oxiscript-lower/src/expr.rs`, `dlmalloc-0.2.14`, and `wasmi-2.0.3` pinpoints every instruction behind the measured gap:

1. **`wasmi 2.0.3` fuel model (`wasmi-2.0.3/src/engine/costs.rs:505–538`):** By default, every Wasm operator except `nop`, `drop`, `block`, `loop`, `unreachable`, `return`, `else`, `end` charges **1 fuel** per executed Wasm opcode, and `memory.copy` / `memory.fill` additionally charge per-byte bulk fuel (`fuel_for_copying_values`, `wasmi-2.0.3/src/engine/executor/handler/exec.rs:530,592`). A Wasm `call` itself charges 1 fuel for the opcode, **plus every instruction inside the callee** (including Rust/LLVM stack-pointer prologue/epilogue and callee-saved spills) and incurs interpreter frame-push/pop wall-time cost.

> **Corrected by Claude (Opus), 2026-11-26:** the cost table is right, but fuel is **not** charged per executed opcode. wasmi 2.0 charges each region's **static** operator sum, plus 1, when the region is entered. Regions start at function entry, `loop`, the `if` then-arm and `else`; `block` opens no region (`translator/func/mod.rs:301`, `visit.rs:118-130, 146, 200, 233`, `encoder.rs:396-410`). Code after a `br_if` exit is paid for even when it is skipped. LLVM emits Rust as `block`/`br_if` (0 `if` in all runtime functions), so dlmalloc's inlined `malloc` costs its 1651-operator entry region on every call. That explains 6.5-10.5x fuel next to only 2.5x wall time. Bulk ops cost 1 fuel per **64** bytes, and a call adds `ceil(locals/8)`. Ledger A1.

2. **Exact cost of `oxi_alloc` (`oxiscript-rc/src/rt.rs:571–581`, `wasm.rs:57–60`):**
   - Calls `self.drain(self.cfg.drain_budget)` unconditionally on every allocation (even when `q_head == 0`).
   - Validates `id < td_count` and checks single-entry cache `tdc_id` (misses whenever allocating alternating types, e.g. array + struct + string).
   - Enters `raw_alloc` loop → `WasmHeap::alloc` → `DL.calloc(total, 8)`.
   - Inside `dlmalloc-0.2.14`: `calloc` calls `c_memalign(8, total)` → `c_malloc(total)` (smallbin bitmap lookup, chunk splitting/unlinking from `dv`/`top`, boundary-tag writes) **and then always runs `ptr::write_bytes(ptr, 0, total)`** because `calloc_must_clear` returns `true` for all non-mmapped chunks (`dlmalloc.rs:319–320`), even on fresh zeroed Wasm pages and even for strings/clones that immediately overwrite every byte!

> **Corrected by Claude (Opus), 2026-11-26:** the code fact is right: this dlmalloc never mmaps a chunk, so every chunk is cleared, even though `WasmSys::allocates_zeros()` is true. The cost is negligible: with `+bulk-memory` the clear is a single `memory.fill`, about 1-2 fuel for 24 B. Eliminating zeroing (and `alloc_uninit`) is not a fuel win. Ledger A3.

   - `dlmalloc` adds a 4-byte `head` chunk header and rounds up to `malloc_alignment = 8` (`pad_request(amt) = align_up(amt + 4, 8)`, `dlmalloc.rs:258–260`): a 16 B object occupies **24 B** (+50%) and a 24 B object (`NODE`) occupies **32 B** (+33%).
   - Updates `live_objects: u32` and `live_bytes: u64` (64-bit arithmetic in 32-bit Wasm) and in debug builds updates the `livemap` bitmap.
3. **Exact cost of `oxi_release` (`rt.rs:618–656`, `745–795`):**
   - When a short-lived object dies (`rc == 1`), `dec()` does **not** free it directly: it marks the colour `PENDING`, writes `q_head` into `p - 8`, increments `q_len`, returns to `release()`, which immediately sees `q_head != 0` and calls `drain(256)` — which immediately pops `p` back off `q_head`, decrements `q_len`, reloads `meta`, calls `td(id)`, loops over `pmap`, calls `finish_free(p)` → `dealloc_object(p)` → calls `obj_total(p, id)` (calling `td(id)` a **second** time!), decrements `live_objects`/`live_bytes`, and calls `dlmalloc::free` (which calls `validate_size`, inspects adjacent chunk footers/headers for coalescing, and relinks into smallbins/treebins).
   - For a cyclic-capable object whose `rc` drops from `2 → 1`, `dec()` sets `PURPLE | BUFFERED` and pushes `p` onto `self.roots` (a heap-allocated `V32`), even if the object is retained again two instructions later.
4. **Why `array literal` (579 → 5,584 fuel, 9.6×) and `template string` (1,657 → 12,006 fuel, 7.2×) are even worse than `new class` (398 → 3,070, 7.7×):**
   - `oxiscript-lower/src/expr.rs:489–501` lowers every array literal `[e0, e1, ...]` as `Op::ArrayEmpty` (`oxi_array_new(id, 0)`) followed by a sequence of `Op::ArrayPush` instructions! Pushing 5 elements calls `oxi_array_new` (alloc #1) → `oxi_array_grow(4)` (alloc #2) → `oxi_array_grow(8)` (alloc #3 + copy + free #1) — **3 `dlmalloc` allocs + 1 copy + 1 free** for a compile-time constant length!
   - `oxiscript-lower/src/expr.rs:283,1188` lowers template strings as left-associative pairwise `Op::StrConcat`: a 4-part template `` `a${x}b${y}` `` performs 3 separate `oxi_str_concat` calls (each doing `drain` + `dlmalloc::calloc` + zero-fill + 2× `memory.copy`) plus `oxi_str_from_i64`/`f64` plus 3× `oxi_release` of intermediate strings!

> **Corrected by Claude (Opus), 2026-11-26:** BL06 measured `[1, 2, 3]` and `` `k${i}` ``, not the examples above. `[1,2,3]` is 2 allocations (the header, plus data at capacity 4; `oxi_array_new(id, 0)` allocates no data block) and 0 copies. Direct sizing measured only -5% (5448 → 5181 fuel). `` `k${i}` `` has exactly one concat; its 4 allocations per iteration come from `lw.itoa`/`lw.fmt_u`, which builds a `[u8]` and then copies it into a string (`oxiscript-lower/src/helpers.rs:8-62`). The template lowering is at `expr.rs:288-330`; `:1188` is binary `+`. Ledger A4.

   - `oxiscript-lower` performs **zero RC elision**: every load from a field, array, global, or cell emits `Op::Retain` immediately and `Op::Release` at scope exit (`docs/stages/8.L/implementation.md:20`), and `oxiscript-wasm/src/func.rs:733–736,1023–1030` emits a full `call` for every `IsUnique`, `Retain`, and `Release`.

---

## 1. Small-Object Allocators for Single-Threaded Wasm Linear Memory

### 1.1 Survey of Allocators

In OxiScript, every Wasm instance is **strictly single-threaded** (`docs/oxiscript/abi.md:22`, no Wasm threads proposal), uses **32-bit addresses** in a single contiguous linear memory growing only in **64 KiB pages** (`memory.grow`), has **no `memory.shrink` or `munmap`/`madvise`** in Core Wasm, and targets tiny devices (Raspberry Pi 1 with 512 MB RAM and 700 MHz ARMv6; ESP32-S3 with ~8 MB PSRAM / 512 KB SRAM). Consequently, multi-threaded scalability machinery (thread-local caches, atomic cross-thread free queues, virtual-memory reservation tricks) adds dead weight, while **instruction count on the fast path, zero per-object metadata overhead, tight small-object packing, $O(1)$ determinism, and small `.wasm` code size** dominate.

1. **`mimalloc`** ([Leijen, Zorn, de Moura, APLAS 2019 / MSR-TR-2019-18](https://www.microsoft.com/en-us/research/publication/mimalloc-free-list-sharding-in-action/), [github.com/microsoft/mimalloc](https://github.com/microsoft/mimalloc), **MIT**):
   - *Design:* **Free-list sharding** — divides memory into segments (4 MiB on 64-bit) containing fixed-size-class pages (typically 64 KiB for small objects), with a per-page local free list (popped in ~5 native instructions), plus thread-local and concurrent deferred free lists. Built specifically to accelerate RC languages (Koka, Lean 4). Reported 7% faster than `tcmalloc` and 14% faster than `jemalloc` on Redis, with low fragmentation and ~12.5% size-class spacing.
   - *Fit for OxiScript:* The **core idea (per-page/per-slab sharded free lists with zero per-object header and 1-instruction page-header lookup via bitmask `ptr & !(SLAB_SIZE - 1)`)** is ideal for OxiScript. However, upstream C `mimalloc` itself is too heavy for tiny Wasm/ESP32 (4 MiB segments, ~30–60 KB compiled Wasm, heavy virtual-memory assumptions). **Adopt the algorithm with 4 KiB slabs, not the C library.**
2. **`snmalloc`** ([Liétar et al., ISMM 2019](https://doi.org/10.1145/3315573.3329980), [github.com/microsoft/snmalloc](https://github.com/microsoft/snmalloc), **MIT**):
   - *Design:* Slab allocator using lock-free message passing for cross-thread deallocations and a global pagemap for size-class lookup.
   - *Fit for OxiScript:* Message passing solves remote frees across threads, which do not exist in single-threaded Wasm; its 64-bit address-space pagemap assumptions are overkill for 1–16 MiB linear memories.
3. **`rpmalloc`** ([github.com/mjansson/rpmalloc](https://github.com/mjansson/rpmalloc), **Public Domain / MIT**):
   - *Design:* Lock-free thread-caching 64 KiB span allocator with 16-byte–spaced small size classes (up to 1 KiB), medium spans up to 256 KiB.
   - *Fit for OxiScript:* Good small-class span design, but 64 KiB spans per size class waste too much memory on ESP32-S3 / small-heap instances (32 classes × 64 KiB = 2 MiB minimum just to touch every class).
4. **`jemalloc`** ([Evans 2006 / FreeBSD](https://jemalloc.net/), [github.com/jemalloc/jemalloc](https://github.com/jemalloc/jemalloc), **2-clause BSD**) & **`tcmalloc`** ([Google](https://google.github.io/tcmalloc/design.html), [github.com/google/tcmalloc](https://github.com/google/tcmalloc), **Apache-2.0**):
   - *Design:* Size-class segregated slab/run allocators with thread/per-CPU caches. Both pioneered **geometric sub-power-of-two size classes** (4 size classes per power-of-two doubling: e.g., $2^k + \{1,2,3,4\} \cdot 2^{k-2}$, yielding $\le 12.5\%$ worst-case internal fragmentation for `tcmalloc` and $\le 20\%$ / ~12.5% for `jemalloc`, with metadata overhead $< 2\%$ of heap; see [TCMalloc Design](https://google.github.io/tcmalloc/design.html) and [jemalloc paper](https://people.freebsd.org/~jasone/jemalloc/bsdcan2006/jemalloc.pdf)).
   - *Fit for OxiScript:* Neither compiles cleanly to tiny `no_std` wasm32, but their **4-step geometric size-class table** is directly reusable as the mathematical blueprint for OxiScript's small-object slabs.
5. **`dlmalloc`** ([Doug Lea](https://gee.cs.oswego.edu/dl/html/malloc.html); Rust port [`dlmalloc-rs`](https://github.com/alexcrichton/dlmalloc-rs), **MIT OR Apache-2.0**, C original **Public Domain**):
   - *Design:* Boundary-tag segregated free lists (32 exact 8-byte smallbins up to 256 B + 32 bitwise-trie treebins for large blocks) + designated victim (`dv`) + wilderness (`top`) chunk. Rust's default `wasm32-unknown-unknown` allocator (~10–15 KB Wasm).
   - *Fit for OxiScript:* Current baseline. Solid fragmentation, **but poor for RC micro-objects**: 4-byte chunk header + 8-byte alignment forces **+8 B waste on every 16 B / 24 B / 32 B OxiScript object** (since OxiScript sizes are already multiples of 8!), boundary-tag coalescing on every `free` executes dozens of Wasm instructions, and `calloc` always runs `memset`.
6. **`talc`** ([github.com/SFBdragon/talc](https://github.com/SFBdragon/talc), **MIT**):
   - *Design:* Pure-Rust `no_std` allocator using boundary-tagged doubly-linked lists segregated into TLSF-like power-of-two + linear sub-buckets (`TalcCell` lockless single-threaded mode). Supports explicit arenas, heap extension/shrinking (reclaiming tail space), and optional byte counters.
   - *Performance & size:* In the `talc` benchmarks ([SFBdragon/talc](https://github.com/SFBdragon/talc)), `talc` is consistently faster than `dlmalloc` on single-threaded random alloc/free workloads while producing smaller Wasm code (~5–8 KB vs `dlmalloc`'s ~10–15 KB) **[verify exact KB on wasm32 build]**. Uses 1 `usize` (4 B on wasm32) overhead per live chunk (same 8-byte rounding caveat as `dlmalloc` for tiny multiples-of-8 objects unless fronted by a slab cache).
   - *Fit for OxiScript:* Excellent candidate for the **backing / large-object allocator** (replacing `dlmalloc`), especially paired with a small-object slab front-end.

   > **Corrected by Claude (Opus), 2026-11-26:** talc's own Wasm benchmark concludes "DLmalloc and RLSF's GlobalTlsf are worse in both size and performance to Talc", but talc's default Wasm configuration uses "10%-15% higher runtime memory" (`BENCHMARKS_WASM.md`). rlsf's README measures **1267 B** of Wasm against dlmalloc's **9613 B**, not 4-7 KB / 12-15 KB. Licences CONFIRMED on the repositories: talc MIT, rlsf MIT/Apache-2.0, mattconte/tlsf 3-clause BSD.

7. **`TLSF` (Two-Level Segregated Fit)** ([Masmano et al., ECRTS 2004](https://doi.org/10.1109/EMRTS.2004.1311009), [J. Systems & Software 2008](https://doi.org/10.1016/j.jss.2007.07.018)); Rust crate **`rlsf`** ([github.com/yvt/rlsf](https://github.com/yvt/rlsf), **MIT OR Apache-2.0**); C reference [`mattconte/tlsf`](https://github.com/mattconte/tlsf) (**3-clause BSD**):
   - *Design:* Two-level bitmaps: First Level (FL) splits sizes by $\lfloor \log_2 s \rfloor$, Second Level (SL) subdivides each power of two into $2^k$ (typically 16 or 32) linear sub-bins. Lookup uses Wasm native `i32.clz` / `i32.ctz` (both 1-fuel MVP instructions!), guaranteeing **strict $O(1)$ worst-case execution time (WCET)** for `malloc`, `free`, and `realloc` with average fragmentation ~10–15% (Masmano et al. 2004). Used as the allocator of AssemblyScript (`~lib/rt/tlsf.ts`).
   - *Fit for OxiScript:* **Strongest backing allocator** for OxiScript's `realtime`/`safety` roadmap because WCET is strictly $O(1)$ (unlike `dlmalloc`'s treebin traversal) and `i32.clz`/`i32.ctz` are single Wasm MVP instructions. Still has a 4-byte block header + boundary-tag coalescing, so fronting it with a small-object slab tier gives the best of both worlds.
8. **`emmalloc`** ([Emscripten `system/lib/emmalloc.c`](https://github.com/emscripten-core/emscripten/blob/main/system/lib/emmalloc.c), **MIT / NCSA**):
   - *Design:* Minimalist segregated-fit allocator written for Emscripten (`-sMALLOC=emmalloc`) to cut Wasm code size (~5–7 KB vs `dlmalloc`'s ~12–15 KB; [Emscripten Optimizing Code](https://emscripten.org/docs/optimizing/Optimizing-Code.html)).
   - *Fit for OxiScript:* Optimised for code size over speed/fragmentation; `talc` and `rlsf` are pure Rust, equally small, and faster/better-bounded.
9. **`lol_alloc`** ([github.com/Craig-Macomber/lol_alloc](https://github.com/Craig-Macomber/lol_alloc), **MIT**):
   - *Design:* Ultra-tiny (~1–2 KB Wasm) Rust allocator (`FreeListAllocator` / `LeakingPageAllocator`) using a simple sorted/linked free list (worst-case $O(n)$ search).
   - *Fit for OxiScript:* Too slow ($O(n)$) under alloc/free churn for a general scripting runtime; only suitable for code-size-obsessed static apps.
10. **`wee_alloc`** ([github.com/rustwasm/wee_alloc](https://github.com/rustwasm/wee_alloc), **MPL-2.0**):
    - *Why abandoned:* Officially unmaintained since ~2020 (RUSTSEC-2022-0054 / [rustwasm/wee_alloc#106](https://github.com/rustwasm/wee_alloc/issues/106)): suffers from an unfixed memory leak in free-list handling and poor allocation throughput, plus MPL-2.0 licence. **Do not use.**
11. **Buddy Allocators** (Knuth TAOCP Vol. 1):
    - *Design:* Power-of-two blocks split/merged by XORing addresses.
    - *Fit for OxiScript:* Poor for small objects (up to **50% internal fragmentation**: a 24 B object takes 32 B, a 40 B object takes 64 B), which is unacceptable on 512 KB–512 MB devices; only useful as a page-level (4 KiB+) backing manager.
12. **Bump + Region / Arena Allocation** (e.g., Rust `bumpalo` MIT/Apache-2.0; MLKit regions [Tofte & Talpin, POPL 1994](https://doi.org/10.1145/174675.177855)):
    - *Design:* Pointer increment (`ptr += size`), bulk free at region exit. Fast path is ~5 Wasm instructions!
    - *Fit for OxiScript:* Already planned as an OxiIR extension (`alloc.arena`, `docs/oxiscript/ir.md:175`). Ideal for per-frame UI display-list/layout scratch or scoped arenas where objects do not escape the region, and as the **slab initial-carve fast path** (bump inside a fresh 4 KiB slab before switching to its free list!).

### 1.2 Allocator Comparison Table (Evaluated for Single-Threaded wasm32)

| Allocator | Algorithm | Per-Obj Alloc Overhead (wasm32) | Complexity (Alloc / Free) | Frag. (Internal / External) | Approx. Wasm Code Size | Determinism / WCET | Licence | Fit for OxiScript |
|---|---|---|---|---|---|---|---|---|
| **Stub (current test)** | Linear bump (no free) | 0 B | $O(1)$ / N/A | 0% / ∞ (leaks) | ~0.5 KB | Strict $O(1)$ | Ours (MIT/Apache) | Benchmark lower bound only |
| **`dlmalloc` (current 8.M)** | Boundary tags + smallbins + treebins | **4 B header → +8 B** on 8n sizes | Amort. $O(1)$ / $O(\log n)$ worst | Low / Low | ~12–15 KB | Bounded, not strict $O(1)$ | MIT/Apache-2.0 | Replace (slow + 33–50% small-obj bloat) |
| **`talc` (`TalcCell`)** | Boundary tags + TLSF-like bins | **4 B header → +8 B** on 8n sizes | Amort. $O(1)$ (bin scan) | Low / Low | ~5–8 KB | Near-$O(1)$ | **MIT** | Strong backing allocator |
| **TLSF (`rlsf`)** | 2-level bitmap segregated fit | **4 B header → +8 B** on 8n sizes | **Strict $O(1)$ / $O(1)$** | ~10–15% avg | ~4–7 KB | **Strict $O(1)$ WCET** | **MIT/Apache-2.0** | **Best backing allocator** (fits `realtime`) |
| **`emmalloc`** | Segregated free lists | 4–8 B | $O(n)$ worst in bucket | Med / Med | ~5–7 KB | Not strict $O(1)$ | MIT / NCSA | Inferior to `talc`/`rlsf` |
| **`lol_alloc`** | Simple free list | 4–8 B | $O(n)$ | Low / High | ~1–2 KB | Poor ($O(n)$) | MIT | Too slow under churn |
| **`wee_alloc`** | Size-class + free list | 4 B | $O(n)$ | High (leaks) | ~1 KB | Broken (#106 leak) | MPL-2.0 | **Rejected (abandoned, leaks)** |
| **Buddy** | Power-of-2 split/merge | 0–4 B (bitmap) | $O(\log n)$ | **Up to 50%** / Low | ~2–3 KB | Bounded | N/A (algo) | Only for 4 KiB+ page management |
| **`mimalloc` (upstream C)** | 64 KiB page-sharded slabs | 0 B (page header) | $O(1)$ | $\le 12.5\%$ / Low | ~30–60 KB | $O(1)$ fast path | MIT | Too large as-is; **borrow algorithm** |
| **Recommended: Custom 4 KiB Slabs + `rlsf` (TLSF)** | Per-class 4 KiB slabs (bump+freelist) over TLSF | **0 B** (≤256 B); 4–8 B (>256 B) | **Strict $O(1)$ / $O(1)$** | **$\le 12.5\%$** / Very low | ~6–9 KB total | **Strict $O(1)$ WCET** | **MIT/Apache-2.0** | **Recommended target design** |

> **Corrected by Claude (Opus), 2026-11-26:** code-size cells: rlsf 1267 B and dlmalloc 9613 B (rlsf README); other sizes are unmeasured. After the fuel-model correction, a backing allocator's wasmi fuel depends mostly on its entry-region size, so the "Best backing allocator" ranking must come from measurement (ledger A5).

### 1.3 Recommended Allocator Design for OxiScript

Because OxiScript's ABI already guarantees **all object sizes are multiples of 8 bytes** (`abi.md:19`) and **all managed objects carry an 8-byte header (`rc`, `meta`)**, we can eliminate 100% of allocator per-object headers for small objects ($\le 256$ B — typically 85–95% of allocations in managed languages):

1. **Two-Tier Architecture:**
   - **Tier 1 — Small-Object Segregated Slabs ($\text{total size} \le 256$ B, 14 size classes):**
     - **Size classes (total bytes including 8 B RC header):**
       - Exact 8-byte steps up to 64 B: **`16, 24, 32, 40, 48, 56, 64`** (7 classes — **0% internal fragmentation** for every small struct, closure, union variant, array header [24 B], and short string $\le 47$ bytes!).
       - 4-step geometric spacing (TCMalloc/mimalloc style, $\le 12.5\%$ max internal fragmentation) from 64 B to 256 B: **`80, 96, 112, 128, 160, 192, 224, 256`** (8 classes; total **15 size classes**, indices `0..14`).
       - Note: `alloc(type_id)` knows the exact size class **at compile time** for every struct, closure, union variant, and array header (`24 B` = class 1)! No runtime size-to-class division or lookup table is needed on `Op::Alloc`!
     - **Slab size: 4 KiB (`4096` B), 4 KiB-aligned.**
       - Why 4 KiB instead of `mimalloc`'s 64 KiB? On a small script or ESP32-S3 / Raspberry Pi 1 instance, touching all 15 size classes consumes at most **$15 \times 4\text{ KiB} = 60\text{ KiB}$** (less than 1 Wasm page!) instead of $15 \times 64\text{ KiB} = 960\text{ KiB}$.
       - A 4 KiB slab holds `16` to `252` objects. Metadata sits in a **64-byte slab header** at `slab = ptr & !4095` (1 Wasm `i32.and` instruction!):
         - `next_slab`, `prev_slab` (intrusive partial-slab doubly-linked list), `free_head` (intrusive singly-linked free list stored in the dead object's `rc` word at `slot + 0`), `bump_cursor` + `bump_end` (lazy carving: new slab needs **no O(N) free-list pre-linking**!), `live_count: u16`, `class_idx: u8`, `flags: u8` (bit 0 = cyclic-capable slab, bit 1 = debug slab).
       - Optional **in-slab live bitmap** (for debug `T_STALE_OBJECT` checks AND zero-headroom cycle collection — see §3.4): 256 slots × 1 bit = **32 bytes**, fitting right inside a 96-byte slab header (`< 2.4%` overhead)!
     - **Zeroing elimination:** Fresh memory from `bump_cursor` in a newly `memory.grow`n Wasm page is **already zeroed** by the Wasm spec; reused slots from `free_head` only need zeroing of fields NOT immediately written by `setfield.init` (or `memory.fill` only for the payload, never the header). For strings (`str_new`, `str_concat`, `str_slice`) and `clone`, **skip zeroing completely** (only write the single trailing NUL byte).
   - **Tier 2 — Large Objects & Slab Backing ($> 256$ B, plus 4 KiB aligned slab blocks):**
     - Backed by **`rlsf`** (Rust TLSF, MIT/Apache-2.0) or **`talc`** (`TalcCell`, MIT) operating over Wasm linear memory grown in 64 KiB pages.
     - When a 4 KiB slab's `live_count` drops to `0` (and it isn't the sole active cached slab for its class—keep 1 hysteresis slab per class to avoid thrash), return the entire 4 KiB slab to Tier 2 (`rlsf`/`talc`), immediately reclaiming it for *any* other size class or large array!
     - **Memory return to the host on small devices:** Core Wasm has no `memory.shrink`, so returning physical RAM inside a long-lived instance relies on: (a) **reusing freed slabs across size classes** via Tier 2 so peak Wasm pages stay flat, (b) **hot-reload / instance swap** (which OxiScript already has in `abi.md §4`) releasing the entire `wasmi` `Vec<u8>` back to the host OS, and (c) on Linux/Pi hosts with `mmap`-backed linear memories (e.g. Wasmtime or future custom wasmi host), an optional host hint (`madvise(MADV_DONTNEED)`) on completely idle trailing 64 KiB pages.
2. **Inline Fast-Path Length in Wasm Instructions (and `wasmi` Fuel):**
   - Maintain a small static table of active free-list heads in linear memory at a fixed compile-time address `SLAB_FREE_HEADS[15]` (60 bytes at fixed offset, e.g. `1024..1084` — **no global lookup indirection, constant base address!**).
   - **Inline `alloc` fast path for compile-time-known class `c` (e.g., `new Point()` → class 1, 24 B, `meta_const = tid | (GREEN << 24)`):**
     ```wat
     ;; 1. Load head = SLAB_FREE_HEADS[c]          (2 ops: i32.const, i32.load)
     ;; 2. If head != 0: pop and init header       (12 ops total!)
     (local.tee $b (i32.load offset=HEAD_c (i32.const 0)))
     (if (result i32)
       (then
         ;; SLAB_FREE_HEADS[c] = *(u32*)$b        (4 ops)
         (i32.store offset=HEAD_c (i32.const 0) (i32.load (local.get $b)))
         ;; *(u32*)$b = 1 (rc); *(u32*)($b+4) = meta_const (6 ops)
         (i32.store (local.get $b) (i32.const 1))
         (i32.store offset=4 (local.get $b) (i32.const META_CONST))
         ;; return $b + 8                         (3 ops)
         (i32.add (local.get $b) (i32.const 8))
       )
       (else
         ;; cold path: bump from slab / refill slab / drain queue
         (call $oxi_alloc_slow (i32.const TYPE_ID))
       )
     )
     ```
   - **Instruction / fuel count on fast path:** **~16–18 Wasm instructions = ~16–18 fuel** (including `live_count` increment on slab header if tracked, or defer `live_count` reconciliation to slab-refill/sweep time — just like `mimalloc` keeps a per-slab `used` counter incremented in the slow path or via `slab = b & !4095`, which adds only 5 ops!). Compare **~18–23 fuel** vs today's **~1,500+ fuel** inside `oxi_alloc` + `dlmalloc::calloc`!
   - **Caveat on `oxi_live_objects()` / `oxi_live_bytes()`:** Keep exact accounting **without** touching global `u64` counters on every fast-path alloc/free by deriving `live_objects` on demand (walking active slab headers + large-object counter) or keeping a single `i32` global `G_LIVE_OBJS` (+3 fuel) when leak checks are enabled. Note that `oxi_live_objects`/`oxi_live_bytes` are diagnostic/test calls, never hot-path ops!
   - **Determinism:** 100% deterministic (no ASLR, no random free-list shuffling, fixed LIFO order per size class, strict $O(1)$ fast path and $O(1)$ TLSF refill).

---

## 2. Reference Counting Cost Reduction (Compiler & Runtime)

### 2.1 Literature Survey & Quantitative Results

1. **Lean 4 — "Counting Immutable Beans"** ([Ullrich & de Moura, IFL 2019 / arXiv:1908.05647](https://arxiv.org/abs/1908.05647)):
   - *Mechanisms:* (a) **Owned vs. Borrowed calling convention** inferred by a fixed-point interprocedural **borrow inference** pass (callee parameters default to borrowed unless consumed into a constructor, mutated in place, or returned); (b) **Reset/Reuse (`reset x` / `reuse w`)** turning a decrement-to-zero of an unshared cell immediately followed by an allocation of the same size into an in-place update; (c) **Delayed/hoisted RC insertion** minimising live ranges.
   - *Published results:* In the paper's benchmark suite (`rbmap`, `deriv`, `const_fold`, `qsort`, `binarytrees`), Lean 4's RC with borrow inference + reuse matches or beats GHC/OCaml/MLton on tree and symbolic benchmarks; disabling borrow inference (`-borrow`) causes **10–40%+ slowdowns** on traversal-heavy benchmarks, and reuse removes **up to 50–90%+ of allocations** on in-place tree updates like `rbmap` (where every unique-path rotation reuses the exact node cell).

> **Corrected by Claude (Opus), 2026-11-26:** the paper reports run time, not allocation or RC-op counts. Fig. 6, `-borrow`: geometric mean 1.27; binarytrees 1.14, deriv 1.16, **const_fold 0.90 (faster without it)**, others ~1.00. `-reuse`: geometric mean 1.74 (const_fold 1.64, rbmap 3.23, unionfind 1.41). The "50-90%+ of allocations" figure is not in the paper. Lean is "competitive and often outperforms"; on rbmap it is faster than everything except OCaml.

2. **Koka — Perceus & FBIP (Functional But In-Place)** ([Reinking, Xie, de Moura, Leijen, PLDI 2021](https://doi.org/10.1145/3453483.3454032), [MSR-TR-2020-42](https://www.microsoft.com/en-us/research/publication/perceus-garbage-free-reference-counting-with-reuse/)):
   - *Mechanisms:* Linear-resource-calculus compilation producing **garbage-free** RC (objects are dropped immediately at their last use, not at lexical scope exit — crucial for freeing references *before* a subsequent `is_unique` / COW check or tail call!). Adds **Drop Specialization** (inlining a type's destructor when statically known, skipping `dec` on fields that are about to be reused), **Reuse Specialization** (skipping writes to unchanged fields when reusing a cell in place), and **Tail-Recursion Modulo Cons (TRMC)** ([Leijen & Lorenzen, POPL 2023](https://doi.org/10.1145/3571233)).
   - *Published results:* Koka with Perceus + `mimalloc` outperforms OCaml, Haskell, Swift, and Java on `rbtree`, `deriv`, `nqueens`, and `cfold` (up to **2× faster than OCaml** on `rbtree` due to in-place reuse and zero GC pauses) while cutting peak heap to ~1× live set.

> **Corrected by Claude (Opus), 2026-11-26:** wrong. On rbtree the paper says "OCaml is close in performance"; the 2x is Koka's own "no-opt" build against the optimised one. OCaml uses less memory than Koka on deriv and cfold, and the optimisations "are less effective" with sharing (deriv, nqueens). Perceus TR v4, §4.

   - *Applicability to OxiScript:* **Extremely high.** OxiScript's `struct` has **value semantics with copy-on-write (`is_unique` ? mutate : `clone`)** (`docs/oxiscript/ir.md:156`). Last-use dropping (Perceus) is the *exact* mechanism that turns `let s2 = moveX(s1)` from a `clone` into an in-place mutation when `s1` is dead after the call!
3. **Swift ARC Optimiser & Ownership SSA (OSSA)** ([Swift SIL docs](https://github.com/swiftlang/swift/blob/main/docs/SIL/SIL.rst), [Ownership Manifesto](https://github.com/swiftlang/swift/blob/main/docs/OwnershipManifesto.md), [LLVM Dev Meeting 2019 — OSSA](https://www.youtube.com/watch?v=0s1160M33b8)):
   - *Mechanisms:* Swift's SIL encodes ownership directly in SSA (`@owned`, `@guaranteed` [borrowed], `@unowned`). Key passes:
     - **Guaranteed (borrowed) default calling convention** (Swift 5 switched default parameters from `@owned` to `@guaranteed`, eliminating caller retains + callee releases across call boundaries).

> **Corrected by Claude (Opus), 2026-11-26:** the switch to +0 (guaranteed) parameters came in **Swift 4.2** (2018), not Swift 5. No source was found for the 50-80% figure below.

     - **ARC Sequence Optimization (`ARCSequenceOpts`)**: dataflow pairing of `strong_retain` and `strong_release` (moving retains later, releases earlier, cancelling adjacent pairs when no intervening instruction can decrement the refcount to zero).
     - **Immortal/static object specialization** (no RC ops on static constants).
     - **Stack promotion (`StackPromotion`)**: escape analysis converting non-escaping class instances into stack allocations with zero RC traffic.
   - *Published/reported impact:* Switching parameters to borrowed (`@guaranteed`) and running OSSA ARC elimination removes **~50–80%** of naive retain/release traffic in typical Swift code **[estimate from Swift SIL talks; exact corpus-wide percentage varies heavily by workload]**, though opaque generics / existentials remain a known barrier (which OxiScript **avoids** because OxiIR is fully **monomorphised**!).
4. **Nim ARC / ORC** ([Nim Destructors & Move Semantics](https://nim-lang.org/docs/destructors.html), [Rumpf, FOSDEM / NimConf 2020](https://nim-lang.org/blog/2020/10/15/introduction-to-arc-orc-in-nim.html)):
   - *Mechanisms:*
     - **Move semantics (`=wasMoved`)**: last use of a local transfers ownership with zero RC traffic.
     - **`sink` parameters**: explicitly/inferred owned handoff without copy+retain.
     - **`lent` returns**: borrowed views into containers without retaining.
     - **Cursor inference**: local variables that only traverse/read a data structure without mutating the owner become raw non-owning pointers (`{.cursor.}`), removing **100%** of `retain`/`release` inside linked-list/tree traversal loops!
     - **Compile-time `acyclic` analysis**: automatically proves types cannot participate in cycles, omitting cycle-collector root buffering completely.
   - *Applicability to OxiScript:* **Very high.** OxiIR already has `own` parameters vs borrowed parameters, `#acyclic` type attributes, and monomorphised SSA! Adding **last-use move optimization**, **cursor inference for read-only loop variables**, and **interprocedural `own`/borrow inference** maps directly onto OxiIR.
5. **Roc** ([Roc Compiler](https://github.com/roc-lang/roc), [Lorenzen MSc Thesis, Utrecht 2021](https://studenttheses.uu.nl/handle/20.500.12932/40485)):
   - *Mechanisms:* Adopted Lean 4's borrow inference + Perceus reuse analysis ("morphed" memory layouts, specialized drop procedures, seamless slices). Identified a subtle pitfall in naive Counting-Immutable-Beans reuse where keeping a reuse token alive across a recursive call can inflate peak memory — solved by Perceus's eager drop-before-call discipline.
6. **Rust `Rc<T>` Patterns:**
   - Explicit borrowing (`&T` vs `Rc::clone`), `Rc::make_mut` (identical to OxiScript's `is_unique` COW pattern!), and `Weak<T>` for breaking cycles manually. For OxiScript, the lesson is that **borrowing by default (`&T`) eliminates almost all RC mutations in pure read paths** — our compiler should infer `&T` (borrowed SSA values) automatically wherever mutation/aliasing cannot drop the owner.
7. **Deferred Reference Counting** ([Deutsch & Bobrow, CACM 1976](https://doi.org/10.1145/360336.360345)):
   - *Mechanism:* Do not maintain reference counts for local variables / stack slots at all; only count heap-to-heap edges. Objects reaching `rc == 0` are placed in a Zero Count Table (ZCT) and periodically reconciled against stack roots.
   - *Applicability to OxiScript:* **Not directly applicable without a shadow stack!** Core Wasm hides Wasm locals from linear memory (there is no stack-map inspection of Wasm locals), so scanning roots would require spilling every managed local to a linear-memory shadow stack — adding memory stores on every local write (often *more* expensive in `wasmi` fuel than compiler-elided RC!).
8. **Coalescing Reference Counting** ([Levanoni & Petrank, OOPSLA 2001](https://doi.org/10.1145/504282.504309) / [TOPLAS 2006](https://doi.org/10.1145/1111320.1111322)):
   - *Mechanism:* Logs only the *first* modification of an object per epoch (recording its original pointer slots into a mod-buffer); at collection time, compares original vs current fields so intermediate overwrites cancel out. Eliminates **~90–99%** of heap RC mutations in mutation-heavy Java workloads.
   - *Applicability to OxiScript:* Requires a tracing/root-scanning epoch boundary (same Wasm local stack-map constraint as Deferred RC), plus write-barrier cost on every `setfield`. Better suited to tracing-hybrid JVM runtimes than immediate-RC Wasm.
9. **Biased Reference Counting (BRC)** ([Choi, Shull, Torrellas, PACT 2018](https://doi.org/10.1145/3243176.3243195)):
   - *Mechanism:* Splits count into non-atomic owner-thread count + atomic shared count, achieving ~17–22% speedup over atomic RC in multi-threaded Swift/C++.

> **Corrected by Claude (Opus), 2026-11-26:** the abstract gives 22.5% lower average execution time for client programs and 7.3% higher throughput for server programs (Swift runtime).

   - *Applicability to OxiScript:* **Not needed today** because OxiScript Wasm instances are already single-threaded with **non-atomic** RC (`abi.md:22`). Revisit only if Stage 8.4 adds shared-memory threads (currently OxiScript plans actor/isolate message passing, which preserves non-atomic per-isolate RC!).
10. **Ulterior RC, RC Immix, and LXR:**
    - **Ulterior RC** ([Blackburn & McKinley, OOPSLA 2003](https://doi.org/10.1145/949305.949336)): Copying nursery for young objects + coalescing RC for mature objects; ~1% overhead vs generational tracing GC.
    - **Down for the Count?** ([Shahriyar, Blackburn, Frampton, ISMM 2012](https://doi.org/10.1145/2258996.2259008)): Showed naive RC is ~30% slower than tracing GC, but combining coalescing + deferred RC + small stuck counts (3–4 bits) brings RC within ~1–5% of tracing GC.
    - **RC Immix** ([Shahriyar et al., OOPSLA 2013](https://doi.org/10.1145/2509136.2509527)): Combines coalescing RC with Immix line-based mark-region allocation and opportunistic copying defragmentation; matches or beats generational copying GC throughput.
    - **LXR (Low-Latency, High-Throughput GC)** ([Zhao, Jacob, Blackburn, PLDI 2022](https://doi.org/10.1145/3519939.3523440), [arXiv:2204.01918](https://arxiv.org/abs/2204.01918)): Immix blocks + coalescing RC + concurrent satellite tracing for cycles; achieves sub-millisecond pauses and beats G1/Shenandoah/ZGC throughput by 10–30%.

> **Corrected by Claude (Opus), 2026-11-26:** arXiv:2204.01918 is an unrelated paper; LXR's extended version is arXiv:2210.17175. Across 17 workloads with a moderate heap, LXR beats G1 throughput by 4% and Shenandoah by 43%. "Down for the Count?" says optimised RC reaches parity ("on par") with a well-tuned mark-sweep, without giving a 1-5% figure.

    - *Applicability to OxiScript:* The **moving/copying and stack-scanning pieces** of Ulterior RC / RC Immix / LXR do **not** transfer to Core Wasm (no stack maps to update Wasm locals on object relocation). What *does* transfer: (a) **narrow saturating (sticky) RC** (Shahriyar et al. 2012 showed >99.9% of objects never exceed a tiny refcount; OxiScript can pack `rc` into 16 bits or 24 bits if header shrinking is ever desired, though 32-bit `rc` is cheap on wasm32), and (b) **line/slab-based allocation** (Immix-style bump carving inside blocks before free-list reuse).

### 2.2 RC Optimisation Comparison Table (for OxiScript / OxiIR)

| Technique | Where It Runs | Published RC / Alloc Reduction | Transfers to OxiScript (Core Wasm)? | Implementation Complexity |
|---|---|---|---|---|
| **Immortal / static RC skip** | OxiIR (`str.const` known sticky) | 100% of RC on string literals | **Yes — trivial** | Very low (1–2 days) |
| **Intraprocedural pair elision + last-use move** | OxiIR pass (dominators + alias/effect check) | **30–60%** of local retain/release **[estimate]** | **Yes — exact fit for OxiIR** | Low (3–5 days) |
| **Borrow inference (caller-guaranteed params)** | Interprocedural OxiIR fixed-point (Lean 4) | **40–70%** of cross-call retain/release | **Yes — OxiIR already has `own` vs borrowed!** | Medium (1–2 weeks) |
| **Cursor inference (non-owning loop traversal)** | OxiIR loop/alias analysis (Nim ARC) | **~100%** inside read-only list/tree loops | **Yes — high win for graphs/trees** | Medium (1–2 weeks) |
| **Inline `retain`/`release`/`is_unique` fast path** | `oxiscript-wasm` codegen | **60–85% fuel/op reduction** on remaining RC | **Yes — huge `wasmi` win** | Low (3–5 days) |
| **Perceus reuse (`reset`/`reuse`) + drop spec.** | OxiIR pass | **50–90% allocs** on COW/tree updates | **Yes — ideal for OxiScript COW `struct`** | Medium-High (2–3 weeks) |
| **Escape analysis + scalar replacement** | OxiIR pass | **100% alloc + RC** on non-escaping structs/opts | **Yes — huge win for `T?` scalars & small structs** | Medium-High (2–3 weeks) |
| **Deferred / Coalescing RC (Levanoni-Petrank)** | Runtime + stack maps | 90–99% heap RC mutations (JVM) | **No** (needs Wasm local stack maps or shadow stack) | N/A |
| **Biased RC (Choi 2018)** | Runtime | 17–22% over atomic RC | **Unnecessary** (already non-atomic single-threaded) | N/A |
| **RC Immix / LXR copying defrag** | Runtime + stack maps | Matches generational GC | **Partial** (slab bump-carving yes; copying no) | N/A |

### 2.3 Which Compiler Analyses on OxiIR Remove the Most Pairs?

OxiIR (`docs/oxiscript/ir.md`) is unusually well-suited for static RC elision because it is **monomorphised**, **typed SSA with block parameters**, carries **explicit `own` vs borrowed parameters**, has **explicit `#effects(...)` summaries** (`#pure`, no globals/alloc/host calls), has **explicit `#acyclic` type tags**, and has **no exceptions** (only fatal traps, which never run destructors!). In priority order of ROI on OxiIR:

1. **Skip `retain`/`release` on provably immortal values (`str.const`) and provably `null` values** (trivially tracked by a forward dataflow pass).
2. **Last-use ownership transfer (move semantics, Nim/Perceus style):** Currently `oxiscript-lower` retains a local before storing it into a field/array/return and releases the local at scope exit (`retain x; setfield o.f, x; release x`). A single backward liveness pass proves `x` has no further uses → deletes both `retain x` and `release x`, moving `x`'s owned ref directly into `setfield`/`array.push`/`ret`/owned-arg!
3. **Intraprocedural `retain`/`release` cancellation using OxiIR's effect rules (`ir.md §4`):** Any `retain x ... release x` span containing no instruction that can mutate or drop an alias of `x` (leveraging `#pure` calls and type-based alias disjointness—e.g. writing a `Point` field cannot drop a `string` unless `Point` holds a `string`) is deleted immediately. Furthermore, **hoist `release` to last use** (Perceus rule) so `is_unique` COW checks see `rc == 1` instead of `2`.
4. **Borrowed-load & cursor inference:** Currently `oxiscript-lower` retains every `getfield`/`array.get`/`global.get` immediately. If the loaded value `v` is only read before the next mutating/call barrier (e.g. `s.x + s.y`, `arr[i].len`, loop cursor `cur = cur.next` in a read-only loop), keep `v` borrowed (`c = 0`) and emit **zero** `retain`/`release`!
5. **Interprocedural borrow & `own` (sink) inference (Lean 4 / Counting Immutable Beans):** Run a bottom-up/fixed-point pass over the monomorphised call graph: if parameter `%p` is never stored into a escaping container or returned (or mutated via COW), keep it borrowed; if `%p` *is* stored into a returned struct/array (e.g. a constructor or builder), mark it `own` so callers can move their last-use temporary straight in without a retain+release pair!
6. **Perceus Reuse Analysis + Drop Specialisation:** For COW struct updates and union-variant replacements (`switch (node) { case "add": return { kind: "add", ... } }`), pair the last-use `release` with `alloc` of the same size class: if `rc == 1`, reuse the slot in place (zero allocator traffic!) and skip freeing/retaining unchanged child fields.

---

## 3. Cycle Collection: Algorithms, Intrusive/Constant-Memory Designs, and Triggering

### 3.1 Survey of Cycle Collectors

1. **Bacon–Rajan Synchronous Trial Deletion** ([Bacon & Rajan, ECOOP 2001](https://doi.org/10.1007/3-540-45337-7_12), [IBM Research](https://dominoweb.draco.res.ibm.com/reports/rc21921.pdf)):
   - *Algorithm:* Four colours (`BLACK`, `GRAY`, `WHITE`, `PURPLE`) + `buffered` bit + `GREEN` acyclic optimisation. Any non-zero decrement on a non-green object colours it `PURPLE` and buffers it as a candidate root. Collection runs 3 phases over the root buffer: (1) `MarkGray` (trial-decrement internal edges), (2) `Scan` (if `rc > 0` → `ScanBlack` restoring counts; else `WHITE`), (3) `CollectWhite` (reclaim dead cycles).
   - *Current OxiScript status:* Implemented in `oxiscript-rc/src/rt.rs`. Works cleanly and is deterministic, **but** suffers from two issues: (a) **memory headroom** (dynamic `roots`, `stack`, and 2-word-per-node `log` vectors in guest memory, reserving `2 * work + roots + 8` words before Phase 2), and (b) **false-alarm scanning** when cyclic-capable live structures suffer frequent temporary refcount drops (e.g. traversing a cyclic graph without cursor inference buffers thousands of live nodes).
   - *Asynchronous/concurrent Bacon-Rajan* (same paper; refined in [Paz, Bacon, Kolodner, Petrank, Rajan, TOPLAS 2007](https://doi.org/10.1145/1255450.1255453)): Uses an extra epoch/sigma test for concurrent mutators; unnecessary in single-threaded OxiScript where collection runs between host events.
2. **CPython Cyclic GC** ([CPython `Modules/gcmodule.c` / `Python/gc.c`](https://github.com/python/cpython/blob/main/Python/gc.c), [GC Design Docs](https://devguide.python.org/internals/garbage-collector/)):
   - *Design:* **Intrusive doubly-linked lists** via a 16-byte (32-bit: 8-byte) `PyGC_Head` prepended to every container object (strings/ints/tuples-of-primitives excluded). Uses **3 generations** (`gen0`, `gen1`, `gen2`, default thresholds `(700, 10, 10)` on net allocs-minus-deallocs) and runs a **non-trial-deletion** variant of Bacon-Rajan over each generation (copies `ob_refcnt` into `gc_refs`, subtracts intra-generation refs, promotes reachable objects with `gc_refs > 0` to next generation, collects remainder).
   - *Recent evolution (3.12–3.14):*
     - **PEP 683 (CPython 3.12, Immortal Objects)** ([peps.python.org/pep-0683](https://peps.python.org/pep-0683/)): High-bit sticky refcount skips all RC writes and GC tracking (OxiScript already has this via `STICKY = 0x8000_0000`!).
     - **CPython 3.14 Incremental GC** ([gh-108362](https://github.com/python/cpython/issues/108362), later reverted in 3.14.5 due to memory-pressure regressions in cyclic workloads — see [Python Discourse / 3.14 release notes](https://docs.python.org/3.14/whatsnew/3.14.html)): Merged gen1/gen2 into a single old generation swept incrementally in slices. The production lesson for OxiScript: **incremental collection that falls behind the allocation rate of cyclic garbage inflates peak RSS**; slice budgets must scale proportionally to allocation/root-buffering work!
   - *Overhead lesson for OxiScript:* CPython's `PyGC_Head` costs 8 bytes on every container object (2 pointers on 32-bit) *even when not buffered*. For OxiScript, storing 8 bytes per object on every cyclic struct is expensive (33% on 24 B objects), **unless** we restrict it to cyclic-only types or use slab bitmaps!
3. **PHP Zend Cycle Collector** ([PHP Manual — Collecting Cycles](https://www.php.net/manual/en/features.gc.collecting-cycles.php), [`Zend/zend_gc.c`](https://github.com/php/php-src/blob/master/Zend/zend_gc.c)):
   - *Design:* Classic synchronous Bacon-Rajan with a **pre-allocated fixed-size root buffer** (`GC_ROOT_BUFFER_MAX_ENTRIES = 10_000` entries initially in PHP 5.3–7.2; dynamic adaptive threshold in PHP 7.3+ that grows up to 1,000,000 when a collection frees little, avoiding $O(N^2)$ thrashing when 10,000 live roots sit in the buffer!).
   - *Key trick:* Each `zend_refcounted_h` stores its **index in the root buffer (`gc_info`)**! When an object is destroyed (`rc == 0`) or retained back to black, PHP removes it from the root buffer in **$O(1)$** via its stored index — **completely eliminating `ZOMBIE` objects**!

> **Corrected by Claude (Opus), 2026-11-26:** in php-src today, `GC_THRESHOLD_DEFAULT` is 10,000 + first root, `GC_THRESHOLD_STEP` 10,000, `GC_THRESHOLD_MAX` **1,000,000,000**, and the default buffer is 16 Ki entries. The root index lives in the `GC_INFO` bits of the existing 32-bit type_info word, so it costs no extra 4 bytes (`Zend/zend_gc.c:88-195`).

4. **Nim ORC** ([Nim ORC Design / `lib/system/orc.nim`](https://github.com/nim-lang/Nim/blob/devel/lib/system/orc.nim)):
   - *Design:* Synchronous Bacon-Rajan trial deletion over a `roots` `CellSeq` (hash-set/array) with **aggressive static `#acyclic` analysis** (most Nim types—strings, seqs of primitives, non-recursive refs—are statically acyclic and never touch the cycle collector!). Uses `rc` shifted by `rcShift` with colour bits in the low bits of the single `rc` word!
   - *OxiScript relevance:* `oxiscript-rc` was explicitly modelled on Nim ORC, and Nim's biggest practical win is **compiler-proven acyclicity + cursor inference** keeping the candidate root set near-empty on real programs.
5. **Swift (No Cycle Collector — `weak` / `unowned`):**
   - *Design:* Pure ARC, zero cycle collector overhead, relies on programmer-annotated `weak` / `unowned` references (implemented via a 64-bit side-table pointer when weak refs exist).
   - *OxiScript relevance:* OxiScript explicitly chose **GC-like convenience for closures/UI event handlers** (`docs/oxi-ui.md:711`), so dropping cycle collection altogether is out of scope, *however* offering optional `weak` references later lets heavy graph code opt into `#acyclic` zero-CC overhead.
6. **Pony ORCA** ([Clebsch et al., AGERE 2015](https://doi.org/10.1145/2824815.2824819), [OOPSLA 2017](https://doi.org/10.1145/3133896)):
   - *Design:* Deferred distributed RC across actors paired with per-actor tracing GC (enabled by reference capabilities `iso`/`val`/`ref` guaranteeing no cross-actor mutable aliasing) + a dedicated Cycle Detector actor that receives snapshot-style messages from idle actors.
   - *OxiScript relevance:* Irrelevant for single-instance linear memory today, but directly informs Stage 8.4 if OxiScript introduces multi-isolate actors.

### 3.2 Cycle Collector Comparison Table

| Collector | Algorithm | Per-Object Header Cost (32-bit) | Extra Dynamic Memory Needed | Worst-Case Pause | Handles ZOMBIEs Without Leak? | Determinism |
|---|---|---|---|---|---|---|
| **Current `oxiscript-rc`** | Bounded Bacon-Rajan + undo log | 0 extra (uses `meta` 4 bits) | **High** (`roots` + `log` + `stack`, up to `~16 B/node`) | Bounded by `budget` (abort+undo spikes) | Delays free as `ZOMBIE` until sweep | Deterministic |
| **CPython (3.12 / 3.14.5)** | Generational non-trial RC subtraction | **8 B** (`PyGC_Head` next/prev) on containers | **0 B** (fully intrusive lists!) | Stop-the-world per gen | Immediate $O(1)$ unlink | Deterministic |
| **PHP 7.3+ Zend GC** | Adaptive Bacon-Rajan | 4 B (`gc_info` index in root buf) | Fixed/adaptive root buf + DFS stack | Unbounded per trigger (adaptive) | **Yes — $O(1)$ removal via `gc_info`** | Deterministic |
| **Nim ORC** | Bacon-Rajan + static acyclicity | 0 extra (bits in `rc`) | `roots` `CellSeq` + DFS stack | Unbounded (`GC_fullCollect`) | Deferred or removed from `CellSeq` | Deterministic |
| **Swift ARC** | None (`weak`/`unowned`) | 0 B (side table on demand) | 0 B | 0 ms | N/A | Deterministic (leaks cycles) |
| **Recommended for OxiScript** | **Slab-Bitmap / Intrusive Bacon-Rajan with Mark-Stack Overflow** | **0 B** in object header (uses slab header bitmaps + 4-bit `meta`) | **0 B dynamic** (fixed static 2–4 KiB work stack + overflow scan) | Strictly bounded per step | Immediate (`live_count` / slab bit cleared) | **100% deterministic, zero-headroom** |

### 3.3 Avoiding Headroom: Constant-Extra-Memory Cycle Collection for OxiScript

Why does `oxiscript-rc` fail under tight memory caps today? Because `roots`, `stack`, and `log` are heap-allocated `V32` vectors (`rt.rs:183–185`), and `trial_inner` reserves `stack.cap >= 2 * work + (end - start) + 8` words (`rt.rs:1056`) to guarantee Phases 2 and 3 cannot OOM. On a tight 256 KiB heap full of 24-byte cyclic nodes (~10,000 nodes), `log` (80 KB) + `stack` (160 KB) + `roots` (40 KB) = **280 KB of scratch memory** — **more than the entire heap!**

We can eliminate **100% of dynamic collector headroom** by combining three classic techniques adapted to our 4 KiB slab allocator:

1. **Zero-allocation root set via Per-Slab Cyclic Bitmaps (or Intrusive Cyclic-Only List):**
   - Separate cyclic-capable objects (`!#acyclic`) into **cyclic slabs** (marked `SLAB_CYCLIC = 1` in the 4 KiB slab header; acyclic objects like strings, primitive arrays, and `#acyclic` structs live in acyclic slabs and are **never scanned**).
   - Each 4 KiB cyclic slab already has at most 252 slots. A **32-byte `purple` bitmap** in the slab header (1 bit per slot = 0.78% of the slab!) replaces the entire `roots` vector for small objects!
   - *Bonus #1:* Buffering a candidate is a **single bit-set** (`slab.purple_bits[slot >> 5] |= 1 << (slot & 31)`) + linking the slab onto a global `dirty_cyclic_slabs` intrusive list (if not already linked). **Zero allocation, zero `roots` vector, zero duplicates!**
   - *Bonus #2:* When an object is retained back to black or freed at `rc == 0`, clearing its `purple` bit is **$O(1)$** — **completely eliminating `ZOMBIE` objects and zombie memory bloat!**
   - *(For large cyclic objects $> 256$ B, e.g., arrays of cyclic refs, store a small intrusive 8-byte `(next_root, prev_root)` prefix before their header — negligible on $> 256$ B objects, `< 3%`!)*
2. **Fixed Static Work Stack + Mark-Stack Overflow Fallback (Boehm / HotSpot / Go technique):**
   - Allocate a single **fixed 4 KiB static scratch stack** (1,024 object pointers, e.g. reusing `oxi_scratch_ptr` since `oxi_collect_cycles` / `oxi_gc_step` only runs between host events when the scratch buffer is idle! — **0 extra bytes of static RAM!**).
   - In `mark_gray`, `scan`, `scan_black`, and `collect_white`: push children onto this 1,024-entry static stack. If the stack overflows (deep graph $> 1,024$), set an `overflow = true` flag and drop the push; when the stack empties with `overflow == true`, perform a linear sweep over cyclic slabs checking the `GRAY`/`WHITE` colour bits to resume traversal!
   - This guarantees **strictly $O(1)$ extra memory** (0 heap allocations during collection — collection can run even when the heap is 100% full and `memory.grow` fails!).
3. **Eliminating the `log` Vector for Bounded Steps:**
   - Why did `oxiscript-rc` need a 2-word-per-node `log` vector? Only to *undo* `mark_gray` when `oxi_gc_step(budget)` aborts mid-trial!
   - **Alternative 1 (Cleanest for small devices):** Instead of trial-mutating `rc` in place and undoing on budget overflow, either:
     - Run `oxi_gc_step` on **one connected component / small root batch** with a **fixed static undo ring** (e.g., 512 entries inside the 4 KiB scratch buffer) for fast incremental trials, falling back to full unbounded `oxi_collect_cycles` (which **never aborts** and therefore **needs no `log` vector at all!**) when idle or on OOM!
     - Even better: during full `oxi_collect_cycles`, **zero `log` vector is needed** — `gather_white` does not need a `log` either if `collect_white` releases untraced children directly in Pass A (scanning `WHITE` bits in cyclic slabs) and frees `WHITE` objects in Pass B (scanning `WHITE` bits in cyclic slabs)!

> **Corrected by Claude (Opus), 2026-11-26:** three corrections.
> * `log` is not only an undo log. Phase 3 reuses it as the **list of dead nodes** (`rt.rs:1066-1104`), because untraced children of every dead node are released before any node is freed.
> * Full `oxi_collect_cycles` *does* abort: on OOM while reserving the stack it undoes the trial and retries (`rt.rs:1169-1176`). The undo log is what makes OOM recoverable.
> * A bounded `gc_step` that aborts needs undo state proportional to the work it did, which a fixed ring cannot guarantee.
>
> Mark-stack overflow plus rescan needs an extra "gray, edges not yet subtracted" state, because MarkGray's decrements are not idempotent. Its worst-case time is O(heap) per overflow. Reusing `oxi_scratch_ptr` is not "strictly safe": `abi.md:15` lets the host write the scratch region whenever the guest is not running, which is exactly when the collector runs. Ledger A7.

4. **Triggering Policy (Adaptive + Host-Cooperative):**
   - Keep OxiScript's core invariant (`abi.md:81`): **never run cycle collection inside a guest call**, only at quiescent points between events (or as an OOM recovery fallback if allowed by policy).
   - **Adaptive threshold (PHP 7.3+ / Boehm style):** Track `purple_count` (incremented when a slab purple bit transitions `0 → 1`, decremented on `1 → 0`). Trigger `oxi_gc_step` from the host event loop (`Runtime::pump` / after `oxi_tick`) when `purple_count >= threshold` (initial `threshold = 256`). If a full sweep reclaims `< 10%` of scanned candidates (live cyclic working set, like a DOM/UI graph), **double `threshold`** (up to e.g. 16,384) so the collector does not repeatedly re-scan a live cyclic graph on every frame; reset `threshold` downward when a sweep reclaims $> 25\%$!
   - **Compounding win with §2:** Once `#acyclic` analysis is complete (marking strings, primitive arrays, and non-recursive structs `#acyclic`) and cursor inference removes non-final releases on cyclic traversals, `purple_count` stays near zero for 95%+ of UI scripts!

---

## 4. Transferable Techniques from Production Language Engines

Production JIT/AOT engines (V8, JavaScriptCore, SpiderMonkey, Go, OCaml 5, .NET, JVM) have spent decades optimising allocation and object overhead. Because OxiScript targets **Core WebAssembly without stack maps**, some techniques transfer directly with huge gains while others are fundamentally incompatible.

### 4.1 Survey of Engine Techniques

1. **V8 (Chromium / Node.js)** ([V8 Blog — Orinoco](https://v8.dev/blog/trash-talk), [Pointer Compression](https://v8.dev/blog/pointer-compression), [Oilpan / `cppgc`](https://v8.dev/blog/high-performance-cpp-gc), [TurboFan Escape Analysis & Allocation Folding](https://v8.dev/blog/turbofan-jit)):
   - *Orinoco / Scavenger:* Parallel/incremental/concurrent generational Cheney-copying nursery + mark-compact old space.
   - *Oilpan (`cppgc`):* Precise heap tracing + **conservative stack scanning** for C++ objects in Blink, with 32-bit pointer compression (halving pointer memory footprint on 64-bit hosts, improving V8/Blink heap RSS by ~30–40%).
   - *Inline bump allocation + Allocation Folding:* TurboFan inlines TLAB bump-pointer checks (`top + size <= limit`), and **folds multiple sequential allocations** in the same basic block (e.g., allocating a wrapper + inner array + point) into a **single** combined bump check!
   - *Escape Analysis & Scalar Replacement:* TurboFan eliminates non-escaping objects completely, keeping their fields in SSA registers.
2. **JavaScriptCore (`Riptide`, `bmalloc`, `libpas`)** ([WebKit — Introducing Riptide](https://webkit.org/blog/7122/introducing-riptide-webkits-retreating-wavefront-concurrent-garbage-collector/), [`libpas` announcement & design](https://github.com/WebKit/WebKit/blob/main/Source/bmalloc/libpas/Documentation.md)):
   - *Riptide:* Concurrent retreating-wavefront mark-sweep collector (non-moving) over segregated size-class blocks (`MarkedBlock`, 16 KiB).
   - *`bmalloc` → `libpas`:* Segregated size-class heaps with **IsoHeaps** (type-isolated slabs to mitigate use-after-free type confusion), bitfit allocators for medium sizes, and aggressive physical-page decommit (`madvise`) to shrink RSS on mobile/embedded devices.
   - *Lesson for OxiScript:* **Type-segregated or cyclic-vs-acyclic segregated 4 KiB slabs** mirror `libpas`'s segregated page layout (simplifying both free-list pop and cycle-collector slab scanning!).
3. **SpiderMonkey (Firefox)** ([SpiderMonkey GC](https://firefox-source-docs.mozilla.org/js/index.html)):
   - *Design:* Generational nursery (bump-allocated) + tenured segregated **Arenas** (4 KiB fixed-size-class slabs grouped into 1 MiB Chunks, each 4 KiB arena storing a header with size class + free-span list + mark bitmap!).
   - *Lesson for OxiScript:* SpiderMonkey's **4 KiB arena with embedded header and slot mark-bitmap** is almost identical to our recommended 4 KiB slab layout for OxiScript (§1.3 + §3.3).
4. **Go Runtime** ([Go Memory Allocator `runtime/malloc.go`](https://github.com/golang/go/blob/master/src/runtime/malloc.go), [Go Escape Analysis](https://go.dev/doc/faq#stack_or_heap)):
   - *Design:* TCMalloc-inspired 67 size classes (`mcache` → `mcentral` → `mheap` spans), non-moving concurrent tri-colour mark-sweep, and **compile-time whole-package escape analysis** that places non-escaping structs and slices directly on the stack (zero heap allocation, zero GC pressure).
5. **OCaml 5** ([Sivaramakrishnan et al., PLDI 2020](https://doi.org/10.1145/3385412.3386000), [OCaml 5 Runtime](https://ocaml.org/manual/5.2/runtime.html)):
   - *Design:* Domain-local bump-allocated minor heap (default 2 MiB per domain, ~3 machine instructions per allocation) + non-moving size-class segregated major heap (pool allocator for small blocks $\le 128$ words, best-fit free list for larger blocks).
6. **.NET CLR & Java HotSpot** ([HotSpot C2 Escape Analysis](https://docs.oracle.com/en/java/javase/21/vm/java-hotspot-virtual-machine-performance-enhancements.html), [OpenJDK TLAB](https://shipilev.net/jvm/anatomy-quarks/4-tlab-allocation/)):
   - *TLABs (Thread-Local Allocation Buffers):* Each thread owns a contiguous Eden slice; `new` is an inline bump-pointer increment (~4–6 instructions), calling the runtime only on TLAB refill.
   - *Scalar Replacement (HotSpot C2):* Rarely stack-allocates objects as contiguous frames; instead decomposes non-escaping object fields directly into SSA scalar registers/locals — eliminating both allocation AND memory loads/stores!

### 4.2 Concrete Transfer Matrix: What Applies to OxiScript-in-Wasm vs. What Does Not

| Engine Technique | Transfers to OxiScript (RC in Core Wasm)? | Concrete Implementation in OxiScript | Why / Why Not |
|---|---|---|---|
| **Inline allocation fast path (TLAB / slab pop)** | **YES — Top Priority** | Codegen (`oxiscript-wasm`) emits 10–18 Wasm ops popping `SLAB_FREE_HEADS[class]` (or bumping `SLAB_BUMP[class]`) before calling `oxi_alloc_slow`. | Eliminates Wasm `call` + `dlmalloc` overhead on >95% of allocations. |
| **Scalar replacement / Escape analysis (HotSpot C2, V8, Go)** | **YES — Huge Win** | OxiIR pass decomposes non-escaping `struct`s, boxed `T?` scalars (`abi.md:53`), `Result<T, E>` temporaries, and non-escaping closures directly into Wasm locals! | Wasm locals live in `wasmi` registers (and C locals in `w2c2`); zero linear-memory traffic, zero RC, zero heap! Note: Core Wasm already forbids taking the address of locals, which *matches* scalar replacement naturally. |
| **Stack/frame allocation on Wasm shadow stack** | **YES (for non-escaping fixed arrays/large structs)** | For non-escaping objects too large for scalar replacement (or indexed dynamically), bump/pop a dedicated `$__oxi_frame_sp` linear-memory stack with zero RC. | Safe because OxiScript has no `try/catch` unwinding (only fatal traps) and single-threaded execution. |
| **Type-specialised alloc & drop (`libpas` / Perceus)** | **YES — High Win** | Codegen knows exact size class, `meta_const`, and field pointer offsets statically: inline header init + inline child release for small fixed types instead of generic `td(id)` + `pmap` loop! | Eliminates runtime `td(id)` table lookups and `Slots` iterator loops completely. |
| **Inline `array.from_elems` & `str.concat_n` (V8 allocation folding analogue)** | **YES — Immediate Win** | Lower `[e0..ek]` to a single `oxi_array_new_cap(id, k)` + direct element stores; lower template strings to 2-pass length-sum + single `new_string(total)` + direct copies. | Directly removes the 2–3× excess allocs/copies seen in the 5,584 and 12,006 fuel/iter benchmarks! |
| **Pointer compression (V8 / `cppgc`)** | **ALREADY HAVE IT** | `wasm32` pointers are already 32-bit `i32` (`docs/oxi-ui.md:735`), giving V8-style 32-bit pointer density automatically even on 64-bit hosts. | Maintain 32-bit `wasm32` as default; `memory64` only opt-in on `server`. |
| **4 KiB segregated arenas with bitmaps (SpiderMonkey / `libpas`)** | **YES — Core Design** | Our recommended Tier 1 allocator + zero-headroom cycle collector (§1.3, §3.3). | Maps cleanly to 64 KiB Wasm pages (16 slabs per Wasm page). |
| **Bump-pointer allocation folding of arbitrary heap objects (V8 TurboFan)** | **PARTIAL** | Only possible if freed together (arena/region `alloc.arena`) or if sub-objects share one lifetime (e.g., **inlining fixed-size structs** or **inlining small array header + buffer** into a single allocation when capacity is immutable!). Cannot fold independently-freed RC objects into one slab slot. | RC frees objects individually; however, **merging an array literal's header + initial buffer** (or a `struct` holding an immutable fixed sub-struct) into one contiguous allocation works when lifetimes match. |
| **Copying nurseries / Scavenger / Compaction / Moving GC (V8 Orinoco, OCaml 5 minor heap, JVM Eden, RC Immix)** | **NO** | Do not attempt. | Core Wasm provides **no stack maps** and no way to inspect or update `i32` pointers held in Wasm locals/operand stack during a collection without spilling every local to a shadow stack (which destroys `wasmi` fuel efficiency). |
| **Conservative stack scanning (Boehm, TinyGo, Oilpan)** | **NO** | Do not attempt. | Core Wasm locals are completely opaque to guest code; conservative scanning only works if the compiler spills all pointers to a linear-memory shadow stack, AND conservative retention breaks OxiScript's determinism (`abi.md`, `ir.md §6`) and exact `oxi_live_objects() == 0` leak guarantees. |

---

## 5. WebAssembly-Specific Mechanics and Ecosystem Precedents

### 5.1 Execution Engine Costs: Interpreters (`wasmi`, `wasm3`, `WAMR`) vs. AOT (`w2c2`, `wasm2c`)

| Engine / Compiler | Architecture | Call vs. Inline Trade-Off | Shadow Stack & `memory.grow` Implications |
|---|---|---|---|
| **`wasmi` 2.0.3** ([github.com/wasmi-labs/wasmi](https://github.com/wasmi-labs/wasmi), **MIT OR Apache-2.0**) | Register-based bytecode interpreter with LLVM tail-call dispatch (`auto-dispatch`); 2.0 is ~2.2× faster than 1.0 ([wasmi 2.0 release notes](https://wasmi-labs.github.io/blog/posts/wasmi-v1.0/)). | **Fuel:** Every Wasm op costs 1 fuel (`costs.rs:536`); `call` costs 1 fuel *plus* every prologue/body/epilogue op inside the callee. **Wall time:** A `call` pushes a call frame on `wasmi`'s value/call stack, loads the callee `FuncEntity`, checks recursion depth, and sets up locals. Inlining a 10-op fast path removes both the call-frame transition AND the ~100–300 ops of a generic Rust runtime callee (a **10–25× fuel and ~5–15× wall-time reduction** on that op **[estimate]**). | `wasmi` meters `memory.copy`/`memory.fill` per byte (`fuel_for_copying_values`), so eliminating redundant `calloc` zero-fills saves *both* instruction fuel and bulk-memory fuel! |
| **`wasm3`** ([github.com/wasm3/wasm3](https://github.com/wasm3/wasm3), **MIT**) | Direct-threaded C tail-call interpreter (M3). Very fast on tiny MCUs (ESP32), but maintenance has slowed and it lacks newer proposals. | Same rule: function calls allocate interpreter stack frames; inline fast paths avoid dispatch + frame overhead. | Linear memory backed by `realloc` on embedded targets (see §5.2). |
| **`WAMR` Fast-Interpreter** ([Bytecode Alliance `wasm-micro-runtime`](https://github.com/bytecodealliance/wasm-micro-runtime), **Apache-2.0 with LLVM exception**) | Rewrites stack Wasm to register-based bytecode with computed-goto dispatch ([Intel WAMR design](https://github.com/bytecodealliance/wasm-micro-runtime/blob/main/doc/perf_tune.md)). | Register-based dispatch makes local-register arithmetic very fast (~1.5–2× faster than classic interp), making `call` + memory-alloc overhead proportionally *more* dominant. | Supports fixed pre-allocated memory pools on MCUs to avoid `realloc` spikes. |
| **`w2c2`** ([github.com/turbolent/w2c2](https://github.com/turbolent/w2c2), **MIT**) & **`wasm2c`** ([WebAssembly/wabt](https://github.com/WebAssembly/wabt), **Apache-2.0**) | Source-to-source AOT: Wasm → C → GCC/Clang native code (planned OxiScript Stage 8.2 release path for RPi 1 ARMv6 & ESP32-S3). | Because the whole runtime is statically linked into one `.wasm` module (`oxiscript-wasm`), GCC/Clang (`-O2`/`-O3`) *can* inline small runtime helpers across functions — **provided** they are small and have simple control flow! Large monolithic Rust functions (`dlmalloc` + `Rt::alloc` + `drain`) defeat C inlining and spill ARMv6 registers. Explicit Wasm fast-path/slow-path splitting (`inline fast path` + cold `#[inline(never)]` slow call) helps **both `wasmi` AND `w2c2`+GCC**! | On 32-bit ARMv6 (Raspberry Pi 1) and Xtensa/RISC-V (ESP32-S3), `w2c2` cannot use 4 GiB `mmap` guard pages; every memory access is bounds-checked, so **scalar replacement** (eliminating heap loads/stores) and **fewer memory touches per RC op** yield outsized native speedups! |

> **Corrected by Claude (Opus), 2026-11-26:** in the `wasmi` row, fuel is charged by static region at entry, not per executed op (see the §0.1 correction). This makes the fast-path/slow-path split the main fuel lever in wasmi as well: even unchanged Rust gains if the cold paths move to `#[cold] #[inline(never)]` functions. The "10-25x fuel" for inlining a retain or release is unrealistic: the call costs 41-120 fuel in total today.

### 5.2 The Wasm Shadow Stack and `memory.grow` Costs

- **Wasm Shadow Stack (`$__stack_pointer`):** Core Wasm separates the control/value stack (hidden inside the engine) from linear memory. When Rust (`rustc`/LLVM) compiles `oxiscript-rc` to `wasm32`, any function that takes the address of a local (`&mut buf`, `core::fmt`, slices, `Dlmalloc` structs) decrements a mutable Wasm global (`$__stack_pointer`), spills locals to linear memory, and restores `$__stack_pointer` on return (`oxiscript-rc/build.rs` reserves 64 KiB for it). Every single call into Rust-compiled `oxi_alloc` / `oxi_release` that touches the shadow stack executes extra `global.get`/`i32.sub`/`global.set` + memory loads/stores — **pure wasted fuel** when the fast path is written directly in Wasm or minimal leaf Rust!
- **`memory.grow` Costs:**
  - On 64-bit OS hosts with virtual-memory guard reservations (Wasmtime, V8), `memory.grow` commits OS pages (`mprotect`/demand-zero), costing microseconds.
  - On **`wasmi` (Rust `Vec<u8>`)**, **Raspberry Pi 1 (32-bit Linux)**, and **ESP32-S3 (no MMU virtual reservation)**, `memory.grow` calls `Vec::resize` / `realloc` — which may **allocate a brand-new contiguous buffer and `memcpy` the entire linear memory** (e.g. copying 4–16 MiB on a 700 MHz ARMv6 takes tens of milliseconds!), plus zeroing every newly added 64 KiB page!
  - **Mitigation:** (1) Tight small-object packing (0 B allocator header via slabs) + immediate slab reuse reduces peak pages by ~25–35%, cutting `memory.grow` frequency; (2) Pre-size initial linear memory appropriately for the target (e.g. 2–4 pages = 128–256 KiB instead of growing 1 page at a time from 2 pages); (3) Never grow by more than needed, and always drain the destruction queue (`self.drain(u32::MAX)`) *and* reclaim empty slabs before calling `memory.grow`.

### 5.3 Ecosystem Precedents in Linear-Memory Wasm Languages

1. **AssemblyScript — The Critical Case Study (RC → ITCMS + TLSF)** ([AssemblyScript Memory Docs](https://www.assemblyscript.org/runtime.html), [PR #1600 / Incremental GC](https://github.com/AssemblyScript/assemblyscript/pull/1600), **Apache-2.0**):
   - *History:* AssemblyScript originally shipped a **TLSF allocator + Pure Reference Counting runtime (`full` / `half`)** with a Bacon-Rajan cycle collector (`rt/pure.ts`). In v0.18+ (2020–2021), AssemblyScript **abandoned RC completely** and replaced it with **TLSF + ITCMS (Incremental Tri-Color Mark-Sweep)** (`--runtime incremental`, plus `--runtime minimal` and `--runtime stub`), using a compiler-maintained linear-memory shadow stack for roots.

> **Corrected by Claude (Opus), 2026-11-26:** the change is **PR #1559** ("Rewrite runtime, switch to tracing GC and bootstrap"), released in **v0.18.0 on 2021-01-29**; #1600 is an unrelated fix. The stated reason is removing "the error-prone automatic reference counting implementation that has been haunting us for a long time", plus preparing for Wasm GC. Reason 1 below is an inference. In reason 3, the RC-era host API was `__retain`/`__release`; `__pin`/`__unpin` is the *new* GC-era API.

   - *Why AssemblyScript moved away from RC — and what OxiScript MUST learn from it:*
     1. **No SSA RC elision pass:** AssemblyScript compiles AST → Binaryen IR with almost no static ownership/borrow/escape analysis. Consequently, *every* local assignment, loop iteration, and function call emitted `__pin`/`__retain`/`__release` calls — destroying throughput and bloating `.wasm` code size.
     2. **Cycle-collector complexity + cascading drops:** Without static `#acyclic` analysis or cursor inference, cyclic-capable JS-style objects flooded the Bacon-Rajan root buffer, while deep drops caused frame-time spikes.
     3. **Host-interop footguns:** JS callers holding Wasm object pointers had to manually `__pin`/`__unpin` objects across calls.
   - **Why OxiScript's position is fundamentally different (and how we avoid AssemblyScript's trap):**
     - OxiScript *already* solved #2's cascading drops (bounded intrusive `drain(budget)`), *already* solved #3 (ABI 1.0 forbids raw managed pointers crossing the host boundary — only flattened `str`/`bytes` and host-owned callbacks cross!), and *already* has a **typed SSA IR with `#acyclic`, `own` vs borrowed params, and `#effects`**!
     - **The existential lesson from AssemblyScript:** *Unoptimised naive RC in Wasm is slow and bloated.* OxiScript **must** implement inline fast paths + OxiIR RC elision/borrow inference (Pillars 1–3) so we never suffer AssemblyScript's PureRC performance cliff!
2. **Grain** ([grain-lang.org](https://grain-lang.org/), [Grain runtime](https://github.com/grain-lang/grain/tree/main/stdlib/runtime), **Apache-2.0**):
   - Historically used **reference counting in Wasm linear memory** (with a custom free-list allocator and static `incRef`/`decRef` emitted by its compiler, without a cycle collector for immutable-default values), and has been transitioning toward WasmGC for browsers. Confirms that RC in linear memory works well when values are mostly immutable/acyclic and RC ops are aggressively optimised.

> **Corrected by Claude (Opus), 2026-11-26:** Grain's licence is **LGPL-3.0** (its repository LICENSE), not Apache-2.0. Learning from its design is fine; reusing its code is forbidden by our policy.

3. **MoonBit** ([moonbitlang.com](https://www.moonbitlang.com/), [MoonBit WasmGC vs Wasm1](https://www.moonbitlang.com/blog/)):
   - Compiles to **WasmGC** on browsers (achieving tiny ~KB bundles and fast execution via V8's GC) and to **linear memory with custom Perceus-style RC** (on native C / linear-memory Wasm backends), using static RC elision and aggressive unboxing/inlining to stay competitive without WasmGC.
4. **Kotlin/Wasm** ([kotlinlang.org/docs/wasm-overview.html](https://kotlinlang.org/docs/wasm-overview.html), **Apache-2.0**):
   - Built **exclusively on WasmGC** — unavailable to OxiScript because `wasmi`, `w2c2`, and ESP32 targets do not support WasmGC and WasmGC breaks OxiScript's deterministic destruction / `is_unique` COW semantics (`docs/oxi-ui.md:624,654`).
5. **Go / TinyGo** ([tinygo.org/docs/concepts/compiler-internals/heap-allocation/](https://tinygo.org/docs/concepts/compiler-internals/heap-allocation/), **BSD-3-Clause**):
   - Standard Go ships its full multi-MB concurrent GC runtime into Wasm (huge binary, poor fit for MB-class devices).
   - **TinyGo** uses a tiny **conservative mark-sweep collector** (`-gc=conservative`, ~few KB) scanning the Wasm shadow stack + globals + heap bitmap in 16-byte blocks (or `-gc=leaking` bump allocator). Proves tiny-footprint linear-memory GCs work on ESP32/Wasm, **but** conservative scanning suffers from false pointer retention (nondeterministic leaks on 32-bit address spaces) and pauses proportional to live heap size.
6. **Emscripten (C/C++) & Rust (`wasm32-unknown-unknown`)**:
   - No managed GC; rely on manual memory management / RAII over `dlmalloc` (default), `emmalloc` (size-focused), `talc`/`rlsf` (Rust embedded/Wasm), or `mimalloc` (Emscripten multi-threaded).

---

## 6. Benchmark Suite Design, Baselines, Metrics, and Literature Reference Numbers

### 6.1 Proposed Microbenchmark & Macrobenchmark Suite (`oxiscript-bench` / `oxiscript-rc` + `oxiscript-wasm`)

To drive every optimisation stage empirically (matching OxiScript's "measure before claiming" rule in Stage `8.Mem`), establish a deterministic benchmark harness running identical `.oxs` programs across all runtime/compiler variants:

| Benchmark | Workload Pattern | Primary Bottleneck Exercised |
|---|---|---|
| **B1. `alloc_churn_small`** | Allocate & immediately drop 100k small `#acyclic` 24 B classes/structs (`new Point(i, i)`). | Allocator fast path + `rc 1→0` immediate free + zero-fill elimination. |
| **B2. `alloc_churn_mixed`** | Randomised deterministic lifetimes (keep sliding window of 1,024 objects across sizes 16 B, 24 B, 48 B, 96 B, 200 B, 1 KiB). | Fragmentation, peak Wasm pages, slab reuse vs `dlmalloc` coalescing. |
| **B3. `rc_read_traverse`** | Build a 10,000-node linked list or binary tree, then traverse/sum it 100× (read-only). | `retain`/`release` overhead on loads, cursor inference, borrow inference, inline fast path. |
| **B4. `struct_cow_update`** | Update a `struct` 100k times in a loop (`p.x += 1`, `rbmap` / immutable tree insert with `is_unique` COW). | `is_unique` inline fast path, Perceus last-use drop hoisting, `reset`/`reuse` analysis. |
| **B5. `array_literal_and_push`** | Build 10k array literals `[a, b, c, d, e]` and grow dynamic arrays via `push`/`pop`. | Direct-sized `array_new_cap`, eliminating `ArrayEmpty` + 2× `ArrayGrow`, slab vs Tier 2 array buffer. |
| **B6. `string_template_build`** | Format 10k template strings `` `item #${i}: (${x}, ${y}) ok=${flag}` `` and concatenate strings. | Multi-part string builder (`str.concat_n`), unzeroed string alloc, `str_from_i64` buffer-direct writing, immortal `str.const` RC skip. |
| **B7. `map_set_churn`** | `std/collections` `Map<string, Value>` insert/lookup/delete 20k keys (once Stage `8.Std` lands). | Mixed strings + arrays + buckets, hash-table resize, realistic UI state workload. |
| **B8. `cycle_garbage`** | Create 20k dead 2-cycles and 1k dead 10-cycles (closures capturing mutual state, as in UI event handlers); run `oxi_collect_cycles` and `oxi_gc_step(1000)`. | Cycle collector throughput, step p99 latency, **collector headroom at 90% full heap** (must not OOM!). |
| **B9. `cycle_false_alarm`** | Live 50k-node doubly-linked UI tree; mutate/traverse 100k times without creating garbage cycles. | `#acyclic` filtering, cursor inference, adaptive root-buffer threshold (tests against PHP-style false-alarm thrashing). |
| **B10. `ui_frame_e2e`** | Realistic OxiUI `<script lang="oxs">` tick: update state, format 20 labels, diff a 100-item list, emit host calls (Stage 8.S1 / 8.Mem). | End-to-end frame time on Raspberry Pi 1 (700 MHz ARMv6) and `wasmi` fuel/frame. |

### 6.2 Baselines to Compare Against

1. **Stub Bump Allocator (`RuntimeMode::Stub`):** Theoretical speed/fuel floor for allocation (leaks memory, no cycle collector). Every optimisation is measured as a ratio to Stub (target: **1.1–1.8× Stub fuel** instead of today's **6.5–10.5×**).
2. **Current Linked Runtime (`dlmalloc` + current `oxiscript-rc` + unoptimised OxiIR):** Today's 8.M-link baseline (`3,070` / `5,584` / `12,006` fuel/iter).
3. **Swap-only Backing Allocator (`rlsf` / `talc` dropping in for `dlmalloc`, no slabs/inlining yet):** Isolates pure allocator gain.
4. **Candidate Design Incremental Milestones:** (a) + unzeroed alloc + fast `rc 1→0` free + direct array/template lowering, (b) + 4 KiB slabs + inline Wasm fast paths, (c) + OxiIR RC elision / borrow / cursor / scalar replacement, (d) + zero-headroom slab-bitmap cycle collector.

### 6.3 Metrics & Literature Reference Numbers

**Metrics to record per run:**
- **Fuel/op in `wasmi` 2.0** (deterministic across machines — ideal CI regression gate!).
- **Wall time (ns/op, ms/frame)** on (i) x86_64 `wasmi`, (ii) x86_64 `w2c2`+GCC `-O2`, (iii) Raspberry Pi 1 ARMv6 `wasmi` and `w2c2`+GCC `-O2` (Stage `8.Mem`), and (iv) Chrome V8 Wasm.
- **Peak Wasm pages (64 KiB units) & Heap Overhead %** (`(peak_linear_bytes - live_logical_bytes) / live_logical_bytes`) after churn (B2) and under cycle collection (B8).
- **Wasm binary size (raw & gzip)** of the linked runtime (target: `< 25–35 KB` raw without `float-fmt`, `< 60 KB` with `float-fmt`).
- **Tail latency:** `oxi_gc_step` p99 / max fuel & wall time.

**Reference Numbers from Literature (Sanity Benchmarks):**
- **OxiScript native `oxiscript-rc` baseline** (`oxiscript-rc/src/lib.rs:56–59`, x86_64): `oxi_gc_step` ~0.09 µs/unit mean, p99 ~8× mean; worst observed 4 ms at budget 1,000.
- **OxiScript `wasmi` 8.M-link baseline (from prompt):** `new class` 398 (stub) → **3,070** (linked, 7.7×); `array literal` 579 → **5,584** (9.6×); `template string` 1,657 → **12,006** (7.2×); wall time **~2.5×**.
- **Native small-object allocators:** `mimalloc` / `tcmalloc` / `jemalloc` small-object alloc+free fast path is **~5–15 ns/op** (~10–20 native instructions) on modern x86_64 ([Leijen et al., APLAS 2019](https://www.microsoft.com/en-us/research/publication/mimalloc-free-list-sharding-in-action/)); `rlsf` (TLSF) achieves **strict $O(1)$** with ~10–15% average fragmentation ([Masmano et al., ECRTS 2004](https://doi.org/10.1109/EMRTS.2004.1311009)); `jemalloc`/`tcmalloc` 4-step geometric classes cap internal fragmentation at **$\le 12.5\%$** ([TCMalloc Design](https://google.github.io/tcmalloc/design.html)).
- **Compiler RC elision:** Lean 4 borrow inference + Perceus reuse eliminate **50–90%** of dynamic RC/alloc operations in tree/functional workloads ([Ullrich & de Moura, IFL 2019](https://arxiv.org/abs/1908.05647); [Reinking et al., PLDI 2021](https://doi.org/10.1145/3453483.3454032)); Shahriyar et al. ([ISMM 2012](https://doi.org/10.1145/2258996.2259008)) showed unoptimised RC carries **~30%** overhead vs tracing GC, collapsing to **~1–5%** once redundant counts are removed.

> **Corrected by Claude (Opus), 2026-11-26:** see the corrections in §2.1. Neither Lean nor Perceus reports a percentage of RC/alloc operations eliminated, and Shahriyar et al. report parity ("on par") for deferred and coalesced JVM RC, which is not compiler elision.

- **Interpreter vs AOT factor:** `wasmi` 2.0 register interpreter is ~2.2× faster than `wasmi` 1.0, yet still typically **~10–30×** slower than native C (`w2c2`+GCC) on compute/allocation loops **[estimate; Stage `8.Mem` will measure the exact RPi 1 ARMv6 factor]**.

---

## 7. Concrete Prioritised Plan for OxiScript

### 7.1 Prioritised Roadmap Table

| Tier & Step | Change | Expected Fuel Reduction (Cumulative vs 6.5–10.5× Baseline) | Justification | Risks & Mitigations (Correctness, Determinism, Debug Checks) |
|---|---|---|---|---|
| **Q1 (Quick, 1–2 d)** | **Fix naive lowering of array literals & template strings** (`oxiscript-lower` + `oxiscript-rc`): emit `oxi_array_new_cap(id, n)` + direct stores for `[e0..en]`; emit 2-pass `oxi_str_concat_n` (or stack-buffer builder) for template strings; skip `retain`/`release` on `str.const`. | **Array literal: −55 to −65%** (5,584 → ~2,000); **Template string: −50 to −60%** (12,006 → ~5,000) **[estimate]** | Eliminates 2–3× redundant `dlmalloc` allocs/copies/frees per array literal and $O(k^2)$ intermediate string allocs/copies/releases. | Very low risk; preserves exact left-to-right evaluation order (`ir.md §6`) and exact OOM/UTF-8 trap order. |
| **Q2 (Quick, 2–3 d)** | **Runtime fast-path cleanup in `oxiscript-rc`**: (a) Add `alloc_uninit` for strings, clones, and `setfield.init`-covered structs (stop calling `dlmalloc::calloc` + `write_bytes` when overwritten); (b) In `release`, fast-path `rc == 1` for leaf/small objects (free directly without `PENDING` push + immediate `drain` pop + double `td(id)` lookup!); (c) Skip `drain()` entry when `q_head == 0`; (d) Swap `dlmalloc` → `rlsf` (TLSF) or `talc` (`TalcCell`). | **−30 to −45%** across all benchmarks (`new class` 3,070 → ~1,700–2,000) **[estimate]** | Removes redundant zeroing (`wasmi` charges bulk-op fuel per byte!), removes queue push+pop churn and double `td` lookup on every dead object, and speeds up backing alloc/free. | **Correctness:** `alloc_uninit` MUST only be used when verifier guarantees every managed field is overwritten before any read/alloc/trap (already enforced by `ir.md §4` `setfield.init`!). **Debug:** Keep `0xDD` poison + `T_STALE_OBJECT` check behind `#[cfg(debug_assertions)]`. **WCET:** Fast-path direct free only for objects with `pcount == 0` (leafs/strings) or depth = 1 so cascading deep drops still use `q_head` bounded drain! |
| **Q3 (Quick, 3–4 d)** | **Inline Wasm fast paths in `oxiscript-wasm`** (release builds; debug builds can keep calls for `T_STALE_OBJECT`): inline `oxi_is_unique` (5 ops), inline `oxi_retain` (9–11 ops for known-acyclic/non-null), inline `oxi_release` non-final decrement (`rc > 1` for `#acyclic` types: 10 ops, calling `oxi_release_slow` on `rc == 1` or cyclic purple). | **−40 to −60%** on RC-heavy code (`rc_read_traverse`, `struct_cow_update`); further **−15 to −25%** on `new class` **[estimate]** | Eliminates `call` + Rust shadow-stack prologue/epilogue on the 80–95% of RC ops that don't free or buffer roots. | **Debug checks:** Emit calls (or inline `cfg` toggle) when `--debug-rt` is enabled so `T_STALE_OBJECT` live-bitmap checks still catch compiler bugs during BAD ACTOR/fuzzing! **Code size:** ~30–40 bytes per site; gate on `opt-speed` vs `opt-size` if needed. |
| **M1 (Medium, 1–2 wk)** | **Tier 1 4 KiB Segregated Slab Allocator + Inline Slab Free-List Pop** (15 size classes `16..256 B`, 0 B per-object overhead, backed by `rlsf` TLSF for slabs & `> 256 B` blocks). Emit 16-op inline free-list pop in `oxiscript-wasm` for `Op::Alloc` of fixed-size types. | **Alloc/free fuel drops to ~1.2–1.8× Stub** (`new class` → **~500–700 fuel/iter**, **~4.5–6× total speedup**); **−25 to −33% heap RAM** on small objects **[estimate]** | Removes `dlmalloc`/`rlsf` completely from small-object fast path; eliminates +8 B per-object padding; strict $O(1)$ deterministic WCET. | **Determinism:** Deterministic LIFO free list; no randomness. **Fragmentation:** Return empty 4 KiB slabs (keeping 1 hysteresis slab/class) to Tier 2 TLSF so idle classes free RAM for other classes/arrays. **Debug:** 32-byte in-slab `live_bits` gives $O(1)$ `T_STALE_OBJECT` check without a growing global `livemap` vector! |
| **M2 (Medium, 1–2 wk)** | **OxiIR RC Elision Pass v1**: (a) Last-use ownership move (eliminate `retain` + scope-exit `release` on final use); (b) Intraprocedural `retain`/`release` pair cancellation using OxiIR `#effects` + type-based alias disjointness; (c) Borrowed-load analysis (keep `getfield`/`array.get` borrowed when no intervening mutating barrier); (d) Hoist `release` to last use (Perceus rule, unlocking `is_unique` COW!). | **−50 to −70% dynamic RC ops**; eliminates unnecessary `clone`s on COW `struct`s **[estimate, aligned with Lean 4 / Swift OSSA]** | Moves OxiScript from "naive RC" (AssemblyScript's failure mode) to compiler-optimised RC; directly shrinks candidate roots too. | **Correctness (HIGH RISK):** An unsound elision causes use-after-free! **Mitigation:** Mandatory **OxiIR RC-balance + borrow-lifetime verifier** run after the pass, plus differential fuzzing (unoptimised vs optimised IR) under `SimGuest` + debug `T_STALE_OBJECT`! |
| **M3 (Medium, 1–2 wk)** | **Zero-Headroom Intrusive / Slab-Bitmap Cycle Collector**: Replace `roots` `V32` with 32-byte per-slab `purple_bits` + intrusive dirty-slab list (and 8 B intrusive links on large cyclic objects $> 256$ B); reuse `oxi_scratch_ptr` (4 KiB) as fixed DFS work stack with mark-stack overflow slab-scan fallback; remove `log` vector on full `oxi_collect_cycles`; adaptive root threshold in host pump. | **0 bytes dynamic collector headroom**; eliminates `ZOMBIE` objects & OOM aborts; **−30 to −50%** cycle-collector fuel on small cycles **[estimate]** | Collection runs reliably even at 99% heap utilisation (critical for ESP32-S3 / tight sandboxes); eliminates vector realloc/copy overhead. | **Correctness:** Fuzz heavily with adversarial graphs (self-cycles, deep 50k chains triggering stack overflow, mixed cyclic/acyclic arrays, OOM during collection). **ABI:** `oxi_scratch_ptr` reuse is strictly safe because `oxi_collect_cycles`/`oxi_gc_step` only run between host events (`abi.md:15,53`). |
| **M4 (Medium, 2 wk)** | **Interprocedural Borrow / Sink (`own`) Inference + Cursor Inference** (Lean 4 / Nim ARC style) on monomorphised OxiIR. | **Additional −20 to −40% RC ops** across function boundaries and **~100%** inside read-only list/tree loops **[estimate]** | OxiIR is already monomorphised and already supports `own` vs borrowed params; zero ABI break for internal functions. | **Correctness:** Fixed-point analysis must conservatively treat indirect/closure calls (`call_closure`) and `@host` exports as escaping/mutating. |
| **R1 (Research, 3–4 wk)** | **Escape Analysis & Scalar Replacement** on OxiIR: unbox non-escaping `struct`s, boxed nullable scalars (`T?`), `Result<T, E>`, and non-escaping closures into Wasm locals (and shadow-stack frames for small non-escaping arrays). | **Up to −80 to −100% alloc+RC** on numeric/struct-heavy & `Result`/`T?`-heavy hot loops **[estimate]** | Eliminates heap traffic entirely for short-lived `Result`/`T?`/Point/Vec2 values — massive win on both `wasmi` and `w2c2` ARMv6! | **Complexity:** Must respect OxiScript's `LIMIT_LOCALS = 12_000` (`oxiscript-wasm/src/lib.rs:94`) by adding Wasm local-slot reuse (liveness coloring) first! |
| **R2 (Research, 3–4 wk)** | **Perceus Reuse Analysis (`reset`/`reuse`) + Drop Specialisation + Arena Regions (`alloc.arena`)**: Pair last-use `release` with same-class `alloc` for in-place reuse; inline specialised destructors for hot types; add scoped `alloc.arena` for per-frame UI scratch. | **−50 to −90% allocations** on persistent/COW trees & functional updates; bump-speed per-frame arenas **[estimate, aligned with Perceus PLDI 2021]** | Matches Koka/Lean 4 FBIP performance on COW `struct`s and union ASTs; prepares `realtime` profile arenas. | **Memory safety:** Verify reuse tokens cannot keep dead cells alive across recursive calls (the Roc/Counting-Immutable-Beans pitfall, [Lorenzen 2021](https://studenttheses.uu.nl/handle/20.500.12932/40485)). |

> **Corrected by Claude (Opus), 2026-11-26:** the estimates in this table were checked against measurements (ledger section H).
>
> * **Q1** is unrealistic for the measured rows. Direct sizing of `[1,2,3]` gives -5%. Folding the header and data into one allocation, and formatting integers straight into a string (removing 2 of the 4 allocations of `` `k${i}` ``), are the real wins. `concat_n` helps only templates with three or more parts.
> * **Q2(a)** (`alloc_uninit`, zeroing) saves about 0 fuel and weakens the zero-fill contract of `ir.md:150`, which the verifier relies on for nullable and scalar fields. Drop it.
> * **Q2(b)** (leaf direct free) and a small allocator fast path with cold paths out of line are the large wins: a scratch free-list front gave `oxi_alloc` 1908 → 241 and final release 1107 → 668.
> * **Q3** saves tens of fuel per RC op (retain is 41 today).
> * **M2**'s -50 to -70% is not supported by the cited literature; measure it per corpus.
> * Because fuel is charged by static region, report wall time (wasmi and w2c2) next to fuel in every row.

### 7.2 Permissive-Licence Code Reuse Assessment

Every candidate below complies strictly with OxiScript's permissive-licence policy (`docs/HANDOFF.md:8`: MIT / Apache-2.0 / BSD / Zlib / ISC / Public Domain only; **zero GPL/LGPL/MPL**):

| Project / Crate | Licence | Role in OxiScript | Reuse Verdict |
|---|---|---|---|
| **[`rlsf`](https://github.com/yvt/rlsf)** (Rust TLSF) | **MIT OR Apache-2.0** | Tier 2 backing allocator (4 KiB slabs + large blocks $> 256$ B). Strict $O(1)$ WCET, `no_std`, pure Rust, uses `leading_zeros`/`trailing_zeros` (`i32.clz`/`ctz`). | **Recommended (Primary)** — evaluate on `wasm32` alongside `talc`. |
| **[`talc`](https://github.com/SFBdragon/talc)** (Rust allocator) | **MIT** | Alternative Tier 2 backing allocator (`TalcCell` single-threaded, `no_std`, pure Rust, tiny Wasm size, built-in heap resize/counters). | **Recommended (Alternative)** — benchmark fuel + Wasm KB head-to-head against `rlsf`. |
| **[`dlmalloc-rs`](https://github.com/alexcrichton/dlmalloc-rs)** | **MIT OR Apache-2.0** | Current baseline allocator. | Keep only as benchmark baseline; replace with Tier 1 Slabs + `rlsf`/`talc`. |
| **[`mattconte/tlsf`](https://github.com/mattconte/tlsf)** | **3-clause BSD** | Clean C reference for TLSF bitmaps/constants (if ever porting a custom minimal TLSF directly into `oxiscript-rc`). | Reference / optional port. |
| **[`mimalloc`](https://github.com/microsoft/mimalloc)** | **MIT** | Algorithm reference for page-sharded free lists and size-class spacing. | **Borrow algorithm only** (C codebase is too large/OS-centric for tiny Wasm). |
| **[`bumpalo`](https://github.com/fitzgen/bumpalo)** | **MIT OR Apache-2.0** | Reference for chunked bump arenas (Stage R2 `alloc.arena`). | Reference (OxiScript's guest arena is ~30 lines of custom Rust/Wasm). |
| **`wee_alloc`** | **MPL-2.0** | Abandoned, known memory leak (#106), non-MIT/Apache licence. | **Do NOT use.** |
| **Moddable XS / Toit / Jaculus** | **GPL / LGPL** | Forbidden by licence policy. | **Do NOT reuse code.** |
| **Grain** (added by verifier) | **LGPL-3.0** | Design reference only. | **Do NOT reuse code.** |

> **Corrected by Claude (Opus), 2026-11-26:** `HANDOFF.md:8` allows MPL "only if recorded", so the policy is not "zero MPL". Licences re-checked on the repositories: rlsf MIT/Apache-2.0, talc MIT, dlmalloc-rs MIT/Apache-2.0, mattconte/tlsf 3-clause BSD, mimalloc MIT, bumpalo Apache-2.0 (dual licence not re-checked), wee_alloc MPL-2.0 (archived; RUSTSEC-2022-0054).


---

## 8. Open Questions for the Owner

1. **Exact `oxi_live_objects()` / `oxi_live_bytes()` accounting on release fast paths:** Keeping exact per-allocation updates to a 64-bit `live_bytes` counter on inline Wasm fast paths adds ~5–8 fuel per alloc/free. Should `oxi_live_objects()` / `oxi_live_bytes()` be **computed lazily on demand** (walking slab headers + large-object counter, costing 0 fuel on hot paths and ~µs only when called in tests/diagnostics), or kept eagerly updated via 32-bit globals? *(Recommendation: compute lazily on demand; ABI semantics are 100% identical!)*
2. **Debug vs. Release linking (`T_STALE_OBJECT` live-bitmap checks):** Inline Wasm fast paths for `retain`/`release`/`alloc` bypass `oxiscript-rc`'s Rust debug `live_check`. Are we happy with a two-mode codegen flag (`--debug-rt` = all RC/alloc ops call the checked runtime with slab `live_bits`; default release = inline fast paths + OxiIR RC-balance verifier)? *(Recommendation: yes — keeps BAD ACTOR/fuzzing catch rate at 100% while giving release/benchmarks maximum speed.)*
3. **Minor ABI extension for `oxi_rt_init` and linker-pinned slab table address:** Inline `alloc` fast paths are fastest (~16 ops) when `SLAB_FREE_HEADS[15]` sits at a fixed static linear-memory offset known to both `oxiscript-wasm` and `oxiscript-rc` (or passed/exported as an immutable Wasm global `oxi_slab_heads`, similar to `oxi_typedesc_base`), and `oxi_alloc_uninit(type_id)` + `oxi_array_new_cap(type_id, cap)` + `oxi_str_concat_n` are added as internal/extension runtime exports. Since OxiScript is `0.0.1` (`docs/oxi-ui.md:659`), is expanding the internal `oxi:rt` linking table acceptable right now? *(Recommendation: yes — these are internal statically-linked symbols, not host-visible ABI surfaces!)*
4. **Tier 2 backing allocator choice (`rlsf` vs `talc`):** Both are pure Rust, `no_std`, and permissive (MIT/Apache-2.0 vs MIT). `rlsf` guarantees strict $O(1)$ WCET (aligning with the future `realtime`/`safety` profiles), whereas `talc` has slightly lower metadata overhead on very small heaps. Should we prototype both behind `oxiscript-rc`'s `Heap` trait in our benchmark suite (B1–B9) and pick the winner by measured `wasmi` fuel + Wasm KB + peak pages? *(Recommendation: yes — takes ~1 day behind the existing `Heap` trait.)*
5. **Reusing `oxi_scratch_ptr` (4 KiB) as the cycle collector's fixed DFS work stack:** `abi.md §1, §5.3` guarantees `oxi_scratch_ptr` is only written by the host *before* calling a guest entry, and `oxi_collect_cycles`/`oxi_gc_step` only run between events. Is reusing `oxi_scratch_ptr` during cycle collection acceptable to save 4 KiB of static RAM on ESP32-S3, or should the collector own its own dedicated 2–4 KiB static stack to keep the host scratch region strictly single-purpose? *(Recommendation: dedicated 2 KiB static stack [512 entries] keeps separation of concerns crystal clear at negligible 2 KiB cost.)*

> **Corrected by Claude (Opus), 2026-11-26:** the dedicated stack is required, not just preferable (`abi.md:15`, ledger A7). On Q4: talc's own Wasm benchmark ranks talc ahead of rlsf's `GlobalTlsf`, so measure both by fuel entry-region, wall time on ARMv6 and peak pages. On Q6: the first fix should be the runtime-only allocator front plus leaf direct free (no ABI change), not the lowering quick wins.
6. **Priority order relative to `8.S1` and `8.Mem`:** Quick wins **Q1 + Q2** (array-literal/template lowering + unzeroed alloc + leaf direct free + `rlsf`/`talc`) take ~3–5 days and immediately cut fuel by ~2–3× before running the Raspberry Pi 1 `8.Mem` measurements. Should Q1 + Q2 land *before* `8.Mem` publishes its first numbers, or should `8.Mem` capture both before and after? *(Recommendation: capture current baseline once, land Q1+Q2, and publish the before/after delta in `8.Mem`.)*
