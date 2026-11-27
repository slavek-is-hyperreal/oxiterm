# 8.M-link BAD ACTOR round 2: fixes (`BL2-01` to `BL2-08`)

Fixes for `docs/stages/8.M-link/bad-actor-2.md` (reviewer: Claude Opus), done by Claude Sonnet after the original implementer (Gemini) ran out of credits. Base: `9471355`. No BAD ACTOR test was weakened, deleted or ignored.

## Summary

| Finding | Severity | Status | Proving tests |
|---|---|---|---|
| BL2-03 housekeeping collect poisons the instance | HIGH | fixed | `housekeeping_collect_never_poisons_a_successful_call`, `default_limits_bursts_of_cycle_garbage_starve_the_host_collector`, `fixes_link2::{collector_step_fuel_per_unit_stays_under_the_backstop, live_candidate_graph_too_big_for_a_step_does_not_burn_every_turn, incremental_housekeeping_keeps_up_with_bursts}` |
| BL2-02 collector starvation with large live sets, OOM collect vs call fuel | MEDIUM | fixed, policy documented | `default_limits_40k_live_objects_starve_the_host_collector`, `in_call_oom_collect_scales_linearly`, `cycles_without_a_host_collect_hit_the_memory_limit` |
| BL2-01 size-class memory never reused | MEDIUM | fixed | `memory_freed_in_one_size_class_is_reused_by_others` (rc), `memory_freed_by_one_class_is_reusable_by_another_class`, `memory_freed_by_small_objects_is_reusable_by_a_large_string`, `alternating_size_phases_do_not_ratchet_memory_to_oom` |
| BL2-08 `array_sort_stable_and_total` OutOfFuel in Linked | MEDIUM | fixed (runtime cost) | `bad_actor_8std::array_sort_stable_and_total`, `fixes_link2::bl2_08_stable_sort_fuel_stub_vs_linked` |
| BL2-04 Linked fuel on shared references | LOW | fixed (3.24x to 1.89x) | `fuel_ratio_with_shared_references_is_within_2x_of_stub` |
| BL2-05 debug runtime stale pointer after reuse | INFO | fixed (debug quarantine) | `debug_runtime_stale_pointer_after_same_class_reuse` |
| BL2-06 allocation before `oxi_init` | LOW | fixed | `allocating_before_oxi_init_never_overlaps_program_static_data`, `early_rt_init_with_a_different_table_does_not_survive_oxi_init` |
| BL2-07 user export `__heap_base` / `__data_end` | INFO | fixed | `user_export_named_heap_base_is_rejected` |

## Per finding

### BL2-03 (HIGH) housekeeping collection poisons the instance
- **Cause.** After every turn the host ran the unbounded `oxi_collect_cycles` under a `live_objects * 1000 <= budget` guess. The real cost follows edges, not objects, so a handful of objects with a large reference array (or 1,000 garbage 2-cycles per call) ran the housekeeping call out of fuel and the successful user call was reported as `OutOfFuel` with the instance poisoned.
- **Change.** The linker now exports the runtime's bounded step and counters under the optional ABI names `oxi_collect_step` / `oxi_collect_stat` (`oxiscript-abi` `exports::COLLECT_STEP/STAT`, `oxiscript-wasm/src/link.rs`; `check_layout` still forbids the internal `oxi_gc_*` names; the loader accepts the two names, `call_export` refuses them). `oxiscript-runtime` housekeeping (`Runtime::maybe_collect_cycles`) was rewritten: it reads `oxi_collect_stat`, collects when the candidate buffer reached the threshold, memory grew, or under pressure, and works only through `oxi_collect_step(units)`, bounded in work units (visits, so edges count). Planned cost `GC_FUEL_PER_UNIT = 700` fuel per unit, step fuel limit = 4x the plan as a backstop only (the unit bound ends a step, never fuel, so a trial is never cut off half-way). Budget per turn = `pump_fuel.max(fuel_per_call)`, separate from the guest call; a pass that runs out of budget stops and resumes on the next turn. A step that burns its budget without shrinking the candidate buffer escalates 4x up to `budget / 700` units, then the host backs off 64 turns (doubling to 1,024). New public API: `Runtime::collect_step`, `Runtime::collector_stats`, `CollectStep`, `CollectorStats`, `GC_FUEL_PER_UNIT`. The unbounded `oxi_live_objects`/`oxi_live_bytes` calls (which drain without bound) are no longer used by housekeeping.
- **Measured.** Worst fuel per unit over adversarial shapes (purple list, 50k-edge hub array, wide records, garbage ring, big array drop, 2-cycle garbage), release runtime 560, debug runtime 1,042; plan 700, backstop 2,800. A debug-runtime pass can overshoot its nominal budget by about 1.5x, a release pass cannot.
- **Proof.** `bad_actor_link2`: `housekeeping_collect_never_poisons_a_successful_call` (50 rounds of `grow(10000)` + `touch`, no failure, 73 pages), `default_limits_bursts_of_cycle_garbage_starve_the_host_collector` (burst 1,000 and 3,000 cycles x200, 4 to 5 pages). `fixes_link2`: `collector_step_fuel_per_unit_stays_under_the_backstop`, `live_candidate_graph_too_big_for_a_step_does_not_burn_every_turn` (20k-node purple list, 1,000 calls: 20 steps, 54M total housekeeping fuel = 1.8 budgets), `incremental_housekeeping_keeps_up_with_bursts`.
- **Cost of the rewrite.** Steps cost about 1,970 fuel per collected garbage object (a full `oxi_collect_cycles` of the same garbage costs 1,790), about 3,900 per 2-cycle against about 1,040 for the guest to create it (`burst(1000)` costs 1.04M), all of it housekeeping fuel. `gc_step` now also caps its batch growth after an aborted trial (aborts 901 of 1,201 steps before, 602 of 902 after, total fuel -19%).

