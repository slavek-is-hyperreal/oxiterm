#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.M-link: adversarial programs against the real runtime (cycles, OOM, deinit, reload,
//! trap channel). A test that fails is a finding; passing tests document what held.
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::{Error, Grants, Limits, Value};
use std::time::Instant;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn fat() -> Limits { Limits { fuel_per_call: u64::MAX / 4, ..Limits::default() } }

fn log_of(src: &str) -> (Vec<String>, i32) {
    let mut p = Prog::from_wasm_limits(compile(&format!("{HDR}{src}")), fat());
    let l = p.log();
    (l, p.live_after_deinit())
}

#[test]
fn ring_of_100k_class_nodes_is_collected() {
    let t = Instant::now();
    let (log, live) = log_of(
        r#"
class N { next: N | null; v: i32; constructor(v: i32) { this.next = null; this.v = v; } }
export function main(log: Log): void {
  const first = new N(0);
  let cur = first;
  for (let i = 1; i < 100000; i++) { const n = new N(i); cur.next = n; cur = n; }
  cur.next = first;
  log.write(2, `built ${cur.v}`);
}
"#,
    );
    eprintln!("ring100k: {:?}", t.elapsed());
    assert_eq!(log, vec!["built 99999"]);
    assert_eq!(live, 0, "ring of 100k nodes leaked");
}

#[test]
fn many_small_cycles_in_a_loop_are_collected() {
    let t = Instant::now();
    let (log, live) = log_of(
        r#"
class P { other: P | null; constructor() { this.other = null; } }
export function main(log: Log): void {
  let k = 0;
  for (let i = 0; i < 150000; i++) {
    const a = new P();
    const b = new P();
    a.other = b;
    b.other = a;
    k += 1;
  }
  log.write(2, `${k}`);
}
"#,
    );
    eprintln!("150k 2-cycles: {:?}", t.elapsed());
    assert_eq!(log, vec!["150000"]);
    assert_eq!(live, 0);
}

#[test]
fn cycles_through_arrays_and_closures_are_collected() {
    let (log, live) = log_of(
        r#"
class Node { kids: Node[]; cb: (() => i32) | null; constructor() { this.kids = []; this.cb = null; } }
export function main(log: Log): void {
  for (let i = 0; i < 2000; i++) {
    const a = new Node();
    const b = new Node();
    a.kids.push(b);
    b.kids.push(a);
    a.cb = () => a.kids.length;
    b.cb = () => b.kids.length + a.kids.length;
  }
  log.write(2, "ok");
}
"#,
    );
    assert_eq!(log, vec!["ok"]);
    assert_eq!(live, 0);
}

/// Garbage cycles created inside ONE host call and never collected by the host: the runtime has no
/// automatic trigger inside a call. With the default 16 MiB limit this runs out of memory instead of
/// collecting -- document how far it gets.
#[test]
fn cycles_without_a_host_collect_hit_the_memory_limit() {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; pad: i64; constructor() { this.other = null; this.pad = 0; } }
export function main(log: Log): void {
  for (let i = 0; i < 2000000; i++) {
    const a = new P();
    const b = new P();
    a.other = b;
    b.other = a;
  }
  log.write(2, "done");
}
"#
    );
    let wasm = compile(&src);
    let env = env_with_limits(Limits { fuel_per_call: u64::MAX / 4, ..Limits::default() });
    match env.host.instantiate(&wasm, Grants::all()) {
        Ok(_) => eprintln!("2M garbage 2-cycles in one call: completed within the memory limit"),
        Err(e) => {
            eprintln!("2M garbage 2-cycles in one call: {e}");
            assert!(
                trap_kind(&e) == Some(TrapKind::OutOfMemory),
                "BA-LINK: garbage cycles inside one call must be collected or the trap must be out_of_memory: {e}"
            );
        }
    }
}

