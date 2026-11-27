//! BAD ACTOR 8.M-link round 2: the real wasm32 runtime (with the new `SmallPool` size-class
//! allocator in front of dlmalloc) under wasmi. A FAILING test is a finding (`BL2-nn` in
//! docs/stages/8.M-link/bad-actor-2.md); passing tests document what held.
#![allow(clippy::all, clippy::pedantic, dead_code)]

mod common;

use common::{build_wasm, wasm_available, WasmGuest};
use oxiscript_rc::testkit::{fuzz, ty, Guest, Rng};
use std::collections::BTreeMap;
use std::sync::OnceLock;

fn release_wasm() -> &'static std::path::PathBuf {
    static P: OnceLock<std::path::PathBuf> = OnceLock::new();
    P.get_or_init(|| build_wasm("release"))
}
fn dev_wasm() -> &'static std::path::PathBuf {
    static P: OnceLock<std::path::PathBuf> = OnceLock::new();
    // `OXI_BA_DEV_WASM=<path>` runs the dev-profile tests against another build (used to compare
    // with the pre-SmallPool runtime of da87be6).
    P.get_or_init(|| std::env::var_os("OXI_BA_DEV_WASM").map_or_else(|| build_wasm("dev"), std::path::PathBuf::from))
}

/// Byte-block allocator fuzz against the real allocator: random `alloc_bytes`/`free_bytes` with
/// sizes around every size-class edge (1..=264, the 256/257 boundary, slab-sized 4096 and large
/// blocks). Every live block carries a unique byte pattern that is re-verified; every new block
/// must be 8-aligned, zeroed over its whole requested size and disjoint from every live block.
fn byte_fuzz(path: &std::path::PathBuf, seed: u64, ops: usize) -> (usize, usize) {
    let mut g = WasmGuest::standard(path);
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut live: BTreeMap<u32, (u32, u8)> = BTreeMap::new();
    let mut allocs = 0;
    let mut max_live = 0;
    for step in 0..ops {
        let r = rng.below(100);
        if r < 55 || live.is_empty() {
            let size = match rng.below(10) {
                0..=5 => 1 + rng.below(264) as u32,
                6 => 248 + rng.below(17) as u32,
                7 => 4088 + rng.below(17) as u32,
                8 => 1 + rng.below(4096) as u32,
                _ => 65_000 + rng.below(70_000) as u32,
            };
            let p = g.alloc_bytes(size);
            assert_ne!(p, 0, "seed {seed} step {step}: alloc_bytes({size}) returned 0");
            assert_eq!(p % 8, 0, "seed {seed} step {step}: alloc_bytes({size}) = {p:#x} not 8-aligned");
            let bytes = g.rd_bytes(p, size);
            assert!(bytes.iter().all(|&b| b == 0), "seed {seed} step {step}: alloc_bytes({size}) = {p:#x} not zeroed: {:?}", &bytes[..bytes.len().min(32)]);
            let end = p + ((size + 7) & !7);
            if let Some((&q, &(qs, _))) = live.range(..end).next_back() {
                let qend = q + ((qs + 7) & !7);
                assert!(qend <= p || q >= end, "seed {seed} step {step}: new block {p:#x}+{size} overlaps live {q:#x}+{qs}");
            }
            let pat = (rng.below(250) + 1) as u8;
            g.wr_bytes(p, &vec![pat; size as usize]);
            live.insert(p, (size, pat));
            allocs += 1;
        } else {
            let keys: Vec<u32> = live.keys().copied().collect();
            let p = keys[rng.below(keys.len())];
            let (size, pat) = live.remove(&p).unwrap();
            let bytes = g.rd_bytes(p, size);
            assert!(bytes.iter().all(|&b| b == pat), "seed {seed} step {step}: live block {p:#x}+{size} corrupted before free");
            g.free_bytes(p, size);
        }
        max_live = max_live.max(live.len());
        if step % 512 == 0 {
            for (&p, &(size, pat)) in &live {
                let bytes = g.rd_bytes(p, size);
                assert!(bytes.iter().all(|&b| b == pat), "seed {seed} step {step}: live block {p:#x}+{size} corrupted");
            }
        }
    }
    (allocs, max_live)
}