### BL2-02 (MEDIUM) large live sets, OOM-path collect vs call fuel
- **Cause.** The same guard switched background collection off for good above 30,000 live objects; the in-call OOM collect (`collect_cycles_inner(false)`) then had to free everything inside one call's fuel.
- **Change.** Housekeeping has no live-object guard any more (above). The in-call recovery (`Rt::oom_collect`, used by `raw_alloc_slow` and `push_root`) is bounded: `gc_step` steps of 2,048 work units growing 4x up to 8,192, until the requested bytes (at least 64 KiB) are back in the heap (or, for `push_root`, the candidate buffer has room: one slot is freed from the partial-sweep gap in O(1)), at most 8,192 units per attempt (4.6M fuel release, 8.5M debug, under the default 10M per call), retried with a doubled target while it keeps freeing memory. A first version with a 32,768-unit cap could spend 18M to 34M fuel and was cut down after review; one 8,192-unit step with no escalation was tried and rejected (an aborted first trial then ended the attempt and the 2M-cycle test trapped `out_of_memory`). **Policy:** a garbage subgraph larger than the attempt budget is not reclaimed in-call and the allocation traps `out_of_memory` (clean) rather than running the call out of fuel; the host's incremental passes and `oxi_collect_cycles` handle such graphs. Documented in `abi.md` 3.1 and 4.
- **Proof.** `default_limits_40k_live_objects_starve_the_host_collector` (400k `mk()` calls with 40k live objects, no failure), `default_limits_small_live_set_collects_between_calls`, `in_call_oom_collect_scales_linearly`, `bad_actor_link::cycles_without_a_host_collect_hit_the_memory_limit` (2M cycles in one call complete), and the new `fixes_link2::bounded_oom_recovery_ends_in_out_of_memory_not_out_of_fuel` (default limits, 652,000 live candidates at 245 pages, then one allocating call: `out_of_memory` after 6.6M fuel, not `OutOfFuel`).

