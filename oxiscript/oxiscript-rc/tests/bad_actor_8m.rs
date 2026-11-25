//! BAD ACTOR attack tests for slice 8.M. Each test asserts the CORRECT behaviour.
#![allow(clippy::all, clippy::pedantic, dead_code)]

use oxiscript_rc::heap::Heap;
use oxiscript_rc::rt::Rt;
use oxiscript_rc::sim::{SimHeap, Trap};
use oxiscript_rc::testkit::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// SimHeap that can fail `alloc` on demand (OOM injection).
pub struct FailHeap {
    pub inner: SimHeap,
    pub fail_size: Option<u32>,
    pub rng: Option<(Rng, u64)>,
    just_failed: bool,
    pub failures: u64,
    pub loads: std::cell::Cell<u64>,
}
impl FailHeap {
    pub fn new(arena: usize) -> Self { Self::with_checks(arena, true) }
    pub fn with_checks(arena: usize, checks: bool) -> Self {
        FailHeap { inner: SimHeap::new(arena, checks), fail_size: None, rng: None, just_failed: false, failures: 0, loads: std::cell::Cell::new(0) }
    }
}
impl Heap for FailHeap {
    fn ld32(&self, a: u32) -> u32 { self.loads.set(self.loads.get() + 1); self.inner.ld32(a) }
    fn st32(&mut self, a: u32, v: u32) { self.inner.st32(a, v) }
    fn bytes(&self, a: u32, l: u32) -> &[u8] { self.inner.bytes(a, l) }
    fn bytes_mut(&mut self, a: u32, l: u32) -> &mut [u8] { self.inner.bytes_mut(a, l) }
    fn copy(&mut self, d: u32, s: u32, l: u32) { self.inner.copy(d, s, l) }
    fn alloc(&mut self, size: u32) -> u32 {
        if self.just_failed {
            self.just_failed = false;
            return self.inner.alloc(size);
        }
        if self.fail_size == Some(size) {
            self.fail_size = None;
            self.just_failed = true;
            self.failures += 1;
            return 0;
        }
        if let Some((r, pct)) = self.rng.as_mut() {
            if r.chance(*pct) {
                self.just_failed = true;
                self.failures += 1;
                return 0;
            }
        }
        self.inner.alloc(size)
    }
    fn free(&mut self, a: u32, s: u32) { self.inner.free(a, s) }
    fn trap(&mut self, k: u32) -> ! { self.inner.trap(k) }
}

pub struct FGuest { pub rt: Rt<FailHeap> }
impl FGuest {
    pub fn unchecked() -> Self {
        let mut g = FGuest { rt: Rt::new(FailHeap::with_checks(512 << 20, false)) };
        install_types(&mut g, &standard_types());
        g
    }
    pub fn new() -> Self {
        let mut g = FGuest { rt: Rt::new(FailHeap::new(256 << 20)) };
        install_types(&mut g, &standard_types());
        g
    }
}
impl Guest for FGuest {
    fn init(&mut self, b: u32, c: u32) { self.rt.init(b, c) }
    fn alloc(&mut self, id: u32) -> u32 { self.rt.alloc(id) }
    fn retain(&mut self, p: u32) { self.rt.retain(p) }
    fn release(&mut self, p: u32) { self.rt.release(p) }
    fn clone_obj(&mut self, p: u32) -> u32 { self.rt.clone_obj(p) }
    fn is_unique(&mut self, p: u32) -> bool { self.rt.is_unique(p) == 1 }
    fn collect(&mut self) -> u32 { self.rt.collect_cycles() }
    fn gc_step(&mut self, b: u32) -> u32 { self.rt.gc_step(b) }
    fn stat(&mut self, k: u32) -> u32 { self.rt.stat(k) }
    fn config(&mut self, k: u32, v: u32) { self.rt.config(k, v) }
    fn live_objects(&mut self) -> u32 { self.rt.live_objects() }
    fn live_bytes(&mut self) -> u32 { self.rt.live_bytes() }
    fn rd(&mut self, a: u32) -> u32 { self.rt.heap.ld32(a) }
    fn wr(&mut self, a: u32, v: u32) { self.rt.heap.st32(a, v) }
    fn alloc_bytes(&mut self, n: u32) -> u32 { self.rt.alloc_bytes(n) }
    fn free_bytes(&mut self, p: u32, n: u32) { self.rt.free_bytes(p, n) }
    fn array_new(&mut self, id: u32, len: u32) -> u32 { self.rt.array_new(id, len) }
    fn array_grow(&mut self, a: u32, cap: u32) { self.rt.array_grow(a, cap) }
    fn str_new(&mut self, s: &str) -> u32 {
        let tmp = self.rt.alloc_bytes(s.len().max(1) as u32);
        self.rt.heap.bytes_mut(tmp, s.len() as u32).copy_from_slice(s.as_bytes());
        let r = self.rt.str_new(tmp, s.len() as u32);
        self.rt.free_bytes(tmp, s.len().max(1) as u32);
        r
    }
    fn str_read(&mut self, p: u32) -> String {
        let len = self.rt.heap.ld32(p);
        String::from_utf8(self.rt.heap.bytes(p + 8, len).to_vec()).unwrap()
    }
    fn rd_bytes(&mut self, a: u32, n: u32) -> Vec<u8> { self.rt.heap.bytes(a, n).to_vec() }
    fn wr_bytes(&mut self, a: u32, b: &[u8]) { self.rt.heap.bytes_mut(a, b.len() as u32).copy_from_slice(b) }
}

fn msg(e: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = e.downcast_ref::<String>() { s.clone() }
    else if let Some(s) = e.downcast_ref::<&str>() { s.to_string() }
    else if let Some(t) = e.downcast_ref::<Trap>() { format!("trap {}", t.0) }
    else { "?".into() }
}