#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: wasm byte-allocator fuzz; run with --release")]
fn byte_allocator_fuzz_release_wasm() {
    if !wasm_available() {
        return;
    }
    let mut total = 0;
    for seed in 0..40 {
        total += byte_fuzz(release_wasm(), seed, 4000).0;
    }
    eprintln!("release byte fuzz: {total} allocations");
}

#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: wasm byte-allocator fuzz; run with --release")]
fn byte_allocator_fuzz_debug_wasm() {
    if !wasm_available() {
        return;
    }
    let mut total = 0;
    for seed in 100..115 {
        total += byte_fuzz(dev_wasm(), seed, 3000).0;
    }
    eprintln!("debug byte fuzz: {total} allocations");
}

/// The reference-model fuzzer of the runtime (objects, cycles, collection, aborts), but long
/// programs on the real wasm build: the existing wasm fuzz runs 150 ops per seed, which never
/// gets near the collector's 256-root batches or the 2048-word inline stack.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: long wasm fuzz; run with --release")]
fn long_reference_model_fuzz_on_release_wasm() {
    if !wasm_available() {
        return;
    }
    let (mut ops, mut collects, mut freed, mut aborts, mut max_live) = (0, 0, 0, 0, 0);
    for seed in 20_000..20_012 {
        let mut g = WasmGuest::standard(release_wasm());
        let st = fuzz(&mut g, seed, 6000);
        ops += st.ops;
        collects += st.collects;
        freed += st.cycle_freed;
        aborts += st.aborts;
        max_live = max_live.max(st.max_live);
    }
    eprintln!("long wasm fuzz: {ops} ops, {collects} collects, {freed} cycle-freed, {aborts} aborts, max live {max_live}");
}

#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: long wasm fuzz; run with --release")]
fn long_reference_model_fuzz_on_debug_wasm() {
    if !wasm_available() {
        return;
    }
    let mut ops = 0;
    for seed in 30_000..30_006 {
        let mut g = WasmGuest::standard(dev_wasm());
        ops += fuzz(&mut g, seed, 4000).ops;
    }
    eprintln!("long debug wasm fuzz: {ops} ops");
}

/// Freed objects of a class are reused LIFO: the next allocation of the same size returns the very
/// block just freed. The debug runtime's stale-object check (trap 65) is a live bitmap keyed by
/// address, so a stale pointer whose block was re-allocated passes as live: a release through the
/// stale pointer silently frees the NEW object. Documents the detection gap (debug runtime).
#[test]
fn debug_runtime_stale_pointer_after_same_class_reuse() {
    if !wasm_available() {
        return;
    }
    let mut g = WasmGuest::standard(dev_wasm());
    let a = g.alloc(ty::LEAF);
    let _keep = g.alloc(ty::LEAF);
    g.release(a);
    let b = g.alloc(ty::LEAF); // same 16-byte class
    eprintln!("freed {a:#x}, next same-class alloc {b:#x} (reused: {})", a == b);
    let stale = g.try_call("oxi_release", &[a]);
    eprintln!("release through the stale pointer: {:?} (trap kind {})", stale.as_ref().map(|_| ()).map_err(|e| e.to_string()), g.trap_kind());
    assert!(
        stale.is_err() && g.trap_kind() == 65,
        "BL2-05: debug runtime: release through a stale pointer whose block was reused (a={a:#x}, b={b:#x}) is not caught (trap 65); it frees the new object"
    );
}

