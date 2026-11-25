# HANDOFF 8.M (oxiscript-rc)

## State: implementation + tests done; only review/report left
Files: src/{lib,heap,rt,sim,wasm,testkit}.rs; tests/{fuzz,scenarios,wasm,measure,abi_consts}.rs, tests/common/mod.rs.
All green: `cargo test -p oxiscript-rc` (fuzz native 1650 programs, scenarios 17, wasm 6 incl. debug-poison build, abi constants),
clippy -D warnings native (--all-targets) and wasm32 lib: clean. No contract files touched (oxiscript-abi, docs untouched).
Cargo.lock at workspace root changed (dlmalloc, wasmi dev-dep and transitive).

Measurements (tests/measure.rs, `--release --ignored --nocapture`): see report. wasm: 59.9 KB default release; 48 KB opt-z+LTO+strip;
18.8 KB without float printing (`--no-default-features`); wasm-opt not installed.

## Remaining
- Advisor review fixes (if any), then write the final report (SubagentHandback).

## Key decisions
dlmalloc 0.2.14; Rt generic over Heap; extension exports oxi_rt_init/oxi_rt_scratch_cap/oxi_gc_step/oxi_gc_stat/oxi_gc_config kept in
EXTENSION_EXPORTS (lib.rs), not in abi EXPORTS; internal colours PENDING=5, ZOMBIE=6; zombie accounting subtracted from live_*;
trial has work limit + exact undo; stable Rust cannot write wasm globals (oxi_trap_cell, scratch static, oxi_rt_scratch_cap).