/// FINDING 1: OOM drain-and-retry inside `v32_push` re-enters `v32_push` and the
/// outer call resumes with a stale copy of `roots` (double free / UAF).
#[test]
fn f1_oom_retry_in_roots_growth_is_reentrant() {
    let mut g = FGuest::new();
    const NODE: u32 = ty::NODE;
    // 16 candidates -> roots len == cap == 16
    let mut keep = vec![];
    for _ in 0..16 {
        let n = g.alloc(NODE);
        g.retain(n);
        g.release(n); // rc 2 -> 1, buffered
        keep.push(n);
    }
    assert_eq!(g.stat(1), 16);
    // chain H -> N1 -> N2 -> X, X has an extra ref
    let x = g.alloc(NODE); g.retain(x);
    let n2 = g.alloc(NODE); g.wr(n2, x);
    let n1 = g.alloc(NODE); g.wr(n1, n2);
    let h = g.alloc(NODE); g.wr(h, n1);
    let y = g.alloc(NODE); g.retain(y);
    g.config(0, 1);
    g.release(h); // leaves N1 pending
    assert!(g.stat(0) > 0);
    g.rt.heap.fail_size = Some(128);
    let r = catch_unwind(AssertUnwindSafe(|| g.release(y)));
    if let Err(e) = r {
        panic!("release under OOM-retry corrupted the runtime: {}", msg(&e));
    }
    g.config(0, 256);
    g.collect();
    // everything still reachable except H chain remains: keep(16) + x(rc1 left, ours) + y
    assert_eq!(g.stat(1), 0);
}

/// Fuzz every OOM drain-and-retry path with the existing reference model.
#[test]
fn f1b_fuzz_with_random_first_try_alloc_failures() {
    let full = std::env::var_os("OXI_FULL_FUZZ").is_some();
    let seeds = if full { 3000 } else if cfg!(debug_assertions) { 150 } else { 800 };
    let mut bad = vec![];
    for seed in 0..seeds {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut g = FGuest::new();
            g.rt.heap.rng = Some((Rng(seed * 7 + 1), 3));
            fuzz(&mut g, seed, 200);
        }));
        if let Err(e) = r { bad.push((seed, msg(&e))); }
    }
    eprintln!("f1b: {seeds} seeds, {} failing", bad.len());
    assert!(bad.is_empty(), "first failures: {:?}", &bad[..bad.len().min(3)]);
}

fn loads(g: &FGuest) -> u64 { g.rt.heap.loads.get() }

/// FINDING 2: `gc_step(budget)` is documented as bounded (~3*budget visits) but
/// mark_gray checks its limit only after finishing a node's whole slot list.
#[test]
fn f2_gc_step_is_not_bounded_by_budget_on_a_big_array() {
    let mut g = FGuest::unchecked();
    let n = 200_000;
    let a = g.array_new(ty::REFARRAY, n);
    g.retain(a);
    g.release(a); // purple candidate
    assert_eq!(g.stat(1), 1);
    let before = loads(&g);
    let budget = 16;
    g.gc_step(budget);
    let used = loads(&g) - before;
    assert!(used < 200 * u64::from(budget), "gc_step({budget}) performed {used} heap loads for one {n}-slot array");
}

/// The same candidate is retried (and re-undone) on every sweep: cost per sweep.
#[test]
fn f2b_repeated_steps_redo_the_same_oversized_trial() {
    let mut g = FGuest::unchecked();
    let a = g.array_new(ty::REFARRAY, 100_000);
    g.retain(a);
    g.release(a);
    let mut total = 0;
    for _ in 0..10 {
        let b = loads(&g);
        g.gc_step(16);
        total += loads(&g) - b;
    }
    assert!(total < 10 * 200 * 16, "10 tiny steps cost {total} loads");
}

/// FINDING 3: a pointer-free array is walked element by element on destruction.
#[test]
fn f3_scalar_array_drop_is_o_1() {
    let mut g = FGuest::unchecked();
    let a = g.array_new(ty::SCALARARRAY, 1_000_000);
    g.release(a);
    assert_eq!(g.stat(0), 0, "pointer-free array still queued after release (drain walks len elements)");
}

/// Zero-size elements: no memory, but `len` up to 2^31-1 is walked on drop.
#[test]
fn f3b_zero_sized_element_array_drop_burns_fuel() {
    let mut specs = standard_types();
    specs.push(TypeSpec { kind: 5, size: 0, acyclic: true, ptrs: vec![] }); // 25
    let mut g = FGuest { rt: Rt::new(FailHeap::with_checks(64 << 20, false)) };
    install_types(&mut g, &specs);
    let a = g.array_new(25, 0x7FFF_FFFF);
    let t = std::time::Instant::now();
    g.release(a);
    g.live_objects();
    assert!(t.elapsed().as_millis() < 200, "dropping a 2^31 zero-size array took {:?}", t.elapsed());
}

// ---------------------------------------------------------------- F4 descriptors

/// Build a guest with one extra malformed type (id 25) appended to the standard table.
/// Returns Ok(outcome description) or Err(init trapped = clean rejection).
fn descriptor_outcome(bad: TypeSpec, raw_hook: impl FnOnce(&mut FGuest, u32), _use_id: u32) -> String {
    let mut specs = standard_types();
    specs.push(bad);
    let specs2 = specs.clone();
    let r = catch_unwind(AssertUnwindSafe(|| {
        let specs = specs2;
        let mut g = FGuest { rt: Rt::new(FailHeap::with_checks(64 << 20, true)) };
        install_types(&mut g, &specs);
        raw_hook(&mut g, 25);
        let p = if specs[25].kind == 5 { g.array_new(25, 3) } else { g.alloc(25) };
        let q = g.alloc(ty::NODE);
        g.wr(q, p); // q -> p
        g.release(q);
        g.collect();
        g.live_objects()
    }));
    match r {
        Ok(n) => format!("accepted; live={n}"),
        Err(e) => {
            let m = msg(&e);
            if m.starts_with("trap") { format!("clean:{m}") } else { format!("CORRUPT:{m}") }
        }
    }
}