/// True leak must still show after deinit's collect: closure held by the host (never dropped).
#[test]
fn host_held_closure_still_counts_after_deinit() {
    let src = format!(
        "{HDR}{}",
        r#"
import { Ticker } from "host:ticker";
class Box { n: i32; constructor(n: i32) { this.n = n; } }
export function main(log: Log, t: Ticker): void {
  const b = new Box(7);
  t.register((n: i32, label: string) => { log.write(2, `${b.n}${n}${label}`); });
}
"#
    );
    let mut p = Prog::new(&src);
    let live = p.live_after_deinit();
    eprintln!("host-held closure + captured box: live after deinit = {live}");
    assert!(live >= 2, "host-held closure (and its captured object) must still be reported, got {live}");
}

/// Cycle that is ALSO reachable from a host-held closure must not be collected.
#[test]
fn host_held_cycle_is_not_collected() {
    let src = format!(
        "{HDR}{}",
        r#"
import { Ticker } from "host:ticker";
class P { other: P | null; constructor() { this.other = null; } }
export function main(log: Log, t: Ticker): void {
  const a = new P();
  const b = new P();
  a.other = b;
  b.other = a;
  t.register((n: i32, label: string) => { log.write(2, `${a.other === null}`); });
}
"#
    );
    let mut p = Prog::new(&src);
    let live = p.live_after_deinit();
    assert!(live >= 3, "cycle held by a host closure was collected or hidden: live={live}");
}

#[test]
fn double_deinit_is_harmless() {
    let mut p = Prog::new(&format!(
        "{HDR}{}",
        r#"
class P { s: string; constructor(s: string) { this.s = s; } }
const g: P = new P("global");
let arr: string[] = ["a", "b"];
export function main(log: Log): void { log.write(2, g.s); }
@host export function peek(): i32 { return g.s.byteLength + arr.length; }
"#
    ));
    assert_eq!(p.log(), vec!["global"]);
    let a = p.rt.deinit();
    let b = p.rt.deinit();
    eprintln!("deinit twice: {a:?} then {b:?}");
    assert_eq!(a.as_ref().ok(), Some(&0));
    match b {
        Ok(n) => assert_eq!(n, 0, "second deinit changed live count (double free?)"),
        Err(e) => panic!("second deinit trapped: {e}"),
    }
}

#[test]
fn call_after_deinit_does_not_touch_freed_memory() {
    let mut p = Prog::new(&format!(
        "{HDR}{}",
        r#"
const g: string = "hello world";
export function main(log: Log): void { log.write(2, g); }
@host export function peek(): i32 { return g.byteLength; }
"#
    ));
    let before = p.call("peek", &[]);
    p.live_after_deinit();
    let after = p.rt.call_export("peek", &[]);
    eprintln!("peek before deinit {before:?}, after deinit {after:?}");
    // module-level strings may be sticky/static; accept the same value or a clean trap, never garbage
    match after {
        Ok(v) => assert_eq!(v, before, "UAF: value changed after deinit"),
        Err(_) => {}
    }
}