### BL2-01 (MEDIUM) size-class memory never reused
- **Change.** `SmallPool` (`oxiscript-rc/src/wasm.rs`) now uses 4,096-byte, 4,096-aligned slabs with a 24-byte header (free list, `used`, links of the class's partial-slab list). A block finds its slab with `addr & !4095`. Slab storage comes from `dlmalloc` with `SLAB_REQ = 4092` (chunk of exactly 4,096 bytes, so consecutive slabs align back to back; `memalign` fixes the phase otherwise). A slab that becomes completely free is returned to `dlmalloc`, which coalesces; one empty slab per class is kept as a reserve (a phantom `used` count keeps the hot path off the cold edge function; the most recently emptied slab replaces a reserve that got busy). Removed: tail recycling and the near-OOM exact-size fallback (blocks without a slab header). Hot path: one list pop on alloc, one push on free, one extra branch (`slab was full || slab now empty`).
- **Proof.** `memory_freed_in_one_size_class_is_reused_by_others` (rc, +0 KiB for the 32-byte phase), `memory_freed_by_one_class_is_reusable_by_another_class`, `memory_freed_by_small_objects_is_reusable_by_a_large_string` (2^22 after freeing 8 MiB, same as fresh), `alternating_size_phases_do_not_ratchet_memory_to_oom`, plus the 3 allocator fuzzers (88k and 25k allocations) still green.

### BL2-08 (MEDIUM) `array_sort_stable_and_total` OutOfFuel in Linked
- **Classified: runtime cost problem** (Stub needs 4.87M fuel, Linked needed 13.66M, default budget 10M). Not a test-budget problem, so the test was not touched.
- **Root cause (shared with BL2-04).** wasmi charges the fuel of every instruction of a function body, up to the next `loop`/`if`/`else`, **on entry**, whichever branch runs (`wasmi 2.0.3` translator: only `loop`, `if`, `else` and the function start emit a `ConsumeFuel`; plain `block`s share their parent's). The exported `oxi_release` held the record, string and array freeing code in one straight-line body (about 180 instructions), so a shared release (rc > 1) paid for all of it. A retain plus shared release cost 315 fuel in Linked against 90 in Stub.
- **Change.** `Rt::release` is a thin dispatcher (null check, one load, one branch); `release_unique` (rc == 1), `release_string`, the array path and `release_rc_ne_1` are separate non-inlined functions. Also: masked (bounds-check free) indexing of the inline trace stacks and wrapping work counters in the trace loops (about 6% off the collector). `abi.md` 3.1 records the "fuel layout rule" for runtime authors.
- **Numbers.** `stable(1000)`: Stub 4,872,840, Linked 13,662,486 before, **6,720,010** after (1.38x Stub), linked-debug 12.3M. Proof: `bad_actor_8std::array_sort_stable_and_total` (debug and release), `fixes_link2::bl2_08_stable_sort_fuel_stub_vs_linked`.

### BL2-04 (LOW) Linked fuel on shared references
- Same root and fix as BL2-08. Shared-reference workload: 3.24x of Stub before (3.49x with the new allocator), **1.89x** after. `fuel_ratio_with_shared_references_is_within_2x_of_stub` passes. Element read of a struct in an array: 363 to 180 fuel per iteration (Stub 138). Not every workload is under 2x: `array lit` 1.8x, `struct in array` 1.9x, `push only` 2.1x.

### BL2-05 (INFO) debug runtime stale pointer
- Debug runtime only: freed small blocks go through a 64-entry FIFO quarantine, so the next same-size allocation never returns the block just freed and trap 65 fires on the stale release. Large blocks are not quarantined (they would pin memory). Proof: `debug_runtime_stale_pointer_after_same_class_reuse`.

### BL2-06 (LOW) allocation before `oxi_init`
- Case 1: the runtime exports a zero-initialised `oxi_rt_floor` word; the linker patches it (one 4-byte data segment, no overlap with runtime data) to the 8-aligned end of the program's static data and `WasmSys` starts its cursor at `max(__heap_base, floor)`. An allocation before `oxi_init` can no longer land in static data (standalone runtime: floor 0, unchanged; the testkit allocates its type table before `oxi_rt_init` there, so a trap-before-init guard was not an option).
- Case 2: `oxi_rt_init` keeps the latest valid call until the heap is first used, so an early host call cannot make the program run with the host's table; after first use a repeat is the documented no-op.
- Proof: both tests in `oxiscript-wasm/tests/bad_actor_link2.rs` (with and without `--features debug-runtime`). **Test repair (not a weakening):** `allocating_before_oxi_init_never_overlaps_program_static_data` looped over `[false, true]` debug flavours and unwrapped a compile that is rejected by design without `--features debug-runtime`, so it could not pass in the default build whatever the product did. It now loops over the flavours that exist (`cfg!(feature = "debug-runtime")`); every assertion is unchanged.

### BL2-07 (INFO) reserved export names
- `exports::is_reserved_export_name` now also rejects `__oxi_*`, `__heap_base` and `__data_end` (`LINKER_RESERVED`). Proof: `user_export_named_heap_base_is_rejected`. No new reason code.

### BL2-03 supporting test `collect_cycles_fuel_per_live_object_is_at_most_1000`
- 1,012 to 1,022 before; now **958** (n = 1000), 828 (n = 5000, 9000). Its premise (the host guard assumes 1000 fuel per live object) no longer exists, but the test passes as written.

## Before / after (`bad_actor_link_perf --include-ignored --release`, fuel per iteration)

| Operation | Stub | 9471355 (before) | now | change |
|---|---:|---:|---:|---:|
| empty loop | 49 | 49 | 49 | 0% |
| new class | 398 | 640 | **481** | -25% |
| template int | 1,657 | 2,111 | 2,344 | +11% |
| str concat | 331 | 394 | 461 | +17% |
| array push x4 | 917 | 1,302 | 1,359 | +4% |
| array lit | 579 | 964 | 1,021 | +6% |
| struct in array | 829 | 1,492 | 1,596 | +7% |
| struct copy | 391 | 621 | 462 | -26% |
| closure | 730 | 941 | 928 | -1% |
| `work(1000)` total | 4,155,043 | 6,605,226 | **6,446,888** | -2% |
| `work(20000)` total | 90,238,247 | 137,267,546 | 133,700,085 | -3% |

Nothing regresses by more than 17%; the new-class fuel went down, not up. Other measures: `stable(1000)` 13.66M to 6.72M; shared references 3.24x to 1.89x; `collect_cycles` 1,012-1,022 to 958 fuel per live object; the 2M-cycle single call (`cycles_without_a_host_collect_hit_the_memory_limit`, release) 6.3-6.9 s to **about 12 s** (the bounded recovery frees at most 8,192 units per out-of-memory event, so the 16 MiB heap sees many more events; `in_call_oom_collect_scales_linearly` per cycle 3,787 ns at 0.5M to 6,328 ns at 4M = 1.67x, it was 1.29x). This is the price of the fuel bound.

## Gate (worktree `gemini-rtlink`, `CARGO_BUILD_JOBS=2`, per-package cargo)

- `cargo clippy --all-targets -- -D warnings`: clean for `oxiscript-syntax`, `-abi`, `-ir`, `-rc`, `-runtime`, `-check`, `-wasm` (also `--features debug-runtime`), `-lower`, `-cli`, `-playground` (also `--features run`). `cargo build -p oxiscript-playground --target wasm32-unknown-unknown`: ok.
- `cargo test --no-fail-fast` (debug): all pass for `oxiscript-syntax`, `-abi`, `-ir`, `-rc`, `-runtime`, `-check`, `-wasm` (also `--features debug-runtime`), `-cli`, `-playground` (also `--features run`) and every `oxiscript-lower` test binary **except `bad_actor_link` in debug, which did not finish within the 10-minute cap (tried twice, also after the batch-cap change)** (`collect_cycles_needs_headroom_probe` is host-side wasmi in a debug build, about 1G fuel; it takes 2.8 s in release). That binary passes 18/18 in `--release` (9.2 s), as do the heavy ones below.
- Final release run (after the last code change) with `--include-ignored`: lower `bad_actor_link` 18/18, `bad_actor_link2` 17/17, `fixes_link2` 5/5, `bad_actor_link_perf` 5/5, `parity_and_debug_rt`, `fixes_8l`, rc and runtime suites all green; clippy clean for abi, rc, runtime, wasm (both feature sets), lower, playground (both). Earlier release run: `bad_actor_link` 18/18, `bad_actor_link2` 17/17, `fixes_link2` 4/4, `bad_actor_link_perf` 5/5, `bad_actor_link_diff`, `parity_and_debug_rt`, `fixes_8l`, `bad_actor_link_fuzz` (the fuzz test), rc `bad_actor_link2_wasm` 9/9, rc `bad_actor_8m` 25/25, wasm `bad_actor_link_merge` 8/8, wasm `bad_actor_link2` and `fixes_8m_link` with `--features debug-runtime`. `--include-ignored` also runs two intentionally ignored info/manual tests that fail by construction and are not findings: `bad_actor_8std::map_equal_hash_keys_correct` ("info: O(n^2)") and `bad_actor_link_fuzz::link_dump_seed` (needs `BA_SEED`), and the `dump_*` manual tests of `oxiscript-wasm` (`bad_actor_8g_cfg::dump_cfg`, `diff::dump_seed`).

## Not done / known limits
- `bad_actor_link` debug-profile run not completed within 10 minutes (see gate); verified in release.
- BL2-05 (`debug_runtime_stale_pointer...`) is fixed only in the debug runtime, as intended; release has no stale-pointer detection (unchanged).
- A garbage subgraph larger than 8,192 work units cannot be reclaimed *inside* a call under memory pressure (documented policy: `out_of_memory`); a live purple graph larger than a full-budget step (about 42,000 units at the default 30M) is left alone by housekeeping and backed off.
- `GC_FUEL_PER_UNIT` is one number for both runtime flavours; debug-runtime passes can use about 1.5x the nominal housekeeping budget.
- 2M-cycle single-call time is about 75% above the previous 6.3-6.9 s (see above), the scaling ratio 1.67x instead of 1.29x.
- The remaining fuel levers of the layout rule (thin `oxi_alloc`, trace loops of `mark_gray`/`scan`) were not pulled further.