/// Malformed descriptors must be rejected at `oxi_rt_init` or trap cleanly, never
/// touch memory outside the object. (Reports a table of outcomes.)
#[test]
fn f4_malformed_descriptors_are_rejected_or_trap_cleanly() {
    let rec = |kind, size, ptrs: Vec<u32>| TypeSpec { kind, size, acyclic: false, ptrs };
    let cases: Vec<(&str, TypeSpec, u32)> = vec![
        ("slot offset beyond payload", rec(1, 16, vec![64]), 25),
        ("slot offset huge", rec(1, 16, vec![0xFFFF_FFF0]), 25),
        ("unaligned slot offset", rec(1, 16, vec![2]), 25),
        ("duplicate slot offsets", rec(1, 16, vec![0, 0]), 25),
        ("slot overlapping header-size (offset 14, size 16)", rec(1, 16, vec![14]), 25),
        ("array elem smaller than slot offset+4", rec(5, 4, vec![8]), 25),
        ("unknown kind 77", rec(77, 16, vec![]), 25),
        ("kind 0 (hole)", rec(0, 16, vec![]), 25),
        ("string kind used as user type", rec(4, 0, vec![]), 25),
        ("record size huge (0xFFFFFFF8)", rec(1, 0xFFFF_FFF8, vec![]), 25),
        ("record size 0xFFFFFFFF (align overflow)", rec(1, 0xFFFF_FFFF, vec![]), 25),
        ("array elem size 0xFFFFFFFF", rec(5, 0xFFFF_FFFF, vec![]), 25),
    ];
    let mut bad = vec![];
    for (name, spec, id) in cases {
        let out = descriptor_outcome(spec, |_, _| {}, id);
        eprintln!("f4 {name:55} => {out}");
        if !out.starts_with("clean") { bad.push(format!("{name}: {out}")); }
    }
    assert!(bad.is_empty(), "descriptor handling:\n{}", bad.join("\n"));
}

/// count > 2^24: ids >= 2^24 must be rejected (trap). Release builds wrap the
/// u32 address arithmetic, so we make the lookup of id 0x0100_0010 land on the
/// real NODE entry; `new_object` then ORs the id into the colour bits.
#[test]
fn f4b_type_id_beyond_24_bits_is_rejected() {
    if cfg!(debug_assertions) { return; } // debug build: u32 overflow panic instead of wrap
    let specs = standard_types();
    let mut g = FGuest { rt: Rt::new(FailHeap::with_checks(64 << 20, true)) };
    // replicate install_types to learn the table base
    let base = g.alloc_bytes(specs.len() as u32 * 16);
    install_types(&mut g, &specs); // second table (valid); we only use the first copy below
    for (id, sp) in specs.iter().enumerate() {
        let a = base + id as u32 * 16;
        let pmap = if sp.ptrs.is_empty() { 0 } else { g.alloc_bytes(sp.ptrs.len() as u32 * 4) };
        for (k, off) in sp.ptrs.iter().enumerate() { g.wr(pmap + 4 * k as u32, *off); }
        if sp.kind != 0 {
            g.wr(a, sp.size);
            g.wr(a + 4, sp.kind | (u32::from(sp.acyclic) << 8) | ((sp.ptrs.len() as u32) << 16));
            g.wr(a + 8, pmap);
        }
    }
    let id: u32 = 0x0100_0000 + ty::NODE;
    // CORRECTED (8.M fixes): init itself now rejects count > 2^24 with a trap, so it
    // must run inside the catch; a trap at init is the clean rejection this test wants.
    let r = catch_unwind(AssertUnwindSafe(|| {
        g.rt.init(base.wrapping_add(ty::NODE * 16).wrapping_sub(id.wrapping_mul(16)), 0x0200_0000);
        let p = g.alloc(id);
        (p, g.rd(p - 4))
    }));
    match r {
        Err(e) if msg(&e).starts_with("trap") => {}
        Ok((_, meta)) => panic!("alloc(0x{id:x}) accepted; meta={meta:#x} (type id aliases colour bits: colour {})", (meta >> 24) & 7),
        Err(e) => panic!("alloc(0x{id:x}) -> non-trap failure {}", msg(&e)),
    }
}

// Duplicate pointer-map offsets double-release one child: a still-referenced
/// object is freed. (Silent wrong free, caught only by validating at init.)
#[test]
fn f4d_duplicate_ptr_map_offsets_cause_wrong_free() {
    let mut specs = standard_types();
    specs.push(TypeSpec { kind: 1, size: 16, acyclic: false, ptrs: vec![0, 0] }); // 25
    let mut g = FGuest { rt: Rt::new(FailHeap::with_checks(64 << 20, true)) };
    let r = catch_unwind(AssertUnwindSafe(|| install_types(&mut g, &specs)));
    if r.is_err() { return; } // rejecting at init is the correct behaviour
    let child = g.alloc(ty::NODE);
    g.retain(child); // we keep one ref (rc 2), parent holds the other
    let parent = g.alloc(25);
    g.wr(parent, child);
    let r = catch_unwind(AssertUnwindSafe(|| { g.release(parent); g.rt.live_objects() }));
    match r {
        Ok(live) => assert_eq!(live, 1, "child freed or queue corrupted although we still hold a reference"),
        Err(e) => panic!("duplicate slot offsets corrupt runtime: {}", msg(&e)),
    }
    assert_eq!(g.rd(child - 8), 1, "child rc should be 1");
}

