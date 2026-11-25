//! Reference-model fuzzing of the runtime natively (checked heap).
#![allow(clippy::many_single_char_names, clippy::too_many_lines, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::doc_markdown, clippy::manual_is_multiple_of, clippy::similar_names)]

use oxiscript_rc::testkit::{fuzz, SimGuest};

fn run(seeds: std::ops::Range<u64>, ops: usize) -> (u64, u64, u64, u64) {
    let (mut total_ops, mut collects, mut freed, mut aborts) = (0, 0, 0, 0);
    for seed in seeds {
        let mut g = SimGuest::standard();
        let st = fuzz(&mut g, seed, ops);
        total_ops += st.ops;
        collects += st.collects;
        freed += st.cycle_freed;
        aborts += st.aborts;
    }
    (total_ops, collects, freed, aborts)
}

#[test]
fn fuzz_short_programs() {
    let (ops, collects, freed, aborts) = run(0..1500, 120);
    eprintln!("short: {ops} ops, {collects} collects, {freed} objects freed by cycle collection, {aborts} aborted trials");
    assert!(freed > 0, "fuzzer never produced collectable cycles");
}

#[test]
fn fuzz_long_programs() {
    let (ops, collects, freed, aborts) = run(10_000..10_150, 1500);
    eprintln!("long: {ops} ops, {collects} collects, {freed} objects freed by cycle collection, {aborts} aborted trials");
    assert!(aborts > 0, "the abort/undo path was never exercised");
    assert!(freed > 0);
}

/// Every allocation-failure path (first try fails, retry succeeds) under the
/// reference model: object/block allocation, candidate buffer growth, the
/// collector's stack/log growth inside trials (undo path), the debug live map.
#[test]
fn fuzz_with_allocation_failure_injection() {
    let seeds: u64 = if cfg!(debug_assertions) { 150 } else { 1500 };
    let mut failures = 0;
    for seed in 0..seeds {
        let mut g = SimGuest::standard();
        g.rt.heap.inject_failures(seed, [3, 10, 30][(seed % 3) as usize]);
        fuzz(&mut g, 50_000 + seed, 150);
        failures += g.rt.heap.injected_failures;
    }
    eprintln!("injection: {seeds} programs, {failures} injected allocation failures");
    assert!(failures > 1000);
}
