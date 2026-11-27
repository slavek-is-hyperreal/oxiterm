#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.M-link round 2 (target 0477a99 = Gemini fixes d6b6afd + main): the size-class
//! allocator, the in-call OOM collect, the host's between-events collection policy, fuel
//! determinism. A test that FAILS today is a finding (`BL2-nn` in docs/stages/8.M-link/bad-actor-2.md);
//! passing tests document what held.
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::{Grants, Limits, Value};
use std::time::Instant;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn fat() -> Limits {
    Limits { fuel_per_call: u64::MAX / 4, ..Limits::default() }
}

/// Run `main` with the given limits: Ok(log) or Err(trap description).
fn run_main(src: &str, l: Limits) -> Result<Vec<String>, String> {
    let wasm = compile(&format!("{HDR}{src}"));
    let env = env_with_limits(l);
    match env.host.instantiate(&wasm, Grants::all()) {
        Ok(_) => Ok(env.sink.lines().into_iter().map(|l| l.msg).collect()),
        Err(e) => Err(format!("{e} (kind {:?}); log so far {:?}", trap_kind(&e), env.sink.lines().into_iter().map(|l| l.msg).collect::<Vec<_>>())),
    }
}

// ---------------------------------------------------------------------------------------------
// BL2-01: the size-class free lists never give memory back to dlmalloc, so memory freed in one
// size class cannot be reused by another class or by a large (> 256 B) block.
// ---------------------------------------------------------------------------------------------

const PHASES: &str = r#"
class S { a: i64; constructor() { this.a = 0; } }
class T { a: i64; b: i64; c: i64; constructor() { this.a = 0; this.b = 0; this.c = 0; } }
function smallPhase(n: i32): i32 {
  const keep: S[] = [];
  for (const i of range(0, n)) { keep.push(new S()); }
  return keep.length;
}
function otherClassPhase(n: i32): i32 {
  const keep: T[] = [];
  for (const i of range(0, n)) { keep.push(new T()); }
  return keep.length;
}
function bigStringPhase(k: i32): i32 {
  let s = "x";
  for (const i of range(0, k)) { s = s + s; }
  return s.byteLength;
}
"#;

/// Largest string (2^k bytes by doubling) buildable in a fresh instance vs. in an instance that first
/// allocated and FREED 500k 16-byte objects (8 MiB). Every object of the first phase is dead
/// (live_bytes back to ~0) but its memory stays in the 16-byte free list forever, so the largest
/// buildable string shrinks.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 500k objects; run with --release")]
fn memory_freed_by_small_objects_is_reusable_by_a_large_string() {
    let largest = |pre: &str| -> (u32, Vec<String>) {
        let mut best = 0;
        let mut notes = vec![];
        for k in 16..=23 {
            let r = run_main(
                &format!("{PHASES}export function main(log: Log): void {{ {pre} log.write(2, `b ${{bigStringPhase({k})}}`); }}"),
                fat(),
            );
            notes.push(format!("k={k}: {}", if r.is_ok() { "ok".to_string() } else { "out_of_memory".to_string() }));
            if r.is_ok() {
                best = k;
            }
        }
        (best, notes)
    };
    let (control, cn) = largest("");
    let (attack, an) = largest("log.write(2, `a ${smallPhase(500000)}`);");
    eprintln!("fresh instance: largest 2^{control} ({cn:?})");
    eprintln!("after freeing 8 MiB of 16-byte objects: largest 2^{attack} ({an:?})");
    assert!(
        attack >= control,
        "BL2-01: after freeing 500k small objects the largest string drops from 2^{control} to 2^{attack} bytes"
    );
}