// ---------------------------------------------------------------- F5 wasm (release/debug)
mod common;
use common::{build_wasm, wasm_available, WasmGuest};

/// Returns Err(description) if the guest trapped or objects overlap afterwards.
fn stale_release_probe(path: &std::path::PathBuf, victim_releases: u32) -> Result<(), String> {
    let mut g = WasmGuest::standard(path);
    let a = g.alloc(ty::NODE);
    let guard = g.alloc(ty::NODE);
    g.wr(guard + 12, 0xABCD_0001);
    g.release(a);
    for i in 0..victim_releases {
        g.try_call("oxi_release", &[a]).map_err(|e| format!("stale release #{i} trapped: {e}"))?;
    }
    let mut objs = vec![];
    for i in 0..300u32 {
        let p = g.try_call("oxi_alloc", &[ty::NODE]).map_err(|e| format!("alloc #{i} trapped after stale release: {e}"))?;
        g.wr(p + 12, 0x1000 + i);
        objs.push(p);
    }
    for (i, &p) in objs.iter().enumerate() {
        if g.rd(p + 12) != 0x1000 + i as u32 { return Err(format!("object {i} overlapped by a later allocation")); }
        if p == guard { return Err("allocator handed out the live guard object".into()); }
    }
    if g.rd(guard + 12) != 0xABCD_0001 { return Err("live guard object clobbered".into()); }
    let mut s = objs.clone(); s.sort(); s.dedup();
    if s.len() != objs.len() { return Err("duplicate addresses handed out (free list corrupted)".into()); }
    g.try_call("oxi_collect_cycles", &[]).map_err(|e| format!("collect trapped: {e}"))?;
    Ok(())
}

/// What a buggy compiler's double release does in the RELEASE guest: report, assert it is contained.
#[test]
fn f5_double_release_release_build_must_not_corrupt_allocator() {
    if !wasm_available() { return; }
    let path = build_wasm("release");
    let r = stale_release_probe(&path, 1);
    eprintln!("f5 release-build double release: {r:?}");
    // CORRECTED (8.M fixes): the lead decision is that release builds stay as they are
    // (the RC-balance checker is the defence). A trap anywhere in the probe is therefore
    // acceptable: the instance dies and nothing escapes it. What must never happen is
    // *silent* corruption: overlap, duplicate addresses, a clobbered live object.
    if let Err(e) = &r {
        assert!(e.contains("trapped"), "silent corruption after a stale release: {e}");
    }
}

/// lib.rs claims debug poisoning detects stale releases.
#[test]
fn f5b_double_release_debug_build_must_trap() {
    if !wasm_available() { return; }
    let path = build_wasm("dev");
    let mut g = WasmGuest::standard(&path);
    let a = g.alloc(ty::NODE);
    let _guard = g.alloc(ty::NODE);
    g.release(a);
    let r = g.try_call("oxi_release", &[a]);
    assert!(r.is_err(), "debug build silently accepted a release of a freed (0xDD-poisoned) object");
}

#[test]
fn f5c_release_of_non_object_pointers_stays_in_sandbox() {
    if !wasm_available() { return; }
    let path = build_wasm("release");
    for bad in [1u32, 8, 100, 1024, 4096, 0x00FF_FFFF, 0xFFFF_FFF8, 0xFFFF_FFFF] {
        let mut g = WasmGuest::standard(&path);
        let r = g.try_call("oxi_release", &[bad]);
        eprintln!("f5c release({bad:#x}) -> {}", if r.is_ok() { "ok(silent)" } else { "trap" });
        let r2 = g.try_call("oxi_retain", &[bad]);
        eprintln!("f5c retain({bad:#x}) -> {}", if r2.is_ok() { "ok(silent)" } else { "trap" });
    }
}

// ---------------------------------------------------------------- F6 hostile host API
#[test]
fn f6_gc_step_zero_budget_makes_progress_or_reports_done() {
    let mut g = FGuest::new();
    let n = g.alloc(ty::NODE);
    g.retain(n);
    g.release(n);
    // a host that loops `while step(..) & 1` with budget 0 must not spin forever
    let mut calls = 0;
    while g.gc_step(0) & 1 != 0 {
        calls += 1;
        assert!(calls < 1000, "gc_step(0) returns STEP_MORE forever with a candidate pending (host loop hangs)");
    }
}

