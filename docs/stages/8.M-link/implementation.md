# HANDOFF 8.M-link (Link OxiScript with the real memory runtime)

Status: DONE — all gate requirements verified, all 7 crates pass tests and `clippy -D warnings`, and `oxiscript-playground` builds cleanly for `wasm32-unknown-unknown`.

## Decisions
1. **Embedding `oxiscript-rc`**: `oxiscript-wasm/build.rs` builds `oxiscript-rc` for `wasm32-unknown-unknown --release` into `OUT_DIR/rc-target` with a clean environment (stripping outer `RUSTC_WRAPPER`/`RUSTC_WORKSPACE_WRAPPER`/`CLIPPY_ARGS`/`CARGO_ENCODED_RUSTFLAGS`) and explicit Core Wasm ABI 1.0 target features (`-nontrapping-fptoint,-reference-types,-simd128,-tail-call,+bulk-memory,+sign-ext,+multivalue`), and embeds it with `include_bytes!(concat!(env!("OUT_DIR"), "/oxiscript_rc.wasm"))`.
   - Justification: no checked-in binary blob that can drift from `oxiscript-rc`, automatic incremental rebuild via `cargo:rerun-if-changed`, ~1.4s cold / ~0.05s cached build, and works identically when cross-compiling `oxiscript-playground` to `wasm32-unknown-unknown` (build scripts execute on the host).
2. **Core Wasm Merger (`oxiscript-wasm/src/link.rs`)**: Pure Rust on `wasmparser 0.263` + `wasm-encoder 0.263` (`wasm_encoder::reencode::Reencode`, already in `Cargo.lock`, zero new dependencies). Runs identically natively and in the `wasm32` browser playground.
   - Runtime (`oxiscript-rc`) keeps its low linear memory `[0, __heap_base)` (64 KiB shadow stack, `.rodata`/`.data`/`.bss`, `oxi_scratch_ptr`) and function-table slots `[0, rt_table_len)`.
   - Compiler lays out program static data starting at `align16(__heap_base)` and closure table indices starting at `rt_table_len` — zero address or table-index relocation required in either module.
   - Normalizes element segments to Core Wasm 1.0 active format (`flags = 0`), strips non-ABI runtime exports (`__data_end`, `__heap_base`, `oxi_rt_scratch_cap`, `oxi_gc_*`), resolves `oxi:rt` imports bidirectionally (`P -> R` runtime functions and `R -> P` `oxi_set_trap`), leaving zero `oxi:rt` imports, one memory, one table, one set of globals. Passes strict ABI 1.0 `validate()` (no `reference-types`, no `trunc_sat`, no SIMD, no tail-call) and `oxiscript-runtime` strict load checks.
3. **Heap ownership (8.G conflict 2)**: Promoted `oxi_rt_init(typedesc_base, typedesc_count, heap_base)` into `oxiscript_abi::exports::EXPORTS` (appended at end; additive) and `docs/oxiscript/abi.md`. `oxiscript-rc` uses a custom `dlmalloc::Allocator` (`WasmSys`, 4 KiB chunks) seeded with `heap_base` (and validated `>= __heap_base`, 8-aligned, `<= mem_size`; `0` keeps default `__heap_base`), reusing remaining space in initial pages before calling `memory.grow`. `oxiscript-rc/build.rs` passes `--export=__heap_base`.
4. **One trap channel (8.G conflict 1)**: Removed `oxi_trap_cell` from `oxiscript-rc`; `oxiscript-rc` imports `("oxi:rt", "oxi_set_trap")(kind: i32)`, which the linker resolves to a compiler-generated 2-instruction helper writing the single exported mutable global `oxi_trap_kind`.
5. **Cycle collection on `deinit`**: `oxiscript-runtime`'s `Runtime::deinit()` calls `oxi_deinit()`, then `oxi_collect_cycles()`, then `oxi_live_objects()`, so leak checks reclaim unreachable cycles while still reporting true leaks / host-held references.
6. **RuntimeMode**: `RuntimeMode::Linked` is the default in `oxiscript-lower::Options` (used by `oxi run`, `oxi build`, `oxiscript-playground`, and `oxiscript-lower` program tests). `Stub` remains default in `oxiscript-wasm::CompileOptions` for fast compiler unit/diff tests; `Import` is kept for compiler-only golden `.wat` dumps and `raw.oxir`.
7. **`f64` formatting**: Documented in `docs/oxiscript/abi.md` section 3.1 (shortest round-trip decimal via Rust `core::fmt::Debug` with trailing `.0` stripped for integral values: `1.0` -> `"1"`, `-0.0` -> `"-0"`, `0.1` -> `"0.1"`, `1e21` -> `"1e21"`, `"NaN"`, `"Infinity"`, `"-Infinity"`).

