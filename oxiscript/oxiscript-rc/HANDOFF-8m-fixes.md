# HANDOFF 8m fixes (bad actor findings F1-F9)

## Done (release tests green: all of oxiscript-rc incl. bad_actor_8m 25/25, f3b un-ignored)
- F1: grow_vec / v32_try_push re-read state after a drain; trial aborts + exact undo on OOM; stack reserved before phase 2.
- F2/F9: gc_step per-slot limit, log pairs (node, done), batch trials (halve on abort), deferral flag in low bits of root entries (skip_flagged).
- F3: pcount==0 arrays freed without walk. F4: init validates the table (trap 64). F5: debug live bitmap (trap 65). F6: gc_step(0)=0.
- Tests corrected (explain in report): f4b (init inside the catch), f5 (a trap is acceptable, silent corruption is not).
- tests/fuzz.rs: fuzz_with_allocation_failure_injection added (SimHeap::inject_failures).

## Remaining
- debug-build full test run, OXI_FULL_FUZZ release run, clippy native + wasm32, wasm sizes,
  lib.rs docs (host between-calls, memory limit, F5 claim), abi.md doc text in report, final report per finding.