/// Same with two record classes: 500k dead 16-byte objects, then 250k 32-byte objects (8 MiB).
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 750k objects; run with --release")]
fn memory_freed_by_one_class_is_reusable_by_another_class() {
    let control = run_main(
        &format!("{PHASES}export function main(log: Log): void {{ log.write(2, `t ${{otherClassPhase(250000)}}`); }}"),
        fat(),
    );
    assert_eq!(control, Ok(vec!["t 250000".to_string()]), "control: 250k 32-byte objects must fit in 16 MiB");
    let attack = run_main(
        &format!(
            "{PHASES}export function main(log: Log): void {{ log.write(2, `s ${{smallPhase(500000)}}`); log.write(2, `t ${{otherClassPhase(250000)}}`); }}"
        ),
        fat(),
    );
    eprintln!("class-to-class: {attack:?}");
    assert_eq!(
        attack,
        Ok(vec!["s 500000".to_string(), "t 250000".to_string()]),
        "BL2-01: memory of freed 16-byte objects is never reusable for 32-byte objects"
    );
}

/// The same starvation in a long-running instance: alternating phases (16 B, then 24 B, then 32 B
/// objects, each phase ~3 MiB live and then freed) ratchet memory up until out_of_memory although
/// the live set never exceeds ~4 MiB.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: millions of objects; run with --release")]
fn alternating_size_phases_do_not_ratchet_memory_to_oom() {
    let src = format!(
        "{HDR}{}",
        r#"
class A { a: i64; constructor() { this.a = 0; } }
class B { a: i64; b: i64; constructor() { this.a = 0; this.b = 0; } }
class C { a: i64; b: i64; c: i64; constructor() { this.a = 0; this.b = 0; this.c = 0; } }
class D { a: i64; b: i64; c: i64; d: i64; constructor() { this.a = 0; this.b = 0; this.c = 0; this.d = 0; } }
@host export function phase(k: i32, n: i32): i32 {
  if (k === 0) { const v: A[] = []; for (const i of range(0, n)) { v.push(new A()); } return v.length; }
  if (k === 1) { const v: B[] = []; for (const i of range(0, n)) { v.push(new B()); } return v.length; }
  if (k === 2) { const v: C[] = []; for (const i of range(0, n)) { v.push(new C()); } return v.length; }
  const v: D[] = []; for (const i of range(0, n)) { v.push(new D()); } return v.length;
}
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut p = Prog::from_wasm_limits(compile(&src), fat());
    let mut trace = vec![];
    for round in 0..8 {
        let k = round % 4;
        // ~3.2 MiB of objects per phase whatever the class size
        let n = 3_200_000 / (16 + 8 * k);
        let r = p.rt.call_export("phase", &[Value::I32(k), Value::I32(n)]);
        trace.push(format!("round {round} class {} B x {n}: {:?} pages {}", 16 + 8 * k, r.as_ref().map(|_| "ok").map_err(|e| e.to_string()), p.rt.memory_pages()));
        if r.is_err() {
            break;
        }
    }
    for t in &trace {
        eprintln!("{t}");
    }
    assert!(
        trace.iter().all(|t| t.contains("Ok(\"ok\")")),
        "BL2-01: phases of different object sizes, each ~3 MiB and freed, died:\n{}",
        trace.join("\n")
    );
    // The live set of every round is the same ~3.2 MiB (+ its array): memory should stay near the
    // first round's high-water mark, not grow to the sum of every size class's peak.
    let first: usize = trace[0].rsplit(' ').next().unwrap().parse().unwrap();
    let last = p.rt.memory_pages();
    assert!(last <= 2 * first, "BL2-01: same-sized live set, memory ratchets {first} -> {last} pages (limit 256): freed memory is stuck per size class");
}

// ---------------------------------------------------------------------------------------------
// BL2-02: under the DEFAULT limits the host's between-events collection is switched off for good
// once the program has more than 10,000 live objects (`live_objects * 1000 <= budget`); the only
// remaining collection is the in-call OOM collect, which then has to free the whole heap of garbage
// inside one ordinary call's fuel.
// ---------------------------------------------------------------------------------------------

const STARVE: &str = r#"
class K { v: i32; constructor(v: i32) { this.v = v; } }
class P { other: P | null; constructor() { this.other = null; } }
let keep: K[] = [];
@host export function fill(n: i32): i32 { for (const i of range(0, n)) { keep.push(new K(i)); } return keep.length; }
@host export function mk(): i32 { const a = new P(); const b = new P(); a.other = b; b.other = a; return 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#;

fn starve(live: i32, calls: u32) -> (Vec<String>, Option<String>) {
    let mut p = Prog::from_wasm_limits(compile(&format!("{HDR}{STARVE}")), Limits::default());
    let mut notes = vec![];
    let mut filled = 0;
    while filled < live {
        let n = (live - filled).min(2000);
        p.call("fill", &[Value::I32(n)]);
        filled += n;
    }
    let mut fail = None;
    let mut max_fuel = 0u64;
    let t = Instant::now();
    for i in 0..calls {
        match p.rt.call_export("mk", &[]) {
            Ok(_) => max_fuel = max_fuel.max(p.rt.last_fuel_used()),
            Err(e) => {
                fail = Some(format!("call {i}: {e} (kind {:?}), pages {}", trap_kind(&e), p.rt.memory_pages()));
                break;
            }
        }
    }
    notes.push(format!(
        "live {live}: {calls} mk() calls -> {:?}; pages {}; max fuel of one mk() {max_fuel}; {:?}",
        fail, p.rt.memory_pages(), t.elapsed()
    ));
    (notes, fail)
}

/// Control: with 2,000 live objects the host collects between calls and 400k mk() calls stay flat.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 400k host calls; run with --release")]
fn default_limits_small_live_set_collects_between_calls() {
    let (notes, fail) = starve(2_000, 400_000);
    eprintln!("{}", notes.join("\n"));
    assert!(fail.is_none(), "{notes:?}");
}

/// Attack: 40,000 live objects (a modest cache; the guard is pump_fuel 30M / 1000 = 30k) switch the background collector off forever; the
/// garbage 2-cycles pile up to the 16 MiB limit and then one tiny `mk()` call must pay for
/// collecting ~500k objects within the default 10M fuel per call.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 400k host calls; run with --release")]
fn default_limits_40k_live_objects_starve_the_host_collector() {
    let (notes, fail) = starve(40_000, 400_000);
    eprintln!("{}", notes.join("\n"));
    assert!(
        fail.is_none(),
        "BL2-02: with 40k live objects (> budget/1000) the host never collects; the instance died: {notes:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// BL2-03: the fuel guard `live_objects * 1000 <= budget` is not an upper bound on the collector's
// work: the cost of trial deletion is proportional to EDGES, not objects. A handful of objects with
// a large reference array passes the guard; the host's housekeeping collect then runs out of fuel
// AFTER the user's call returned successfully, and poisons the instance.
// ---------------------------------------------------------------------------------------------

fn housekeeping(l: Limits) -> Option<String> {
    let src = format!(
        "{HDR}{}",
        r#"
class N { next: N | null; constructor() { this.next = null; } }
let hub: N = new N();
let big: N[] = [];
@host export function grow(n: i32): i32 { for (const i of range(0, n)) { big.push(hub); } return big.length; }
@host export function touch(): i32 { const b = big; const c = b; return c.length; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut p = Prog::from_wasm_limits(compile(&src), l);
    let mut trace = vec![];
    let mut fail = None;
    'outer: for round in 0..50 {
        for (name, args) in [("grow", vec![Value::I32(10_000)]), ("touch", vec![])] {
            let r = p.rt.call_export(name, &args);
            match r {
                Ok(v) => {
                    if round % 10 == 0 {
                        trace.push(format!("round {round} {name} -> {v:?} fuel {} pages {}", p.rt.last_fuel_used(), p.rt.memory_pages()));
                    }
                }
                Err(e) => {
                    fail = Some(format!("round {round} {name}: {e} (kind {:?}); last_fuel_used {}", trap_kind(&e), p.rt.last_fuel_used()));
                    break 'outer;
                }
            }
        }
    }
    for t in &trace {
        eprintln!("{t}");
    }
    eprintln!("fail: {fail:?}");
    fail
}

/// Attack, default limits (50 rounds = 500k refs, 2 MiB): `grow(10000)` costs ~4.07M of the 10M per-call fuel and returns, then
/// the host's housekeeping collect traverses the purple 130k-element array (live objects: ~3, so
/// the guard lets it run) and runs out of the 30M housekeeping budget: the successful call is
/// reported as OutOfFuel and the instance is poisoned.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 2M array pushes; run with --release")]
fn housekeeping_collect_never_poisons_a_successful_call() {
    let fail = housekeeping(Limits::default());
    assert!(fail.is_none(), "BL2-03: a call whose own fuel stayed in budget returned an error / poisoned the instance: {fail:?}");
}

/// Control: the same sequence with an unbounded housekeeping budget (`pump_fuel`) passes, so the
/// OutOfFuel above comes from the host's collect, not from the user's call.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: 2M array pushes; run with --release")]
fn housekeeping_control_with_unbounded_pump_fuel() {
    let fail = housekeeping(Limits { pump_fuel: u64::MAX / 4, ..Limits::default() });
    assert!(fail.is_none(), "control failed: {fail:?}");
}



/// Direct measurement of the guard's assumption: fuel of one `oxi_collect_cycles` over a purple
/// candidate with N reachable live objects. The guard assumes <= 1000 fuel per live object.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy; run with --release")]
fn collect_cycles_fuel_per_live_object_is_at_most_1000() {
    let src = format!(
        "{HDR}{}",
        r#"
class N { next: N | null; kids: N[]; constructor() { this.next = null; this.kids = []; } }
let head: N = new N();
@host export function build(n: i32): i32 {
  let cur = head;
  for (const i of range(0, n)) { const x = new N(); cur.next = x; cur = x; }
  return n;
}
@host export function touch(): i32 { const h = head; const g = h; return 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut rows = vec![];
    let mut worst = 0.0f64;
    for n in [1_000, 5_000, 9_000] {
        let mut p = Prog::from_wasm_limits(compile(&src), fat());
        p.call("build", &[Value::I32(n)]);
        p.call("touch", &[]);
        let live = p.rt.live_objects().unwrap();
        p.rt.collect_cycles().unwrap();
        let fuel = p.rt.last_fuel_used();
        let per = fuel as f64 / live as f64;
        worst = worst.max(per);
        rows.push(format!("n {n}: live {live}, collect fuel {fuel} = {per:.0} fuel/live object"));
    }
    for r in &rows {
        eprintln!("{r}");
    }
    assert!(worst <= 1000.0, "BL2-03: collector costs more than the 1000 fuel/object the host guard assumes: {rows:?}");
}

// ---------------------------------------------------------------------------------------------
// Collector scaling inside one call (the 2M-cycle test): no quadratic re-collection.
// ---------------------------------------------------------------------------------------------

#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: up to 4M cycles; run with --release")]
fn in_call_oom_collect_scales_linearly() {
    let mut rows = vec![];
    let mut times = vec![];
    for n in [500_000u32, 1_000_000, 2_000_000, 4_000_000] {
        let src = format!(
            "{HDR}
class P {{ other: P | null; pad: i64; constructor() {{ this.other = null; this.pad = 0; }} }}
export function main(log: Log): void {{
  for (let i = 0; i < {n}; i++) {{ const a = new P(); const b = new P(); a.other = b; b.other = a; }}
  log.write(2, \"done\");
}}
"
        );
        let wasm = compile(&src);
        let env = env_with_limits(fat());
        let t = Instant::now();
        let r = env.host.instantiate(&wasm, Grants::all()).map(|rt| rt.last_fuel_used());
        let el = t.elapsed().as_secs_f64();
        times.push(el / f64::from(n));
        rows.push(format!("{n} cycles: {:?} in {el:.2}s ({:.0} ns/cycle)", r.as_ref().map_err(|e| e.to_string()), el * 1e9 / f64::from(n)));
        assert!(r.is_ok(), "{rows:?}");
    }
    for r in &rows {
        eprintln!("{r}");
    }
    let ratio = times[3] / times[0];
    assert!(ratio < 2.0, "per-cycle time grows {ratio:.2}x from 500k to 4M cycles: {rows:?}");
}

// ---------------------------------------------------------------------------------------------
// Zeroing and stale data: blocks reused from a free list must look fresh.
// ---------------------------------------------------------------------------------------------

/// Objects and arrays reused from the free lists must read as zero / default (ir.md: zeroed fields),
/// including inline struct elements of arrays created by `array_new` and grown by `push`.
#[test]
fn reused_blocks_read_as_fresh() {
    let src = format!(
        "{HDR}{}",
        r#"
struct Q { a: i64; b: i32; c: f64; }
class R { x: i64; y: i64; s: string; constructor() { this.x = 0; this.y = 0; this.s = ""; } }
function dirty(): i32 {
  const v: R[] = [];
  for (const i of range(0, 200)) { const r = new R(); r.x = -1; r.y = -1; r.s = `junk${i}`; v.push(r); }
  const w: Q[] = [];
  for (const i of range(0, 200)) { w.push({ a: -1, b: -1, c: -1.0 }); }
  return v.length + w.length;
}
function fresh(): i64 {
  let acc: i64 = 0;
  for (const i of range(0, 200)) { const r = new R(); acc = acc + r.x + r.y + (r.s.byteLength as i64); }
  const w: Q[] = [];
  for (const i of range(0, 200)) { w.push({ a: 0, b: 0, c: 0.0 }); }
  for (const q of w) { acc = acc + q.a + (q.b as i64); }
  return acc;
}
export function main(log: Log): void {
  let total: i64 = 0;
  for (const k of range(0, 50)) { dirty(); total = total + fresh(); }
  log.write(2, `${total}`);
}
"#
    );
    for (mode, dbg) in [(oxiscript_wasm::RuntimeMode::Linked, false), (oxiscript_wasm::RuntimeMode::Linked, true)] {
        let wasm = compile_with_mode(&src, mode, dbg);
        let env = env_with_limits(fat());
        let mut rt = env.host.instantiate(&wasm, Grants::all()).unwrap_or_else(|e| panic!("debug={dbg}: {e}"));
        let log: Vec<String> = env.sink.lines().into_iter().map(|l| l.msg).collect();
        assert_eq!(log, vec!["0"], "debug={dbg}");
        assert_eq!(rt.deinit().unwrap(), 0, "debug={dbg}");
    }
}

// ---------------------------------------------------------------------------------------------
// Determinism: same program, fresh instances and Stub / Linked / LinkedDebug: identical logs and
// identical fuel per mode across runs; host collection never changes the user-visible result.
// ---------------------------------------------------------------------------------------------

#[test]
fn fuel_and_results_are_deterministic_across_instances_and_modes() {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; v: i32; constructor(v: i32) { this.other = null; this.v = v; } }
let total: i64 = 0;
@host export function step(n: i32): i64 {
  const xs: P[] = [];
  for (const i of range(0, n)) { const a = new P(i); const b = new P(i + 1); a.other = b; b.other = a; xs.push(a); }
  let s: i64 = 0;
  for (const x of xs) { s = s + (x.v as i64); }
  total = total + s;
  return total;
}
export function main(log: Log): void { log.write(2, `t ${step(10)}`); }
"#
    );
    let mut per_mode = vec![];
    for (mode, dbg, name) in [
        (oxiscript_wasm::RuntimeMode::Stub, false, "stub"),
        (oxiscript_wasm::RuntimeMode::Linked, false, "linked"),
        (oxiscript_wasm::RuntimeMode::Linked, true, "linked-debug"),
    ] {
        let wasm = compile_with_mode(&src, mode, dbg);
        let mut runs = vec![];
        for _ in 0..3 {
            let mut p = Prog::from_wasm_limits(wasm.clone(), Limits::default());
            let mut seq = vec![];
            for i in 0..200 {
                let v = p.call("step", &[Value::I32(50 + (i % 7))]);
                seq.push((format!("{v:?}"), p.rt.last_fuel_used()));
            }
            runs.push((seq, p.rt.memory_pages()));
        }
        assert!(runs.windows(2).all(|w| w[0] == w[1]), "{name}: runs differ");
        per_mode.push((name, runs.remove(0)));
    }
    let vals = |i: usize| per_mode[i].1 .0.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
    assert_eq!(vals(0), vals(1), "stub vs linked values");
    assert_eq!(vals(1), vals(2), "linked vs linked-debug values");
    let f = |i: usize| per_mode[i].1 .0.iter().map(|x| x.1).sum::<u64>();
    eprintln!("total fuel over 200 calls: stub {} linked {} ({:.2}x) linked-debug {}", f(0), f(1), f(1) as f64 / f(0) as f64, f(2));
}

/// The fixes claim Linked costs 1.0-1.8x of Stub's fuel (goal: <= 2x). Microbenchmarks release
/// temporaries at rc 1; a workload with shared references (rc > 1 releases, 2-cycles, an array of
/// classes) goes through `release_rc_ne_1` / `buffer_purple` / the roots buffer on every step.
#[test]
fn fuel_ratio_with_shared_references_is_within_2x_of_stub() {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; v: i32; constructor(v: i32) { this.other = null; this.v = v; } }
@host export function step(n: i32): i64 {
  const xs: P[] = [];
  for (const i of range(0, n)) { const a = new P(i); const b = new P(i + 1); a.other = b; b.other = a; xs.push(a); }
  let s: i64 = 0;
  for (const x of xs) { s = s + (x.v as i64); }
  return s;
}
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut fuel = vec![];
    for mode in [oxiscript_wasm::RuntimeMode::Stub, oxiscript_wasm::RuntimeMode::Linked] {
        let mut p = Prog::from_wasm_limits(compile_with_mode(&src, mode, false), Limits::default());
        let mut f = 0u64;
        for _ in 0..50 {
            p.call("step", &[Value::I32(100)]);
            f += p.rt.last_fuel_used();
        }
        fuel.push(f);
    }
    let ratio = fuel[1] as f64 / fuel[0] as f64;
    eprintln!("shared-reference workload: stub {} linked {} = {ratio:.2}x", fuel[0], fuel[1]);
    assert!(ratio <= 2.0, "BL2-04: Linked fuel is {ratio:.2}x of Stub on a shared-reference workload (claim 1.0-1.8x, goal <= 2x)");
}

/// OOM at the exact limit, then recovery: an instance that traps out_of_memory is poisoned, a
/// fresh instance with the same program works; small allocations right after a large failed one
/// (the near-OOM exact-size fallback path) do not corrupt anything.
#[test]
fn near_oom_small_allocations_after_a_failed_large_one() {
    let src = format!(
        "{HDR}{}",
        r#"
class S { a: i64; constructor(a: i64) { this.a = a; } }
let keep: S[] = [];
@host export function fillTo(n: i32): i32 { for (const i of range(0, n)) { keep.push(new S(i as i64)); } return keep.length; }
@host export function sum(): i64 { let s: i64 = 0; for (const k of keep) { s = s + k.a; } return s; }
@host export function hog(k: i32): i32 { let s = "y"; for (const i of range(0, k)) { s = s + s; } return s.byteLength; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let l = Limits { max_memory_pages: 40, ..fat() };
    let mut p = Prog::from_wasm_limits(compile(&src), l.clone());
    // fill memory with small objects in steps until a step fails
    let mut n = 0i64;
    let mut err = None;
    for _ in 0..1000 {
        match p.rt.call_export("fillTo", &[Value::I32(2000)]) {
            Ok(_) => n += 2000,
            Err(e) => {
                err = Some(e);
                break;
            }
        }
    }
    let e = err.expect("never hit the 40-page limit");
    eprintln!("small objects up to the limit: {n} then {e}");
    assert_eq!(trap_kind(&e), Some(TrapKind::OutOfMemory), "{e}");
    // poisoned: next call fails without running
    assert!(p.rt.call_export("sum", &[]).is_err());
    // fresh instance works and computes the right sum
    let mut q = Prog::from_wasm_limits(compile(&src), l);
    q.call("fillTo", &[Value::I32(4000)]);
    let want: i64 = (0..4000).sum();
    assert_eq!(q.call("sum", &[]), vec![Value::I64(want)]);
}

// ---------------------------------------------------------------------------------------------
// Runtime-internal exports are not reachable through the host API, and a program cannot shadow
// one (including the new compiler-internal `oxi_str_from_f32`).
// ---------------------------------------------------------------------------------------------

#[test]
fn runtime_internal_exports_are_not_callable_or_shadowable() {
    let mut p = Prog::new(&format!("{HDR}@host export function ok(): i32 {{ return 1; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}"));
    for name in [
        "oxi_release", "oxi_retain", "oxi_free_bytes", "oxi_alloc_bytes", "oxi_rt_init", "oxi_init", "oxi_collect_cycles",
        "oxi_str_from_f32", "oxi_str_from_f64", "oxi_array_grow", "oxi_live_objects", "oxi_deinit", "memory",
    ] {
        let r = p.rt.call_export(name, &[Value::I32(8), Value::I32(8), Value::I32(8)]);
        assert!(r.is_err(), "host API reaches runtime export {name}: {r:?}");
    }
    assert_eq!(p.call("ok", &[]), vec![Value::I32(1)]);
    for name in ["oxi_str_from_f32", "oxi_rt_init", "oxi_gc_step", "oxi_release", "oxi_rt_scratch_cap"] {
        assert_user_export_rejected(name);
    }
}

/// The linker strips the runtime's `__heap_base` / `__data_end` exports (check_layout in
/// bad_actor_link_merge.rs treats them as leaks), but a program may export a function with that
/// very name. Info only: no runtime state is reachable, but the merged module again exports
/// `__heap_base`, which tools read as the heap start.
#[test]
fn user_export_named_heap_base_is_rejected() {
    for name in ["__heap_base", "__data_end"] {
        assert_user_export_rejected(name);
    }
}

fn assert_user_export_rejected(name: &str) {
    {
        let src = format!("{HDR}@host export function {name}(x: i32): i32 {{ return x; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}");
        let r = oxiscript_lower::compile_source(
            &src,
            &interfaces(),
            &oxiscript_lower::Options { target: test_target(), ..oxiscript_lower::Options::default() },
        );
        if let Ok(wasm) = r {
            // compiled: the user's function must not have replaced / collided with the runtime's
            let env = env();
            let inst = env.host.instantiate(&wasm, Grants::all());
            panic!("user export named {name} compiled (instantiate: {:?})", inst.map(|_| ()).map_err(|e| e.to_string()));
        }
    }
}

/// BL2-02 without any live cache: calls that each leave 1k / 3k garbage 2-cycles (all
/// garbage) already exceeds the guard (`live_objects` counts garbage too), so the host never
/// collects; after a few calls the in-call OOM collect must free the whole heap inside one call.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy; run with --release")]
fn default_limits_bursts_of_cycle_garbage_starve_the_host_collector() {
    let rows = bursts(Limits::default());
    assert!(rows.iter().all(|r| r.contains(": None,")), "BL2-02/03: {rows:#?}");
}

/// Control: unbounded housekeeping fuel (`pump_fuel`), same calls.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy; run with --release")]
fn bursts_control_with_unbounded_pump_fuel() {
    let rows = bursts(Limits { pump_fuel: u64::MAX / 4, ..Limits::default() });
    assert!(rows.iter().all(|r| r.contains(": None,")), "control: {rows:#?}");
}

fn bursts(l: Limits) -> Vec<String> {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; constructor() { this.other = null; } }
@host export function burst(n: i32): i32 { for (const i of range(0, n)) { const a = new P(); const b = new P(); a.other = b; b.other = a; } return n; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut rows = vec![];
    for n in [1_000, 3_000] {
        let mut p = Prog::from_wasm_limits(compile(&src), l.clone());
        let mut fail = None;
        let mut max_fuel = 0;
        for i in 0..200 {
            match p.rt.call_export("burst", &[Value::I32(n)]) {
                Ok(_) => max_fuel = max_fuel.max(p.rt.last_fuel_used()),
                Err(e) => {
                    fail = Some(format!("call {i}: {e}"));
                    break;
                }
            }
        }
        rows.push(format!("burst({n}) x200: {fail:?}, pages {}, max fuel/call {max_fuel}", p.rt.memory_pages()));
    }
    for r in &rows {
        eprintln!("{r}");
    }
    rows
}
