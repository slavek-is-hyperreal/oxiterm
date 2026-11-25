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