#[test]
fn runtime_oom_is_one_exact_trap_and_instance_recovers_on_restart() {
    let src = format!(
        "{HDR}{}",
        r#"
@host export function hog(n: i32): i32 {
  const keep: i32[][] = [];
  for (let i = 0; i < n; i++) {
    const a: i32[] = [];
    for (let j = 0; j < 1000; j++) { a.push(j); }
    keep.push(a);
  }
  return keep.length;
}
@host export function ok(): i32 { return 41 + 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut p = Prog::from_wasm_limits(compile(&src), fat());
    assert_eq!(p.call("hog", &[Value::I32(10)]), vec![Value::I32(10)]);
    let e = p.call_err("hog", &[Value::I32(100000)]);
    assert_eq!(trap_kind(&e), Some(TrapKind::OutOfMemory), "{e}");
    // poisoned until restart; after restart the trap kind must not be stale
    assert!(matches!(p.rt.call_export("ok", &[]), Err(Error::Poisoned)));
    p.rt.restart().unwrap();
    assert_eq!(p.call("ok", &[]), vec![Value::I32(42)]);
    // after OOM-restart the memory is fresh: a medium run works again
    assert_eq!(p.call("hog", &[Value::I32(50)]), vec![Value::I32(50)]);
}

/// Memory limit exactly at the boundary: sweep max_memory_pages so the memory.grow failure falls at every
/// point; the only acceptable outcomes are success or exactly out_of_memory.
#[test]
fn memory_limit_sweep_only_oom_or_success() {
    let src = format!(
        "{HDR}{}",
        r#"
export function main(log: Log): void {
  const keep: string[] = [];
  for (let i = 0; i < 3000; i++) { keep.push(`item-${i}-padding-padding-padding`); }
  log.write(2, `${keep.length}`);
}
"#
    );
    let wasm = compile(&src);
    let mut seen_ok = false;
    for pages in 1..40u32 {
        let env = env_with_limits(Limits { max_memory_pages: pages, fuel_per_call: u64::MAX / 4, ..Limits::default() });
        match env.host.instantiate(&wasm, Grants::all()) {
            Ok(mut rt) => {
                seen_ok = true;
                assert_eq!(rt.deinit().unwrap(), 0, "pages={pages}");
            }
            Err(e) => {
                let k = trap_kind(&e);
                let rej = matches!(e, Error::Rejected(_));
                assert!(k == Some(TrapKind::OutOfMemory) || rej, "pages={pages}: unexpected {e}");
            }
        }
    }
    assert!(seen_ok);
}

/// Reload / instance swap with the real runtime: no growth over many swaps, zero leak at the end.
#[test]
fn reload_many_times_does_not_leak_or_grow() {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; constructor() { this.other = null; } }
export function main(log: Log): void {
  for (let i = 0; i < 100; i++) { const a = new P(); const b = new P(); a.other = b; b.other = a; }
  log.write(2, "x");
}
"#
    );
    let wasm = compile(&src);
    let mut p = Prog::from_wasm(wasm.clone());
    let pages0 = p.rt.memory_pages();
    for _ in 0..300 {
        p.rt.reload(&wasm).unwrap();
    }
    assert_eq!(p.rt.memory_pages(), pages0, "memory grew over reloads");
    assert_eq!(p.env.host.live_instances(), 1);
    assert_eq!(p.live_after_deinit(), 0);
}

/// A user program cannot name the runtime's private import module or its trap helper.
#[test]
fn guest_cannot_import_oxi_rt_or_export_trap_helpers() {
    // IDL module literally named "oxi:rt" with the helper's name
    let idl = r#"
interface Evil "oxi:rt" @1 {
    fn oxi_set_trap(kind: i32);
}
"#;
    let parsed = oxiscript_abi::idl::parse(idl);
    eprintln!("idl 'oxi:rt' parse: {:?}", parsed.as_ref().map(|v| v.len()).map_err(|e| e.to_string()));
    if let Ok(mut ifs) = parsed {
        let mut all = interfaces();
        all.append(&mut ifs);
        let src = "import { Evil } from \"oxi:rt\";\nexport function main(e: Evil): void { e.oxi_set_trap(3); }\n";
        let r = oxiscript_lower::compile_source(src, &all, &oxiscript_lower::Options::default());
        eprintln!("compile with oxi:rt interface: {:?}", r.as_ref().map(|w| w.len()).map_err(|e| e.to_string().chars().take(200).collect::<String>()));
        if let Ok(w) = r {
            let env = env();
            let inst = env.host.instantiate(&w, Grants::all());
            eprintln!("instantiate: {:?}", inst.as_ref().map(|_| "loaded").map_err(|e| e.to_string()));
            assert!(inst.is_err(), "BA-LINK: a guest declared an `oxi:rt` import and the module loaded");
        }
    }
}

/// BA-LINK-01: the cycle collector allocates its own work vectors, so collecting garbage that FITS in
/// memory can itself trap out_of_memory (and poison the instance). Sweep the amount of garbage.
#[test]
fn collect_cycles_needs_headroom_probe() {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; constructor() { this.other = null; } }
@host export function churn(n: i32): i32 {
  for (let i = 0; i < n; i++) { const a = new P(); const b = new P(); a.other = b; b.other = a; }
  return n;
}
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let wasm = compile(&src);
    let mut failures = vec![];
    for n in [10_000, 20_000, 40_000, 60_000, 80_000, 100_000, 120_000, 140_000] {
        let mut p = Prog::from_wasm_limits(wasm.clone(), fat());
        match p.rt.call_export("churn", &[Value::I32(n)]) {
            Err(e) => {
                eprintln!("n={n}: churn itself failed: {e}");
                continue;
            }
            Ok(_) => {}
        }
        let pages = p.rt.memory_pages();
        let r = p.rt.collect_cycles();
        let after = p.rt.memory_pages();
        eprintln!("n={n} cycles: pages before collect {pages}, collect -> {r:?}, pages after {after}");
        if let Err(e) = r {
            failures.push(format!("n={n}: collect_cycles failed with {e} at {pages}/256 pages"));
        }
    }
    assert!(failures.is_empty(), "BA-LINK-01: collector OOMs on garbage that fits:\n{}", failures.join("\n"));
}

