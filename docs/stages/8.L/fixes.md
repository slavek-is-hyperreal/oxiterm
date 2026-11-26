# 8.L fixes handoff (update after each fix)
Order: L08(checker), L10, L01, L03, L02, L06, L07, coverage tests, fuzz, verify.
## Done
- L08: oxiscript-check/src/decl.rs check_export_sig (OXS-E0121, module 0 non-main exports: params scalar/enum/handle/host/str/[u8], ret scalar/void); corpus 09_union_switch.oxs (oxiscript-syntax/tests/corpus/valid) dropped `export`; lowering driver now internal E0700 assertion; compile_source_with_notes added; test w0700_is_visible_and_export_absent FLIPPED -> unfit_exports_are_type_errors. TODO: checker test in oxiscript-check, docs.
- L10: call.rs mutating_call releases extra retain. tests alias_*/leak_mut_* green.
- L01: expr.rs template holds acc across later exprs. tests green.
## Todo
L03, L02, L06 (stubrt.rs), L07 docs, grammar rule L02, contracts note, coverage tests (out-buffer retry, callbacks after state, needs order), fuzz_long_seeded release, verify all, clippy, wasm32, speed.
- L03 done (expr.rs ident: narrowed-null local -> void like literal; coalesce handled via literal path? see tests). L02 done (stmt.rs loop_stmt per_iter hidden cell join var, copy_cells). New tests: tests/fixes_8l.rs. Remaining: L06, docs (ir.md L07, grammar L02, contracts), coverage tests, checker test, fuzz, verify.
- L06 done (stubrt.rs iterative free_obj + release_child; tests l06_* in fixes_8l.rs use run_big with fuel). Docs done: ir.md (L07), grammar.md (L02), contracts.md (8.L notes). Remaining: coverage tests (out-buffer retry, callbacks after state change, needs order), checker unit test, fuzz release, verify all, clippy, wasm32, speed, advisor.
- Coverage tests written: tests/host_coverage_8l.rs (all green), oxiscript-check/tests/export_abi.rs (not yet run), 09_union_switch edit; bad_actor_8l_str.rs slTxt lost `export` (relied on silent W0700 drop). Full check suite green; lower full suite green except those fixed. L03 panic probe added. Remaining: run it, export_abi, fuzz release, clippy, wasm32, speed, release tests, advisor, report.
## FINAL STATE
All fixes + tests done. Verified: lower debug (full, after fixes: only string_ops_table/host_coverage failures fixed), lower release 18 binaries ok, fuzz_long_seeded release 9600x2 fns 0 failures, check full ok (+export_abi), wasm ok, ir/abi ok, clippy -D warnings check/lower/wasm clean (added allow attrs on bad_actor_8l*.rs tests), wasm32 ok, speed ~50k lines/s. Remaining: report only.
Residual: closure `() => s` returning a narrowed-null local still E0700 (inferred closure return type is bare Null); L05 documented only.
L06 budget: drop of 120k nodes in one entry exceeds DEFAULT fuel_per_call (OutOfFuel, not a stack trap); passes with raised fuel. abi.md does not cap result<str> by HOST_PAYLOAD_MAX; runtime caps at Limits::max_host_payload (64 KiB).
