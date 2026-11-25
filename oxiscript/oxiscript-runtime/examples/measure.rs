//! Measurements for the 8.R report. Run:
//! `cargo run -p oxiscript-runtime --release --example measure`
#![allow(clippy::pedantic)]

#[path = "../tests/common/mod.rs"]
mod common;

use common::{app, env, env_with, Fixture, LOG, STATE};
use oxiscript_runtime::{Grants, Limits, Value};
use std::time::Instant;

fn rss_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    s.split_whitespace()
        .nth(1)
        .and_then(|p| p.parse::<u64>().ok())
        .unwrap_or(0)
        * 4
}

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn filler(n: usize) -> String {
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!(
            "(func $filler{i} (param $a i32) (param $b i32) (result i32) (local $c i32)\n"
        ));
        for k in 0..6 {
            s.push_str(&format!(
                "  (local.set $c (i32.add (local.get $c) (i32.mul (local.get $b) (i32.const {}))))\n  (if (i32.gt_s (local.get $c) (local.get $a)) (then (local.set $a (i32.sub (local.get $c) (i32.const {})))))\n",
                3 + k, 1 + k
            ));
        }
        s.push_str("  (i32.add (local.get $a) (local.get $c)))\n");
    }
    s
}

fn main() {
    // --- instantiate + reload latency (small module) ---
    let e = env();
    let bytes = app().bytes();
    println!("small module: {} bytes", bytes.len());
    let mut times = vec![];
    for _ in 0..2000 {
        let t = Instant::now();
        let rt = e.host.instantiate(&bytes, Grants::all()).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1e6);
        drop(rt);
    }
    println!(
        "instantiate (validate+translate+link+oxi_init): median {:.0} us",
        median(times.clone())
    );
    let mut rt = e.host.instantiate(&bytes, Grants::all()).unwrap();
    let mut times = vec![];
    for _ in 0..2000 {
        let t = Instant::now();
        rt.reload(&bytes).unwrap();
        times.push(t.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!(
        "reload latency (small): median {:.0} us, p99 {:.0} us, max {:.0} us",
        times[times.len() / 2],
        times[times.len() * 99 / 100],
        times[times.len() - 1]
    );

    // --- translation time for ~10 KB and ~100 KB modules ---
    for (label, n) in [("~10 KB", 55usize), ("~100 KB", 550)] {
        let f = Fixture::new(&[LOG, STATE]).body(&filler(n));
        let f = f
            .imports(common::IMPORT_LOG)
            .imports(common::IMPORT_STATE)
            .body(r#"(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32) nop)
                     (func (export "f") (call $s_watch (i32.const 0) (i32.const 0) (i32.const 0)))"#);
        let b = f.bytes();
        let mut ts = vec![];
        for _ in 0..30 {
            // The runtime refuses this module's init handles? `f` is never called; only build.
            let t = Instant::now();
            let r = e.host.instantiate(&b, Grants::all());
            ts.push(t.elapsed().as_secs_f64() * 1e3);
            r.expect("filler module must load");
        }
        println!(
            "{label} module ({} bytes, {n} functions): full load median {:.2} ms",
            b.len(),
            median(ts)
        );
    }

    // --- host call overhead ---
    let big = env_with(Limits {
        fuel_per_call: u64::MAX / 4,
        ..Limits::default()
    });
    let f = app().body(
        r#"
(func (export "loop_get") (param $n i32)
  (local $i i32) (local $v i64)
  (loop $l
    (drop (call $s_get (global.get $cap1) (global.get $slot)))
    (drop)
    (local.set $i (i32.add (local.get $i) (i32.const 1)))
    (br_if $l (i32.lt_u (local.get $i) (local.get $n)))))
(func (export "loop_plain") (param $n i32)
  (local $i i32)
  (loop $l
    (local.set $i (i32.add (local.get $i) (i32.const 1)))
    (br_if $l (i32.lt_u (local.get $i) (local.get $n)))))
"#,
    );
    big.store
        .set("k", oxiscript_runtime::caps::state::StateValue::I64(1))
        .unwrap();
    let mut rt = big.host.instantiate(&f.bytes(), Grants::all()).unwrap();
    let n = 1_000_000;
    let t = Instant::now();
    rt.call_export("loop_plain", &[Value::I32(n)]).unwrap();
    let plain = t.elapsed();
    let t = Instant::now();
    rt.call_export("loop_get", &[Value::I32(n)]).unwrap();
    let with_calls = t.elapsed();
    println!(
        "loop iteration {:.1} ns; iteration + state.get_i64 host call {:.0} ns => host call overhead ~{:.0} ns",
        plain.as_secs_f64() * 1e9 / n as f64,
        with_calls.as_secs_f64() * 1e9 / n as f64,
        (with_calls.as_secs_f64() - plain.as_secs_f64()) * 1e9 / n as f64
    );
    // entering the guest from the host (call_export of an empty function)
    let mut rt2 = big.host.instantiate(&app().bytes(), Grants::all()).unwrap();
    let t = Instant::now();
    for _ in 0..100_000 {
        rt2.call_export("local", &[]).unwrap();
    }
    println!(
        "host->guest call_export round trip: {:.0} ns",
        t.elapsed().as_secs_f64() * 1e9 / 100_000.0
    );

    // --- fuel overhead (raw wasmi, same config except consume_fuel) ---
    let wasm = wat::parse_str(
        r#"(module (func (export "run") (param $n i32) (local $i i32) (local $a i64)
             (loop $l
               (local.set $a (i64.add (local.get $a) (i64.extend_i32_u (local.get $i))))
               (local.set $i (i32.add (local.get $i) (i32.const 1)))
               (br_if $l (i32.lt_u (local.get $i) (local.get $n))))))"#,
    )
    .unwrap();
    for fuel in [false, true, true, false, false, true] {
        let mut c = wasmi::Config::default();
        c.consume_fuel(fuel);
        let engine = wasmi::Engine::new(&c);
        let module = wasmi::Module::new(&engine, &wasm).unwrap();
        let mut store = wasmi::Store::new(&engine, ());
        if fuel {
            store.set_fuel(u64::MAX / 2).unwrap();
        }
        let inst = wasmi::Linker::new(&engine)
            .instantiate_and_start(&mut store, &module)
            .unwrap();
        let run = inst.get_typed_func::<i32, ()>(&store, "run").unwrap();
        let t = Instant::now();
        run.call(&mut store, 200_000_000).unwrap();
        println!(
            "loop of 200M iterations, fuel metering {}: {:.0} ms",
            if fuel { "ON " } else { "OFF" },
            t.elapsed().as_secs_f64() * 1e3
        );
    }

    // --- engine code leak on reload ---
    let e = env();
    let mut rt = e.host.instantiate(&bytes, Grants::all()).unwrap();
    for _ in 0..200 {
        rt.reload(&bytes).unwrap();
    }
    let before = rss_kib();
    for _ in 0..5000 {
        rt.reload(&bytes).unwrap();
    }
    let after = rss_kib();
    println!(
        "RSS before/after 5000 reloads: {before} KiB -> {after} KiB (delta {} KiB)",
        after as i64 - before as i64
    );
}
