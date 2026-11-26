# Verification ledger: `oxiscript-memory-performance.md` (Gemini draft)

Verifier: Claude (Opus), 2026-11-26. Method: same as `oxiscript-simd-gpu-design-verification.md`. Every claim about our code was checked against the repository; every sourced external claim, and every unsourced one that drives a decision, was checked against the cited source or a primary substitute.

* **Our code.** This worktree (`gemini-memres`, e4b9f7d) for `oxiscript-rc/src/rt.rs`, `oxiscript-lower`, `oxiscript-wasm/src/func.rs` and the docs. The linked runtime (8.M-link) comes from branch `gemini-rtlink` at **0cf56d3**, read with `git show` and never touched. That branch holds `oxiscript-rc/src/wasm.rs`, `oxiscript-wasm/src/link.rs` and `build.rs`, `docs/stages/8.M-link/bad-actor.md` (BL06) and `oxiscript-lower/tests/bad_actor_link_perf.rs`. `rt.rs` and `func.rs` are identical on both branches.
* **Crates.** Under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`: `wasmi-2.0.3`, `wasmi_core-2.0.3` and `dlmalloc-0.2.14`. `rlsf` and `talc` are not in the registry; they were checked on GitHub.
* **Web facts.** Taken from the cited page, or from the primary page when the cited one was dead or wrong. Every licence was checked on the project's own repository (GitHub licence detection plus the LICENSE, Cargo.toml or header file).

**Own measurement (new, not in the report).** The questions about fuel could not be settled by reading code alone, so I measured.
* **Build.** The rtlink `oxiscript-rc` (0cf56d3) was built for wasm32 with the exact flags of `oxiscript-wasm/build.rs`: `--release`, `RUSTFLAGS="-C target-feature=-nontrapping-fptoint,-reference-types,-simd128,-tail-call,+bulk-memory,+sign-ext,+multivalue"`, rustc 1.98.0.
* **Harness.** It ran under wasmi 2.0.3 with `consume_fuel(true)` and `CompilationMode::Eager`, the same mode as `oxiscript-runtime/src/runtime.rs:163`. Eager mode matters: a lazy first run also charges translation fuel at 7 per byte, which made `is_unique` look like 171 instead of 10.
* **Probe.** A small wasmparser pass computed, for every function, the operators that wasmi charges at function entry.
* **Location.** The harness and its sources are in `/vectorlegis_ssd_pool/oxiterm/scratch/memres-verify/` (scratch, not committed). Section I has the numbers.

Status values:
* **CONFIRMED**: the source or code says it.
* **PARTLY**: the gist is right, but a number, scope, line or attribution is off.
* **WRONG**: the source or code contradicts it.
* **UNVERIFIABLE**: the source is dead or blocked, or gives no figure.
* **NRC** (not re-checked): a background fact that matches what is widely known but that I did not reopen. No decision rests on one.

## Counts

| | Count |
|---|---|
| Claims checked (ledger rows B-G, excluding NRC) | 114 |
| CONFIRMED (incl. 4 gaps added by the verifier, C5-C7, and design properties) | 70 |
| PARTLY | 26 |
| WRONG | 13 |
| UNVERIFIABLE | 5 |
| NRC (not counted above) | 18 |

Section A lists the 10 findings that change the design (A1-A10). Each also appears as a row in B-H and is not counted twice. The report's own **[estimate]** labels are judged separately in section H.

## A. Corrections that change the design (read first)

| # | Report section | Gemini claim | Finding | Evidence |
|---|---|---|---|---|
| A1 | §0.1, Exec. summary 2, §5.1 | "every Wasm operator ... costs 1 fuel per executed Wasm opcode"; a `call` costs 1 plus every instruction inside the callee | **WRONG, and this is the root cause the report misses.** The per-operator cost table is right: 1, and 0 for `nop drop block loop unreachable return else end`. But wasmi 2.0 charges fuel **per region and in advance**, not per executed operator. Each `Op::ConsumeFuel` sits at function entry, at a `loop` header, at an `if` then-arm and at an `else`, and it charges the **static** sum of every operator in its region, plus 1. A `block` opens no region. Code after a `br_if` exit or an early `return` is therefore paid for even when it never runs. The CHANGELOG says so: fuel is "tied to the input Wasm bytecode ... less precise". LLVM emits Rust as `block`/`br_if` and almost never as `if`: my probe found **no `if` in any of the 89 defined runtime functions**. A call into a large inlined Rust function therefore pays for its whole non-loop body. dlmalloc's `malloc`, inlined into `calloc`, has a **1651-operator entry region**, so every allocation costs about 1700 fuel whatever path it takes. That is why fuel is 6.5-10.5x the stub while wall time is only 2.5x (BL06). The stub runtime is emitted by our own encoder and uses `if_` regions (`stubrt.rs`); its metering was not measured separately. Further costs: a call adds `ceil(zeroed_locals/8)`, and `memory.copy`/`memory.fill` add `len/64`. | `wasmi-2.0.3/src/engine/costs.rs:505-538`; `translator/func/mod.rs:301` (function region), `visit.rs:118-130` (`block`: no region), `:146` (`loop`), `:200` (`if`), `:233` (`else`), `mod.rs:376-378`, `:384-404` (locals), `encoder.rs:396-410` ("charges at least 1"); `wasmi_core-2.0.3/src/fuel.rs:20-22, 30-32`; wasmi CHANGELOG 2.0.0 "Fuel metering"; probe, section I |
| A2 | Exec. summary 1-3, §0.2-0.3, Pillar 2 | Excess fuel splits about 40-50% allocator, 25-35% call overhead, 20-40% lowering; "hot retain/release/alloc ~150-400 fuel" | **WRONG split.** Measured: `oxi_retain` **41** fuel, `oxi_is_unique` **10**, non-final `oxi_release` **120**, `oxi_alloc` **1908**, final release (rc 1 to 0) **1107**. For `new class`, alloc + final release = 3015 against BL06's 3070 per iteration. So **the allocator plus the destruction path is ~95% of that row**, and call overhead on retain and is_unique is a few percent. Inlining RC ops (Pillar 2 a-c) is worth tens of fuel per op, not hundreds. | Section I; `bad-actor.md:19` (rtlink) |
| A3 | §0.2, §1.3, Q2(a), Pillar 2(e) | `calloc` zeroing is a major cost ("wasmi charges bulk-op fuel per byte"); add `alloc_uninit` | **The code claim is CONFIRMED; the cost claim is WRONG.** `calloc_must_clear` is `!allocates_zeros() \|\| !mmapped`. This dlmalloc never mmaps a chunk, so every allocation is cleared even though `WasmSys::allocates_zeros()` returns true. But with `+bulk-memory` the clear is **one `memory.fill`** in `calloc`, costing 1 fuel plus `len/64`: **about 1-2 fuel for a 24-byte object**. `alloc_uninit` is also less safe than claimed. `ir.md:150` *relies* on `alloc` zero-filling nullable and scalar fields, and the verifier demands `setfield.init` only for **non-nullable managed** fields, not for "every field". Drop `alloc_uninit` from the quick wins. | `dlmalloc-0.2.14/src/lib.rs:178-184`, `dlmalloc.rs:319-320, 1805-1807`, no mmap path in `sys_alloc` (`:440`); rtlink `wasm.rs:74-76, 123-126`; probe (`f28`: one `memory.fill`); `docs/oxiscript/ir.md:150` |
| A4 | §0.4, Exec. summary 3, Q1 | The array-literal and template rows blow up because of `ArrayEmpty`+push (3 allocs, copy, free) and pairwise `StrConcat`; Q1 cuts them by 55-65% and 50-60% | **WRONG for the measured rows.** BL06's array literal is `[1, 2, 3]`. `oxi_array_new(id, 0)` allocates only the 24-byte header (`alloc_block(0)` returns 0), and one grow to capacity 4 allocates the data. That is 2 allocations and no copy; the 5-element case in the report is not what was measured. Measured: new(0) 1949 + grow 1921 + release 1578 = 5448 against BL06's 5584. Direct sizing (`oxi_array_new(id, 3)`) measures **3603 + 1578 = 5181, only -5%**, because header and data are still two allocations. The template row is `` `k${i}` `` and has **exactly one concat**; its cost comes from `lw.itoa` → `lw.fmt_u`. That builds a `[u8]` (`array.new`: 2 allocations), copies it into a string (`str.from_utf8`: 1 allocation), releases the buffer and then concatenates (1 allocation): **4 allocations and 4 frees per iteration**. `concat_n` saves nothing there; formatting straight into a string (`oxi_str_from_i64` already exists in the runtime) removes 2 allocations and 2 frees. | `rt.rs:1354-1404` (`array_new`, `array_grow`), `:556-563` (`alloc_block`); `oxiscript-lower/src/expr.rs:489-501` (array lit), `:288-330` (template), `:234-272` (`to_string_op`); `oxiscript-lower/src/helpers.rs:8-62` (`fmt_u`, `itoa`); rtlink `bad_actor_link_perf.rs:99-107` |
| A5 | §1.1 (6-7), §1.2, §7.2, Q4 | talc ~5-8 KB and rlsf ~4-7 KB of Wasm; rlsf is the "Best backing allocator" | **PARTLY / WRONG on numbers and ranking.** rlsf's README measures **1267 B** of Wasm (1584 B with pool coalescing), against dlmalloc's **9613 B**. Talc's own Wasm benchmark concludes: "DLmalloc and RLSF's GlobalTlsf are worse in both size and performance to Talc", but talc's default Wasm configuration uses "10%-15% higher runtime memory". After A1, a backing allocator's fuel is decided mostly by **how its code is laid out** (entry-region size), not by its O(1) bound. Measure both; do not pre-rank. | https://github.com/yvt/rlsf (README); https://github.com/SFBdragon/talc/blob/master/BENCHMARKS_WASM.md |
| A6 | §5.3 (2), §7.2 | Grain is **Apache-2.0** | **WRONG.** grain-lang/grain is **LGPL-3.0** (its LICENSE file). Learning from it is fine; no code may be reused under our policy. | https://github.com/grain-lang/grain (LICENSE: "GNU LESSER GENERAL PUBLIC LICENSE Version 3") |
| A7 | §3.3 (3), §3.2, M3, §8 Q5 | Full `oxi_collect_cycles` "never aborts and therefore needs no log"; `log` is only for undo; reusing `oxi_scratch_ptr` is "strictly safe" | **WRONG on all three points.** (1) `log` doubles as the **list of dead nodes** for phase 3 (`gather_white`, then pass A and pass B), which is needed because untraced children must be released before anything is freed. (2) `collect_cycles` *does* abort: on OOM while reserving the stack it undoes and retries (`rt.rs:1169-1176`). The undo log is what makes OOM non-fatal. (3) `abi.md:15` lets the host write the scratch region whenever the guest is not running, and `oxi_gc_step` runs exactly then, so the two can collide. A bounded `gc_step` that aborts needs undo state proportional to the work it did. A fixed ring cannot guarantee that; the design has to stop aborting (finish the component) or bound undo some other way. Mark-stack overflow with a rescan works for Bacon-Rajan only with an extra "gray, edges not yet subtracted" state, because MarkGray's decrements are not idempotent. Worst-case time then becomes O(heap) per overflow. | `rt.rs:858-910` (log in `mark_gray`), `:1066-1104` (phase 3 reuses `log`), `:1154-1210`; `docs/oxiscript/abi.md:15` |
| A8 | §2.1, §2.2, §6.3, Pillar 3, M2 | "50-80% of dynamic retain/release removed (matching Lean 4 / Perceus / Swift OSSA literature)"; Lean reuse removes "50-90%+ of allocations"; disabling borrow inference gives "10-40%+ slowdowns" | **UNVERIFIABLE / WRONG attribution.** Neither paper reports a percentage of RC operations or allocations removed. Lean (IFL'19, Fig. 6) reports *run time* with an optimisation disabled. `-borrow`: geometric mean **1.27**; binarytrees 1.14, deriv 1.16, **const_fold 0.90 (faster without it)**, the rest about 1.00 (rbmap_1 4.47 is normalised to rbmap). `-reuse`: geometric mean 1.74 (const_fold 1.64, rbmap 3.23, unionfind 1.41). Perceus says optimisations "are less effective" with sharing (deriv, nqueens). For Swift the report itself labels the 50-80% an estimate, and no source was found. 8.Perf's own metric ("% of retain/release removed per corpus") is the right way to get our number. | `arxiv.org/abs/1908.05647` (PDF, §8, Fig. 6); Perceus MSR-TR-2020-42 v4 §4 |
| A9 | §2.1 (2) | Koka with Perceus is "up to 2x faster than OCaml on rbtree", "peak heap ~1x live set" | **WRONG.** On rbtree "OCaml is close in performance". The 2x is Koka's **"no-opt" vs optimised** build. OCaml uses *less* memory than Koka on deriv and cfold. | Perceus TR v4, §4 benchmark discussion |
| A10 | §5.3 (1) | AssemblyScript abandoned RC in "PR #1600"; one reason was that JS callers had to `__pin`/`__unpin` | **PARTLY / WRONG.** The change is **PR #1559**, "Rewrite runtime, switch to tracing GC and bootstrap", released in **v0.18.0 (2021-01-29)**. #1600 is an unrelated interface fix. Its stated reason is that it "removes the error-prone automatic reference counting implementation that has been haunting us for a long time", plus preparation for Wasm GC. `__pin`/`__unpin` is the **new** GC-era API that replaced `__retain`/`__release`. "No SSA elision pass" is the report's plausible inference, not AssemblyScript's stated reason. | https://github.com/AssemblyScript/assemblyscript/pull/1559 ; release v0.18.0 |

## B. Ledger: our code (task 1)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| B1 | BL06 numbers: 398→3070, 579→5584, 1657→12006; 6.5-10.5x fuel; ~2.5x wall | CONFIRMED | rtlink `docs/stages/8.M-link/bad-actor.md:19,27` (wall 2.5-2.7x) |
| B2 | `oxi_alloc` at `rt.rs:571-581` calls `drain(drain_budget)` on every allocation | CONFIRMED (the call is unconditional; `drain` returns at once when `q_head == 0`) | `rt.rs:571-581, 745-746` |
| B3 | `oxi_alloc` at `wasm.rs:57-60` | PARTLY | That is `WasmHeap::alloc` (`calloc`) in this worktree's `wasm.rs:57-60`; rtlink has the export at `:155-157` and `calloc` at `:125` |
| B4 | `td()` checks `id < td_count` and keeps a one-entry cache that misses when types alternate | CONFIRMED | `rt.rs:339-358` |
| B5 | `raw_alloc` loop → `WasmHeap::alloc` → `DL.calloc(total, 8)` | CONFIRMED | `rt.rs:380-394`; rtlink `wasm.rs:123-126` |
| B6 | `calloc` → `c_memalign(8)` → `c_malloc` | CONFIRMED | `dlmalloc-0.2.14/src/lib.rs:171-184, 348-354` (align 8 = `malloc_alignment` on wasm32) |
| B7 | `calloc_must_clear` is true for every non-mmapped chunk (`dlmalloc.rs:319-320`) | CONFIRMED (and this dlmalloc never produces mmapped chunks) | A3 |
| B8 | Zeroing is a significant fuel cost | WRONG | A3 (1 `memory.fill` = 1 + len/64 fuel) |
| B9 | 4-byte head and 8-byte rounding: 16→24, 24→32 (`pad_request`, `dlmalloc.rs:258-260`) | CONFIRMED | `dlmalloc.rs:201-212, 258-260, 303-307` (`chunk_overhead` = 4, `malloc_alignment` = 8, minimum chunk 16) |
| B10 | `live_bytes` is `u64`; `live_objects` is `u32`; `livemap` is updated in debug builds only | CONFIRMED | `rt.rs:187-188, 528-566` |
| B11 | Release path: `dec` marks PENDING, pushes on `q_head`; `release` drains immediately; `drain` pops, `td`, pmap loop, `finish_free` → `dealloc_object` → `obj_total` → second `td` → `free` (`validate_size`, coalescing) | CONFIRMED (the second `td` hits the one-entry cache, so it is cheap) | `rt.rs:618-656, 664-743, 745-795`; `dlmalloc lib.rs:204-208`, `dlmalloc.rs:1252-1340` |
| B12 | rc 2→1 on a cyclic-capable object sets PURPLE\|BUFFERED and pushes onto `roots`, even if it is retained again right after | CONFIRMED | `rt.rs:636-645` |
| B13 | Array literal lowered as `ArrayEmpty` + `ArrayPush` (`expr.rs:489-501`) | CONFIRMED | `oxiscript-lower/src/expr.rs:489-501` |
| B14 | 5 pushes = 3 allocations + 1 copy + 1 free | PARTLY (correct for 5; BL06 measures 3 elements = 2 allocations, 0 copies) | A4 |
| B15 | Template strings: left-associative pairwise `StrConcat` (`expr.rs:283,1188`) | PARTLY (the template lowering is `:288-330` via `concat` at `:281-286`; `:1188` is binary `+`) | `expr.rs:281-330, 1186-1190` |
| B16 | The template row blows up because of pairwise concat | WRONG | A4 (one concat; 4 allocations come from `itoa`/`fmt_u`) |
| B17 | "Own everything" lowering (`docs/stages/8.L/implementation.md:20`) | CONFIRMED | `docs/stages/8.L/implementation.md:20` |
| B18 | `func.rs:733-736, 1023-1030` emit a call for every IsUnique, Retain and Release | CONFIRMED | `oxiscript-wasm/src/func.rs:733-737, 1023-1030` (identical on rtlink) |
| B19 | `roots`, `stack`, `log` are growable `V32` vectors (`rt.rs:183-185`) | CONFIRMED | `rt.rs:183-185` |
| B20 | Before phase 2, `stack.cap >= 2*work + roots + 8` is reserved (`rt.rs:1056`) | CONFIRMED | `rt.rs:1054-1061` |
| B21 | 256 KiB heap / 10,000 nodes → 280 KB of scratch | PARTLY (synthetic; it assumes every node is a candidate. The measured fact is BL06's "100k garbage 2-cycles collect = grows 92 -> 248 pages") | rtlink `bad-actor.md:27` |
| B22 | `log` exists only to undo; full collection needs no log | WRONG | A7 |
| B23 | `oxi_scratch_ptr` reuse is strictly safe (`abi.md:15,53`) | WRONG | A7; `abi.md:53` is the `T \| null` boxing row, unrelated |
| B24 | `oxiscript-rc/build.rs` reserves a 64 KiB shadow stack | CONFIRMED | rtlink `oxiscript-rc/build.rs:1-9` |
| B25 | Retain/release pay shadow-stack (`$__stack_pointer`) traffic | UNVERIFIABLE as a cost (`oxi_retain` measures 41 fuel in total, so the traffic is at most a few fuel) | section I |
| B26 | Single-threaded, non-atomic rc (`abi.md:22`) | CONFIRMED | `abi.md:22` |
| B27 | All object sizes are multiples of 8 (`abi.md:19`) | CONFIRMED | `abi.md:19` |
| B28 | Array header 24 B; a string ≤ 47 bytes fits 64 B | CONFIRMED | `rt.rs:69` (`ARR_OBJ_SIZE = 24`), `:1406-1415` (8 + 8 + len + 1, rounded to 8) |
| B29 | `alloc(type_id)` knows the size class at compile time | CONFIRMED for records, closures and variants (`TD.size` is static) | `rt.rs:571-581` |
| B30 | No GC inside a guest call (`abi.md:81`) | CONFIRMED | `abi.md:81, 178` |
| B31 | Hot reload / instance swap (`abi.md §4`) | CONFIRMED | `abi.md:126` |
| B32 | Boxed `T?` scalars (`abi.md:53`) | CONFIRMED | `abi.md:53` |
| B33 | COW `is_unique ? mutate : clone` (`ir.md:156`); `alloc.arena` planned (`ir.md:175`) | CONFIRMED | `ir.md:156, 175` |
| B34 | `setfield.init` guarantees every managed field is overwritten, so `alloc_uninit` is safe | WRONG | A3; `ir.md:85, 150` |
| B35 | OxiIR has `own` params, `#acyclic`, `#effects`, `#pure`, is monomorphised SSA with no exceptions | CONFIRMED | `ir.md:29, 120-160` |
| B36 | The lowering already uses `#effects` and `own` inference | PARTLY (the attributes exist; `#effects` is "absent = unknown", and the lowering does no inference) | `ir.md:29`; `8.L/implementation.md:20-22` |
| B37 | `LIMIT_LOCALS = 12_000` (`oxiscript-wasm/src/lib.rs:94`) | CONFIRMED | `oxiscript-wasm/src/lib.rs:94` |
| B38 | Native gc_step: 0.09 µs/unit, p99 ~8x, worst 4 ms at budget 1000 (`lib.rs:56-59`) | CONFIRMED | `oxiscript-rc/src/lib.rs:56-59` |
| B39 | `STICKY = 0x8000_0000` acts as an immortal flag | CONFIRMED | `rt.rs:41, 595-604` |
| B40 | Decision 1 / GC-like convenience (`oxi-ui.md:711`); wasm32 pointer width (`:735`); WasmGC excluded (`:624`); 0.0.1 (`:659`) | CONFIRMED (`:654` is the 8.2 targets row, not WasmGC) | rtlink `docs/oxi-ui.md:624, 654, 659, 711, 735` |
| B41 | Licence policy "zero GPL/LGPL/MPL" (`HANDOFF.md:8`) | PARTLY ("MPL only if recorded") | `docs/HANDOFF.md:8` |
| B42 | Linked runtime: Wasm size, compile and instantiate costs | NRC (not used by the report) | rtlink `bad-actor.md:27` |

