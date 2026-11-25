# HANDOFF 8.M (oxiscript-rc)

## Done (milestone 1)
- src/heap.rs (Heap trait), src/rt.rs (all algorithms, generic `Rt<H>`), src/sim.rs (native checked dlmalloc arena),
  src/wasm.rs (wasm32 exports, dlmalloc), src/testkit.rs (Guest trait, standard types, reference-model fuzz), tests/fuzz.rs (passes).
- Extension exports (not in abi table): oxi_rt_init, oxi_rt_scratch_cap, oxi_gc_step, oxi_gc_stat, oxi_gc_config.
- Decisions: dlmalloc 0.2.14 (MIT/Apache); internal colours PENDING=5 / ZOMBIE=6 for dead objects; trial has work limit + exact undo;
  stable Rust cannot write wasm globals -> oxi_trap_cell memory cell + scratch static (see lib.rs "Linking model").
- Cargo.lock at workspace root changed (new deps dlmalloc, wasmi dev-dep): mention in report.

## Remaining
1. tests/dbg.rs is a scratch file: DELETE it. Remove FUZZ_DIS / FUZZ_TRACE env hacks from testkit.rs (or keep FUZZ_TRACE).
2. Unit scenario tests (tests/scenarios.rs): 1M list, deep tree, DAG, self/mutual cycles, live-root cycle, closure env cycle, sticky, green never buffered, gc_step abort/undo, OOM, trap kinds, strings/arrays API.
3. wasm build (`cargo build -p oxiscript-rc --target wasm32-unknown-unknown --release`), tests/wasm.rs with wasmi (check every Runtime row's signature vs oxiscript_abi::exports::EXPORTS, rerun fuzz through wasm Guest).
4. benches/measurements (tests/measure.rs ignored test printing numbers); wasm size raw (+ env CARGO_PROFILE_RELEASE_OPT_LEVEL=z LTO), wasm-opt only if present.
5. clippy -D warnings native + `--target wasm32-unknown-unknown`; final report text.

## Verify
`CARGO_BUILD_JOBS=2 cargo test -p oxiscript-rc`; `cargo clippy -p oxiscript-rc --all-targets -- -D warnings`; wasm: `cargo clippy -p oxiscript-rc --target wasm32-unknown-unknown -- -D warnings`.
