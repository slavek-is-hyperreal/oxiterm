#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.M-link: size and speed, Stub vs Linked (numbers go to the report; asserts are loose guards).
mod common;
use common::*;
use oxiscript_lower::{compile_source, Options};
use oxiscript_runtime::{Grants, Limits, Value};
use oxiscript_wasm::RuntimeMode;
use std::time::Instant;

fn comp(src: &str, m: RuntimeMode) -> Vec<u8> {
    compile_source(src, &interfaces(), &Options { runtime: m, ..Options::default() }).unwrap_or_else(|e| panic!("{e}"))
}

const WORK: &str = r#"
import { Log } from "host:log";
struct Pt { x: i32; y: i32; }
class Node { next: Node | null; v: i32; constructor(v: i32) { this.next = null; this.v = v; } }
@host export function work(n: i32): i32 {
  let head: Node | null = null;
  let total = 0;
  for (let i = 0; i < n; i++) {
    const nd = new Node(i);
    nd.next = head;
    head = nd;
    const s = `k${i}`;
    total += s.byteLength;
    const a: Pt[] = [];
    for (let j = 0; j < 4; j++) { a.push({ x: j, y: i }); }
    total += a.length;
  }
  return total;
}
export function main(log: Log): void { log.write(2, "up"); }
"#;

#[test]
#[ignore = "measurement (100 compiles per program); run with --release -- --ignored --nocapture"]
fn size_and_time_stub_vs_linked() {
    let root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    let mut srcs: Vec<(String, String)> = vec![("work".into(), WORK.into())];
    for n in ["hello", "counter", "shapes", "structs", "errors"] {
        if let Ok(s) = std::fs::read_to_string(format!("{root}/sites/oxiscript/examples/{n}.oxs")) {
            srcs.push((n.into(), s));
        }
    }
    for (name, src) in &srcs {
        let (mut tc_s, mut tc_l) = (0u128, 0u128);
        let iters = 100;
        let (mut ws, mut wl) = (vec![], vec![]);
        for _ in 0..iters {
            let t = Instant::now();
            ws = comp(src, RuntimeMode::Stub);
            tc_s += t.elapsed().as_micros();
            let t = Instant::now();
            wl = comp(src, RuntimeMode::Linked);
            tc_l += t.elapsed().as_micros();
        }
        let (mut ti_s, mut ti_l) = (0u128, 0u128);
        for (w, acc) in [(&ws, &mut ti_s), (&wl, &mut ti_l)] {
            for _ in 0..iters {
                let env = env();
                let t = Instant::now();
                let rt = env.host.instantiate(w, Grants::all());
                *acc += t.elapsed().as_micros();
                drop(rt);
            }
        }
        eprintln!(
            "{name:8} size stub {:6} linked {:6} | compile us stub {:6} linked {:6} | instantiate us stub {:6} linked {:6}",
            ws.len(), wl.len(), tc_s / iters, tc_l / iters, ti_s / iters, ti_l / iters
        );
    }
}

#[test]
#[ignore = "measurement; run with --release -- --ignored --nocapture"]
fn per_call_overhead_stub_vs_linked() {
    for n in [1000, 20000] {
        let mut row = String::new();
        for m in [RuntimeMode::Stub, RuntimeMode::Linked] {
            let wasm = comp(WORK, m);
            let env = env_with_limits(Limits { fuel_per_call: u64::MAX / 4, max_memory_pages: 1024, ..Limits::default() });
            let mut rt = env.host.instantiate(&wasm, Grants::all()).unwrap();
            let t = Instant::now();
            let r = rt.call_export("work", &[Value::I32(n)]);
            let dt = t.elapsed();
            let fuel = rt.last_fuel_used();
            row.push_str(&format!(" | {m:?}: {:?} {dt:?} fuel {fuel} pages {}", r.map(|v| format!("{v:?}")).map_err(|e| e.to_string()), rt.memory_pages()));
        }
        eprintln!("work({n}){row}");
    }
}

/// Fuel per loop iteration for single operations, Stub vs Linked.
#[test]
fn fuel_per_operation() {
    let ops: &[(&str, &str)] = &[
        ("empty loop", "total += 1;"),
        ("new class", "const nd = new Node(i); total += nd.v;"),
        ("template int", "const s = `k${i}`; total += s.byteLength;"),
        ("str concat", "const s = \"ab\" + \"cd\"; total += s.byteLength;"),
        ("array push x4", "const a: i32[] = []; for (let j = 0; j < 4; j++) { a.push(j); } total += a.length;"),
        ("array lit", "const a: i32[] = [1, 2, 3]; total += a.length;"),
        ("struct in array", "const a: Pt[] = []; a.push({ x: 1, y: i }); total += a.length;"),
        ("struct copy", "const p: Pt = { x: 1, y: i }; const q = p; total += q.x;"),
        ("closure", "const f = (k: i32) => k + i; total += f(1);"),
    ];
    let n = 1000;
    for (name, body) in ops {
        let src = format!(
            "import {{ Log }} from \"host:log\";\nstruct Pt {{ x: i32; y: i32; }}\nclass Node {{ next: Node | null; v: i32; constructor(v: i32) {{ this.next = null; this.v = v; }} }}\n@host export function work(n: i32): i32 {{ let total = 0; for (let i = 0; i < n; i++) {{ {body} }} return total; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n"
        );
        let mut row = String::new();
        let mut fuels = vec![];
        for m in [RuntimeMode::Stub, RuntimeMode::Linked] {
            let wasm = comp(&src, m);
            let env = env_with_limits(Limits { fuel_per_call: u64::MAX / 4, max_memory_pages: 1024, ..Limits::default() });
            let mut rt = env.host.instantiate(&wasm, Grants::all()).unwrap();
            let _ = rt.call_export("work", &[Value::I32(n)]).unwrap();
            fuels.push(rt.last_fuel_used() / n as u64);
        }
        row.push_str(&format!("{name:16} fuel/iter stub {:6} linked {:6} ratio {:.1}", fuels[0], fuels[1], fuels[1] as f64 / fuels[0].max(1) as f64));
        eprintln!("{row}");
    }
}

#[test]
#[ignore = "writes $TMPDIR/hello_linked.wasm"]
fn dump_hello_linked() {
    let root = format!("{}/../..", env!("CARGO_MANIFEST_DIR"));
    let src = std::fs::read_to_string(format!("{root}/sites/oxiscript/examples/hello.oxs")).unwrap();
    let w = comp(&src, RuntimeMode::Linked);
    let dir = std::env::var("TMPDIR").unwrap();
    std::fs::write(format!("{dir}/hello_linked.wasm"), &w).unwrap();
    let w2 = comp(&src, RuntimeMode::Linked);
    assert_eq!(w, w2, "linked output is not deterministic in-process");
}

/// BL06 demonstration: the SAME program under the DEFAULT limits (10M fuel per call): passes in Stub, traps in Linked.
#[test]
fn default_fuel_budget_regression() {
    let mut res = vec![];
    for m in [RuntimeMode::Stub, RuntimeMode::Linked] {
        let wasm = comp(WORK, m);
        let env = env();
        let mut rt = env.host.instantiate(&wasm, Grants::all()).unwrap();
        res.push(rt.call_export("work", &[Value::I32(1000)]).map(|v| format!("{v:?}")).map_err(|e| e.to_string()));
    }
    eprintln!("work(1000) default limits: stub {:?} | linked {:?}", res[0], res[1]);
    assert!(res[0].is_ok(), "stub should pass");
    assert!(res[1].is_ok(), "BL06: work(1000) passes in Stub but fails in Linked under default Limits: {:?}", res[1]);
}