/// Every compiler-raised trap must report the same kind Stub vs Linked (single trap channel unchanged).
#[test]
fn compiler_traps_report_same_kind_stub_and_linked() {
    use oxiscript_wasm::RuntimeMode;
    let cases: &[(&str, &str)] = &[
        ("bounds", "@host export function t(n: i32): i32 { const a: i32[] = [1, 2, 3]; return a[n + 5]; }"),
        ("overflow", "@host export function t(n: i32): i32 { let x: i32 = 2147483647; x = x + n + 1; return x; }"),
        ("divzero", "@host export function t(n: i32): i32 { return 10 / (n - n); }"),
        ("recursion", "function r(n: i32): i32 { return r(n + 1) + 1; }\n@host export function t(n: i32): i32 { return r(n); }"),
        ("null", "class B { v: i32; constructor(v: i32) { this.v = v; } }\nfunction g(n: i32): B | null { return n > 100 ? new B(1) : null; }\n@host export function t(n: i32): i32 { const b = g(n)!; return b.v; }"),
        ("badslice", "@host export function t(n: i32): i32 { const s = \"h\\u{00e9}llo\"; return s.slice(0, 2 + n).byteLength; }"),
        ("slice_oob", "@host export function t(n: i32): i32 { const s = \"abc\"; return s.slice(2 + n, 1).byteLength; }"),
        ("pop_empty", "@host export function t(n: i32): i32 { const a: i32[] = []; return a.pop()!; }"),
    ];
    for (name, body) in cases {
        let src = format!("{HDR}{body}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n");
        let mut outs = vec![];
        for m in [RuntimeMode::Stub, RuntimeMode::Linked] {
            let opts = oxiscript_lower::Options { runtime: m, ..oxiscript_lower::Options::default() };
            let wasm = match oxiscript_lower::compile_source(&src, &interfaces(), &opts) {
                Ok(w) => w,
                Err(e) => {
                    outs.push(format!("compile error {}", e.to_string().lines().next().unwrap_or("")));
                    continue;
                }
            };
            let env = env_with_limits(Limits { fuel_per_call: u64::MAX / 4, ..Limits::default() });
            let mut rt = env.host.instantiate(&wasm, Grants::all()).unwrap();
            outs.push(match rt.call_export("t", &[Value::I32(0)]) {
                Ok(v) => format!("ok {v:?}"),
                Err(Error::Trap(t)) => format!("trap {:?} raw {} cause {:?}", t.guest_kind, t.guest_kind_raw, t.cause),
                Err(e) => format!("err {e}"),
            });
        }
        eprintln!("{name:10} stub: {} | linked: {}", outs[0], outs[1]);
        assert_eq!(outs[0], outs[1], "{name}: trap differs between Stub and Linked");
    }
}

