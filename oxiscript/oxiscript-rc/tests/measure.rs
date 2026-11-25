//! Measurements for the "code size / cost on small targets" risk (decision 1).
//! Run: `cargo test -p oxiscript-rc --release --test measure -- --ignored --nocapture`
#![allow(clippy::many_single_char_names, clippy::too_many_lines, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::doc_markdown, clippy::manual_is_multiple_of, clippy::similar_names)]

mod common;

use common::{build_wasm, WasmGuest};
use oxiscript_rc::testkit::*;
use std::time::Instant;

fn per_op(label: &str, n: u64, f: impl FnOnce()) {
    let t = Instant::now();
    f();
    let d = t.elapsed();
    println!("{label:<52} {:>9.1} ns/op   ({n} ops, {:.1} ms)", d.as_nanos() as f64 / n as f64, d.as_secs_f64() * 1e3);
}

fn native() -> SimGuest {
    let mut g = SimGuest::new(1 << 30, false);
    install_types(&mut g, &standard_types());
    g
}

#[test]
#[ignore = "measurement, run explicitly with --ignored"]
fn native_costs() {
    println!("--- native (x86-64, unchecked arena, real dlmalloc) ---");
    let n = 1_000_000u32;
    let mut g = native();
    let mut ptrs = Vec::with_capacity(n as usize);
    per_op("alloc NODE", u64::from(n), || {
        for _ in 0..n {
            ptrs.push(g.alloc(ty::NODE));
        }
    });
    per_op("release (free, acyclic path: rc 1 -> 0, deferred drain)", u64::from(n), || {
        for &p in &ptrs {
            g.release(p);
        }
        g.live_objects();
    });
    let p = g.alloc(ty::NODE);
    per_op("retain (non-sticky)", 10_000_000, || {
        for _ in 0..10_000_000 {
            g.retain(p);
        }
    });
    per_op("release (rc stays > 0, already purple+buffered)", 10_000_000, || {
        for _ in 0..10_000_000 {
            g.release(p);
        }
    });
    per_op("retain + release pair on a green object", 10_000_000, || {
        let l = g.alloc(ty::LEAF);
        for _ in 0..10_000_000 {
            g.retain(l);
            g.release(l);
        }
    });
    per_op("retain + release pair on a cyclic-capable object", 10_000_000, || {
        let c = g.alloc(ty::NODE);
        for _ in 0..10_000_000 {
            g.retain(c);
            g.release(c);
        }
    });

    for cands in [1_000u32, 10_000, 100_000, 1_000_000] {
        // N independent 2-cycles (2N candidates)
        let mut g = native();
        for _ in 0..cands {
            let (a, b) = (g.alloc(ty::NODE), g.alloc(ty::NODE));
            let (ra, rb) = (g.rd(a - 8), g.rd(b - 8));
            let _ = (ra, rb);
            g.retain(b);
            g.wr(a, b);
            g.retain(a);
            g.wr(b, a);
            g.release(a);
            g.release(b);
        }
        let t = Instant::now();
        let freed = g.collect();
        let d = t.elapsed();
        println!(
            "collect_cycles: {cands:>9} dead 2-cycles ({} candidates)   {:>8.2} ms  ({:.0} ns/object, freed {freed})",
            2 * cands,
            d.as_secs_f64() * 1e3,
            d.as_nanos() as f64 / f64::from(freed.max(1))
        );
        assert_eq!(freed, 2 * cands);
    }
    // one big ring, and the cost of live (not collectable) candidates
    for n in [100_000u32, 1_000_000] {
        let mut g = native();
        let nodes: Vec<u32> = (0..n).map(|_| g.alloc(ty::NODE)).collect();
        for i in 0..n as usize {
            let next = nodes[(i + 1) % n as usize];
            g.retain(next);
            g.wr(nodes[i], next);
        }
        for &p in &nodes {
            g.release(p);
        }
        let t = Instant::now();
        let freed = g.collect();
        println!("collect_cycles: one ring of {n:>9}                       {:>8.2} ms (freed {freed})", t.elapsed().as_secs_f64() * 1e3);
    }
    {
        let n = 100_000u32;
        let mut g = native();
        let nodes: Vec<u32> = (0..n).map(|_| g.alloc(ty::NODE)).collect();
        for i in 0..n as usize {
            let next = nodes[(i + 1) % n as usize];
            g.retain(next);
            g.wr(nodes[i], next);
        }
        // keep one external reference: nothing is garbage, every release makes a candidate
        for &p in &nodes[1..] {
            g.release(p);
        }
        let t = Instant::now();
        let freed = g.collect();
        println!("collect_cycles: live ring of {n} (false alarm)        {:>8.2} ms (freed {freed})", t.elapsed().as_secs_f64() * 1e3);
    }
    // incremental: single gc_step latency over many small dead cycles
    for budget in [1_000u32, 10_000] {
        let mut g = native();
        for _ in 0..200_000 {
            let (a, b) = (g.alloc(ty::NODE), g.alloc(ty::NODE));
            g.retain(b);
            g.wr(a, b);
            g.retain(a);
            g.wr(b, a);
            g.release(a);
            g.release(b);
        }
        let (mut worst, mut steps, mut total) = (0f64, 0u32, 0f64);
        let mut all = vec![];
        loop {
            let t = Instant::now();
            let s = g.gc_step(budget);
            let us = t.elapsed().as_secs_f64() * 1e6;
            worst = worst.max(us);
            all.push(us);
            total += us;
            steps += 1;
            if s == 0 {
                break;
            }
        }
        all.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p99 = all[all.len() * 99 / 100];
        println!("gc_step({budget} units) over 400k garbage objects: {steps} steps, worst step {worst:.0} us, p99 {p99:.0} us, mean {:.0} us", total / f64::from(steps));
    }
}

#[test]
#[ignore = "measurement, run explicitly with --ignored"]
fn wasm_costs_and_size() {
    println!("--- wasm (wasmi interpreter, one host call per op: dominated by call overhead) ---");
    let path = build_wasm("release");
    let size = std::fs::metadata(&path).unwrap().len();
    println!("wasm size (cargo release defaults, with name section): {size} bytes");
    let mut g = WasmGuest::standard(&path);
    // per-object footprint: distance between consecutive NODE allocations
    let a = g.alloc(ty::NODE);
    let b = g.alloc(ty::NODE);
    let c = g.alloc(ty::NODE);
    println!("NODE (payload 16): header 8 B + 16 B = 24 B logical; allocator stride {} B => dlmalloc adds {} B/object", c - b, (c - b) - 24);
    println!("(consecutive pointers {a:#x} {b:#x} {c:#x})");
    let n = 100_000u32;
    per_op("alloc NODE (wasmi)", u64::from(n), || {
        let v: Vec<u32> = (0..n).map(|_| g.alloc(ty::NODE)).collect();
        for p in v {
            g.release(p);
        }
        g.live_objects();
    });
    let mut g = WasmGuest::standard(&path);
    for _ in 0..20_000 {
        let (a, b) = (g.alloc(ty::NODE), g.alloc(ty::NODE));
        g.retain(b);
        g.wr(a, b);
        g.retain(a);
        g.wr(b, a);
        g.release(a);
        g.release(b);
    }
    let t = Instant::now();
    let freed = g.collect();
    println!("collect_cycles in wasm/wasmi: 20k dead 2-cycles: {:.1} ms (freed {freed})", t.elapsed().as_secs_f64() * 1e3);
}
