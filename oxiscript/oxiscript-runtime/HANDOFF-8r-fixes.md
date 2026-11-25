# 8.R BAD ACTOR fixes (updated after each milestone)

Verify: `CARGO_BUILD_JOBS=2 cargo test -p oxiscript-runtime --release --test bad_actor_8r -- --test-threads=1`
then `cargo test -p oxiscript-runtime` (debug+release), `cargo clippy -p oxiscript-runtime --all-targets -- -D warnings`,
`cargo build -p oxiscript-runtime --target wasm32-unknown-unknown`.

## Plan / decisions
- Namespaces: each Runtime gets an auto namespace (`Host::instantiate_in` to name it); providers get `InstanceCtx{namespace, seed}`.
  `StateStore::set/get` (no ns) = host namespace: read-only fallback layer for guests + notifies watchers. Guest writes go to own ns.
- F4b test is corrected (queue full => trap OXS-E0429, as lead decided; the old test required unbounded growth).
- F8: Host::new = deterministic seed; Host::with_entropy(limits, [u8;16]) mixes entropy + instance counter.
- Reexport check via wasmparser (regular dep, default-features off).

## Status
- [ ] F1 sticky revoke  - [ ] F3 key index + fuel  - [ ] F4 pump budget  - [ ] F4b bounded queues
- [ ] F8 handle seed  - [ ] F9 namespaces/quotas  - [ ] F10 log escape  - [ ] re-export reject
- [ ] full verify, report
