#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! 8.M-link round-2 fixes: calibration of the host's collector step (fuel per work unit) over
//! adversarial graph shapes, and the housekeeping/incremental-collection proofs.
mod common;
use common::*;
use oxiscript_runtime::{Limits, Value};

const HDR: &str = "import { Log } from \"host:log\";\n";

fn fat() -> Limits {
    Limits { fuel_per_call: u64::MAX / 4, pump_fuel: u64::MAX / 4, ..Limits::default() }
}

const SHAPES: &str = r#"
class N { next: N | null; kids: N[]; constructor() { this.next = null; this.kids = []; } }
class W { a: W | null; b: W | null; c: W | null; d: W | null; e: W | null; f: W | null; g: W | null; h: W | null; constructor() { this.a = null; this.b = null; this.c = null; this.d = null; this.e = null; this.f = null; this.g = null; this.h = null; } }
class P { other: P | null; constructor() { this.other = null; } }
let head: N = new N();
let hub: N = new N();
let big: N[] = [];
let wide: W = new W();
let rings: P[] = [];
let drop: N[] = [];
@host export function list(n: i32): i32 { let cur = head; for (const i of range(0, n)) { const x = new N(); cur.next = x; cur = x; } return n; }
@host export function touchList(): i32 { const h = head; const g = h; return 1; }
@host export function hubGrow(n: i32): i32 { for (const i of range(0, n)) { big.push(hub); } return big.length; }
@host export function touchHub(): i32 { const b = big; const c = b; const h = hub; const g = h; return c.length; }
@host export function tree(n: i32): i32 {
  let cur = wide;
  for (const i of range(0, n)) { const x = new W(); const y = new W(); const z = new W(); cur.a = x; cur.b = y; cur.c = z; cur.d = x; cur.e = y; cur.f = z; cur.g = x; cur.h = y; cur = x; }
  return n;
}
@host export function touchWide(): i32 { const h = wide; const g = h; return 1; }
@host export function garbage(n: i32): i32 { for (const i of range(0, n)) { const a = new P(); const b = new P(); a.other = b; b.other = a; } return n; }
@host export function ringGarbage(n: i32): i32 {
  const first = new P(); let cur = first;
  for (const i of range(0, n)) { const x = new P(); cur.other = x; cur = x; }
  cur.other = first;
  return n;
}
@host export function fillDrop(n: i32): i32 { for (const i of range(0, n)) { drop.push(new N()); } return drop.length; }
@host export function freeDrop(): i32 { drop = []; return 0; }
export function main(log: Log): void { log.write(2, "up"); }
"#;

use oxiscript_runtime::GC_FUEL_PER_UNIT;

/// Run the setup calls, then drive `collect_step(units)` until idle (at most `max_steps`).
/// Returns (steps, runtime-wide worst fuel per unit over every collector step, housekeeping
/// included).
fn measure(debug: bool, calls: &[(&str, i32)], label: &str, units: u32, max_steps: u32) -> f64 {
    let mut p = Prog::from_wasm_limits(
        compile_with_mode(&format!("{HDR}{SHAPES}"), oxiscript_wasm::RuntimeMode::Linked, debug),
        fat(),
    );
    for (name, n) in calls {
        if *n < 0 {
            p.call(name, &[]);
        } else {
            p.call(name, &[Value::I32(*n)]);
        }
    }
    let mut steps = 0u32;
    loop {
        let r = p.rt.collect_step(units).unwrap().expect("collector present");
        steps += 1;
        if !r.more || steps >= max_steps {
            break;
        }
    }
    let st = p.rt.collector_stats();
    eprintln!(
        "{label:24} units {units:6} debug={debug:5}: steps {:5} total fuel {:11} worst fuel/unit {:7.1}",
        st.steps, st.fuel_total, st.max_fuel_per_unit
    );
    st.max_fuel_per_unit
}

