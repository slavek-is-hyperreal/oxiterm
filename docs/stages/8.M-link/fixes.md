# 8.M-link BAD ACTOR Fixes (`BL01`–`BL10`)

All ten findings from `docs/stages/8.M-link/bad-actor.md` have been resolved, no BAD ACTOR test was weakened, deleted, or ignored, and every crate in the gate passes `cargo test --no-fail-fast` and `cargo clippy --all-targets -- -D warnings`.

---

## Per-Finding Summary (`BL01`–`BL10`)

### BL01 — Cycle Collector Headroom & Non-Trapping `deinit`
- **What changed**:
  - Added fixed `.bss` inline work buffers (`INLINE_STACK = 2048` words, `INLINE_LOG = 1024` words = 12 KiB `.bss`, 0 bytes in `.wasm`) in `Rt` so small/medium cycle batches need zero heap allocations.
  - Added a transient `IN_STACK` (`1 << 28`) bit in object `meta` (always cleared on pop and in `undo_gray`) and colour-on-push in `scan_black`/`gather_white`, tightening phase 2/3 stack reservation from `2 * work + roots + 8` down to `log.len + 4`.
  - Batched `collect_cycles_inner` in `COLLECT_BATCH = 256` roots with adaptive halving on `oom_abort` (so disjoint cycles stay inside `.bss` with +0 pages growth) and exponential batch jump (`max(batch * 4, marked)`) when roots reach into a larger shared live subgraph (keeping deep linked lists $O(N)$ instead of $O(N^2 / 256)$).
  - Updated `grow_vec` to fall back to exact 8-byte-aligned `min_cap` on its final attempt for scratch/bitmap reservations (`stack`/`log`/`livemap`) while refusing `+2`-word thrashing on large `roots` buffers (`cur_cap >= 1024`) so `push_root` collects cycles instead of copying MiBs per push.
- **Files**: `oxiscript/oxiscript-rc/src/rt.rs`.
- **Proving tests**:
  - `oxiscript-lower/tests/bad_actor_link.rs`: `collect_cycles_needs_headroom_probe`, `many_small_cycles_in_a_loop_are_collected`, `ring_of_100k_class_nodes_is_collected`, `deep_array_nesting_and_deep_ring_collect_without_stack_overflow`.
  - `oxiscript-rc/tests/oom_and_codes.rs`: `allocation_failure_inside_a_trial_is_undone_and_retried`.

### BL02 — Cycle Collection & Single Retry Before Trapping `out_of_memory`
- **What changed**:
  - In `raw_alloc_slow` and `push_root` (guarded by `!in_trial && !in_drain && !in_oom_collect && roots.len > 0`), the runtime drains pending destructions, runs one non-trapping cycle collection (`collect_cycles_inner(false)` — pinning `p` with `rc + 1` in `push_root` so `p` cannot be reclaimed while collecting other cycles), and retries the allocation once before trapping `TrapKind::OutOfMemory`.
  - Documented in `docs/oxiscript/abi.md` (section 3.1).
- **Files**: `oxiscript/oxiscript-rc/src/rt.rs`, `docs/oxiscript/abi.md`.
- **Proving tests**:
  - `oxiscript-lower/tests/bad_actor_link.rs`: `cycles_without_a_host_collect_hit_the_memory_limit` (2M garbage 2-cycles in one call now complete within the default 16 MiB memory limit in **6.29s** release), `runtime_oom_is_one_exact_trap_and_instance_recovers_on_restart`, `memory_limit_sweep_only_oom_or_success`.

