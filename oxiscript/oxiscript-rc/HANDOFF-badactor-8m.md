# HANDOFF BAD ACTOR 8.M
Status: started; tests in tests/bad_actor_8m.rs. Candidate findings (from code reading, to confirm by test):
1 v32_push stale local copy on OOM-drain reentrancy (rt.rs:285-313)
2 gc_step not bounded by budget: mark_gray checks limit per node after all slots (rt.rs:646-664)
3 drain walks pointer-free arrays per element (rt.rs:554-576)
4 init() validates no descriptors
5 stale release in release builds
Confirmed findings:
- F1 CONFIRMED (tests f1_*): v32_push stale copy after OOM drain re-entry; sim panics 'access of 64 bytes outside live block' in copy from freed roots block. rt.rs:285-313 (+raw_alloc 268-283). Fix: reload self.roots after raw_alloc / re-check len==cap; or never drain inside v32_push.
- F2 CONFIRMED (f2, f2b): gc_step(16) on one 200k-slot purple REFARRAY = 800k heap loads (mark+undo, 4/slot), repeated identically every sweep. rt.rs:646-664 (limit checked after whole node), 670-691. Fix: check limit inside slot loop / cap work per node, remember 'too big' candidates (skip until collect_cycles).
- F3 CONFIRMED (f3): pointer-free array (SCALARARRAY 1M) stays queued after release; drain loops per element (rt.rs:554-576). f3b (#[ignore]) zero-size elem array len 2^31-1 burns ~2^31 iterations. Fix: pcount==0 => skip element loop.
- F4 CONFIRMED (f4, f4d): init validates no descriptors. Slot offset >= payload / huge / elem-smaller-than-slot -> access outside object (sim panics; wasm silent in-bounds corruption). Duplicate offsets -> double dec -> still-referenced child freed (f4d). Unaligned offsets accepted. Fix: validate table once in oxi_rt_init (offset+4<=size, aligned, strictly increasing, pmap in memory, kind in 1..5, count<2^24, size sane) or trap.
- TODO: F5 wasm stale release (wasmi), API hostile (gc_step(0), config), strings f64, alloc sizes, fuzz extension, perf.
- F5 CONFIRMED (f5, f5b; f5c held): release-build wasm double release => dlmalloc free-list words (fd/bk = rc/meta words) corrupted; 2nd subsequent oxi_alloc traps 'out of bounds memory access' (contained, but is a trap not a clean diagnostic and can silently overlap in other layouts). Debug build: stale release SILENTLY accepted (0xDD poison => rc>=STICKY => ignored, rt.rs:424-426/405-407) => lib.rs claim 'debug poisoning detects' is false for release/retain. Non-object pointers (1,8,100,1024,4096,...): all trap in release (rc==0 hit) - held.
- F6 gc_step(0) with pending candidate returns STEP_MORE forever (f6) - API misuse/low (host loop hang). Held: extreme budgets/config/unknown stat keys, pre-init exports (f6b,f6c), size overflows (f7), concat/slice edges (f7b), f64 printing round-trips 20k random (f7c; "-0" and "1e16" style outputs, spec says shortest round-trip: fine), i64 extremes.
- TODO: extended model fuzz (green->cyclic child, sticky in cycles, exact undo, zombies, mutations between steps), perf DoS shapes, final report.
- F8 held: extended model fuzz (tests f8, f1b): green record holding cyclic child, array-of-inline-records ARRNODE, clone inside cycles, saturate ANY live object (in cycles), gc_step interleaved with mutations, rc lower-bound check after every step + exact after collect: 20000 seeds x 250 ops, 514k gc_steps, 151k aborted trials, 30.6k cycle-freed: 0 failures. f1b OOM first-try failure injection 3000 seeds: 0 failures (F1 needs the specific roots-full+pending-queue shape).
- TODO: F9 perf amplification test, final report.