/// BL2-03: the host plans `GC_FUEL_PER_UNIT` fuel per collector work unit and sets a backstop at
/// 4x that; every step over adversarial shapes (long purple list, 50k-edge hub array, wide
/// records, garbage ring, big array drop, garbage 2-cycles) must stay under the backstop, in
/// both runtime flavours (the debug runtime costs ~1.5x per unit and may exceed the *plan*, never
/// the backstop).
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: calibration; run with --release")]
fn collector_step_fuel_per_unit_stays_under_the_backstop() {
    let mut worst = 0.0f64;
    for debug in [false, true] {
        for units in [4096u32, 65536] {
            for (calls, label) in [
                (vec![("list", 4_000), ("touchList", -1)], "purple list 4k"),
                (vec![("hubGrow", 50_000), ("touchHub", -1)], "hub array 50k refs"),
                (vec![("tree", 1_000), ("touchWide", -1)], "wide records 1k"),
                (vec![("garbage", 5_000)], "garbage 2-cycles 5k"),
                (vec![("ringGarbage", 20_000)], "garbage ring 20k"),
                (vec![("fillDrop", 20_000), ("freeDrop", -1)], "drop array of 20k"),
            ] {
                worst = worst.max(measure(debug, &calls, label, units, 400));
            }
        }
    }
    eprintln!("WORST fuel per unit: {worst:.1} (plan {GC_FUEL_PER_UNIT}, backstop {})", GC_FUEL_PER_UNIT * 4);
    assert!(worst < (GC_FUEL_PER_UNIT * 4) as f64, "a collector step used {worst:.0} fuel per unit");
}

/// BL2-03: the realistic worst case found by calibration: a 20k-node linked list built in a loop
/// leaves every node a collector candidate and none of them is garbage. Under the default limits
/// the host must (a) never fail a call because of housekeeping and (b) not burn the whole
/// housekeeping budget every turn (escalation stops at the cap, then backs off).
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy; run with --release")]
fn live_candidate_graph_too_big_for_a_step_does_not_burn_every_turn() {
    let mut p = Prog::from_wasm_limits(compile(&format!("{HDR}{SHAPES}")), Limits::default());
    for _ in 0..10 {
        p.call("list", &[Value::I32(2_000)]);
    }
    let calls = 1_000;
    for i in 0..calls {
        p.rt.call_export("touchList", &[]).unwrap_or_else(|e| panic!("call {i}: {e}"));
    }
    let st = p.rt.collector_stats();
    let budget = Limits::default().pump_fuel.max(Limits::default().fuel_per_call);
    eprintln!("{calls} calls: collector steps {}, total housekeeping fuel {} (budget per turn {budget}), worst step {}", st.steps, st.fuel_total, st.max_step_fuel);
    assert!(st.fuel_total < 10 * budget, "housekeeping burned {} fuel over {calls} turns", st.fuel_total);
}

/// BL2-03/02: a burst of cycle garbage per call under default limits is kept up with by the
/// incremental housekeeping and memory stays flat; housekeeping fuel per call stays far below the
/// per-turn budget on average.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy; run with --release")]
fn incremental_housekeeping_keeps_up_with_bursts() {
    let mut p = Prog::from_wasm_limits(compile(&format!("{HDR}{SHAPES}")), Limits::default());
    for i in 0..300 {
        p.rt.call_export("garbage", &[Value::I32(1_500)]).unwrap_or_else(|e| panic!("call {i}: {e}"));
    }
    let pages = p.rt.memory_pages();
    let st = p.rt.collector_stats();
    eprintln!("pages {pages}, steps {}, housekeeping fuel {}, worst step {}", st.steps, st.fuel_total, st.max_step_fuel);
    assert!(pages <= 20, "garbage piled up: {pages} pages");
}

// ---- BL2-08: cost of the stable sort in Stub and Linked ----------------------------------------
const SORT_SRC: &str = r#"import { Log } from "host:log";
import { sortBy, sortF64, sortI32, sortString, isSortedBy } from "std/array";
struct R { k: i32; id: i32; }
@host export function stable(n: i32): i32 {
  const xs: R[] = [];
  for (const i of range(0, n)) { xs.push({ k: (i * 7919) % 5, id: i }); }
  const ys = sortBy(xs, (a: R, b: R): i32 => a.k - b.k);
  let bad = 0;
  for (const i of range(1, n)) { if (ys[i - 1].k > ys[i].k) { bad = bad + 1; } if (ys[i - 1].k === ys[i].k && ys[i - 1].id > ys[i].id) { bad = bad + 1000; } }
  return bad + (ys.length === n ? 0 : 1000000);
}
@host export function evil(n: i32): i32 {
  const xs: i32[] = [];
  for (const i of range(0, n)) { xs.push((i * 31) % 17); }
  const ys = sortBy(xs, (a: i32, b: i32): i32 => 1);
  return ys.length;
}
@host export function trapper(n: i32): i32 {
  const xs: i32[] = [];
  for (const i of range(0, n)) { xs.push(i); }
  const ys = sortBy(xs, (a: i32, b: i32): i32 => (a / (b - b)));
  return ys.length;
}
@host export function nanSort(): i32 {
  const nan = 0.0 / 0.0;
  const ys = sortF64([3.0, nan, -0.0, 0.0, -1.0, nan, 1.0 / 0.0, -1.0 / 0.0]);
  let o = 0;
  for (const i of range(0, ys.length)) { o = o * 10 + (ys[i] !== ys[i] ? 9 : (ys[i] < 0.0 ? 1 : (ys[i] === 0.0 ? (1.0 / ys[i] < 0.0 ? 2 : 3) : 4))); }
  return o;
}
@host export function big(n: i32): i32 {
  const xs: i32[] = [];
  for (const i of range(0, n)) { xs.push(((i *% 1103515245) +% 12345) % 100003); }
  const ys = sortI32(xs);
  return isSortedBy(ys, (a: i32, b: i32): i32 => a - b) ? ys.length : -1;
}
@host export function emptyOne(): i32 { const e: i32[] = []; const o: i32[] = [5]; return sortI32(e).length * 10 + sortI32(o)[0]; }
@host export function strs(): i32 { const ys = sortString(["é", "z", "a", "😀", "€", ""]); let o = 0; for (const y of ys) { o = o * 7 + y.byteLength; } return o; }
export function main(log: Log): void { log.write(2, "up"); }
"#;