## C. Ledger: wasmi fuel model and engines

| ID | Claim | Status | Evidence |
|---|---|---|---|
| C1 | Default operator cost 1; 0 for nop, drop, block, loop, unreachable, return, else, end (`costs.rs:505-538`) | CONFIRMED | `costs.rs:505-538` |
| C2 | Fuel is charged per executed operator | WRONG | A1 |
| C3 | `memory.copy`/`memory.fill` charge per-byte bulk fuel (`exec.rs:530,592`) | PARTLY (1 fuel per **64** bytes by default; the lines are right) | `executor/handler/exec.rs:530, 553, 592, 643`; `wasmi_core fuel.rs:30-32, 160-187` |
| C4 | A call costs 1 plus the callee's ops | PARTLY (plus `ceil(zeroed_locals/8)`; the callee is charged by region, A1) | `mod.rs:384-404`; `wasmi_core fuel.rs:20-22` |
| C5 | Gap (new): every `ConsumeFuel` charges at least 1 | CONFIRMED (added) | `encoder.rs:393-410` |
| C6 | Gap (new): fuel costs are configurable (`Config::operator_cost`, `Config::fuel_cost`) | CONFIRMED (added) | `config.rs:430`; CHANGELOG 2.0.0 |
| C7 | Gap (new): a lazily compiled function charges translation fuel (7 per byte + 2 per byte validated) on its first call; we use Eager | CONFIRMED (added) | `code_map/mod.rs:860-874`; `oxiscript-runtime/src/runtime.rs:163` |
| C8 | wasmi 2.0 is ~2.2x faster than 1.0 | CONFIRMED (the claim; the link points to the v1.0 blog, and the source is the 2.0.0 CHANGELOG) | wasmi CHANGELOG 2.0.0: "geomean roughly 2.2x faster" |
| C9 | wasmi: register-based, tail-call dispatch (`auto-dispatch`) | CONFIRMED | CHANGELOG 2.0.0; `oxiscript-runtime` features |
| C10 | wasmi licence MIT OR Apache-2.0 | NRC | |
| C11 | `memory.grow` on wasmi is `Vec` growth that may copy the whole memory | NRC (plausible; not measured) | |
| C12 | wasmi ~10-30x slower than native | UNVERIFIABLE (labelled estimate; 8.Mem measures it) | |
| C13 | WAMR fast interpreter is 1.5-2x faster than its classic interpreter | NRC | |
| C14 | wasm3: MIT, slowed maintenance | PARTLY (MIT CONFIRMED on GitHub; maintenance NRC) | github.com/wasm3/wasm3 |
| C15 | w2c2 MIT; wabt (wasm2c) Apache-2.0; WAMR Apache-2.0 (with LLVM exception) | CONFIRMED (GitHub licence detection; LLVM exception NRC) | GitHub API |
| C16 | GCC/Clang can inline small runtime helpers in w2c2 output, and fast/slow splitting helps both wasmi and w2c2 | CONFIRMED in principle (after A1, the split is *the* fuel fix in wasmi as well) | section I experiment |
| C17 | On ARMv6/ESP32 w2c2 cannot use guard pages, so memory accesses are bounds-checked | NRC | |