## Test Results
- `oxiscript-abi`: **46 passed**, 0 failed (23 unit + 23 `bad_actor_8c`)
- `oxiscript-rc`: **62 passed**, 0 failed, 2 ignored (`measure`) (2 `abi_consts` + 25 `bad_actor_8m` + 3 `fuzz` + 6 `oom_and_codes` + 17 `scenarios` + 9 `wasm`)
- `oxiscript-runtime`: **64 passed**, 0 failed, 1 ignored (`reload_does_not_leak`) (2 unit + 17 `bad_actor_8r` + 2 `fuzz` + 43 `runtime`)
- `oxiscript-wasm`: **98 passed**, 0 failed, 2 ignored (`dump_cfg`, `diff` long) (6 `adversarial` + 41 `bad_actor_8g_*` + 5 `corpus` incl. Linked & runtime traps + 1 `diff` + 1 `golden` + 15 `programs` + 27 `programs2` + 2 `programs_lower_ops`)
- `oxiscript-lower` (all running **Linked**): **208 passed**, 0 failed, 1 ignored (`fuzz_long_seeded`) (102 `bad_actor_8l` incl. un-ignored closure cycles `cl_recursion_via_closure_var` & `cl_class_field_closure_capturing_self` + 1 `bad_actor_8l_fuzz` + 2 `bad_actor_8l_ir` + 1 `bad_actor_8l_num` + 14 `bad_actor_8l_robust` + 1 `bad_actor_8l_str` + 1 `corpus` + 15 `fixes_8l` incl. 100k deep list drop + 6 `host_coverage_8l` + 6 `host_export` + 4 `linked_runtime` + 2 `misc` + 12 `programs_hard` + 7 `programs_host` + 12 `programs_managed` + 10 `programs_scalar` + 10 `programs_types` + 2 `slice`)
- `oxiscript-cli`: **54 passed**, 0 failed (1 unit + 18 `bad_actor_8d` + 5 `bad_actor_run` + 26 `cli` + 4 `semicolons`)
- `oxiscript-playground`: **23 passed**, 0 failed, 1 ignored (`mutation_fuzz_no_panic`) (7 unit + 2 `bad_actor_8w` + 3 `bad_actor_run` + 8 `run_examples` incl. `f64_to_text_works` + 2 `run_fixes` + 1 `site_examples`)
- `cargo clippy --all-targets -- -D warnings`: clean on all 7 crates + `oxiscript-rc --target wasm32-unknown-unknown --lib`.
- `cargo build -p oxiscript-playground --target wasm32-unknown-unknown`: green.

## Module Size (`sites/oxiscript/examples/hello.oxs`)
- `RuntimeMode::Stub`: **3,554 bytes**
- `RuntimeMode::Linked`: **58,339 bytes** (single self-contained Core Wasm module including `dlmalloc`, deferred destruction, Bacon-Rajan cycle collector, and `f64` round-trip formatting)

## Verify
```
cargo test -p oxiscript-abi
cargo test -p oxiscript-rc
cargo test -p oxiscript-runtime
cargo test -p oxiscript-wasm
cargo test -p oxiscript-lower
cargo test -p oxiscript-cli
cargo test -p oxiscript-playground
cargo build -p oxiscript-playground --target wasm32-unknown-unknown
```
