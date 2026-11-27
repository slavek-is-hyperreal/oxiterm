# BAD ACTOR 8.M-link, round 2

Reviewer: Claude Opus (BAD ACTOR). Target: `gemini-rtlink` at 0477a99 (Gemini's fixes d6b6afd for BL01-BL10, merged with main). I wrote tests and this report only; no product code was changed.

Harness:
- `oxiscript-lower/tests/bad_actor_link2.rs`: programs run against the real runtime through `oxiscript-runtime`.
- `oxiscript-rc/tests/bad_actor_link2_wasm.rs`: the real wasm32 runtime driven directly, including the new `SmallPool` allocator.
- `oxiscript-wasm/tests/bad_actor_link2.rs`: call orders around `oxi_init` on linked modules.

Convention (same as round 1): a test that FAILS today is a finding. Heavy tests are `#[cfg_attr(debug_assertions, ignore = "heavy: ...")]` and run with `--release`.

Environment for every command below:

```
export TMPDIR=/vectorlegis_ssd_pool/oxiterm/scratch/ba-link2 CARGO_BUILD_JOBS=2
```

Repro commands:

```
cargo test --release --offline -p oxiscript-lower --test bad_actor_link2 -- --nocapture
cargo test --release --offline -p oxiscript-rc --test bad_actor_link2_wasm -- --nocapture
cargo test --release --offline -p oxiscript-wasm --features debug-runtime --test bad_actor_link2 -- --nocapture
```

## Findings

### BL2-03 HIGH: the host's housekeeping collect poisons the instance after a successful call

**Where:** `oxiscript-runtime/src/runtime.rs` `maybe_collect_cycles`.

**What happens:** after every `call_export`, `tick` or `pump`, the runtime may run `oxi_collect_cycles` through `invoke_internal`, with a budget of `pump_fuel.max(fuel_per_call)` (30M by default). It runs only if `live_objects * 1000 <= budget`. That guard is not an upper bound on the collector's work, for two reasons:

- **Garbage cycles cost more than 1000 fuel per object.** A program that creates 1000 short-lived 2-cycles per call (2.0M fuel per call, well inside the 10M budget) gets `Err(Trap OutOfFuel)` from `call_export` on call 18. Memory is only 12 pages of 256 at that point. The instance is poisoned.
  - With 3000 cycles per call it fails on call 4.
  - Control: the same calls with `pump_fuel: u64::MAX/4` pass (200 calls).
  - Measured cost of a collect over a purple linked list: 1012-1022 fuel per live object (n = 1000 and 5000) and 893 (n = 9000).
- **The cost tracks edges, not objects.** A purple `N[]` array holding 130,000 references to one node passes the guard: about 3 live objects means about 3,000 fuel by the estimate. Trial deletion then walks every edge three times and runs out of the 30M budget.
  - `grow(10000)` itself used 4,074,150 fuel of its 10M. The OutOfFuel comes after it returned, from the housekeeping collect.
  - Control: unbounded `pump_fuel` passes 50 rounds.

**Consequence:** the most basic cycle workload, a few thousand temporary cycles per event, kills an instance under the default `Limits` within about 20 events. The guest's successful result is lost and the caller sees OutOfFuel for a call that did not run out of fuel.

**Expected:** housekeeping never turns a successful call into an error. Either collect incrementally within a budget (`oxi_gc_step` exists but is stripped from linked modules), or abort the housekeeping run without poisoning (wasmi fuel exhaustion inside `oxi_collect_cycles` leaves the runtime mid-trial, so plain "ignore the error" is not safe). The fuel estimate must also count edges.

**Tests (fail):**
- `default_limits_bursts_of_cycle_garbage_starve_the_host_collector`
- `housekeeping_collect_never_poisons_a_successful_call`
- `collect_cycles_fuel_per_live_object_is_at_most_1000` (supporting data; fails only marginally)

**Controls (pass):**
- `bursts_control_with_unbounded_pump_fuel`
- `housekeeping_control_with_unbounded_pump_fuel`

### BL2-02 MEDIUM: with more than 30,000 live objects background collection is off for good, and the in-call OOM collect then dies of OutOfFuel

**What happens:** `fits_budget` is `live_objects * 1000 <= 30M`. Once a program holds more than 30k objects, the host never collects again. The fixes text says this "defer[s] to OOM recovery (BL02)".

**Reproduction under the default limits:**
- Keep a cache of 40,000 small objects (`fill` in steps of 2000).
- Call `mk()`, which creates one garbage 2-cycle per call.
- Garbage piles up to the 16 MiB limit (256 pages).
- On call 359,601 the in-call OOM collect has to free about 500k objects inside one `mk()` call's 10M fuel, and the instance dies with `OutOfFuel`. It is not `out_of_memory`, and it is not recoverable.

**Control:** with 2,000 live objects the same 400k calls stay flat at 3 pages.

**Expected:** a long-running instance with a moderate live set survives any number of events that each create a little cyclic garbage.

**Tests:**
- `default_limits_40k_live_objects_starve_the_host_collector` (fails)
- `default_limits_small_live_set_collects_between_calls` (control, passes)

### BL2-01 MEDIUM (regression vs da87be6): `SmallPool` free lists never coalesce and never give memory back

**Where:** `oxiscript-rc/src/wasm.rs` `SmallPool`.

**What happens:** a freed block of 8..=256 bytes goes onto its class's free list forever. It cannot be reused by another class or by a block larger than 256 bytes, and slabs are never returned to dlmalloc. The OOM path (drain, collect, retry) cannot help, because nothing it frees reaches dlmalloc.

**Evidence:**
- **Allocator level, raw wasm, no memory limit.** Allocate and free 8 MiB of 16-byte blocks, then allocate 8 MiB of 32-byte blocks:
  - 0477a99: memory grows by +8192 KiB.
  - Pre-fix runtime da87be6, built from `git archive` and run through `OXI_BA_DEV_WASM=<path>`: +0 KiB.
  - Test: `memory_freed_in_one_size_class_is_reused_by_others` (rc). Fails on 0477a99, passes on da87be6.
- **Program, 16 MiB limit:**
  - `smallPhase(500000)` (16-byte objects, all freed) followed by `otherClassPhase(250000)` (32-byte objects) traps `out_of_memory`. Each phase alone fits.
  - Test: `memory_freed_by_one_class_is_reusable_by_another_class`.
- **Program, large string:** the largest string buildable by doubling drops from 2^22 bytes (fresh instance) to 2^21 bytes after freeing 8 MiB of small objects.
  - Test: `memory_freed_by_small_objects_is_reusable_by_a_large_string`.
- **Long-running instance:** rounds with the same ~3.2 MiB live set, but cycling through 16/24/32/40-byte classes, ratchet memory from 68 to 234 of 256 pages. The high-water mark becomes the sum of every class's peak.
  - Test: `alternating_size_phases_do_not_ratchet_memory_to_oom`.

**Expected:** freed memory is reusable by any size. For example, return empty slabs to dlmalloc, or cap the per-class lists, or fall back to flushing the free lists into dlmalloc before the OOM trap.

### BL2-04 LOW: the fuel claim "1.0-1.8x of Stub" holds only for the microbenchmarks

**Reproduced exactly:** `bad_actor_link_perf --include-ignored --release` gives the same table as fixes.md:
- new class 640/398 (1.6x)
- template 2111/1657 (1.3x)
- struct in array 1492/829 (1.8x)
- `work(1000)` 6,605,226 vs 4,155,043 fuel (1.59x)

**Not holding:** a workload with shared references (2-cycles pushed into an array, so releases happen at rc > 1 and go through `release_rc_ne_1`, `buffer_purple` and the roots buffer) costs 3.24x of Stub. Linked-debug costs about 6.1x of Stub (53.1M vs 8.66M over 200 calls).

**Consequence:** about 2000 fuel per garbage 2-cycle limits a call to about 5000 cycles. `burst(20000)` runs out of the 10M fuel by itself, in Linked mode.

**Test:** `fuel_ratio_with_shared_references_is_within_2x_of_stub` (fails: 3.24x).

### BL2-08 MEDIUM (gate red): `bad_actor_8std::array_sort_stable_and_total` fails with OutOfFuel in Linked, passes in Stub

This test came from main through the merge 0477a99. Under the default `Limits`, `call stable` traps `OutOfFuel` in Linked, the default runtime for the lower tests. It fails in both debug and release builds of the test.

With `OXI_RUNTIME=stub` it passes.

This is BL2-04 in a real std workload: a stable sort over class elements fits the 10M fuel per call in Stub and not in Linked. The gate claim "every crate passes" in fixes.md was made before the merge and does not hold on 0477a99.

**Repro:**

```
cargo test --release --offline -p oxiscript-lower --test bad_actor_8std array_sort_stable_and_total
OXI_RUNTIME=stub cargo test --release --offline -p oxiscript-lower --test bad_actor_8std array_sort_stable_and_total
```

The first command FAILS and the second passes.

### BL2-06 LOW (host misuse, BL03 follow-up): allocation before `oxi_init` lands in program static data

The BL03 guard wraps `oxi_rt_init` only. The ABI says `oxi_rt_init` "must run before any other runtime call", but nothing enforces it.

**Case 1: a runtime export called before `oxi_init`.**
- `oxi_alloc_bytes(16)`, `(200)`, `(300)` or `(5000)` called on a linked module before `oxi_init` returns 0x155e8. That is inside the program's static data [0x155e0, 0x15790).
- `WasmSys` seeds its cursor at the runtime's own `__heap_base`, and `SmallPool` carves its slab there.
- `oxi_init` later only raises the cursor, so the slab keeps serving blocks inside static data: 10 of 400 post-init small blocks.
- Static data is corrupted. The program's `lens()` traps `unreachable`, and with 300 bytes even `oxi_init` itself traps.
- Control without the early call: clean, `lens() = 110`.

**Case 2: an early `oxi_rt_init` with a shorter table.**
- `oxi_rt_init(base, count - 1, static_end)` called before `oxi_init` is accepted.
- `RT_INITED` then turns the program's own `oxi_rt_init` into a no-op, so the program runs with the host's table and traps.

**Expected:** runtime exports refuse to run before `oxi_rt_init`, or the allocator floor is a link-time constant. The program's own init should win, or the early call should trap.

**Impact:** `oxiscript-runtime` always calls `oxi_init` first, so this needs a third-party host.

**Tests:**
- `allocating_before_oxi_init_never_overlaps_program_static_data` (wasm, fails)
- `early_rt_init_with_a_different_table_does_not_survive_oxi_init` (wasm, fails)

### BL2-05 INFO (pre-existing, not a regression): the debug runtime cannot see a stale pointer whose block was reused

Freed blocks are reused LIFO: the next same-size allocation returns the block just freed. The `live` bitmap is keyed by address, so a release through the stale pointer passes as live and frees the new object, with no trap 65. The same happens on the da87be6 dlmalloc runtime (`OXI_BA_DEV_WASM`), so this is not new.

**Expected for a debug tool:** quarantine freed blocks, for example a FIFO delay before reuse in the debug build only.

**Test:** `debug_runtime_stale_pointer_after_same_class_reuse` (rc, fails on both runtimes).

### BL2-07 INFO: a program may export a function named `__heap_base` or `__data_end`

The linker strips the runtime's `__heap_base` (`check_layout` treats it as a leak), but `@host export function __heap_base(x: i32): i32` compiles and instantiates. No runtime state is reachable through it. Tools that read `__heap_base` as the heap start get a function instead.

**Test:** `user_export_named_heap_base_is_rejected` (fails).

## Held up (tests pass)

**Allocator correctness (real wasm, release and debug):**
- Byte-allocator fuzz: 40 x 4000 and 15 x 3000 ops, 88,105 and 24,681 allocations. Sizes cover 1..=264, the 248-264 edge, 4088-4104 (slab size) and 65-135 KB.
- Every new block was 8-aligned, zeroed over its whole size and disjoint from every live block, and every live block kept its pattern.
- `byte_allocator_fuzz_release_wasm`, `byte_allocator_fuzz_debug_wasm`.

**Zeroing (ir.md:150):**
- Objects of 5 types reused from dirty free lists read as zero.
- `alloc_bytes(n)` after a free of a dirty block with another exact size in the same class reads as zero.
- Array tails after `array_grow` across 4..70,000 elements read as zero, including the pool-to-dlmalloc crossing at 256 bytes.
- At program level, records and inline structs reused 50 times read as default in Linked and LinkedDebug.
- `objects_reused_from_the_free_list_are_zeroed`, `byte_blocks_reused_across_exact_sizes_are_zeroed`, `reused_blocks_read_as_fresh`.

**Huge and odd sizes:**
- `alloc_bytes` with 0xFFFF_FFF0..0xFFFF_FFFF traps out_of_memory (7).
- `array_new` / `array_grow` with 2^30..2^32-1 elements traps out_of_memory or bounds.
- 0x7FFF_FFF8 bytes and 2^29 x 4 bytes really allocate 2 GiB in raw wasmi (memory covers it, no wrap).
- `huge_sizes_trap_cleanly`.

**Collector under long fuzz:**
- The reference-model fuzzer on the real wasm, 12 seeds x 6000 ops (release: 72k ops, 1421 collects, 1400 cycle-freed, 542 aborted trials) and 6 x 4000 (debug).
- 0 mismatches with the model: no live object freed, no garbage leaked.
- `long_reference_model_fuzz_on_release_wasm`, `long_reference_model_fuzz_on_debug_wasm`.

**In-call OOM collect scaling (no quadratic re-collection):**

| Cycles | Time | Per cycle |
|---:|---:|---:|
| 0.5M | 1.52 s | 3040 ns |
| 1M | 3.34 s | 3340 ns |
| 2M | 8.17 s | 4087 ns |
| 4M | 15.68 s | 3921 ns |

The ratio from 0.5M to 4M is 1.29x. These times were measured beside other tests, so use them for the trend, not as absolute values. Test: `in_call_oom_collect_scales_linearly`.

**The 2M-cycle test `cycles_without_a_host_collect_hit_the_memory_limit`**, release, run alone: completed within the limit, test time 6.63 s and 6.93 s (wall 6.84 s and 7.14 s, max RSS 77 MB). Gemini reported 6.29 s.

**Near OOM:**
- Small objects filled to the 40-page limit (120k), then `out_of_memory` (one exact trap).
- The poisoned instance refuses further calls; a fresh instance computes the right sum.
- `near_oom_small_allocations_after_a_failed_large_one`.

**Determinism:**
- 3 fresh instances per mode x 200 calls: identical values, fuel per call and pages within Stub, within Linked and within LinkedDebug.
- Values identical across the three modes.
- `fuel_and_results_are_deterministic_across_instances_and_modes`.

**Exports:**
- `Runtime::call_export` refuses `oxi_release`, `oxi_retain`, `oxi_free_bytes`, `oxi_alloc_bytes`, `oxi_rt_init`, `oxi_init`, `oxi_collect_cycles`, `oxi_str_from_f32`, `oxi_str_from_f64`, `oxi_array_grow`, `oxi_live_objects`, `oxi_deinit` and `memory`.
- A program cannot export `oxi_str_from_f32`, `oxi_rt_init`, `oxi_gc_step`, `oxi_release` or `oxi_rt_scratch_cap`.
- `runtime_internal_exports_are_not_callable_or_shadowable`.

**Build reproducibility:**
- Two clean release builds into different target directories (the worktree's `target/` and `scratch/ba-link2/repro1`) give byte-identical embedded runtimes: `oxiscript_rc.wasm` 1c93ebdc..., `oxiscript_rc_debug.wasm` dc6d9e9f....
- `hello.oxs` compiled by the two `oxi` binaries is byte-identical: 72,579 B, sha256 687aa8dc....
- No `/home`, `/Users`, `.cargo/registry`, `rustc/<hash>`, user name, `vectorlegis` or `worktrees` strings in the embedded runtimes or in the compiled module. What remains is only `/rust/core/____/...`, `/reg/index.crates.io-.../dlmalloc-0.2.14/src/dlmalloc.rs` and `oxiscript/oxiscript-rc/src/rt.rs`.
- An older `target/` build from before the merge (16:24) differed, because the merge changed `oxiscript-rc/Cargo.toml`. That is not nondeterminism.

## Gate (my run, separate `CARGO_TARGET_DIR`, debug profile, `--no-fail-fast`)

Every crate passes clippy `--all-targets -D warnings`, including `oxiscript-wasm --features debug-runtime` and `oxiscript-playground --features run`.

These pass `cargo test`:
- `oxiscript-syntax`, `-abi`, `-ir`, `-runtime`, `-check`, `-cli`, `-playground`
- `oxiscript-playground --features run`
- `cargo build -p oxiscript-playground --target wasm32-unknown-unknown`

These fail `cargo test`:
- **`oxiscript-rc`:** only my `debug_runtime_stale_pointer_after_same_class_reuse` (BL2-05).
- **`oxiscript-wasm`, with and without `debug-runtime`:** only my two BL2-06 tests.
- **`oxiscript-lower`:**
  - My `fuel_ratio_with_shared_references_is_within_2x_of_stub` (BL2-04) and `user_export_named_heap_base_is_rejected` (BL2-07). My heavy tests are ignored in debug.
  - Pre-existing **`bad_actor_8std::array_sort_stable_and_total`**, verbatim: `panicked at oxiscript/oxiscript-lower/tests/common/mod.rs:264:60: call stable: OXS-E0420: trap OutOfFuel` (BL2-08).