#[test]
fn f6b_extreme_budgets_and_config_values_hold() {
    let mut g = FGuest::new();
    let mut ring = vec![];
    for _ in 0..50 {
        let a = g.alloc(ty::NODE);
        let b = g.alloc(ty::NODE);
        g.retain(b); g.wr(a, b);
        g.retain(a); g.wr(b, a);
        g.release(a); g.release(b);
        ring.push(a);
    }
    for b in [u32::MAX, 0x8000_0000, 1, u32::MAX - 1] {
        let _ = g.gc_step(b);
    }
    for (k, v) in [(0, 0), (0, u32::MAX), (1, 0), (1, u32::MAX), (99, 5), (u32::MAX, u32::MAX)] {
        g.config(k, v);
    }
    for k in [7, 8, 1000, u32::MAX] { assert_eq!(g.stat(k), 0, "unknown stat key {k}"); }
    g.collect();
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn f6c_exports_before_init_trap_cleanly_or_work() {
    let mut g = FGuest { rt: Rt::new(FailHeap::new(64 << 20)) };
    let r = catch_unwind(AssertUnwindSafe(|| {
        let s = g.str_new("hi");
        g.release(s);
        let b = g.alloc_bytes(100);
        g.free_bytes(b, 100);
        g.collect();
        g.gc_step(10);
        g.live_objects()
    }));
    assert_eq!(r.map_err(|e| msg(&e)), Ok(0));
    let r = catch_unwind(AssertUnwindSafe(|| g.alloc(16)));
    assert!(r.is_err() && msg(&r.unwrap_err()).starts_with("trap"), "alloc before init must trap");
}

// ---------------------------------------------------------------- F7 allocation edges
#[test]
fn f7_size_overflow_paths_trap_not_wrap() {
    let mut g = FGuest::new();
    for sz in [u32::MAX, u32::MAX - 7, u32::MAX - 8, 0x8000_0000, 0xFFFF_FFF0] {
        let r = catch_unwind(AssertUnwindSafe(|| g.alloc_bytes(sz)));
        match r { Err(e) if msg(&e).starts_with("trap") => {}, other => panic!("alloc_bytes({sz:#x}) -> {:?}", other.map_err(|e| msg(&e))) }
    }
    for len in [0x7FFF_FFFFu32, 0x4000_0000, 0x2000_0000, 0x8000_0000, u32::MAX] {
        let r = catch_unwind(AssertUnwindSafe(|| g.array_new(ty::INLINEARRAY, len)));
        match r { Err(e) if msg(&e).starts_with("trap") => {}, other => panic!("array_new(inline,{len:#x}) -> {:?}", other.map_err(|e| msg(&e))) }
    }
    let a = g.array_new(ty::REFARRAY, 2);
    for cap in [0x7FFF_FFFFu32, 0x8000_0000, u32::MAX, 0x2000_0000] {
        let r = catch_unwind(AssertUnwindSafe(|| g.array_grow(a, cap)));
        match r { Err(e) if msg(&e).starts_with("trap") => {}, other => panic!("array_grow({cap:#x}) -> {:?}", other.map_err(|e| msg(&e))) }
    }
}

#[test]
fn f7b_string_concat_and_slice_edges() {
    let mut g = FGuest::new();
    let s = g.str_new("h\u{e9}llo"); // h=0 e-acute=1..3 l=3 l=4 o=5, len 6
    let trap_kind = |g: &mut FGuest, a: u32, b: u32| -> Option<u32> {
        match catch_unwind(AssertUnwindSafe(|| g.rt.str_slice(s, a, b))) {
            Ok(_) => None,
            Err(e) => e.downcast_ref::<Trap>().map(|t| t.0),
        }
    };
    assert_eq!(trap_kind(&mut g, 0, 6), None);
    assert_eq!(trap_kind(&mut g, 1, 3), None);
    assert_eq!(trap_kind(&mut g, 6, 6), None);
    assert_eq!(trap_kind(&mut g, 2, 3), Some(9), "start inside a code point");
    assert_eq!(trap_kind(&mut g, 1, 2), Some(9), "end inside a code point");
    assert_eq!(trap_kind(&mut g, 3, 2), Some(4), "start > end");
    assert_eq!(trap_kind(&mut g, 0x8000_0000, 1), Some(4), "negative start");
    assert_eq!(trap_kind(&mut g, 0, u32::MAX), Some(4), "end past len");
    assert_eq!(trap_kind(&mut g, u32::MAX, 0), Some(4));
    let empty = g.str_new("");
    let c = g.rt.str_concat(empty, empty);
    assert_eq!(g.str_read(c), "");
    assert_eq!(g.rt.str_eq(c, empty), 1);
}

#[test]
fn f7c_f64_printing_is_deterministic_roundtrip_and_specified() {
    let mut g = FGuest::new();
    let mut specials = vec![0.0f64, -0.0, 1.0, -1.0, f64::MIN_POSITIVE, 5e-324, f64::MAX, f64::MIN, 1e15, 1e16, 1e21, 1e-7, 0.1 + 0.2, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 123456789012345680.0];
    let mut r = Rng(42);
    for _ in 0..20000 { specials.push(f64::from_bits(r.next())); }
    for v in specials {
        let p = g.rt.str_from_f64(v);
        let s = g.str_read(p);
        g.release(p);
        if v.is_nan() { assert_eq!(s, "NaN"); continue; }
        if v.is_infinite() { assert_eq!(s, if v > 0.0 { "Infinity" } else { "-Infinity" }); continue; }
        let back: f64 = s.parse().unwrap_or_else(|_| panic!("unparsable {s:?} for {v:e}"));
        assert_eq!(back.to_bits(), v.to_bits(), "{v:e} printed as {s:?} does not round-trip (sign of zero?)");
        assert!(!s.contains('+'), "{s}");
    }
}

#[test]
fn f7d_i64_extremes() {
    let mut g = FGuest::new();
    for v in [i64::MIN, i64::MAX, 0, -1, 10, -10] {
        let p = g.rt.str_from_i64(v);
        assert_eq!(g.str_read(p), v.to_string());
    }
}

#[test]
fn f7e_wasm_alignment_and_huge_alloc_trap() {
    if !wasm_available() { return; }
    let path = build_wasm("release");
    let mut g = WasmGuest::standard(&path);
    for _ in 0..200 {
        let p = g.alloc(ty::NODE);
        assert_eq!(p % 8, 0);
        let b = g.alloc_bytes(13);
        assert_eq!(b % 8, 0);
        let s = g.call_vals("oxi_str_from_i64", &[wasmi::Val::I64(12345)]).unwrap()[0].i32().unwrap() as u32;
        assert_eq!(s % 8, 0);
    }
    for sz in [u32::MAX, 0xFFFF_FFF9u32, 0xFFFF_FFF0] {
        let mut g = WasmGuest::standard(&path);
        let r = g.try_call("oxi_alloc_bytes", &[sz]);
        assert!(r.is_err(), "alloc_bytes({sz:#x}) returned {r:?} (memory limit not enforced?)");
    }
}

// ---------------------------------------------------------------- F8 extended model fuzz
const GREENREC: u32 = 25; // acyclic { ref } (may point at cyclic types)
const ARRNODE: u32 = 26; // array of inline {ref, ref}
fn ext_types() -> Vec<TypeSpec> {
    let mut v = standard_types();
    v.push(TypeSpec { kind: 1, size: 8, acyclic: true, ptrs: vec![0] });
    v.push(TypeSpec { kind: 5, size: 8, acyclic: false, ptrs: vec![0, 4] });
    v
}
fn is_green(t: u32) -> bool { matches!(t, ty::STRING | ty::LEAF | ty::SCALARARRAY | ty::GREENBOX | GREENREC) }

struct O { rt: u32, ty: u32, slots: Vec<Option<usize>>, immortal: bool }
struct M { objs: Vec<O>, roots: Vec<usize> }
impl M {
    fn reach_from(&self, start: &[usize]) -> Vec<bool> {
        let mut seen = vec![false; self.objs.len()];
        let mut st: Vec<usize> = start.to_vec();
        while let Some(n) = st.pop() {
            if seen[n] { continue; }
            seen[n] = true;
            st.extend(self.objs[n].slots.iter().flatten());
        }
        seen
    }
    fn live(&self) -> Vec<bool> {
        let mut s = self.roots.clone();
        s.extend((0..self.objs.len()).filter(|&i| self.objs[i].immortal));
        self.reach_from(&s)
    }
}
fn saddr(g: &mut impl Guest, o: &O, i: usize) -> u32 {
    match o.ty {
        ty::NODE | ty::WIDE => o.rt + 4 * i as u32,
        GREENREC | ty::GREENBOX => o.rt,
        ty::VARIANT => o.rt + 4,
        ty::CLOSURE => o.rt + 8 + 4 * i as u32,
        ty::REFARRAY => g.rd(o.rt + 8) + 4 * i as u32,
        ARRNODE => g.rd(o.rt + 8) + 4 * i as u32 + if i % 2 == 0 { 0 } else { 0 }, // two refs per 8-byte elem, contiguous
        ty::INLINEARRAY => g.rd(o.rt + 8) + 8 * i as u32 + 4,
        _ => unreachable!(),
    }
}
fn nslots(t: u32, n: usize) -> usize {
    match t { ty::NODE | ty::CLOSURE => 2, ty::WIDE => 8, GREENREC | ty::GREENBOX | ty::VARIANT => 1, ty::REFARRAY | ty::INLINEARRAY => n, ARRNODE => 2 * n, _ => 0 }
}

/// Check invariants. `exact`: after a full collect (rc exact, live == reachable).
fn ext_check(g: &mut impl Guest, m: &M, exact: bool, ctx: &str) {
    let live = m.live();
    let mut want = vec![0u32; m.objs.len()];
    for &r in &m.roots { want[r] += 1; }
    for (i, o) in m.objs.iter().enumerate() {
        if !live[i] { continue; }
        for c in o.slots.iter().flatten() { want[*c] += 1; }
    }
    let mut n = 0;
    for (i, o) in m.objs.iter().enumerate() {
        if !live[i] { continue; }
        n += 1;
        for (j, c) in o.slots.iter().enumerate() {
            let a = saddr(g, o, j);
            assert_eq!(g.rd(a), c.map_or(0, |c| m.objs[c].rt), "{ctx}: slot {j} of ty {} clobbered", o.ty);
        }
        let meta = g.rd(o.rt - 4);
        assert_eq!(meta & 0xFF_FFFF, o.ty, "{ctx}: type id");
        assert!(((meta >> 24) & 7) <= 4, "{ctx}: live object with internal colour");
        let rc = g.rd(o.rt - 8);
        if o.immortal { assert!(rc >= 0x8000_0000, "{ctx}: sticky lost"); continue; }
        assert!(rc >= want[i].max(1), "{ctx}: rc {rc} < required {} (ty {}) - premature decrement / bad undo", want[i], o.ty);
        if exact { assert_eq!(rc, want[i], "{ctx}: rc not exact after collect (ty {})", o.ty); }
    }
    if exact {
        assert_eq!(g.live_objects() as usize, n + 0, "{ctx}: live set != reachable set");
    }
}

pub fn ext_fuzz(g: &mut impl Guest, seed: u64, ops: usize) -> (u64, u64, u64) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut m = M { objs: vec![], roots: vec![] };
    let base = g.live_objects();
    let (mut steps, mut aborted) = (0u64, 0u64);
    let kinds = [ty::NODE, ty::NODE, ty::NODE, ty::WIDE, GREENREC, GREENREC, ty::GREENBOX, ty::CLOSURE, ty::VARIANT, ty::REFARRAY, ty::INLINEARRAY, ARRNODE, ty::LEAF, ty::SCALARARRAY, ty::STRING];
    for op in 0..ops {
        let live: Vec<usize> = { let l = m.live(); (0..m.objs.len()).filter(|&i| l[i]).collect() };
        let pick = |rng: &mut Rng| if live.is_empty() { None } else { Some(live[rng.below(live.len())]) };
        match rng.below(100) {
            0..=24 => {
                let t = kinds[rng.below(kinds.len())];
                let n = rng.below(4);
                let p = match t {
                    ty::STRING => g.str_new("xyz"),
                    ty::REFARRAY | ty::INLINEARRAY | ARRNODE | ty::SCALARARRAY => g.array_new(t, n as u32),
                    _ => g.alloc(t),
                };
                m.objs.push(O { rt: p, ty: t, slots: vec![None; nslots(t, n)], immortal: false });
                m.roots.push(m.objs.len() - 1);
            }
            25..=32 => if let Some(o) = pick(&mut rng) { if m.roots.contains(&o) || rng.chance(30) {
                // retain via a live object we hold
                if m.roots.contains(&o) { g.retain(m.objs[o].rt); m.roots.push(o); }
            } },
            33..=48 => if !m.roots.is_empty() {
                let i = rng.below(m.roots.len());
                let o = m.roots.swap_remove(i);
                g.release(m.objs[o].rt);
            },
            49..=78 => {
                let (Some(s), Some(d)) = (pick(&mut rng), pick(&mut rng)) else { continue };
                if m.objs[s].slots.is_empty() { continue; }
                // preserve the ACYCLIC invariant: no green object may lie on a cycle
                let reach_d = m.reach_from(&[d]);
                if reach_d[s] {
                    let co: Vec<bool> = (0..m.objs.len()).map(|x| m.reach_from(&[x])[s]).collect();
                    if (0..m.objs.len()).any(|x| reach_d[x] && co[x] && is_green(m.objs[x].ty)) { continue; }
                }
                let j = rng.below(m.objs[s].slots.len());
                let a = saddr(g, &m.objs[s], j);
                let old = g.rd(a);
                let dst = if rng.chance(8) { None } else { Some(d) };
                if let Some(d) = dst { g.retain(m.objs[d].rt); }
                g.wr(a, dst.map_or(0, |d| m.objs[d].rt));
                g.release(old);
                m.objs[s].slots[j] = dst;
            }
            79..=82 => if let Some(&r) = m.roots.get(rng.below(m.roots.len().max(1))) {
                let src = &m.objs[r];
                let p = g.clone_obj(src.rt);
                let (t, sl) = (src.ty, src.slots.clone());
                m.objs.push(O { rt: p, ty: t, slots: sl, immortal: false });
                m.roots.push(m.objs.len() - 1);
            },
            83..=85 => if let Some(o) = pick(&mut rng) { // saturate ANY live object (inside cycles too)
                if !m.objs[o].immortal && !matches!(m.objs[o].ty, ty::STRING) {
                    g.wr(m.objs[o].rt - 8, 0x7FFF_FFFF);
                    g.retain(m.objs[o].rt);
                    m.objs[o].immortal = true;
                }
            },
            86..=94 => {
                let b = [1u32, 2, 3, 5, 9, 17, 40][rng.below(7)];
                let before = g.stat(6);
                for _ in 0..rng.below(4) + 1 {
                    let r = g.gc_step(b);
                    steps += 1;
                    if r & 1 == 0 { break; }
                }
                aborted += u64::from(g.stat(6) - before);
                ext_check(g, &m, false, &format!("seed {seed} op {op} after gc_step"));
            }
            95..=97 => g.config(0, [1, 2, 64, 256][rng.below(4)]),
            _ => { g.collect(); ext_check(g, &m, true, &format!("seed {seed} op {op} after collect")); }
        }
    }
    while let Some(o) = m.roots.pop() { g.release(m.objs[o].rt); }
    g.collect();
    ext_check(g, &m, true, &format!("seed {seed} teardown"));
    let live = m.live();
    let imm = (0..m.objs.len()).filter(|&i| live[i]).count() as u32;
    assert_eq!(g.live_objects() - base, imm, "seed {seed}: leak at teardown");
    (steps, aborted, u64::from(g.stat(3)))
}