/// Deep structures: freeing and collecting must not recurse on the Wasm stack.
#[test]
fn deep_array_nesting_and_deep_ring_collect_without_stack_overflow() {
    let src = format!(
        "{HDR}{}",
        r#"
class N { next: N | null; kids: N[]; constructor() { this.next = null; this.kids = []; } }
@host export function deep(n: i32): i32 {
  let head = new N();
  for (let i = 0; i < n; i++) { const m = new N(); m.kids.push(head); head = m; }
  return n;
}
@host export function deepring(n: i32): i32 {
  const first = new N();
  let cur = first;
  for (let i = 0; i < n; i++) { const m = new N(); m.kids.push(cur); cur.next = m; cur = m; }
  first.kids.push(cur);
  return n;
}
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let wasm = compile(&src);
    let lim = Limits { fuel_per_call: u64::MAX / 4, max_memory_pages: 4096, ..Limits::default() };
    let mut p = Prog::from_wasm_limits(wasm, lim);
    for f in ["deep", "deepring"] {
        let r = p.rt.call_export(f, &[Value::I32(200_000)]);
        assert!(r.is_ok(), "{f}: {r:?}");
        let c = p.rt.collect_cycles();
        assert!(c.is_ok(), "{f}: collect {c:?}");
    }
    assert_eq!(p.live_after_deinit(), 0);
}

/// Scratch is now the runtime's 4096-byte static buffer in .bss (neighbours: allocator/RT state).
/// Max-size payloads (4096 bytes, all 0xFF-ish UTF-8) must not corrupt the runtime.
#[test]
fn max_scratch_payloads_do_not_corrupt_runtime_state() {
    let src = format!(
        "{HDR}{}",
        r#"
class Keep { s: string; constructor(s: string) { this.s = s; } }
let kept: Keep[] = [];
@host export function eat(s: string): i32 { kept.push(new Keep(s + "!")); return s.byteLength; }
@host export function total(): i32 { let t = 0; for (const k of kept) { t += k.s.byteLength; } return t; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let mut p = Prog::new(&src);
    let mut sent = 0i32;
    for i in 0..200 {
        let len = [4096usize, 4095, 1, 0, 4000][i % 5];
        let s: String = std::iter::repeat('a').take(len).collect();
        let r = p.call("eat", &[Value::Str(s)]);
        assert_eq!(r, vec![Value::I32(len as i32)]);
        sent += len as i32 + 1;
    }
    assert_eq!(p.call("total", &[]), vec![Value::I32(sent)]);
    // multi-byte payload at the exact limit
    let s: String = std::iter::repeat('é').take(2048).collect();
    assert_eq!(p.call("eat", &[Value::Str(s)]), vec![Value::I32(4096)]);
    let over: String = std::iter::repeat('a').take(4097).collect();
    assert!(p.rt.call_export("eat", &[Value::Str(over)]).is_err());
    p.rt.collect_cycles().unwrap();
    assert_eq!(p.live_after_deinit(), 0);
}

/// BL08: nothing in oxiscript-runtime ever calls `oxi_collect_cycles` (pump/call_export/tick never do), so in
/// a long-running instance cyclic garbage accumulates until the memory limit. The new `deinit` collect hides
/// this from every leak check. Each `mk()` call creates ONE garbage 2-cycle.
#[test]
fn long_running_instance_leaks_cycles_until_oom_without_host_collect() {
    let src = format!(
        "{HDR}{}",
        r#"
class P { other: P | null; constructor() { this.other = null; } }
@host export function mk(): i32 { const a = new P(); const b = new P(); a.other = b; b.other = a; return 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#
    );
    let wasm = compile(&src);
    let mut p = Prog::from_wasm_limits(wasm, fat());
    let mut calls = 0u32;
    let mut oom_at = None;
    for i in 0..300_000u32 {
        match p.rt.call_export("mk", &[]) {
            Ok(_) => calls += 1,
            Err(e) => {
                oom_at = Some((i, trap_kind(&e)));
                break;
            }
        }
    }
    eprintln!("mk() calls survived: {calls}, stop: {oom_at:?}, pages {}", p.rt.memory_pages());
    assert!(oom_at.is_none(), "BL08: a long-running instance that only creates short-lived cycles died with {oom_at:?} after {calls} calls (nothing in oxiscript-runtime calls oxi_collect_cycles)");
}

/// f32 text: abi.md documents f64 only. What does `${f32}` print for a value that is inexact in f32?
#[test]
fn f32_text_probe() {
    let mut p = Prog::new(&format!(
        "{HDR}{}",
        r#"
export function main(log: Log): void {
  const a: f32 = 0.1f32;
  const b: f32 = 16777216.0f32;
  const c: f32 = 3.4028235e38f32;
  log.write(2, `${a}|${b}|${c}`);
  log.write(2, a.toString());
}
"#
    ));
    let l = p.log();
    eprintln!("f32 text: {l:?}");
    p.finish();
    assert_eq!(l[1], "0.1", "BL09: (0.1f32).toString() is {:?}", l[1]);
}
