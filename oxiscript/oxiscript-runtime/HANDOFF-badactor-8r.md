# BAD ACTOR 8.R handoff (updated after each milestone)

Harness: oxiscript/oxiscript-runtime/tests/bad_actor_8r.rs (uses tests/common). Run:
`CARGO_BUILD_JOBS=2 cargo test -p oxiscript-runtime --release --offline --test bad_actor_8r -- --nocapture --test-threads=1`
Env knobs: BA_SMITH_CASES (default 20000), BA_MUT_CASES (default 30000).

## Findings (all confirmed by running; tests below FAIL today)
- F1 wrong denial semantics (medium): `f1_revocation_survives_restart_and_reload`. Runtime::revoke only clears the
  current instance's CapSlot; restart()/reload() rebuild from `self.grants` (runtime.rs Runtime::revoke ~L560, Instance::build)
  so the capability returns (state get -> status 16 instead of 2). Guest can force it by trapping itself.
  Fix: keep a per-Runtime revoked set and filter plan.providers in build (or narrow grants on revoke).
- F3 host DoS (medium): `f3_set_scans_all_subscriptions`. StateStore::set (caps/state.rs ~L95) scans ALL subs of all
  instances by string compare for 200 fuel. 14 runtimes x 64 watchers, 256-byte keys: 65x per-call slowdown,
  one 10M-fuel turn ~0.8 s. Fix: index subs by key (HashMap<String, Vec<id>>) and/or charge fuel by sub count.
- F4 host DoS (medium): `f4_pump_has_no_aggregate_budget`. Pump runs up to 256 releases + 256 events each with full
  fuel_per_call (runtime.rs pump). 64 watchers + 256 queued releases burning 9M fuel: 1.5 s per pump; with memory.fill
  callbacks (10M fuel = ~97 ms) worst case ~30 s. Fix: aggregate fuel/time budget per pump.
- F4b host memory growth (low-medium): `f4b_release_backlog_is_unbounded`. StateCap.releases (state.rs watch) and
  Runtime.pending_releases unbounded; 800k queued in 0.25 s, drained 256/pump. Fix: cap queue (trap when full).
- F8 determinism/info leak (low): `f8_cap_handle_value_depends_on_host_history`. Cap handle seed = host-global
  epoch*2654 (runtime.rs build), guest-visible in oxi_init: same module differs per host history, leaks instance count.
- F9 cross-script DoS (medium): `f9_one_script_exhausts_the_shared_state_store`. Shared StateStore, no quota/delete;
  one script (restarting to reset 256-slot table) fills all 1024 keys permanently; others get status 18.
- F10 log injection (low): `f10_log_injection_is_not_sanitised`. LogCap stores raw newlines/ANSI/NUL.
- Low/doc: guest may re-export a host import as a user export or as the `cb:` callback (same signature as log.write);
  load.rs does not reject; call_export("relog") reaches the trampoline from the embedder; pump calling it traps.
  (probe: reexported_import_and_reserved_names_via_call_export)

## Held up
- host.rs pointer/length/out-buffer arithmetic: `f5_trampoline_invariant_fuzz` 150000 edge-value cases (ok=91717,
  trap=58283, status1=398): no write outside [out_ptr,out_ptr+len), no write on status!=0, no panic.
- Translation time (f7, f7b): linear; worst 3.6 MB module 137 ms, 91 MB RSS; engine limits reject br_table/locals/etc.
- Reload storm: 270 us/reload small module, RSS flat. Reserved names not callable via call_export. NaN results canonical
  for arithmetic (neg/abs keep payload, spec-legal).
- Fuzz: wasm-smith all-features 20000 cases, 0 panics (codes E0408 16987, E0406 2301, E0405 674, E0400 38);
  mutation fuzz 30000 cases, 3710 loaded, 70490 export calls, 0 panics.
- F2 (status-1 read loop) only ~8x wall per fuel vs pure wasm: not a finding.
- No panic=abort in workspace Cargo.toml (grep), so catch_unwind in build works.

## Remaining
- Final full run of the file; second advisor call; final report via SubagentHandback.
