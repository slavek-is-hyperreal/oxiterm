# BAD ACTOR 8.T handoff (final state; update after each finding)
Harness files (all under oxiscript/oxiscript-check/tests/): bad_actor_8t.rs (failing finding tests F01..F19, 38 tests, all currently FAIL = bugs),
bad_actor_8t_probe{,2,3,4}.rs (~1050 candidate programs; print REJ/ACCEPTED), bad_actor_8t_robust.rs (determinism, span/registry sanity, side-table invariants, mutation fuzz),
bad_actor_8t_perf.rs (growth exponents), bad_actor_8t_stack.rs (hostile shapes 1MB stack), bad_actor_8t_fixes.rs (apply suggested fixes mechanically).
Run: CARGO_BUILD_JOBS=2 cargo test --release -p oxiscript-check --test bad_actor_8t  (and others; --nocapture for probes)
## Findings
F01 never fall-through (stmt.rs:1059 skips E0114 for `never`)       F02/F14 uninitialised managed global/static read (module let fwd-read, class static w/o init)
F03 array.length / string.byteLength assignable                      F04 private method satisfies interface
F05 law purity bypass via interface-bound call (note_effect not called for Res::IfaceMethod)   F06 law mutates array arg (low)
F07 polymorphic recursion                                             F08 void/never type args via generics
F09 shift amount type                                                 F10 enum dup values  F11 bool dup case
F12 infinite-size structs                                             F13 `?` in default param
F15 enum member assignable   F16 imported let assignable  F17 private objlit cross-module  F18 local decl capturing outer tparam
F19 quadratic: params / type params / enum members (decl.rs:116, 743, 1213)
Held up: determinism (5 runs x ~900 programs), spans/codes/registry, no ERROR in tables for accepted programs (invariants walker), 1MB stack in debug+release on 55 hostile shapes x3 sizes, perf exponents ~1 elsewhere.