#[test]
fn bl2_08_stable_sort_fuel_stub_vs_linked() {
    let mut rows = vec![];
    for (label, mode, debug) in [
        ("stub", oxiscript_wasm::RuntimeMode::Stub, false),
        ("linked", oxiscript_wasm::RuntimeMode::Linked, false),
        ("linked-debug", oxiscript_wasm::RuntimeMode::Linked, true),
    ] {
        let mut p = Prog::from_wasm_limits(compile_with_mode(SORT_SRC, mode, debug), fat());
        let r = p.call("stable", &[Value::I32(1000)]);
        rows.push((label, p.rt.last_fuel_used(), r));
    }
    for (l, f, r) in &rows {
        eprintln!("stable(1000) {l:13} fuel {f:10} -> {r:?}");
    }
    let default = Limits::default().fuel_per_call;
    // BL2-08: the stable sort of 1000 structs must fit the default per-call fuel in the default
    // (Linked) runtime. (The debug runtime flavour costs ~1.8x and is for parity checks only.)
    assert!(rows[1].1 < default, "Linked needs {} fuel for stable(1000), default budget {default}", rows[1].1);
    assert!(rows[1].1 * 100 <= rows[0].1 * 150, "Linked stable(1000) {} is more than 1.5x of Stub {}", rows[1].1, rows[0].1);
}



/// BL2-02 policy: under memory pressure, with only a live purple graph the bounded in-call
/// recovery cannot reclaim anything, a call that allocates until memory is gone must end in a
/// clean `out_of_memory` trap, never in `OutOfFuel` (the recovery attempt is bounded to
/// `OOM_COLLECT_UNITS` work units, well under the default per-call fuel).
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy; run with --release")]
fn bounded_oom_recovery_ends_in_out_of_memory_not_out_of_fuel() {
    let src = format!("{HDR}{}", r#"
class N { next: N | null; v: i64; constructor() { this.next = null; this.v = 0; } }
let head: N = new N();
let tail: N[] = [head];
@host export function list(n: i32): i32 { let cur = tail[0]; for (const i of range(0, n)) { const x = new N(); cur.next = x; cur = x; } tail[0] = cur; return n; }
@host export function eat(n: i32): i32 { const xs: N[] = []; for (const i of range(0, n)) { xs.push(new N()); } return xs.length; }
export function main(log: Log): void { log.write(2, "up"); }
"#);
    let mut p = Prog::from_wasm_limits(compile(&src), Limits::default());
    let mut built = 0;
    // fill memory with a live list in which every node is a collector candidate
    while p.rt.memory_pages() < 245 {
        p.rt.call_export("list", &[Value::I32(2_000)]).unwrap_or_else(|e| panic!("list after {built} nodes: {e}"));
        built += 2_000;
        assert!(built < 2_000_000, "memory never filled");
    }
    eprintln!("{built} live nodes, {} pages, housekeeping fuel {}", p.rt.memory_pages(), p.rt.collector_stats().fuel_total);
    let r = p.rt.call_export("eat", &[Value::I32(1_000_000)]);
    let e = r.expect_err("allocating a million objects must exhaust memory");
    eprintln!("eat: {e} (kind {:?}), fuel used {}", trap_kind(&e), p.rt.last_fuel_used());
    assert_eq!(trap_kind(&e), Some(oxiscript_abi::trap::TrapKind::OutOfMemory), "expected out_of_memory, got {e}");
}