#[test]
fn f8_extended_shapes_match_reference_model() {
    let full = std::env::var_os("OXI_FULL_FUZZ").is_some();
    let seeds: u64 = if full { 20_000 } else if cfg!(debug_assertions) { 300 } else { 3000 };
    let (mut steps, mut aborts, mut fails, mut freed) = (0, 0, vec![], 0);
    for seed in 0..seeds {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let mut g = SimGuest::new(256 << 20, true);
            install_types(&mut g, &ext_types());
            ext_fuzz(&mut g, seed, 250)
        }));
        match r { Ok((s, a, f)) => { steps += s; aborts += a; freed += f; } Err(e) => fails.push((seed, msg(&e))) }
    }
    eprintln!("f8: {seeds} seeds x 250 ops, {steps} gc_steps, {aborts} aborted trials, {freed} objects cycle-freed, {} failing", fails.len());
    assert!(fails.is_empty(), "first failures: {:?}", &fails[..fails.len().min(4)]);
}

// ---------------------------------------------------------------- F9 performance amplification
/// Many candidates share one big live subgraph. Every `gc_step` trial re-marks
/// the whole shared subgraph; a guest pays 2 cheap ops (retain+release) per
/// candidate and the collector pays O(subgraph): amplification = subgraph size.
#[test]
fn f9_candidate_churn_over_a_shared_live_graph_amplifies_collector_work() {
    let mut g = FGuest::unchecked();
    let n = 20_000;
    let mut head = 0;
    for _ in 0..n { let x = g.alloc(ty::NODE); g.wr(x, head); head = x; }
    let k = 300;
    let mut holders = vec![];
    for _ in 0..k { let h = g.alloc(ty::NODE); g.retain(head); g.wr(h, head); holders.push(h); }
    g.release(head); // holders own it now
    for &h in &holders { g.retain(h); g.release(h); } // candidates
    let before = loads(&g);
    while g.gc_step(u32::MAX) & 1 != 0 {}
    let cost = loads(&g) - before;
    let guest_ops = 2 * k as u64;
    eprintln!("f9: {guest_ops} guest ops -> {cost} collector loads (x{} per op)", cost / guest_ops);
    // sanity: single full collect over the same candidates is linear
    for &h in &holders { g.retain(h); g.release(h); }
    let b2 = loads(&g);
    g.collect();
    let one = loads(&g) - b2;
    eprintln!("f9: full collect over same candidates: {one} loads");
    assert!(one < 20 * n as u64 * 4, "collect_cycles superlinear: {one}");
    assert!(cost / guest_ops < 1000, "amplification x{} per guest op", cost / guest_ops);

}

