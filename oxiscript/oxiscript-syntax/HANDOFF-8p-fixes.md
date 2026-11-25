# 8.P BAD ACTOR fixes: progress (update after every milestone)

Verify: `CARGO_BUILD_JOBS=2 cargo test -p oxiscript-syntax` and `... --release`; ignored oracles:
`cargo test -p oxiscript-syntax --release --test bad_actor_8p_gen -- --ignored`; clippy `-D warnings --all-targets`; `cargo fmt -p oxiscript-syntax -- --check`;
wasm: `CARGO_TARGET_WASM32_WASIP1_RUNNER="wasmtime --dir=/" cargo test --target wasm32-wasip1 --release -p oxiscript-syntax --test fuzz`.

## Plan
M1 small fixes: X1 X2 L1 L2 L3 F1 F2 F3 F4 F5 F6 S1 S2 + tests/codes_registered.rs
M2 stack: ParseOptions/max_depth, iterative Drop (Expr/Stmt), iterative formatter chains, frame shrink, optional thread, K1-K4
M3 full verification + report

## Status
(nothing done yet)
