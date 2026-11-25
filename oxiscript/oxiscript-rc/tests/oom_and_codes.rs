//! Persistent-OOM behaviour of the collector, zombie reaping by bounded steps,
//! and the dedicated trap codes (64 bad descriptor table, 65 stale object).
#![allow(clippy::many_single_char_names, clippy::too_many_lines, clippy::cast_possible_truncation)]

use oxiscript_rc::rt::{STEP_DEFERRED, STEP_MORE};
use oxiscript_rc::sim::Trap;
use oxiscript_rc::testkit::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

fn trap_of<R>(f: impl FnOnce() -> R) -> Option<u32> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(_) => None,
        Err(e) => e.downcast_ref::<Trap>().map(|t| t.0),
    }
}
fn colour(g: &mut SimGuest, p: u32) -> u32 {
    (g.rd(p - 4) >> 24) & 7
}
fn link(g: &mut SimGuest, src: u32, off: u32, dst: u32) {
    let old = g.rd(src + off);
    g.retain(dst);
    g.wr(src + off, dst);
    g.release(old);
}
fn big_guest() -> SimGuest {
    let mut g = SimGuest::new(256 << 20, false);
    install_types(&mut g, &standard_types());
    g
}
/// A dead ring of `n` nodes (every node a purple candidate).
fn ring(g: &mut SimGuest, n: u32) -> Vec<u32> {
    let nodes: Vec<u32> = (0..n).map(|_| g.alloc(ty::NODE)).collect();
    for i in 0..n as usize {
        link(g, nodes[i], 0, nodes[(i + 1) % n as usize]);
    }
    for &p in &nodes {
        g.release(p);
    }
    nodes
}

#[test]
fn allocation_failure_inside_a_trial_is_undone_and_retried() {
    let mut g = big_guest();
    ring(&mut g, 10_000);
    // the collector's stack and log are empty: the first growth inside the trial fails on
    // all three attempts, the trial is undone, the queue drained, the retry succeeds
    g.rt.heap.fail_next(3);
    assert_eq!(g.collect(), 10_000);
    assert!(g.stat(6) >= 1, "the OOM abort path ran");
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn persistent_oom_traps_cleanly_and_leaves_the_heap_intact() {
    let mut g = big_guest();
    let nodes = ring(&mut g, 10_000);
    g.rt.heap.fail_next(1_000_000);
    assert_eq!(trap_of(|| g.collect()), Some(7), "OutOfMemory");
    for &p in &nodes {
        assert_eq!(g.rd(p - 8), 1, "rc restored by the undo");
        let c = colour(&mut g, p);
        assert!(c == 3 || c == 0, "colour {c} is neither candidate nor black");
    }
    g.rt.heap.fail_next(0);
    assert_eq!(g.collect(), 10_000, "the heap is still collectable once memory is back");
}

#[test]
fn persistent_oom_does_not_make_gc_step_spin() {
    let mut g = big_guest();
    ring(&mut g, 10_000);
    g.rt.heap.fail_next(1_000_000);
    let mut s = g.gc_step(u32::MAX);
    let mut calls = 1;
    while s & STEP_MORE != 0 {
        s = g.gc_step(u32::MAX);
        calls += 1;
        assert!(calls < 10, "gc_step reports MORE forever under persistent OOM");
    }
    assert_eq!(s & STEP_DEFERRED, STEP_DEFERRED, "host is told to fall back to collect_cycles");
    g.rt.heap.fail_next(0);
    assert_eq!(g.collect(), 10_000);
}

#[test]
fn deferred_candidate_that_dies_is_reaped_by_later_steps() {
    let mut g = SimGuest::standard();
    let arr = g.array_new(ty::REFARRAY, 5000);
    g.retain(arr);
    g.release(arr); // purple candidate whose mark phase is 5000 slots
    assert_eq!(g.gc_step(16) & STEP_DEFERRED, STEP_DEFERRED);
    g.release(arr); // dies through plain RC; still listed, so it lingers as a zombie
    let before = g.rt.heap.live_blocks();
    let (mut s, mut n) = (g.gc_step(16), 0);
    while s & STEP_MORE != 0 {
        s = g.gc_step(16); // first the queued element walk, then the sweep
        n += 1;
        assert!(n < 1000);
    }
    assert_eq!(s, 0, "same budget: the entry is skipped, but a dead one is reaped");
    assert_eq!(g.stat(1), 0, "entry unlisted");
    assert!(g.rt.heap.live_blocks() < before, "zombie block freed without collect_cycles");
}

#[test]
fn descriptor_rejections_trap_with_the_dedicated_code() {
    let rec = |kind, size, ptrs: Vec<u32>| TypeSpec { kind, size, acyclic: false, ptrs };
    let bad = vec![
        rec(1, 16, vec![64]),
        rec(1, 16, vec![0xFFFF_FFF0]),
        rec(1, 16, vec![2]),
        rec(1, 16, vec![0, 0]),
        rec(1, 16, vec![8, 4]),
        rec(5, 4, vec![8]),
        rec(77, 16, vec![]),
        rec(4, 0, vec![]),
        rec(1, 0xFFFF_FFF8, vec![]),
        rec(1, 0xFFFF_FFFF, vec![]),
        rec(1, 12, vec![]),
        rec(5, 0xFFFF_FFFF, vec![]),
    ];
    for (i, spec) in bad.into_iter().enumerate() {
        let mut specs = standard_types();
        specs.push(spec);
        let mut g = SimGuest::new(64 << 20, true);
        assert_eq!(trap_of(|| install_types(&mut g, &specs)), Some(64), "case {i}");
        assert_eq!(trap_of(|| g.alloc(ty::NODE)), Some(1), "case {i}: a rejected table leaves the runtime uninitialised");
    }
    // a hole (kind 0) is fine and traps at use
    let mut specs = standard_types();
    specs.push(TypeSpec::default());
    let mut g = SimGuest::new(64 << 20, true);
    install_types(&mut g, &specs);
    assert_eq!(trap_of(|| g.alloc(25)), Some(5));
}

#[test]
fn debug_builds_trap_65_on_stale_and_non_object_pointers() {
    if !cfg!(debug_assertions) {
        return; // release builds keep the unchecked behaviour by design
    }
    let mut g = SimGuest::standard();
    let n = g.alloc(ty::NODE);
    g.release(n);
    assert_eq!(trap_of(|| g.release(n)), Some(65), "stale release");
    assert_eq!(trap_of(|| g.retain(n)), Some(65), "stale retain");
    let b = g.alloc_bytes(64);
    assert_eq!(trap_of(|| g.release(b + 8)), Some(65), "release of a non-object block");
    assert_eq!(trap_of(|| g.release(4)), Some(65), "wild pointer");
}