// ---------------------------------------------------------------- F5d probe, F10 exact undo, F11 f64 native vs wasm
/// Probe what a stale release actually reads: print rc/meta words of a freed object.
#[test]
fn f5d_probe_stale_header_words() {
    if !wasm_available() { return; }
    for prof in ["release", "dev"] {
        let path = build_wasm(prof);
        let mut g = WasmGuest::standard(&path);
        let a = g.alloc(ty::NODE);
        let _guard = g.alloc(ty::NODE);
        g.release(a);
        eprintln!("f5d {prof}: freed object header rc={:#x} meta={:#x}", g.rd(a - 8), g.rd(a - 4));
    }
}

/// Exact undo: a garbage-free graph (everything held) with every node buffered;
/// for every budget b, gc_step(b) calls must leave every rc exactly as before.
#[test]
fn f10_gc_step_undo_is_exact_at_every_budget() {
    let seeds = if cfg!(debug_assertions) { 10 } else { 40 };
    let mut total_aborts = 0u64;
    for seed in 0..seeds {
        for b in (1..=64u32).chain([100, 150, 400]) {
            let mut g = SimGuest::new(64 << 20, true);
            install_types(&mut g, &ext_types());
            let mut rng = Rng(seed * 31 + 7);
            let mut objs: Vec<(u32, u32)> = vec![]; // (ptr, ty)
            for _ in 0..24 {
                let t = [ty::NODE, ty::NODE, ty::WIDE, GREENREC, ty::REFARRAY, ARRNODE, ty::CLOSURE][rng.below(7)];
                let p = match t { ty::REFARRAY | ARRNODE => g.array_new(t, 1 + rng.below(3) as u32), _ => g.alloc(t) };
                objs.push((p, t));
            }
            // random links (green records only point forward => no cycle through them)
            for i in 0..objs.len() {
                let (p, t) = objs[i];
                let slots = ptr_slot_addrs_ext(&mut g, p, t);
                for a in slots {
                    if rng.chance(70) {
                        let j = rng.below(objs.len());
                        if t == GREENREC && (objs[j].1 == GREENREC || j <= i) { continue; }
                        if objs[j].1 == GREENREC && j < i { /* edge into green from earlier node ok */ }
                        if objs[j].1 == GREENREC && t != GREENREC && j <= i { }
                        let (d, dt) = objs[j];
                        // avoid cycles through green records: green may only be pointed at by lower index and point to higher
                        if dt == GREENREC && j <= i && false { continue; }
                        g.retain(d);
                        g.wr(a, d);
                    }
                }
            }
            for &(p, _) in &objs { g.retain(p); g.release(p); } // buffer all
            let snap: Vec<u32> = objs.iter().map(|&(p, _)| g.rd(p - 8)).collect();
            let live = g.live_objects();
            for call in 0..6 {
                g.gc_step(b);
                for (k, &(p, _)) in objs.iter().enumerate() {
                    assert_eq!(g.rd(p - 8), snap[k], "seed {seed} budget {b} call {call}: rc of obj {k} changed");
                    assert_eq!(g.rd(p - 4) & 0xFF_FFFF, objs[k].1);
                }
                assert_eq!(g.live_objects(), live, "seed {seed} budget {b}: object freed though everything is held");
            }
            total_aborts += u64::from(g.stat(6));
            g.collect();
            for (k, &(p, _)) in objs.iter().enumerate() { assert_eq!(g.rd(p - 8), snap[k], "seed {seed} b {b}: after collect, obj {k}"); }
        }
    }
    eprintln!("f10: {total_aborts} aborted trials checked for exact undo");
    assert!(total_aborts > 1000, "undo path barely exercised");
}
fn ptr_slot_addrs_ext(g: &mut SimGuest, p: u32, t: u32) -> Vec<u32> {
    if t == GREENREC { return vec![p]; }
    if t == ARRNODE {
        let len = g.rd(p); let data = g.rd(p + 8);
        return (0..len * 2).map(|i| data + 4 * i).collect();
    }
    ptr_slot_addrs(g, p)
}

#[test]
fn f11_f64_printing_native_equals_wasm() {
    if !wasm_available() { return; }
    let path = build_wasm("release");
    let mut w = WasmGuest::standard(&path);
    let mut n = FGuest::new();
    let mut vals = vec![0.0f64, -0.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN_POSITIVE, 5e-324, 1e16, 1e21, 1e-7, 0.1 + 0.2, 123456789012345680.0];
    let mut r = Rng(9);
    for _ in 0..20000 { vals.push(f64::from_bits(r.next())); }
    for v in vals {
        let pw = w.call_vals("oxi_str_from_f64", &[wasmi::Val::F64(v.into())]).unwrap()[0].i32().unwrap() as u32;
        let sw = w.str_read(pw);
        let pn = n.rt.str_from_f64(v);
        let sn = n.str_read(pn);
        assert_eq!(sw, sn, "bits {:#x}", v.to_bits());
        w.release(pw); n.release(pn);
    }
}
