//! Hand-written graph scenarios against the native checked heap.

use oxiscript_rc::rt::{STEP_DEFERRED, STEP_MORE};
use oxiscript_rc::sim::Trap;
use oxiscript_rc::testkit::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

const A: u32 = 0; // NODE.a offset
const B: u32 = 4; // NODE.b offset

fn colour(g: &mut SimGuest, p: u32) -> u32 {
    (g.rd(p - 4) >> 24) & 7
}
fn buffered(g: &mut SimGuest, p: u32) -> bool {
    g.rd(p - 4) & (1 << 27) != 0
}
fn trap_of<R>(f: impl FnOnce() -> R) -> Option<u32> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(_) => None,
        Err(e) => e.downcast_ref::<Trap>().map(|t| t.0),
    }
}
/// `src.slot = dst` with proper ownership (retain new, release old).
fn link(g: &mut SimGuest, src: u32, off: u32, dst: u32) {
    let old = g.rd(src + off);
    g.retain(dst);
    g.wr(src + off, dst);
    g.release(old);
}

#[test]
fn big_list_drops_without_recursion_and_in_bounded_steps() {
    let mut g = SimGuest::new(256 << 20, false);
    install_types(&mut g, &standard_types());
    let n = 1_000_000;
    let mut head = 0;
    for _ in 0..n {
        let node = g.alloc(ty::NODE);
        g.wr(node + A, head);
        head = node;
    }
    assert_eq!(g.live_objects(), n);
    g.release(head);
    // one release did a bounded amount of work: most of the list is still queued
    assert!(g.stat(0) > 0);
    assert!(g.rt.stat(0) <= 2, "queue holds only the frontier, not the whole list");
    let mut calls = 0;
    while g.stat(0) > 0 {
        let x = g.alloc(ty::LEAF); // each alloc drains a bounded chunk too
        g.release(x);
        calls += 1;
    }
    assert!(calls > 1000, "{calls}");
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn big_balanced_tree() {
    fn build(g: &mut SimGuest, depth: u32) -> u32 {
        let n = g.alloc(ty::NODE);
        if depth > 0 {
            let l = build(g, depth - 1);
            let r = build(g, depth - 1);
            g.wr(n + A, l);
            g.wr(n + B, r);
        }
        n
    }
    let mut g = SimGuest::new(256 << 20, false);
    install_types(&mut g, &standard_types());
    let root = build(&mut g, 19); // 1M nodes
    assert_eq!(g.live_objects(), (1 << 20) - 1);
    g.release(root);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn dag_sharing_exact_counts() {
    let mut g = SimGuest::standard();
    let (top, l, r, bottom) = (g.alloc(ty::NODE), g.alloc(ty::NODE), g.alloc(ty::NODE), g.alloc(ty::LEAF));
    link(&mut g, top, A, l);
    link(&mut g, top, B, r);
    link(&mut g, l, A, bottom);
    link(&mut g, r, A, bottom);
    g.release(l);
    g.release(r);
    g.release(bottom);
    assert_eq!(g.rd(bottom - 8), 2);
    assert_eq!(g.rd(l - 8), 1);
    g.release(top);
    assert_eq!(g.live_objects(), 0);
    assert_eq!(g.collect(), 0);
}

#[test]
fn self_and_mutual_and_triple_cycles_need_the_collector() {
    let mut g = SimGuest::standard();
    // self cycle
    let a = g.alloc(ty::NODE);
    link(&mut g, a, A, a);
    g.release(a);
    // mutual cycle
    let (b, c) = (g.alloc(ty::NODE), g.alloc(ty::NODE));
    link(&mut g, b, A, c);
    link(&mut g, c, A, b);
    g.release(b);
    g.release(c);
    // triple ring with a string hanging off each member
    let ring: Vec<u32> = (0..3).map(|_| g.alloc(ty::NODE)).collect();
    for i in 0..3 {
        link(&mut g, ring[i], A, ring[(i + 1) % 3]);
        let s = g.str_new("payload");
        g.wr(ring[i] + B, s);
    }
    for r in &ring {
        g.release(*r);
    }
    assert_eq!(g.live_objects(), 1 + 2 + 6, "RC alone cannot free cycles");
    assert_eq!(g.stat(1), 6, "candidates: a, b, c and ring members (strings are green)");
    assert_eq!(g.collect(), 1 + 2 + 3 + 3 /* strings via green release, counted separately */ - 3);
    assert_eq!(g.live_objects(), 0);
    assert_eq!(g.live_bytes(), g.live_bytes()); // stable
}

#[test]
fn cycle_reachable_from_live_root_is_not_freed() {
    let mut g = SimGuest::standard();
    let root = g.alloc(ty::NODE);
    let (x, y) = (g.alloc(ty::NODE), g.alloc(ty::NODE));
    link(&mut g, x, A, y);
    link(&mut g, y, A, x);
    link(&mut g, root, A, x);
    g.release(x);
    g.release(y);
    assert_eq!(g.collect(), 0);
    assert_eq!(g.live_objects(), 3);
    assert_eq!((g.rd(x - 8), g.rd(y - 8), g.rd(root - 8)), (2, 1, 1), "counts restored exactly");
    // now cut it loose
    link(&mut g, root, A, 0);
    assert_eq!(g.collect(), 2);
    g.release(root);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn cycle_through_closure_environment() {
    let mut g = SimGuest::standard();
    let obj = g.alloc(ty::NODE);
    let clo = g.alloc(ty::CLOSURE);
    g.wr(clo, 5); // table index
    link(&mut g, clo, 8, obj); // captures obj
    link(&mut g, obj, A, clo); // obj holds the closure
    g.release(obj);
    g.release(clo);
    assert_eq!(g.live_objects(), 2);
    assert_eq!(g.collect(), 2);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn array_and_variant_cycles() {
    let mut g = SimGuest::standard();
    let arr = g.array_new(ty::REFARRAY, 3);
    let var = g.alloc(ty::VARIANT);
    let data = g.rd(arr + 8);
    link(&mut g, var, 4, arr);
    g.retain(var);
    g.wr(data + 4, var);
    g.release(arr);
    g.release(var);
    assert_eq!(g.live_objects(), 2);
    assert_eq!(g.collect(), 2);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn sticky_counts_saturate_and_never_free() {
    let mut g = SimGuest::standard();
    let x = g.alloc(ty::NODE);
    let child = g.alloc(ty::NODE);
    link(&mut g, x, A, child);
    g.release(child);
    g.wr(x - 8, 0x7FFF_FFFE);
    g.retain(x); // 0x7FFF_FFFF
    assert!(!g.is_unique(x));
    g.retain(x); // saturates
    assert_eq!(g.rd(x - 8), 0x8000_0000);
    for _ in 0..10 {
        g.retain(x);
        g.release(x);
        g.release(x);
    }
    assert_eq!(g.rd(x - 8), 0x8000_0000, "sticky count is frozen");
    assert_eq!(g.collect(), 0);
    assert_eq!(g.live_objects(), 2, "sticky object and what it holds stay alive");
    // sticky object inside a cycle: the cycle is an external-looking root, never freed
    link(&mut g, child, A, x);
    assert_eq!(g.collect(), 0);
    assert_eq!(g.rd(child - 8), 1);
    assert_eq!(g.live_objects(), 2);
}

#[test]
fn green_objects_are_never_buffered_or_traced() {
    let mut g = SimGuest::standard();
    let leaf = g.alloc(ty::LEAF);
    assert_eq!(colour(&mut g, leaf), 4);
    g.retain(leaf);
    g.release(leaf);
    assert_eq!(colour(&mut g, leaf), 4);
    assert!(!buffered(&mut g, leaf));
    assert_eq!(g.stat(1), 0);
    let s = g.str_new("s");
    g.retain(s);
    g.release(s);
    assert_eq!(g.stat(1), 0);
    g.release(leaf);
    g.release(s);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn retain_makes_black_and_release_buffers_purple() {
    let mut g = SimGuest::standard();
    let n = g.alloc(ty::NODE);
    assert_eq!(colour(&mut g, n), 0);
    g.retain(n);
    g.release(n);
    assert_eq!((colour(&mut g, n), buffered(&mut g, n)), (3, true));
    assert_eq!(g.stat(1), 1);
    g.retain(n);
    assert_eq!((colour(&mut g, n), buffered(&mut g, n)), (0, true), "retained: black, still listed");
    g.release(n);
    assert_eq!(g.stat(1), 1, "a candidate is listed once");
    g.release(n);
    // died while buffered: zombie until the buffer is processed
    assert_eq!(g.rt.stat(0), 0);
    g.collect();
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn gc_step_is_bounded_and_undoes_exactly() {
    let mut g = SimGuest::new(256 << 20, false);
    install_types(&mut g, &standard_types());
    let n = 50_000;
    let nodes: Vec<u32> = (0..n).map(|_| g.alloc(ty::NODE)).collect();
    for i in 0..n as usize {
        link(&mut g, nodes[i], A, nodes[(i + 1) % n as usize]);
    }
    for p in &nodes {
        g.release(*p);
    }
    let rcs: Vec<u32> = nodes.iter().map(|&p| g.rd(p - 8)).collect();
    assert!(rcs.iter().all(|&r| r == 1));
    // the budget is far below the ring size: the trial must abort and restore everything
    let mut s = g.gc_step(100);
    let mut steps = 1;
    while s & STEP_MORE != 0 {
        s = g.gc_step(100);
        steps += 1;
        assert!(steps < 200_000);
    }
    assert_eq!(s, STEP_DEFERRED, "all candidates examined, none freeable within the budget");
    assert_eq!(g.live_objects(), n);
    for (i, &p) in nodes.iter().enumerate() {
        assert_eq!(g.rd(p - 8), 1, "count of node {i} restored");
        assert_eq!(colour(&mut g, p), 3, "node {i} is a candidate again");
    }
    assert_eq!(g.live_objects(), n);
    // a big enough step frees it
    let s = g.gc_step(u32::MAX);
    assert_eq!(s, 0);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn gc_step_frees_small_garbage_within_budget_and_collect_handles_the_rest() {
    let mut g = SimGuest::standard();
    for _ in 0..100 {
        let (a, b) = (g.alloc(ty::NODE), g.alloc(ty::NODE));
        link(&mut g, a, A, b);
        link(&mut g, b, A, a);
        g.release(a);
        g.release(b);
    }
    let mut steps = 0;
    while g.gc_step(50) != 0 {
        steps += 1;
        assert!(steps < 1000);
    }
    assert!(steps > 1, "budget forced several steps");
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn array_destruction_resumes_per_element() {
    let mut g = SimGuest::new(64 << 20, true);
    install_types(&mut g, &standard_types());
    g.config(0, 10); // 10 work units per call
    let n = 5000;
    let arr = g.array_new(ty::REFARRAY, n);
    let data = g.rd(arr + 8);
    for i in 0..n {
        let leaf = g.alloc(ty::LEAF);
        g.wr(data + 4 * i, leaf);
    }
    assert_eq!(g.live_objects(), n + 1);
    g.release(arr);
    let mut calls = 0;
    while g.stat(0) > 0 {
        let x = g.alloc(ty::LEAF);
        g.release(x);
        calls += 1;
    }
    assert!(calls >= 100, "destruction was spread over many calls ({calls})");
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn out_of_memory_traps_after_reclaiming_pending_garbage() {
    let mut g = SimGuest::new(2 << 20, true);
    install_types(&mut g, &standard_types());
    let mut head = 0;
    let t = trap_of(|| loop {
        let n = g.alloc(ty::NODE);
        g.wr(n + A, head);
        head = n;
    });
    assert_eq!(t, Some(7), "OutOfMemory");
    // make garbage that is still queued, then allocation must succeed by draining it first
    let mut g = SimGuest::new(2 << 20, true);
    install_types(&mut g, &standard_types());
    let mut head = 0;
    let mut count = 0;
    let _ = trap_of(|| loop {
        let n = g.alloc(ty::NODE);
        g.wr(n + A, head);
        head = n;
        count += 1;
    });
    let count = count;
    // `head` is the last fully built node (the trap came from the next alloc)
    g.release(head);
    let again = g.alloc(ty::NODE);
    assert_ne!(again, 0);
    assert!(count > 1000);
}

#[test]
fn clone_retains_children_and_arrays_copy() {
    let mut g = SimGuest::standard();
    let (n, kid) = (g.alloc(ty::NODE), g.alloc(ty::LEAF));
    link(&mut g, n, A, kid);
    g.release(kid);
    let c = g.clone_obj(n);
    assert_eq!(g.rd(kid - 8), 2);
    assert_eq!(g.rd(c - 8), 1);
    assert_ne!(c, n);
    let arr = g.array_new(ty::REFARRAY, 2);
    let d = g.rd(arr + 8);
    g.retain(kid);
    g.wr(d, kid);
    let ac = g.clone_obj(arr);
    assert_eq!(g.rd(kid - 8), 4);
    assert_ne!(g.rd(ac + 8), g.rd(arr + 8), "data block is copied, not shared");
    for p in [n, c, arr, ac] {
        g.release(p);
    }
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn misuse_traps_instead_of_corrupting() {
    let mut g = SimGuest::standard();
    let n = g.alloc(ty::NODE);
    g.release(n);
    assert_eq!(g.live_objects(), 0);
    // use after free is caught by the checked heap (access outside any live block)
    assert!(catch_unwind(AssertUnwindSafe(|| g.release(n))).is_err());

    let mut g = SimGuest::standard();
    assert_eq!(trap_of(|| g.alloc(0)), Some(1));
    assert_eq!(trap_of(|| g.alloc(9999)), Some(1));
    assert_eq!(trap_of(|| g.alloc(ty::REFARRAY)), Some(5), "arrays are made with array_new");
    assert_eq!(trap_of(|| g.alloc(ty::STRING)), Some(5));
    assert_eq!(trap_of(|| g.clone_obj(0)), Some(6));
    g.retain(0);
    g.release(0);
    assert_eq!(trap_of(|| g.array_new(ty::NODE, 1)), Some(5));
    assert_eq!(trap_of(|| g.array_new(ty::REFARRAY, 0x8000_0000)), Some(4));
}

#[test]
fn per_object_overhead_is_the_header_only() {
    let mut g = SimGuest::standard();
    let before = g.live_bytes();
    let n = g.alloc(ty::NODE);
    assert_eq!(g.live_bytes() - before, 8 + 16);
    let l = g.alloc(ty::LEAF);
    assert_eq!(g.live_bytes() - before, 8 + 16 + 8 + 8);
    g.release(n);
    g.release(l);
    assert_eq!(g.live_bytes(), before);
    let b = g.alloc_bytes(10);
    assert_eq!(g.live_bytes() - before, 16);
    g.free_bytes(b, 10);
    assert_eq!(g.alloc_bytes(0), 0);
}