/// Every freshly allocated object must have a zeroed payload even when its block comes from the
/// free list of a dirty object of the same class (ir.md: fields of a new record read as zero).
#[test]
fn objects_reused_from_the_free_list_are_zeroed() {
    if !wasm_available() {
        return;
    }
    for path in [release_wasm(), dev_wasm()] {
        let mut g = WasmGuest::standard(path);
        let mut dirty = vec![];
        for t in [ty::LEAF, ty::NODE, ty::WIDE, ty::VARIANT, ty::CLOSURE] {
            for _ in 0..64 {
                let p = g.alloc(t);
                dirty.push((t, p));
            }
        }
        // scribble over the scalar words only (ref slots must stay valid for release)
        for &(t, p) in &dirty {
            if t == ty::LEAF {
                g.wr(p, 0xDEAD_BEEF);
                g.wr(p + 4, 0xDEAD_BEEF);
            }
            if t == ty::NODE {
                g.wr(p + 8, 0xDEAD_BEEF);
                g.wr(p + 12, 0xDEAD_BEEF);
            }
        }
        for &(_, p) in &dirty {
            g.release(p);
        }
        for t in [ty::LEAF, ty::NODE, ty::WIDE, ty::VARIANT, ty::CLOSURE] {
            for _ in 0..64 {
                let p = g.alloc(t);
                let size = match t {
                    ty::LEAF | ty::VARIANT | ty::WIDE => 8,
                    _ => 16,
                };
                let bytes = g.rd_bytes(p, size);
                assert!(bytes.iter().all(|&b| b == 0), "type {t}: reused object {p:#x} not zeroed: {bytes:?}");
            }
        }
    }
}

/// A block of one size class freed and requested again with a different exact size inside the
/// same class (9 vs 16 bytes) or of the neighbouring class must come back zeroed over the whole
/// requested size; a wide array grown past 256 bytes moves from the pool to dlmalloc and back.
#[test]
fn byte_blocks_reused_across_exact_sizes_are_zeroed() {
    if !wasm_available() {
        return;
    }
    for path in [release_wasm(), dev_wasm()] {
        let mut g = WasmGuest::standard(path);
        for size in 1..=264u32 {
            let p = g.alloc_bytes(size);
            g.wr_bytes(p, &vec![0xAB; size as usize]);
            g.free_bytes(p, size);
            let size2 = ((size + 7) & !7).max(1);
            let q = g.alloc_bytes(size2);
            let bytes = g.rd_bytes(q, size2);
            assert!(bytes.iter().all(|&b| b == 0), "alloc_bytes({size2}) after free of {size}: not zeroed");
            g.free_bytes(q, size2);
        }
        let a = g.array_new(ty::SCALARARRAY, 0);
        for cap in [4u32, 8, 16, 32, 63, 64, 65, 128, 1000, 70_000] {
            g.array_grow(a, cap);
            let data = g.rd(a + 8);
            let c = g.rd(a + 4);
            let len = g.rd(a);
            let bytes = g.rd_bytes(data + 4 * len, 4 * (c - len));
            assert!(bytes.iter().all(|&b| b == 0), "array grown to {c}: tail not zeroed");
            g.wr_bytes(data, &vec![0x11; 4 * c as usize]);
            g.wr(a, c); // len = cap: next grow copies everything
        }
        g.release(a);
        assert_eq!(g.live_objects(), 0);
    }
}