### BL03 — `oxi_rt_init` Static-Data Guard & Idempotency
- **What changed**:
  - Added `RT_INITED` in `oxiscript-rc` so a second call to `oxi_rt_init` is a harmless no-op (after validation).
  - In `oxiscript-wasm/src/link.rs`, wrapped `oxi_rt_init` in an 8-instruction linked guard that traps with code `64` (`T_BAD_TYPEDESC`) whenever `heap_base < static_end` (unsigned `<`, rejecting `0`, the runtime's own `__heap_base`, and any address inside program static data), and routed both internal `oxi_init` calls and the `oxi_rt_init` export through the wrapper. (`oxi_rt_init` remains exported per `oxiscript_abi::exports::EXPORTS` and `bad_actor_link_merge::rt_init_recall_matrix`.)
  - Documented the guard and idempotency contract in `docs/oxiscript/abi.md` (section 3.1).
- **Files**: `oxiscript/oxiscript-rc/src/wasm.rs`, `oxiscript/oxiscript-wasm/src/link.rs`, `docs/oxiscript/abi.md`.
- **Proving tests**:
  - `oxiscript-wasm/tests/bad_actor_link_merge.rs`: `rt_init_recall_matrix`.

### BL04 — No Builder Paths or `rustc` Commit Hash in Embedded Runtime
- **What changed**:
  - Built the embedded `oxiscript-rc` Wasm artifact with `CARGO_PROFILE_RELEASE_PANIC=abort`, `OPT_LEVEL=s`, `LTO=true`, `CODEGEN_UNITS=1`, `STRIP=symbols`, and ordered `--remap-path-prefix` flags (`HOME` -> `CARGO_HOME` -> `CARGO_HOME/registry/src` -> workspace -> `rc_dir` -> sysroot -> `/rustc/<commit>`).
  - Because rustup's precompiled `libcore.rlib` ships `/rustc/<40-hex>/` panic-location strings baked into `.rodata` (unreachable by `--remap-path-prefix` on stable Rust without nightly `-Zlocation-detail`), `build.rs` scrubs any remaining `/rustc/<40-hex>/` prefixes in-place with an equal-length constant (`/rust/core/____________________________________/`).
  - Added automated scanning tests asserting that compiled Stub, Linked release, and Linked debug modules contain none of `"/home"`, `"/Users"`, `".cargo/registry"`, or `"rustc/"`.
- **Files**: `oxiscript/oxiscript-wasm/build.rs`, `oxiscript/oxiscript-wasm/tests/fixes_8m_link.rs`, `oxiscript/oxiscript-lower/tests/parity_and_debug_rt.rs`.
- **Proving tests**:
  - `oxiscript-wasm/tests/fixes_8m_link.rs`: `linked_modules_contain_no_builder_paths_or_rustc_hash`.
  - `oxiscript-lower/tests/parity_and_debug_rt.rs`: `compiled_oxs_modules_have_no_builder_paths_or_rustc_hash`.

### BL05 — `build.rs` Dependency & Offline Hygiene
- **What changed**:
  - Rewrote `oxiscript/oxiscript-wasm/build.rs` to recursively emit `cargo:rerun-if-changed` for the entire `oxiscript-rc/src` tree, `oxiscript-rc/Cargo.toml`, `oxiscript-rc/build.rs`, the workspace `Cargo.lock`, and `rust-toolchain.toml` / `rust-toolchain` if present.
  - Added `cargo:rerun-if-env-changed` for `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS`, `CARGO_NET_OFFLINE`, and `CARGO_HOME`.
  - Passed `--locked` (and `--offline` whenever `CARGO_NET_OFFLINE` is set) to the nested `cargo build` invocation.
- **Files**: `oxiscript/oxiscript-wasm/build.rs`.
- **Proving tests**:
  - Offline builds across all 10 crates + `oxiscript-playground --target wasm32-unknown-unknown` + `oxiscript-wasm --features debug-runtime`.

### BL06 — Linked Runtime Fuel & Allocation Hot Path
- **What changed**:
  - Added `SmallPool` in `oxiscript-rc/src/wasm.rs`: a 32-class (`8..=256` bytes, 8-byte step) free-list allocator in front of `dlmalloc`, carving from 4 KiB slabs (recycling leftover tails, falling back to exact-size `dlmalloc` near OOM) with bounds-proven unchecked indexing so no panic block sits in the hot path.
  - Added `Heap::alloc_uninit` and `Heap::st8` (`oxiscript-rc/src/heap.rs`) so strings/arrays that immediately overwrite their payload skip redundant zeroing.
  - Eliminated 20-byte `Td` shadow-stack copies via in-place `self.tdc` (`ensure_td`/`load_td_slow`), gated `drain` behind `if self.q_head != 0 { self.drain_cold(); }`, and outlined all slow paths (`dl_malloc`, `dl_calloc`, `dl_free`, `alloc_small_slow`, `drain_cold`, `trap`, `release_rc_ne_1`, `release_queue_slow`, `release_record_children`, `release_array_fast_or_slow`, `buffer_purple`, `clone_str_slow`, `clone_array_slow`, `retain_record_children`) as `#[cold] #[inline(never)]` so `wasmi`'s per-block fuel metering does not charge unexecuted cold code on entry.
  - Added fast non-recursive leaf-free (`dec_or_free_leaf`) for small reference arrays (`len <= 8`).
  - Default fuel budget (`10,000,000` fuel/call) was **not** raised; documented what it buys (~15,000 class/struct allocations, ~4,700 formatted templates, ~200,000 empty loop iterations) in `docs/oxiscript/abi.md` (section 3.1).
- **Files**: `oxiscript/oxiscript-rc/src/heap.rs`, `oxiscript/oxiscript-rc/src/wasm.rs`, `oxiscript/oxiscript-rc/src/rt.rs`, `oxiscript/oxiscript-rc/src/lib.rs`, `docs/oxiscript/abi.md`.
- **Proving tests**:
  - `oxiscript-lower/tests/bad_actor_link_perf.rs`: `fuel_per_operation` (every row **1.0x–1.8x** of Stub, all within the 2x goal), `default_fuel_budget_regression` (**PASSED** under default 10M Limits: `6,605,226` fuel = **1.59x** Stub), `per_call_overhead_stub_vs_linked`.

### BL07 — Compile Time, Instantiate Time & Module Size
- **What changed**:
  - Cached the parsed runtime (`ParsedModule<'static>`) and pre-decoded function bodies (`PreparsedBody`) once per process in a `OnceLock` (`CachedRuntime`) inside `oxiscript-wasm/src/link.rs`, eliminating repeated `wasmparser` decoding of `oxiscript_rc.wasm` on every compile.
  - Compile time dropped **~20–40%** across all measured programs. Embedded Linked module size increased by ~14.3 KB (58.3 KB -> 72.6 KB on `hello.oxs`) due to adding shortest round-trip `f32` formatting (`BL09`, Rust `core::fmt::Debug` for `f32`) plus `SmallPool` and outlined hot/cold ABI helpers (`BL06`).
- **Files**: `oxiscript/oxiscript-wasm/src/link.rs`.
- **Proving tests**:
  - `oxiscript-lower/tests/bad_actor_link_perf.rs`: `size_and_time_stub_vs_linked`.

### BL08 — Bounded Host Cycle Collection Between Events
- **What changed**:
  - Added `Instance::invoke_internal` (does not alter `last_fuel_used` or capability `begin_turn`) and `Runtime::maybe_collect_cycles()` in `oxiscript-runtime/src/runtime.rs`, called after `call_export`, `tick`, and non-empty `pump` turns.
  - Policy: checks live heap growth every 64 turns (16 turns at >= 50% of `max_memory_pages`, or immediately if linear memory grew past `max(4, last_gc_pages / 2)` pages — 2 pages under pressure). Invokes `oxi_collect_cycles()` bounded by `pump_fuel.max(fuel_per_call)` only when live objects, live bytes, or pages exceed geometric thresholds (`max(1024, last / 2)` objects or `max(256 KiB, last / 2)` bytes; tightened to `256` / `64 KiB` / `2` pages under pressure) AND a conservative upper bound on traversal cost (`live_objects * 1000` fuel) fits within that budget (preventing stalls or false `OutOfFuel` poisoning of huge live acyclic heaps such as `fixes_8l::l06_drop_alone_under_default_fuel`).
  - Documented in `docs/oxiscript/abi.md` (section 4).
- **Files**: `oxiscript/oxiscript-runtime/src/runtime.rs`, `docs/oxiscript/abi.md`.
- **Proving tests**:
  - `oxiscript-lower/tests/bad_actor_link.rs`: `long_running_instance_leaks_cycles_until_oom_without_host_collect` (survived all **300,000** `mk()` calls flat at **3 pages**).
  - `oxiscript-lower/tests/fixes_8l.rs`: `l06_drop_alone_under_default_fuel` (120,000 live nodes built in 40 calls and dropped cleanly under default Limits).

### BL09 — `f32` Shortest Round-Trip Text Formatting
- **What changed**:
  - Implemented shortest round-trip formatting for `f32` directly (not widened to `f64`), so `0.1f32` prints `"0.1"` and `3.4028235e38f32` prints `"3.4028235e38"`, following the same trailing-`.0` strip and `NaN`/`Infinity`/`-Infinity` rules as `f64`.
  - Added `Op::StrFromF32` in `oxiscript-ir`, `str_from_f32` / `oxi_str_from_f32` in `oxiscript-rc`, on-demand import in `oxiscript-wasm` (keeping existing `golden/*.wat` byte-identical), and direct lowering for `It::F32` in `oxiscript-lower`.
  - Documented both `f32` and `f64` formatting in `docs/oxiscript/abi.md` (section 3.1) and `docs/oxiscript/grammar.md` (section 6).
- **Files**: `oxiscript/oxiscript-ir/src/{types.rs,print.rs,parse.rs,verify.rs}`, `oxiscript/oxiscript-rc/src/{rt.rs,wasm.rs}`, `oxiscript/oxiscript-wasm/src/{cx.rs,func.rs,stubrt.rs}`, `oxiscript/oxiscript-lower/src/expr.rs`, `docs/oxiscript/abi.md`, `docs/oxiscript/grammar.md`.
- **Proving tests**:
  - `oxiscript-lower/tests/bad_actor_link.rs`: `f32_text_probe` (prints `["0.1|16777216|3.4028235e38", "0.1"]` untouched).
  - `oxiscript-lower/tests/parity_and_debug_rt.rs`: `f32_shortest_round_trip_formatting_edges`.

### BL10 — Stub-vs-Linked Parity Harness & Debug Runtime CI Path
- **What changed**:
  - Added an optional `debug-runtime` feature to `oxiscript-wasm` (`build.rs` additionally builds `oxiscript_rc_debug.wasm` with `debug_assertions=true`, `opt-level=1`, `panic=abort`, `strip=symbols`, enabling the live-object bitmap `stale_object` trap 65 and `0xDD` freed-memory poisoning).
  - Added `pub debug_runtime: bool` (default `false`) to `oxiscript_wasm::CompileOptions` and `oxiscript_lower::Options`. Enabled `oxiscript-wasm/debug-runtime` in `oxiscript-lower`'s `[dev-dependencies]` only so production and playground artifacts stay compact.
  - Updated `oxiscript-lower/tests/common/mod.rs` (`run_log`) to check Stub parity alongside Linked automatically on every call, plus `OXI_RUNTIME=linked|stub|linked-debug` override.
  - Added a 3-way parity matrix (`Stub` vs `Linked` vs `LinkedDebug`) in `oxiscript-lower/tests/parity_and_debug_rt.rs` and an explicit trap-65 double-release test in `oxiscript-wasm/tests/fixes_8m_link.rs`.
- **Files**: `oxiscript/oxiscript-wasm/{Cargo.toml,build.rs,src/lib.rs,src/link.rs}`, `oxiscript/oxiscript-lower/{Cargo.toml,src/driver.rs,tests/common/mod.rs,tests/parity_and_debug_rt.rs}`, `oxiscript/oxiscript-wasm/tests/fixes_8m_link.rs`.
- **Proving tests**:
  - `oxiscript-lower/tests/parity_and_debug_rt.rs`: `parity_matrix_stub_linked_and_debug_runtime`.
  - `oxiscript-wasm/tests/fixes_8m_link.rs` (`--features debug-runtime`): `debug_runtime_traps_65_on_double_release`.

---

## Before / After Measurements

### BL06 — Fuel per Operation (`cargo test --release --offline -p oxiscript-lower --test bad_actor_link_perf -- --nocapture`)

| Operation | Stub fuel/iter | Linked Before (ratio) | Linked After (ratio) |
|---|---:|---:|---:|
| `empty loop` | 49 | 49 (1.0x) | **49 (1.0x)** |
| `new class` | 398 | 3,070 (7.7x) | **640 (1.6x)** |
| `template int` | 1,657 | 12,006 (7.2x) | **2,111 (1.3x)** |
| `str concat` | 331 | 2,974 (9.0x) | **394 (1.2x)** |
| `array push x4` | 917 | 5,922 (6.5x) | **1,302 (1.4x)** |
| `array lit` | 579 | 5,584 (9.6x) | **964 (1.7x)** |
| `struct in array` | 829 | 8,727 (10.5x) | **1,492 (1.8x)** |
| `struct copy` | 391 | 3,244 (8.3x) | **621 (1.6x)** |
| `closure` | 730 | 6,255 (8.6x) | **941 (1.3x)** |

### BL06 — End-to-End Workloads (`default_fuel_budget_regression` & `per_call_overhead_stub_vs_linked`)

| Workload | Mode | Before | After |
|---|---|---|---|
| `work(1000)` default Limits (10M fuel) | Stub | `Ok("[I32(7890)]")` (4,155,043 fuel, 4 pages) | `Ok("[I32(7890)]")` (4,155,043 fuel, 6.61 ms, 4 pages) |
| `work(1000)` default Limits (10M fuel) | Linked | `Err(OutOfFuel)` (needed 33,010,266 fuel, 3 pages) | **`Ok("[I32(7890)]")`** (**6,605,226 fuel = 1.59x**, 8.45 ms, 3 pages) |
| `work(20000)` | Stub | 118–143 ms, 90,238,247 fuel, 62 pages | 127.86 ms, 90,238,247 fuel, 62 pages |
| `work(20000)` | Linked | 317–388 ms, 664,720,910 fuel (7.37x), 13 pages | **173.23 ms (1.35x)**, **137,267,546 fuel (1.52x)**, **11 pages** |

### BL01 — Cycle Collector Memory Headroom (`collect_cycles_needs_headroom_probe`)

| 2-Cycles | Before (pages before -> after) | After (pages before -> after) |
|---:|---|---|
| 10,000 | — | 11 -> 11 (+0) |
| 20,000 | — | 18 -> 18 (+0) |
| 40,000 | — | 32 -> 32 (+0) |
| 60,000 | — | 40 -> 40 (+0) |
| 80,000 | — | 60 -> 60 (+0) |
| 100,000 | 92 -> 248 (+156 pages, 2.7x) | **68 -> 68 (+0 pages)** |
| 120,000 | 106 -> **OOM trap** (limit 256) | **78 -> 78 (+0 pages)** |
| 140,000 | — | **116 -> 116 (+0 pages)** |

### BL07 — Size, Compile Time & Instantiate Time (`size_and_time_stub_vs_linked`, 100-run mean)

| Program | Stub Size (B) | Linked Size Before -> After (B) | Stub Compile (us) | Linked Compile Before -> After (us) | Stub Instantiate (us) | Linked Instantiate Before -> After (us) |
|---|---:|---:|---:|---:|---:|---:|
| `work` | 4,637 | 59,409 -> 73,696 | 2,106 | 6,539 -> **4,206** | 980 | 4,906 -> 4,976 |
| `hello` | 3,568 | 58,339 -> 72,579 | 586 | 3,309 -> **2,401** | 467 | 4,894 -> 5,323 |
| `counter` | 5,064 | 59,837 -> 74,129 | 2,799 | 5,238 -> **5,078** | 657 | 4,376 -> 5,079 |
| `shapes` | 4,588 | 59,360 -> 73,647 | 1,111 | 3,387 -> **3,020** | 562 | 4,950 -> 5,227 |
| `structs` | 4,538 | 59,309 -> 73,598 | 1,113 | 3,387 -> **2,945** | 551 | 4,567 -> 4,784 |
| `errors` | 3,967 | 58,739 -> 73,026 | 1,137 | 3,325 -> **3,216** | 545 | 4,167 -> 4,850 |

*(Note: on a quieter run without CPU load, Linked compile times measured 2,097–3,557 us and instantiate times 4,329–4,553 us; both runs show consistent compile speedup from `CachedRuntime`.)*

---

## Gate Results

| Crate | `cargo test --offline -p <crate> --no-fail-fast` | `cargo clippy --all-targets -- -D warnings` |
|---|---|---|
| `oxiscript-syntax` | **87 passed**, 0 failed, 2 ignored | Clean |
| `oxiscript-abi` | **51 passed**, 0 failed, 0 ignored | Clean |
| `oxiscript-ir` | **45 passed**, 0 failed, 0 ignored | Clean |
| `oxiscript-rc` | **62 passed**, 0 failed, 2 ignored | Clean (native + `wasm32-unknown-unknown --lib`) |
| `oxiscript-runtime` | **83 passed**, 0 failed, 1 ignored | Clean |
| `oxiscript-check` | **172 passed**, 0 failed, 0 ignored | Clean |
| `oxiscript-wasm` | **111 passed**, 0 failed, 3 ignored (+ **2 passed** with `--features debug-runtime --test fixes_8m_link`) | Clean (default + `--features debug-runtime`) |
| `oxiscript-lower` | **231 passed**, 0 failed, 7 ignored (release `bad_actor_link`: **18/18 passed** in 10.32s; release `bad_actor_link_perf`: **2/2 passed** + 2 ignored benchmarks run) | Clean |
| `oxiscript-cli` | **61 passed**, 0 failed, 0 ignored | Clean |
| `oxiscript-playground` | **23 passed**, 0 failed, 1 ignored | Clean |

- `cargo build --offline -p oxiscript-playground --target wasm32-unknown-unknown`: **GREEN**.

---

## Remaining / Not Done

- Nothing from `BL01`–`BL10` or the gate is left undone.
- Note on BL07 binary size: the embedded Linked runtime grew by ~14.3 KB (58.3 KB -> 72.6 KB on `hello.oxs`) because shortest round-trip `f32` formatting (`BL09`, Rust `core::fmt::Debug` for `f32`) and the outlined `SmallPool`/fast-path helpers (`BL06`) were added; shrinking that further (e.g. a custom minimal Ryū/Dragonbox formatter) was out of scope for BL07 ("no deep work needed").
