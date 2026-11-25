//! Reference-model fuzzing of the runtime natively (checked heap).

use oxiscript_rc::testkit::{fuzz, SimGuest};

fn run(seeds: std::ops::Range<u64>, ops: usize) -> (u64, u64, u64) {
    let (mut total_ops, mut collects, mut freed) = (0, 0, 0);
    for seed in seeds {
        let mut g = SimGuest::standard();
        let st = fuzz(&mut g, seed, ops);
        total_ops += st.ops;
        collects += st.collects;
        freed += st.cycle_freed;
    }
    (total_ops, collects, freed)
}

#[test]
fn fuzz_short_programs() {
    let (ops, collects, freed) = run(0..1500, 120);
    eprintln!("short: {ops} ops, {collects} collects, {freed} objects freed by cycle collection");
    assert!(freed > 0, "fuzzer never produced collectable cycles");
}

#[test]
fn fuzz_long_programs() {
    let (ops, collects, freed) = run(10_000..10_150, 1500);
    eprintln!("long: {ops} ops, {collects} collects, {freed} objects freed by cycle collection");
    assert!(freed > 0);
}
