# BAD ACTOR 8.G handoff (running notes)
Scratchpad: /tmp/claude-1000/-home-slavekm--claude-spares-spare-iat75D/146c009c-1a7f-4423-b76c-7e3d46b17f61/scratchpad/badactor-8g/
Tests: tests/bad_actor_8g_*.rs (run: CARGO_BUILD_JOBS=2 cargo test -p oxiscript-wasm --release --test <name>)

## Findings
F1 (determinism, low-med) FLOAT RETURNS NOT CANONICALISED: export wrappers / direct exports / cb: trampolines return f32/f64 raw.
  ir.md s6: a float handed to the host is canonicalised. Repro: `export func @id(%a: f32) -> f32 { e: ret %a }` called with bits 0x7fc00001 returns 0x7fc00001.
  Root cause: src/wrap.rs step 4 (user exports: inner called directly / wrapper returns raw), step 5 (cb trampolines) -- no push_canon_float on results; func.rs push_host_arg does it for import args only.
  Test: tests/bad_actor_8g_nan.rs::host_nan_payload_does_not_round_trip (fails). Fix: wrap float-returning exports + trampolines, canonicalise result.

## Read so far (nothing wrong): arith/trap/conv/shift/divrem/sat, cfg.rs structuring, helpers f2i/mul64 (by reading)
## TODO: random arbitrary-CFG differential, utf8 differential, host boundary, caps, import-mode

## Progress (t+~50min)
- tests/bad_actor_8g_cfg.rs: random arbitrary-CFG differential (back edges, switch with args, critical edges, u8/i64 block params, wrap/sat/trap arith). 3000 modules: 2900 compiled (100 irreducible -> E0600), 27401 cases, 0 mismatches. Larger runs in progress.
  Harness bug (test only, tests/common/interp.rs): fuel is only checked inside instruction execution, so an instruction-free infinite loop of blocks spins forever. My generator avoids it (every block has a nop const).
- tests/bad_actor_8g_host.rs (raw wasmi, hostile wrapper input): oversized/negative len -> bounds, no allocation; OOB ptr -> engine trap; live objects 0 after wrappers; non-canonical bool/u8/i16 canonicalised; stub UTF-8 validator == std exhaustively (1-3 byte, 4-byte edge) + 300k random. ALL HELD.
- tests/bad_actor_8g_limits.rs: from_utf8 trap order, array.new extremes (OOM not wrap), param/result limits (E0603), 3000-case switch chain/table, array.push growth for u8/f64, all held. 70000-field struct test: slow (see below).

## Findings (cont.)
F2 (perf, verifier slice 8.C, hit by 8.G) IR verifier is O(n^2) in fields per struct: 4000 fields = 2.1 s verify, 70000 fields ~ 10+ min. compile() calls the verifier so codegen inherits it. (parse 17 ms, codegen itself fast.)
F3 (limit mismatch, low) functions with many SSA values: codegen LIMIT_LOCALS=50000 but the strict host engine loads only up to ~28000-32000 locals and EVERY call of a function with >= ~20000 locals (value count) traps StackOverflow (engine, guest kind 0); >= ~32000 is rejected at load (OXS-E0408 "too many function parameters"). Test: tests/bad_actor_8g_limits.rs::many_locals_load_and_run_or_diagnose (prints). Fix: lower LIMIT_LOCALS (~16000) or reuse locals (liveness) in codegen.
F4 (descriptor truncation, potential leak) src/modlayout.rs: `ptr_count: l.ptr_slots.len() as u16` - a struct/variant/closure env with >= 65536 managed fields gets a truncated pointer count (TypeDesc word1 holds 16 bits); release/clone would skip children (leak/UB-ish). No cap in verifier or layout. Repro generator: tests/bad_actor_8g_limits.rs::struct_with_more_than_65535_managed_fields_frees_all_children (#[ignore], slow because of F2; OXI_WIDE_N=70000). Result: see below.
F5 (ABI conformance, low) exports with `ref T` / `[i32]` params or `ref T` results compile and load (verify accepts); the wrapper path only handles str/[u8]/bool/narrow ints. Host can then forge guest pointers (arbitrary in-guest read/write through setfield) and owned refs returned raw leak. ABI says the checker (8.T) rejects; codegen/verify do not enforce. Test: tests/bad_actor_8g_names.rs::export_with_raw_object_params (prints only).
Known/acknowledged: the 40 B/function strict-engine rule bit my own int tests (needed a pad function) - same as author's finding 3.

## What held (so far)
- 330k+ random CFG cases (3 shapes, incl. paramless jump-table switches), E0600 only for irreducible, deterministic output, Import mode valid wasm for all of them.
- exhaustive 8-bit + boundary binary/unary/conv semantics (2.3M cases) vs i128 model; float->int boundary (1952 cases) all dst types/modes.
- wrappers: bounds on len, no allocation on reject, release balance, canonicalisation, UTF-8 validator == std (exhaustive <=3 bytes).
- managed: 300 random sequences of shared refs / COW / clone / array.set / push, zero leaks.
- export_name shadowing/collision rejected by verifier (E0602).

## Final
F6 (limit, low) >= ~290 sequential loops in one function -> E0603 (each loop adds nesting; test many_sequential_loops_compile_or_diagnose_cleanly prints).
F4 empirical run (70000 fields) was still inside the O(n^2) verifier after 16 min; F4 stands from code reading (modlayout.rs ptr_count as u16).
Catalogue coverage: tests/bad_actor_8g_catalogue.rs (cat01,02,04,05,06,07,09,10,11,14) all held; cat03 -> int/float files; cat13 -> cfg file; cat08 -> limits (array_new_extremes, push growth); cat12 -> cat06 (no codegen metering).
Only failing test: bad_actor_8g_nan::host_nan_payload_does_not_round_trip (F1).