## D. Ledger: allocators and licences (task 2)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| D1 | mimalloc MIT | CONFIRMED | GitHub |
| D2 | mimalloc 7% faster than tcmalloc, 14% than jemalloc on redis | CONFIRMED | MSR-TR-2019-18 abstract |
| D3 | mimalloc: 64 KiB pages, free-list sharding, built for Koka and Lean | CONFIRMED | mimalloc README ("usually 64KiB on a 64-bit system") |
| D4 | mimalloc page free list popped in ~5 instructions; ~12.5% class spacing | NRC | |
| D5 | snmalloc MIT, message-passing frees | CONFIRMED (licence); design NRC | GitHub |
| D6 | rpmalloc "Public Domain / MIT" | PARTLY (LICENSE today is MIT only) | github.com/mjansson/rpmalloc LICENSE |
| D7 | jemalloc 2-clause BSD; tcmalloc Apache-2.0 | PARTLY (tcmalloc Apache-2.0 CONFIRMED; jemalloc licence NOASSERTION in GitHub detection, BSD-2 NRC) | GitHub API |
| D8 | dlmalloc-rs MIT OR Apache-2.0 | CONFIRMED | dlmalloc-rs Cargo.toml `MIT/Apache-2.0` |
| D9 | dlmalloc smallbins up to 256 B + treebins | CONFIRMED | `dlmalloc.rs:215-228` (`TREEBIN_SHIFT`) |
| D10 | dlmalloc ~10-15 KB Wasm | PARTLY (rlsf's measurement: 9613 B) | rlsf README |
| D11 | talc MIT, pure Rust, `TalcCell` single-threaded, counters, heap resize | CONFIRMED | talc README; `talc/Cargo.toml` `license = "MIT"` (v5.1.1) |
| D12 | talc faster than dlmalloc on Wasm and smaller (~5-8 KB) | PARTLY (faster and smaller CONFIRMED; KB not in text, plot only; +10-15% memory caveat omitted) | talc BENCHMARKS_WASM.md |
| D13 | rlsf MIT OR Apache-2.0, constant-time alloc and free, `no_std` | CONFIRMED | rlsf README and Cargo.toml (`MIT/Apache-2.0`; GitHub detects only Apache) |
| D14 | rlsf ~4-7 KB | WRONG (1267 B) | rlsf README |
| D15 | TLSF paper: Masmano et al., ECRTS 2004, DOI 10.1109/EMRTS.2004.1311009 | CONFIRMED | cited by rlsf README |
| D16 | TLSF average fragmentation ~10-15% | NRC | |
| D17 | mattconte/tlsf 3-clause BSD | CONFIRMED | `tlsf.h` header (3 clauses incl. "Neither the name ...") |
| D18 | TLSF is AssemblyScript's allocator | CONFIRMED | assemblyscript.org/runtime.html |
| D19 | emmalloc MIT/NCSA, ~5-7 KB | NRC | |
| D20 | lol_alloc MIT, ~1-2 KB, O(n) | PARTLY (MIT CONFIRMED; talc's benchmark ranks it smallest; size NRC) | lol_alloc Cargo.toml; talc BENCHMARKS_WASM |
| D21 | wee_alloc unmaintained (RUSTSEC-2022-0054), leak issue #106, MPL-2.0 | CONFIRMED (repo archived) | rustsec advisory-db; issue #106 "Unbounded Memory Leak" (open); GitHub |
| D22 | bumpalo MIT OR Apache-2.0 | PARTLY (GitHub detects Apache-2.0; dual licence NRC) | GitHub |
| D23 | Buddy: up to 50% internal fragmentation | CONFIRMED (textbook) | |
| D24 | "Zero per-object allocator header" for slabs with `ptr & !4095` | CONFIRMED as a design property (needs 4 KiB-aligned slabs from the backing allocator) | |
| D25 | Slab fast path = ~16-18 fuel inline | PARTLY (plausible for inline Wasm; my Rust-only free-list front measured 241 fuel for the whole `oxi_alloc`, of which the RC header work and `drain` are most) | section I |
| D26 | Size classes `16..64` step 8, then `80..256` geometric = 15 classes | PARTLY (7 + 8 = 15 classes, but the report also says "14 size classes" in Pillar 1 and §1.3) | report §1.3 |

## E. Ledger: RC literature (task 2)

| ID | Claim | Status | Evidence |
|---|---|---|---|
| E1 | Ullrich & de Moura, IFL 2019, arXiv:1908.05647; borrow inference, reset/reuse | CONFIRMED | arXiv |
| E2 | Lean benchmarks rbmap, deriv, const_fold, qsort, binarytrees | CONFIRMED (also parser, unionfind) | paper §8 |
| E3 | Lean matches or beats GHC/OCaml/MLton | PARTLY ("competitive and often outperforms"; on rbmap Lean is faster than all but OCaml) | paper §8 |
| E4 | -borrow: 10-40%+ slowdowns | PARTLY (14-16% on two benchmarks; const_fold faster without; geometric mean 1.27) | A8 |
| E5 | Reuse removes up to 50-90%+ of allocations | UNVERIFIABLE (no allocation counts in the paper; run time 1.41-3.23x without reuse on three benchmarks) | A8 |
| E6 | Perceus PLDI 2021 / MSR-TR-2020-42, garbage-free, drop/reuse specialisation | CONFIRMED | MSR page; TR v4 |
| E7 | Koka up to 2x faster than OCaml on rbtree; peak heap ~1x live | WRONG | A9 |
| E8 | TRMC: Leijen & Lorenzen, POPL 2023 | NRC | |
| E9 | Last-use dropping turns a clone into an in-place update for OxiScript COW | CONFIRMED (follows from `ir.md:156` + Perceus) | |
| E10 | Swift 5 switched parameters to `@guaranteed` | WRONG (Swift 4.2, 2018) | Swift forum proposal "change the default ownership for passing parameters"; jessesquires.com 2018-07-05 |
| E11 | Swift OSSA removes ~50-80% of RC traffic | UNVERIFIABLE (labelled estimate; no source found) | |
| E12 | Nim ARC/ORC: move, sink, lent, cursor inference, acyclic | CONFIRMED (features as documented in Nim's destructors doc) | nim-lang.org/docs/destructors.html (NRC for "automatic acyclic proof") |
| E13 | Roc pitfall (Lorenzen thesis, Utrecht 2021) | UNVERIFIABLE (studenttheses.uu.nl returns 403) | |
| E14 | Deferred RC needs root scanning, which Core Wasm lacks without a shadow stack | CONFIRMED (argument) | |
| E15 | Levanoni-Petrank coalescing RC removes 90-99% of heap RC updates | NRC | |
| E16 | BRC: ~17-22% speedup | PARTLY (22.5% lower client execution time; 7.3% higher server throughput) | OpenAlex abstract, DOI 10.1145/3243176.3243195 |
| E17 | Shahriyar et al. ISMM 2012: naive RC ~30% slower, optimised within ~1-5% | PARTLY ("30% overhead", then "on par with a well tuned mark-sweep"; that is deferred/coalesced JVM RC, not compiler elision) | ISMM 2012 abstract (ANU portal) |
| E18 | LXR, PLDI 2022, arXiv:2204.01918; beats G1/Shenandoah/ZGC by 10-30% | WRONG (arXiv id is a different paper; the extended version is 2210.17175; +4% throughput over G1 and +43% over Shenandoah on 17 workloads) | https://arxiv.org/abs/2210.17175 |
| E19 | Ulterior RC ~1% overhead; RC Immix matches generational copying | NRC | |
| E20 | "Down for the Count": >99.9% of objects never exceed a tiny count | NRC | |
| E21 | Rust `Rc::make_mut` ≈ OxiScript COW | CONFIRMED | |

## F. Ledger: cycle collectors

| ID | Claim | Status | Evidence |
|---|---|---|---|
| F1 | Bacon-Rajan ECOOP 2001: colours, buffered bit, green, three phases | CONFIRMED (matches `rt.rs`) | `rt.rs:46-55, 858-1104` |
| F2 | Current collector suffers false-alarm buffering on live cyclic structures | CONFIRMED (`dec` buffers every non-green decrement to rc > 0) | `rt.rs:636-645` |
| F3 | CPython: `PyGC_Head`, 3 generations, thresholds (700, 10, 10) | NRC (3.13 values; the reverted 3.14 incremental GC changed them) | |
| F4 | PEP 683 immortal objects (3.12) | CONFIRMED | peps.python.org/pep-0683 |
| F5 | CPython 3.14 incremental GC reverted (3.14.5) because of memory pressure | CONFIRMED | discuss.python.org "Reverting the incremental GC in Python 3.14 and 3.15" (memory "5x" worst case with cyclic garbage) |
| F6 | PHP: root buffer 10,000, adaptive threshold in 7.3+ growing up to 1,000,000 | PARTLY (default 10,000 + first root, step 10,000, **max 1,000,000,000**; buffer 16 Ki entries) | `Zend/zend_gc.c:184-195` |
| F7 | PHP stores the root-buffer index in each object, O(1) removal | CONFIRMED (in the `GC_INFO` bits of the existing type_info word, **not** an extra 4 B) | `zend_gc.c:88-143` |
| F8 | Nim ORC: Bacon-Rajan with colour bits in the rc word | NRC | |
| F9 | Pony ORCA (AGERE 2015, OOPSLA 2017) | NRC | |
| F10 | Per-slab purple bitmap removes zombies with O(1) clearing | CONFIRMED as a design (needs every cyclic object in a slab; large ones need links) | |
| F11 | Mark-stack overflow fallback gives O(1) extra memory | PARTLY (memory yes; time O(heap) per overflow; needs an extra colour state) | A7 |
| F12 | Fully log-free bounded steps | WRONG | A7 |
| F13 | Adaptive trigger threshold (PHP style) | CONFIRMED (PHP has it, F6); fits our `root_threshold` (`rt.rs:168-175`) | |

## G. Ledger: engines and ecosystem

| ID | Claim | Status | Evidence |
|---|---|---|---|
| G1 | AssemblyScript: TLSF + pure RC with a Bacon-Rajan collector, replaced by ITCMS + TLSF in v0.18 | CONFIRMED (v0.18.0, PR #1559) | A10 |
| G2 | The reason was missing elision | PARTLY (stated reason: "error-prone ARC"; elision is an inference) | A10 |
| G3 | JS callers had to `__pin`/`__unpin` in the RC era | WRONG (`__retain`/`__release` in the RC era; `__pin` came with the GC) | A10 |
| G4 | Runtime variants: incremental, minimal, stub; shadow stack for the incremental one | CONFIRMED | assemblyscript.org/runtime.html |
| G5 | AssemblyScript Apache-2.0 | CONFIRMED | GitHub |
| G6 | Grain Apache-2.0 | WRONG (LGPL-3.0) | A6 |
| G7 | Grain RC history, moving to WasmGC | NRC | |
| G8 | MoonBit uses Perceus-style RC on linear-memory and native backends | PARTLY (it uses RC/its own memory management and benchmarks from the Perceus suite; "Perceus-style" not confirmed) | moonbitlang.com/blog/native |
| G9 | Kotlin/Wasm WasmGC-only | NRC | |
| G10 | TinyGo conservative GC; BSD-3 | PARTLY (3-clause BSD text CONFIRMED in LICENSE; GC details NRC) | tinygo LICENSE |
| G11 | V8 pointer compression, Oilpan, escape analysis; JSC libpas; SpiderMonkey 4 KiB arenas; Go escape analysis; OCaml 5; TLAB | NRC (background; no decision depends on exact figures) | |
| G12 | Moving/copying GC and conservative stack scanning do not transfer to Core Wasm | CONFIRMED (argument; also conflicts with determinism, `ir.md §6`) | |
| G13 | Pointer compression "already have it" with wasm32 | CONFIRMED | `oxi-ui.md:735` |

## H. Estimates judged (task 3)

Each **[estimate]** (or unlabelled projection) in the report is rated **realistic**, **optimistic** or **unrealistic** against the measurements in section I. A rating is a judgement, not a measurement.

| Report location | Estimate | Verdict | Why |
|---|---|---|---|
| Exec. summary | Pillars 1-3 → 1.1-1.8x stub, 4-7x less fuel | **optimistic but reachable** for `new class` | alloc + final release must drop from 3015 to ~300. The scratch free-list front reached 241 + 668. Leaf direct free (no queue) and a smaller `drain` entry are the rest. |
| Pillar 2 | retain/release/alloc ~150-400 → ~8-20 | **unrealistic as framed** | retain is 41 and is_unique 10 today; the hundreds are alloc and final release only |
| §5.1 | Inlining a 10-op fast path is 10-25x fuel and ~5-15x wall per op | **unrealistic** | for retain/release, 41 → ~10 is 4x; realistic for alloc only as part of an allocator change |
| Q1 | Array literal -55 to -65%; template -50 to -60% | **unrealistic** | A4: direct sizing measures -5% on `[1,2,3]`; the template has one concat. Realistic alternatives: fold header + data into one allocation (≈ -35% on `[1,2,3]` with today's allocator **[estimate]**); format integers into a string directly (removes 2 of 4 allocations) |
| Q2 | -30 to -45% (new class 3070 → 1700-2000) | **realistic, but for other reasons** | zeroing and `alloc_uninit` give ~0 (A3); swapping dlmalloc for rlsf/talc helps only if their entry regions are small; a size-class front alone measured 1908 → 241 |
| Q3 | -40 to -60% on RC-heavy code; -15 to -25% on new class | **optimistic** | RC calls are 10-120 fuel; the saving on new class is ~1% |
| M1 | new class → 500-700 fuel/iter, -25 to -33% RAM | **realistic** (fuel); RAM plausible for 16/24/32 B objects | section I |
| M2 | -50 to -70% dynamic RC ops | **unrealistic** as a literature-backed figure | A8; measure it per corpus |
| M3 | -30 to -50% collector fuel | **unsupported** | no measurement of collector fuel exists |
| M4 | +20-40% fewer RC ops, ~100% in read-only loops | **optimistic** | |
| R1 | -80 to -100% alloc+RC on struct-heavy loops | **realistic for the eligible values only** (non-escaping `T?`, `Result`, small structs) | |
| R2 | -50 to -90% allocations on COW trees | **optimistic**; Perceus shows little gain with sharing | |
| §1.3 | ~1500+ fuel inside `oxi_alloc` today | **realistic** (measured 1908) | |
| §6.3, §7.1 | "< 15-25 MB RSS", "< 25-35 KB" Wasm | **unsupported** | no measurement |
| §8 Q1 | Exact `live_bytes` accounting costs ~5-8 fuel per op | **realistic** | u64 add/sub, 3-6 ops; irrelevant next to 1900 |

## I. Measurements (scratch, not committed)

Fuel per call, wasmi 2.0.3, default costs, `CompilationMode::Eager`, steady state (the same size class was allocated and freed before). Runtime: rtlink 0cf56d3, built as in `oxiscript-wasm/build.rs`. The "front" column is a **scratch experiment, not a proposal**. It adds a 33-entry per-8-byte-class LIFO free list in front of dlmalloc in `WasmHeap::alloc`/`free` (`#[inline]`), sends misses to `#[cold] #[inline(never)]` dlmalloc calls, and never returns memory to dlmalloc.

| Call | Today | Front |
|---|---|---|
| `oxi_alloc` (8 B payload, acyclic) | 1908 | 241 |
| `oxi_retain` (rc 1→2) | 41 | 41 |
| `oxi_is_unique` | 10 | 10 |
| `oxi_release` (rc 2→1, green) | 120 | 115 |
| `oxi_release` (rc 1→0, no children: queue + drain + free) | 1107 | 668 |
| `oxi_alloc_bytes(16)` (drain + calloc) | 1735 | 128 |
| `oxi_free_bytes(16)` (dlmalloc free) | 499 | 45 |
| `oxi_array_new(i32[], 0)` | 1949 | 310 |
| `oxi_array_grow` (cap 0→4) | 1921 | 339 |
| `oxi_array_new(i32[], 3)` (direct sizing) | 3603 | 391 |
| `oxi_release` (i32 array, rc 1→0) | 1578 | 668 |
| `oxi_str_concat(3 B, 2 B)` | 1806 | 1861 (class not warmed) |

Static entry regions (operators charged at function entry, outside any `loop`; no runtime function contains an `if`). The function names were reconstructed from call graphs, because the build has no name section.

| Function | Entry region |
|---|---|
| `calloc` / `malloc` (dlmalloc, inlined) | **1651** |
| `free` (dlmalloc) | 469 |
| `new_object` path | 173 |
| `oxi_retain` | 39 |
| `oxi_is_unique` | 9 |

`calloc` contains a single `memory.fill`.

Reconciliation with BL06:
* `new class`: 1908 + 1107 = 3015 against 3070.
* `[1,2,3]`: 1949 + 1921 + 1578 = 5448 against 5584.
* `` `k${i}` `` (4 allocations and 4 frees): about 11 000 **[estimate, summed from the single-call rows, not measured as one program]**, plus loop work, against 12 006.

## Summary

The report's diagnosis of *our* code is mostly accurate line by line: 42 code rows, most of them CONFIRMED. Its explanation of *why* the fuel is high is wrong, and that matters more:

* **The fuel gap is mainly a metering artifact of dlmalloc's code shape (A1, A2).** wasmi charges a region's whole static operator count on entry. LLVM builds Rust as `block`/`br_if`, so dlmalloc's inlined `malloc` costs about 1650 fuel on every call, whatever path it takes. Calls and missing RC elision are second-order for the BL06 table.
* **A runtime-only change fixes most of the measured gap.** A small, cheap-to-enter fast path in front of the allocator, with cold paths out of line, took `oxi_alloc` from 1908 to 241 fuel in a 30-line scratch experiment.
* **The quick wins the report proposes do little for the measured rows:** zeroing removal, `alloc_uninit`, `concat_n` and direct array sizing (A3, A4).
* **The fuel metric rewards code shape, not work.** 8.Perf's acceptance criterion ("fuel within 1.5x of the stub") can be met by restructuring code without reducing work. Report wall time and an executed-operator count next to fuel, and keep fuel as the deterministic budget it is.
* **Literature numbers are mostly mis-attributed (A8-A10, E-rows):** Lean, Perceus, LXR, BRC, Swift, AssemblyScript.
* **One licence is wrong:** Grain is LGPL-3.0 (A6).
* **The zero-headroom collector design has real holes:** the `log` vector is the dead list, OOM undo is required, and the scratch region can collide with host writes (A7).

## Corrected recommendations (for our constraints)

Constraints: determinism; fuel in wasmi; wasm2c/w2c2 on ARMv6 and ESP32; permissive licences; no UB; the debug `stale_object` check.

1. **Allocator front first, runtime-only, no ABI change.**
   * Put segregated size-class free lists (later 4 KiB slabs) for objects of 256 B or less in `WasmHeap`. Keep the fast path a small function and move dlmalloc (or rlsf/talc) behind `#[cold] #[inline(never)]`.
   * Return empty slabs to the backing allocator, which the scratch experiment does not do.
   * Keep the debug `livemap`/`stale_object` check unchanged: it lives in `Rt`, not in the heap.
   * Deterministic LIFO order. No UB beyond what `WasmHeap` already does with raw pointers.
   * Measure **fuel and wall time** on wasmi and on w2c2 + GCC.
2. **Direct free on the final release of a leaf object.** For an object that is not BUFFERED and has `pcount == 0` (strings, scalar arrays, leaf records), free it directly instead of pushing it on the queue and draining. Objects with children keep the bounded queue, which preserves the frame-time bound (`abi.md:81`).
3. **Shrink the always-charged regions in `oxi_alloc`, `oxi_release` and `drain`.** Outline rare paths: OOM retry, trap paths, the first `td` miss, debug code. A Binaryen pass that rewrites `br_if` diamonds into `if` regions is an **untested hypothesis**: it could cut fuel without touching Rust. Try it after (1) and (2), and only if the stage owner accepts a build-time Binaryen dependency (Apache-2.0).
4. **Fix the two lowering hot spots that do allocate.**
   * Integer formatting straight into a string: use `oxi_str_from_i64` or a `fmt_u` that writes a string, not `[u8]` + `str.from_utf8`.
   * Array literals as one allocation (header + data in one block when the length is known), or at least direct sizing.
   * `concat_n` only for templates with three or more parts.
5. **Codegen inline of `is_unique`, `retain` and non-final `release`** in release builds, with a mode that keeps the calls (`stale_object` check). Small gain per op (A2), but cheap to do.
6. **OxiIR passes** in the report's order: last-use move, borrowed loads, borrow inference, then reuse and drop specialisation, then escape analysis. Add a mandatory RC-balance verifier and Stub-vs-Linked and optimised-vs-unoptimised differential runs. Report "% removed" from our own corpora, not from the literature.
7. **Collector headroom belongs to 8.Mem, not 8.Perf.**
   * Keep the undo log: it is what makes OOM recoverable.
   * Give the collector its own small static work stack instead of `oxi_scratch_ptr`.
   * Evaluate per-slab purple bits once slabs exist.
   * Base the design on BL06's measured 92→248 pages, not on a synthetic example.
8. **Backing allocator: measure, do not pre-rank.**
   * rlsf (MIT/Apache-2.0, 1267 B) and talc (MIT, faster in its own Wasm benchmark, +10-15% memory by default) are both permissive.
   * Choose by fuel entry-region, wall time on ARMv6 and peak pages.