/// Sizes at the top of the u32 range never wrap the size-class arithmetic: `(size + 7) & !7` in
/// `WasmHeap::alloc` wraps to 0 for size >= 0xFFFF_FFF9 and would index `heads[0x1FFF_FFFF]`.
/// Every public entry must trap out_of_memory (7) or bounds (4) instead.
#[test]
fn huge_sizes_trap_cleanly() {
    if !wasm_available() {
        return;
    }
    for path in [release_wasm(), dev_wasm()] {
        for size in [0x7FFF_FFF8u32, 0xFFFF_FFF0, 0xFFFF_FFF8, 0xFFFF_FFF9, 0xFFFF_FFFC, 0xFFFF_FFFF] {
            let mut g = WasmGuest::standard(path);
            let r = g.try_call("oxi_alloc_bytes", &[size]);
            let k = g.trap_kind();
            match r {
                // raw wasmi has no memory limit: a real 2 GiB block is fine if memory really covers it
                Ok(p) => assert!(p % 8 == 0 && g.mem_size() as u64 >= u64::from(p) + u64::from(size), "alloc_bytes({size:#x}) = {p:#x} but memory is only {} bytes (wrapped size)", g.mem_size()),
                Err(e) => assert!(k == 7 || k == 4, "alloc_bytes({size:#x}): {e} kind {k}"),
            }
        }
        for len in [0x2000_0000u32, 0x4000_0000, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF] {
            for t in [ty::SCALARARRAY, ty::INLINEARRAY, ty::REFARRAY] {
                let mut g = WasmGuest::standard(path);
                let r = g.try_call("oxi_array_new", &[t, len]);
                let k = g.trap_kind();
                let elem = if t == ty::INLINEARRAY { 8u64 } else { 4 };
                match r {
                    Ok(a) => {
                        let data = g.rd(a + 8);
                        let cap = g.rd(a + 4);
                        assert!(
                            u64::from(cap) == u64::from(len) && g.mem_size() as u64 >= u64::from(data) + elem * u64::from(len),
                            "array_new({t}, {len:#x}) = {a:#x}: data {data:#x} cap {cap} but memory only {} bytes (wrapped size)",
                            g.mem_size()
                        );
                        continue;
                    }
                    Err(e) => assert!(k == 7 || k == 4, "array_new({t}, {len:#x}): {e} kind {k}"),
                }
                let mut g = WasmGuest::standard(path);
                let a = g.array_new(t, 0);
                let r = g.try_call("oxi_array_grow", &[a, len]);
                let k = g.trap_kind();
                assert!(r.is_err() && (k == 7 || k == 4), "array_grow({t}, {len:#x}): {:?} kind {k}", r.map_err(|e| e.to_string()));
            }
        }
    }
}

/// BL2-01 at the allocator level (no memory limit in raw wasmi, so measure growth instead):
/// allocate and free 8 MiB of 16-byte objects, then allocate 8 MiB of 32-byte objects and one
/// 4 MiB block. With coalescing (dlmalloc alone, da87be6: +0 KiB for the 32-byte phase) memory
/// barely grows in the second phase;
/// with per-class free lists that never return memory it grows by the full second phase.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 1M wasm calls; run with --release")]
fn memory_freed_in_one_size_class_is_reused_by_others() {
    if !wasm_available() {
        return;
    }
    let mut g = WasmGuest::standard(dev_wasm());
    let n = 8 * 1024 * 1024 / 16;
    let a: Vec<u32> = (0..n).map(|_| g.alloc_bytes(16)).collect();
    for &p in &a {
        g.free_bytes(p, 16);
    }
    let before = g.mem_size();
    let b: Vec<u32> = (0..n / 2).map(|_| g.alloc_bytes(32)).collect();
    let mid = g.mem_size();
    let big = g.alloc_bytes(4 * 1024 * 1024);
    let after = g.mem_size();
    eprintln!(
        "after freeing 8 MiB of 16-byte blocks: memory {} KiB; + 8 MiB of 32-byte blocks: {} KiB (+{} KiB); + one 4 MiB block: {} KiB (+{} KiB)",
        before / 1024, mid / 1024, (mid - before) / 1024, after / 1024, (after - mid) / 1024
    );
    for &p in &b {
        g.free_bytes(p, 32);
    }
    g.free_bytes(big, 4 * 1024 * 1024);
    assert!(mid - before < 1024 * 1024, "BL2-01: 32-byte phase grew memory by {} KiB although 8 MiB of 16-byte blocks had been freed", (mid - before) / 1024);
}
