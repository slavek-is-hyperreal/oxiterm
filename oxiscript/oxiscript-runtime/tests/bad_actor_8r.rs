//! BAD ACTOR attacks on slice 8.R. Findings are tests that FAIL today (or are
//! `#[ignore]`d with a reason when they are slow/huge); passing tests document
//! what held up. See the report for the numbered findings (F1..).
#![allow(clippy::pedantic, dead_code)]
mod common;

use common::*;
use oxiscript_runtime::{Error, Grants, Limits, Runtime, Value};
use std::time::Instant;

/// Fuzz-loop size: full counts in optimised builds or with OXI_FULL_FUZZ=1; 1/20 in debug
/// builds so `cargo test --workspace` (CI, debug) stays fast.
fn cases(full: u64) -> u64 {
    if !cfg!(debug_assertions) || std::env::var_os("OXI_FULL_FUZZ").is_some() { full } else { (full / 20).max(100) }
}

fn i(x: i32) -> Value {
    Value::I32(x)
}
fn st(r: Vec<Value>) -> i32 {
    match r[0] {
        Value::I32(s) => s,
        _ => panic!("not i32"),
    }
}

/// Raw passthrough module: every import wrapped so Rust can feed any i32/i64.
pub fn raw() -> Fixture {
    raw_with(
        r#"(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32))"#,
        None,
    )
}

pub fn raw_with(cb: &str, release: Option<&str>) -> Fixture {
    let f = Fixture::new(&[LOG, STATE])
        .imports(IMPORT_LOG)
        .imports(IMPORT_STATE)
        .body(
            r#"
(data (i32.const 3000) "hello")
(data (i32.const 3100) "k")
(data (i32.const 3200) "hello world")
(data (i32.const 8192) "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkk")
(data (i32.const 8448) "kkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkkz")
(global $slot (mut i32) (i32.const 0))
(func (export "cap0") (result i32) (global.get $cap0))
(func (export "cap1") (result i32) (global.get $cap1))
(func (export "slot") (result i32) (global.get $slot))
(func (export "mem_size") (result i32) (memory.size))
(func (export "r_open") (param i32 i32 i32) (result i32 i32)
  (call $s_open (local.get 0) (local.get 1) (local.get 2)))
(func (export "r_get") (param i32 i32) (result i32 i64)
  (call $s_get (local.get 0) (local.get 1)))
(func (export "r_set") (param i32 i32 i64) (result i32)
  (call $s_set (local.get 0) (local.get 1) (local.get 2)))
(func (export "r_read") (param i32 i32 i32 i32) (result i32 i32)
  (call $s_read (local.get 0) (local.get 1) (local.get 2) (local.get 3)))
(func (export "r_write") (param i32 i32 i32 i32) (result i32)
  (call $s_write (local.get 0) (local.get 1) (local.get 2) (local.get 3)))
(func (export "r_watch") (param i32 i32 i32)
  (call $s_watch (local.get 0) (local.get 1) (local.get 2)))
(func (export "r_log") (param i32 i32 i32 i32)
  (call $log_write (local.get 0) (local.get 1) (local.get 2) (local.get 3)))
(func (export "unreachable") unreachable)
(func (export "spin") (loop $l (br $l)))
;; loop n times: read with out_cap 0 (status 1 path)
(func (export "read0_loop") (param $n i32) (param $ptr i32)
  (loop $l
    (call $s_read (global.get $cap1) (global.get $slot) (local.get $ptr) (i32.const 0)) drop drop
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
(func (export "write_big") (param $len i32) (result i32)
  (call $s_write (global.get $cap1) (global.get $slot) (i32.const 4096) (local.get $len)))
(func (export "set_loop") (param $n i32)
  (loop $l
    (drop (call $s_set (global.get $cap1) (global.get $slot) (i64.const 1)))
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
(func (export "set_loop2") (param $c i32) (param $s i32) (param $n i32)
  (loop $l
    (drop (call $s_set (local.get $c) (local.get $s) (i64.const 1)))
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
(func (export "rewatch_loop") (param $c i32) (param $s i32) (param $n i32)
  (loop $l
    (call $s_watch (local.get $c) (local.get $s) (local.get $n))
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
(func (export "wasm_loop") (param $n i32)
  (loop $l
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
"#,
        )
        .init(
            r#"(call $s_open (global.get $cap1) (i32.const 3100) (i32.const 1))
               (global.set $slot) (drop)"#,
        );
    let f = f.body(cb);
    match release {
        Some(r) => f.over("oxi_release", r),
        None => f,
    }
}

fn rt(e: &Env, f: &Fixture) -> Runtime {
    quick(f, e)
}

fn call(r: &mut Runtime, n: &str, a: &[Value]) -> Vec<Value> {
    r.call_export(n, a).unwrap_or_else(|e| panic!("{n}: {e}"))
}

// ---------------------------------------------------------------- F1
/// F1 revoke is per *instance*: any trap (guest controlled) + the documented
/// recovery `restart()` (or `reload()`) hands the capability back.
#[test]
fn f1_revocation_survives_restart_and_reload() {
    let e = env();
    let f = raw();
    let mut r = rt(&e, &f);
    let (c1, slot) = (
        call(&mut r, "cap1", &[])[0].clone(),
        call(&mut r, "slot", &[])[0].clone(),
    );
    assert_eq!(st(call(&mut r, "r_get", &[c1.clone(), slot.clone()])), 16);
    assert!(r.revoke("host:state@1"));
    assert_eq!(st(call(&mut r, "r_get", &[c1.clone(), slot.clone()])), 2);
    // guest poisons itself
    assert!(r.call_export("unreachable", &[]).is_err());
    r.restart().unwrap();
    let (c1, slot) = (
        call(&mut r, "cap1", &[])[0].clone(),
        call(&mut r, "slot", &[])[0].clone(),
    );
    // EXPECTED (revocation is a host decision, abi.md 5.1): still denied (2).
    assert_eq!(
        st(call(&mut r, "r_get", &[c1, slot])),
        2,
        "F1: revoked capability came back after restart()"
    );
}

// ---------------------------------------------------------------- F8
/// F8 determinism: the cap handle value the guest sees in oxi_init depends on
/// how many instances the *host* created before (global epoch counter).
#[test]
fn f8_cap_handle_value_depends_on_host_history() {
    let e = env();
    let f = raw();
    let mut a = rt(&e, &f);
    let ha = call(&mut a, "cap0", &[])[0].clone();
    drop(a);
    let mut b = rt(&e, &f);
    let hb = call(&mut b, "cap0", &[])[0].clone();
    assert_eq!(
        ha, hb,
        "F8: same module, same inputs, different handle ({ha:?} vs {hb:?})"
    );
}

// ---------------------------------------------------------------- F2
fn put_big(r: &mut Runtime, len: i32) {
    // write_str reads from scratch-independent memory at 4096..: zeroed -> valid utf8
    assert_eq!(st(call(r, "write_big", &[i(len)])), 0);
}

/// F2 status-1 path (and out-of-fuel-cheap copying) is charged only 200 fuel
/// while the host cloned a 16 KiB string.
#[test]
fn f2_status1_read_loop_is_cheap_in_fuel_expensive_in_host() {
    let e = env_with(Limits {
        fuel_per_call: 10_000_000,
        ..Limits::default()
    });
    let f = raw();
    let mut r = rt(&e, &f);
    put_big(&mut r, 16 * 1024);
    let t = Instant::now();
    let res = r.call_export("read0_loop", &[i(1_000_000), i(8192)]);
    let el = t.elapsed();
    let used = r.last_fuel_used();
    let hc = r.host_calls();
    eprintln!("F2 result={res:?} used_fuel={used} host_calls={hc} wall={el:?}");
    let t = Instant::now();
    let mut r2 = rt(&e, &f);
    let _ = r2.call_export("wasm_loop", &[i(2_000_000)]);
    let base = t.elapsed();
    eprintln!("F2 baseline pure-wasm turn wall={base:?}");
    // a host-call-heavy turn should cost about the same wall time as a
    // pure-wasm turn of equal fuel; flag a >20x ratio
    assert!(
        el.as_secs_f64() < base.as_secs_f64() * 20.0 + 0.05,
        "F2: {hc} status-1 reads burned {el:?} vs {base:?} for the same fuel"
    );
}

// ---------------------------------------------------------------- F3
/// F3 StateStore::set scans every subscription of every instance, compared by
/// string, for 200 fuel.
#[test]
fn f3_set_scans_all_subscriptions() {
    let run = |watchers: bool| {
        let e = env_with(Limits::default());
        let f = raw();
        let mut keep = Vec::new();
        if watchers {
            for _ in 0..14 {
                let mut r = rt(&e, &f);
                let c1 = call(&mut r, "cap1", &[])[0].clone();
                for n in 0..64 {
                    let h = call(&mut r, "r_open", &[c1.clone(), i(8192), i(256)]);
                    call(&mut r, "r_watch", &[c1.clone(), h[1].clone(), i(1 + n * 8)]);
                }
                keep.push(r);
            }
        }
        let mut w = rt(&e, &f);
        let c1 = call(&mut w, "cap1", &[])[0].clone();
        let h = call(&mut w, "r_open", &[c1.clone(), i(8448), i(256)]);
        let slot = h[1].clone();
        let t = Instant::now();
        let mut n = 0u64;
        for _ in 0..3 {
            let r = w.call_export("set_loop2", &[c1.clone(), slot.clone(), i(40_000)]);
            n += w.host_calls();
            if r.is_err() {
                break;
            }
        }
        let _ = n;
        let el = t.elapsed();
        eprintln!(
            "F3 watchers={watchers} wall={el:?} fuel={} calls={}",
            w.last_fuel_used(),
            w.host_calls()
        );
        (el, w.host_calls())
    };
    let (with, c1) = run(true);
    let (base, c0) = run(false);
    let per_with = with.as_secs_f64() / c1.max(1) as f64;
    let per_base = base.as_secs_f64() / c0.max(1) as f64;
    eprintln!(
        "F3 per-call with={per_with:e} base={per_base:e} ratio={}",
        per_with / per_base
    );
    assert!(
        per_with < per_base * 5.0,
        "F3: set cost grows with total subscriptions"
    );
}

// ---------------------------------------------------------------- F4
const BURN: &str = r#"
(global $burn (mut i32) (i32.const 0))
(func $burn_fn (local $n i32) (local.set $n (global.get $burn))
  (loop $l (local.set $n (i32.sub (local.get $n) (i32.const 1))) (br_if $l (local.get $n))))
(func (export "set_burn") (param i32) (global.set $burn (local.get 0)))
(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32) (call $burn_fn))
"#;

/// F4 pump has no aggregate fuel/time budget: up to 256 releases + 256 events
/// per pump, each with a full `fuel_per_call`.
#[test]
fn f4_pump_has_no_aggregate_budget() {
    let e = env();
    let f = raw_with(
        BURN,
        Some("(func (export \"oxi_release\") (param i32) (call $burn_fn))"),
    );
    // add the burn helpers (raw_with(.., BURN) already defines $burn_fn before use order-independent)
    let mut r = rt(&e, &f);
    // calibrate: ~9M fuel per entry
    call(&mut r, "set_burn", &[i(1_000)]);
    let t = Instant::now();
    let _ = r.call_export("cap0", &[]);
    let _ = t;
    call(&mut r, "set_burn", &[i(2_000_000)]);
    let c1 = call(&mut r, "cap1", &[])[0].clone();
    let slot0 = call(&mut r, "slot", &[])[0].clone();
    // fuel per iteration: run one release-free callback through a probe: use wasm_loop
    let _ = r.call_export("wasm_loop", &[i(1_000_000)]);
    let per_iter = r.last_fuel_used() as f64 / 1_000_000.0;
    let burn = (9_000_000.0 / per_iter) as i32;
    call(&mut r, "set_burn", &[i(burn)]);
    // 64 watchers (limit), each on its own slot
    let mut slots = vec![slot0.clone()];
    for _ in 0..63 {
        let h = call(&mut r, "r_open", &[c1.clone(), i(3100), i(1)]);
        slots.push(h[1].clone());
    }
    for (n, s) in slots.iter().enumerate() {
        call(
            &mut r,
            "r_watch",
            &[c1.clone(), s.clone(), i(1 + 8 * n as i32)],
        );
    }
    // queue releases: rewatch one slot 256 times (one release each)
    for n in 0..256 {
        call(
            &mut r,
            "r_watch",
            &[c1.clone(), slots[0].clone(), i(8 * (n + 100))],
        );
    }
    e.store
        .set("k", oxiscript_runtime::caps::state::StateValue::I64(1))
        .unwrap();
    let t = Instant::now();
    let st = r.pump().unwrap();
    let el = t.elapsed();
    eprintln!("F4 one pump: {st:?} wall={el:?} (per-entry fuel ~9M, fuel_per_call=10M, iter fuel {per_iter})");
    // A sane host would bound a pump to roughly a few entries' worth of fuel.
    // Wall-clock bound calibrated for optimised builds; an unoptimised build runs one
    // 9M-fuel callback for seconds, so debug builds check the structural bound instead.
    let bound_ms = if cfg!(debug_assertions) { 10_000 } else { 500 };
    assert!(
        el.as_millis() < bound_ms,
        "F4: a single pump blocked the host for {el:?} ({st:?})"
    );
}

/// F4b the release backlog must be bounded. CORRECTED: the original asserted
/// that >100k rewatches are *accepted* and drained, i.e. the unbounded growth
/// itself. The lead's decision is a bounded queue whose overflow traps the
/// enqueuing call with OXS-E0429; this asserts that, that host work stops at
/// the bound, and that a restart leaves nothing of the old backlog behind.
#[test]
fn f4b_release_backlog_is_unbounded() {
    let e = env();
    let f = raw();
    let mut r = rt(&e, &f);
    let c1 = call(&mut r, "cap1", &[])[0].clone();
    let s = call(&mut r, "slot", &[])[0].clone();
    let err = r
        .call_export("rewatch_loop", &[c1.clone(), s.clone(), i(40_000)])
        .unwrap_err();
    assert_eq!(err.code(), "OXS-E0429", "{err}");
    let cap = Limits::default().max_pending_releases as u64;
    assert!(r.host_calls() <= cap + 8, "host_calls={}", r.host_calls());
    r.restart().unwrap();
    let p = r.pump().unwrap();
    assert_eq!((p.released, p.pending_releases), (0, 0));
}

// ---------------------------------------------------------------- F5 (held)
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn pick<T: Copy>(&mut self, v: &[T]) -> T {
        v[(self.next() % v.len() as u64) as usize]
    }
}

/// Trampoline invariant fuzz: edge-valued raw arguments into every import.
/// Invariants: no panic; memory unchanged unless (status 0 and read_str), in
/// which case only [out_ptr, out_ptr+len) may change.
fn run_trampoline_fuzz(cases: u64, seed: u64) -> (u64, u64, u64) {
    let e = env();
    let f = raw();
    let mut r = rt(&e, &f);
    let mem_len: i64 = 2 * 65536;
    let edge: Vec<i32> = vec![
        0,
        1,
        2,
        7,
        8,
        1023,
        1024,
        1025,
        2047,
        2048,
        2049,
        4095,
        4096,
        4097,
        6143,
        6144,
        8192,
        65535,
        65536,
        mem_len as i32 - 17,
        mem_len as i32 - 2,
        mem_len as i32 - 1,
        mem_len as i32,
        mem_len as i32 + 1,
        0x7FFF_FFFF,
        i32::MIN,
        i32::MIN + 1,
        -1,
        -2,
        -4096,
        -65536,
        0x7FFF_F000u32 as i32,
        64 * 1024,
        64 * 1024 + 1,
        16 * 1024,
        16 * 1024 + 1,
        256,
        257,
        3100,
        3000,
        3200,
        100,
        5,
        11,
    ];
    let mut rng = Rng(seed | 1);
    let (mut traps, mut oks, mut st1) = (0u64, 0u64, 0u64);
    let get_handles = |r: &mut Runtime| {
        let c0 = call(r, "cap0", &[])[0].clone();
        let c1 = call(r, "cap1", &[])[0].clone();
        let s = call(r, "slot", &[])[0].clone();
        (c0, c1, s)
    };
    let (mut c0, mut c1, mut slot) = get_handles(&mut r);
    // seed a string value
    let _ = r.call_export("write_big", &[i(11)]);
    for case in 0..cases {
        let hv = |rng: &mut Rng, good: &Value, other: &Value| -> Value {
            match rng.next() % 6 {
                0..=2 => good.clone(),
                3 => other.clone(),
                _ => i(rng.pick(&edge)),
            }
        };
        let which = rng.next() % 7;
        let before: Vec<u8> = r.read_memory(0, mem_len as u32).unwrap();
        let a = [rng.pick(&edge), rng.pick(&edge), rng.pick(&edge)];
        let (name, args): (&str, Vec<Value>) = match which {
            0 => (
                "r_read",
                vec![
                    hv(&mut rng, &c1, &c0),
                    hv(&mut rng, &slot, &c1),
                    i(a[0]),
                    i(a[1]),
                ],
            ),
            1 => (
                "r_write",
                vec![
                    hv(&mut rng, &c1, &c0),
                    hv(&mut rng, &slot, &c1),
                    i(a[0]),
                    i(a[1]),
                ],
            ),
            2 => ("r_open", vec![hv(&mut rng, &c1, &c0), i(a[0]), i(a[1])]),
            3 => (
                "r_log",
                vec![hv(&mut rng, &c0, &c1), i(a[2]), i(a[0]), i(a[1])],
            ),
            4 => (
                "r_get",
                vec![hv(&mut rng, &c1, &c0), hv(&mut rng, &slot, &c1)],
            ),
            5 => (
                "r_watch",
                vec![hv(&mut rng, &c1, &c0), hv(&mut rng, &slot, &c1), i(a[0])],
            ),
            _ => (
                "r_set",
                vec![
                    hv(&mut rng, &c1, &c0),
                    hv(&mut rng, &slot, &c1),
                    Value::I64(a[0] as i64),
                ],
            ),
        };
        let res = r.call_export(name, &args);
        let after: Vec<u8> = r.read_memory(0, mem_len as u32).unwrap();
        // allowed window
        let mut lo = 0usize;
        let mut hi = 0usize;
        match &res {
            Ok(v) => {
                oks += 1;
                if name == "r_read" {
                    if let (Value::I32(s), Value::I32(l), Value::I32(p)) = (&v[0], &v[1], &args[2])
                    {
                        if *s == 0 {
                            lo = *p as u32 as usize;
                            hi = lo + *l as u32 as usize;
                        } else if *s == 1 {
                            st1 += 1;
                        }
                    }
                }
            }
            Err(Error::Trap(_)) => traps += 1,
            Err(e) => panic!("case {case}: unexpected {e} for {name} {args:?}"),
        }
        for (k, (b, a)) in before.iter().zip(after.iter()).enumerate() {
            if b != a {
                assert!(
                    k >= lo && k < hi,
                    "case {case}: {name}{args:?} wrote byte {k} outside [{lo},{hi}) res={res:?}"
                );
            }
        }
        if r.is_poisoned() {
            r.restart().unwrap();
            let h = get_handles(&mut r);
            c0 = h.0;
            c1 = h.1;
            slot = h.2;
            let _ = r.call_export("write_big", &[i(11)]);
        }
    }
    (oks, traps, st1)
}

#[test]
fn f5_trampoline_invariant_fuzz() {
    let (oks, traps, st1) = run_trampoline_fuzz(cases(150_000) as _, 0x9E37_79B9_7F4A_7C15);
    eprintln!("F5 cases=150000 ok={oks} trap={traps} status1={st1}");
}

// ---------------------------------------------------------------- F7
fn rss_kib() -> u64 {
    let s = std::fs::read_to_string("/proc/self/statm").unwrap_or_default();
    s.split_whitespace()
        .nth(1)
        .and_then(|p| p.parse::<u64>().ok())
        .unwrap_or(0)
        * 4
}

fn load_time(label: &str, f: &Fixture) -> (f64, String) {
    let b = f.bytes();
    let e = env();
    let t = Instant::now();
    let r = e.host.instantiate(&b, Grants::all());
    let el = t.elapsed().as_secs_f64();
    let out = match &r {
        Ok(_) => "ACCEPTED".to_string(),
        Err(er) => format!("{}", er).chars().take(90).collect(),
    };
    eprintln!(
        "F7 {label}: {} bytes, {:.1} ms, rss={}KiB -> {out}",
        b.len(),
        el * 1000.0,
        rss_kib()
    );
    (el, out)
}

#[test]
fn f7_translation_time_of_hostile_modules() {
    // many funcs
    let mut body = String::new();
    for n in 0..9000 {
        body.push_str(&format!(
            "(func $f{n} (param i32) (result i32) (i32.add (local.get 0) (i32.const {n})))\n"
        ));
    }
    load_time("9000 tiny funcs", &Fixture::new(&[]).body(&body));
    // huge br_table
    let targets = "0 ".repeat(400_000);
    let body = format!(
        r#"(func (export "bt") (param i32) (block (block (br_table {targets}0 (local.get 0)))))"#
    );
    load_time("br_table 400k", &Fixture::new(&[]).body(&body));
    // one giant straight-line function
    let mut s = String::from("(func (export \"big\") (param i32) (result i32) (local i32)\n");
    for n in 0..300_000 {
        s.push_str(&format!(
            "(local.set 1 (i32.add (local.get 1) (i32.mul (local.get 0) (i32.const {n}))))\n"
        ));
    }
    s.push_str("(local.get 1))");
    load_time("giant function 300k stmts", &Fixture::new(&[]).body(&s));
    // many locals
    let body = format!(
        r#"(func (export "loc") (local {}) )"#,
        "i64 ".repeat(40_000)
    );
    load_time("40k locals", &Fixture::new(&[]).body(&body));
    // deep nesting
    let n = 20_000;
    let body = format!(
        r#"(func (export "deep") {} nop {})"#,
        "(block ".repeat(n),
        ")".repeat(n)
    );
    load_time("nesting 20k", &Fixture::new(&[]).body(&body));
    // many br_ifs in one function with loops (regalloc-ish)
    let mut s = String::from("(func (export \"loops\") (param i32) (result i32) \n");
    for _ in 0..3000 {
        s.push_str("(block (loop (br_if 1 (local.get 0)) (br 0)))\n");
    }
    s.push_str("(i32.const 0))");
    load_time("3000 loops", &Fixture::new(&[]).body(&s));
    // big data segments
    let d = "\\00".repeat(1_000_000);
    load_time(
        "1MB data segment",
        &Fixture::new(&[]).body(&format!("(data (i32.const 70000) \"{d}\")")),
    );
}

#[test]
fn f7b_nested_branchy_translation_time() {
    for (depth, brs) in [(1000usize, 100_000usize), (5000, 60_000), (500, 150_000)] {
        let mut s = String::from("(func (export \"nb\") (param i32) (result i32)\n");
        for _ in 0..depth {
            s.push_str("(block (result i32)\n");
        }
        s.push_str("(i32.const 1)\n");
        for k in 0..brs {
            s.push_str(&format!(
                "(i32.add (br_if {} (i32.const {k}) (local.get 0)))\n",
                k % depth
            ));
        }
        for _ in 0..depth {
            s.push_str(")\n");
        }
        s.push(')');
        load_time(
            &format!("nested depth={depth} brs={brs}"),
            &Fixture::new(&[]).body(&s),
        );
    }
}

// ---------------------------------------------------------------- F6
fn bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    (0..n).map(|_| (r.next() >> 24) as u8).collect()
}

/// wasm-smith with every proposal on and invalid funcs allowed: count engine
/// panics (reported as E0408 "the engine panicked") and runtime panics.
#[test]
fn f6_wasm_smith_all_features_no_host_panic() {
    use arbitrary::{Arbitrary, Unstructured};
    let e = env();
    let n: u64 = std::env::var("BA_SMITH_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(cases(20_000));
    let (mut gen, mut engine_panics, mut codes) =
        (0u64, 0u64, std::collections::BTreeMap::<String, u64>::new());
    let mut panicked_seeds = vec![];
    for seed in 0..n {
        let raw = bytes(seed, 4096);
        let mut u = Unstructured::new(&raw);
        let mut cfg = wasm_smith::Config::arbitrary(&mut u).unwrap_or_default();
        cfg.allow_invalid_funcs = seed % 3 == 0;
        cfg.exceptions_enabled = true;
        cfg.gc_enabled = true;
        cfg.max_imports = 8;
        let Ok(m) = wasm_smith::Module::new(cfg, &mut u) else {
            continue;
        };
        let mut wasm = m.to_bytes();
        gen += 1;
        append_custom(
            &mut wasm,
            "oxi.manifest",
            Fixture::new(&[]).manifest_text().as_bytes(),
        );
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            e.host.instantiate(&wasm, Grants::all())
        }));
        match r {
            Err(_) => panicked_seeds.push(seed),
            Ok(Ok(_)) => panic!("accepted random module seed {seed}"),
            Ok(Err(er)) => {
                if er.to_string().contains("panicked") {
                    engine_panics += 1;
                    panicked_seeds.push(seed);
                }
                *codes.entry(er.code().to_string()).or_default() += 1;
            }
        }
    }
    eprintln!("F6 smith generated={gen} engine_panics_caught={engine_panics} escaped_or_caught_seeds={panicked_seeds:?} codes={codes:?}");
    assert!(
        panicked_seeds.is_empty(),
        "F6: panics on seeds {panicked_seeds:?}"
    );
}

/// Mutated conforming modules, driven through every export with edge args.
#[test]
fn f6b_mutation_fuzz_load_and_call() {
    let e = env_with(Limits {
        fuel_per_call: 200_000,
        ..Limits::default()
    });
    let base = {
        let mut f = raw();
        f.memory_pages = 2;
        f.bytes()
    };
    let n: u64 = std::env::var("BA_MUT_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(cases(30_000));
    let (mut loaded, mut called, mut panics) = (0u64, 0u64, vec![]);
    let edge = [
        0i32,
        1,
        -1,
        1023,
        1024,
        2048,
        3000,
        3100,
        65535,
        65536,
        131071,
        131072,
        i32::MAX,
        i32::MIN,
        16384,
        70000,
    ];
    for seed in 0..n {
        let mut w = base.clone();
        let mut rng = Rng(seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1);
        let nm = 1 + rng.next() % 4;
        for _ in 0..nm {
            let p = (rng.next() as usize) % w.len();
            match rng.next() % 4 {
                0 => w[p] ^= 1 << (rng.next() % 8),
                1 => w[p] = rng.next() as u8,
                2 => w[p] = [0, 1, 0x7f, 0x80, 0xff][(rng.next() % 5) as usize],
                _ => {
                    let q = (rng.next() as usize) % w.len();
                    w.swap(p, q)
                }
            }
        }
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let Ok(mut r) = e.host.instantiate(&w, Grants::all()) else {
                return (0, 0);
            };
            let mut c = 0;
            for name in r.user_exports() {
                let Some((ps, _)) = r.export_type(&name) else {
                    continue;
                };
                let args: Vec<Value> = ps
                    .iter()
                    .map(|t| match t {
                        wasmi::ValType::I64 => Value::I64(edge[(rng.next() % 16) as usize] as i64),
                        wasmi::ValType::F32 => Value::F32(f32::from_bits(rng.next() as u32)),
                        wasmi::ValType::F64 => Value::F64(f64::from_bits(rng.next())),
                        _ => Value::I32(edge[(rng.next() % 16) as usize]),
                    })
                    .collect();
                let _ = r.call_export(&name, &args);
                c += 1;
                if r.is_poisoned() {
                    let _ = r.restart();
                }
            }
            let _ = r.pump();
            let _ = r.tick(5);
            (1, c)
        }));
        match res {
            Ok((l, c)) => {
                loaded += l;
                called += c
            }
            Err(_) => panics.push(seed),
        }
    }
    eprintln!("F6b cases={n} loaded={loaded} exports_called={called} panics={panics:?}");
    assert!(panics.is_empty(), "panic seeds {panics:?}");
}

// ---------------------------------------------------------------- misc probes
#[test]
fn nan_results_are_canonical() {
    let e = env();
    let f = raw().body(
        r#"
(func (export "nan32") (result i32)
  (i32.reinterpret_f32 (f32.add (f32.reinterpret_i32 (i32.const 0x7fa00001)) (f32.const 1))))
(func (export "nan32b") (result i32)
  (i32.reinterpret_f32 (f32.demote_f64 (f64.reinterpret_i64 (i64.const 0x7ff0000000000123)))))
(func (export "nan64") (result i64)
  (i64.reinterpret_f64 (f64.sqrt (f64.reinterpret_i64 (i64.const 0xfff8000000000123)))))
(func (export "nan64b") (result i64)
  (i64.reinterpret_f64 (f64.promote_f32 (f32.reinterpret_i32 (i32.const 0x7fa00001)))))
(func (export "nan32c") (result i32)
  (i32.reinterpret_f32 (f32.min (f32.reinterpret_i32 (i32.const 0x7fa00001)) (f32.const 1))))
(func (export "nan32d") (result i32)
  (i32.reinterpret_f32 (f32.neg (f32.reinterpret_i32 (i32.const 0x7fa00001)))))
"#,
    );
    let mut r = rt(&e, &f);
    for n in ["nan32", "nan32b", "nan64", "nan64b", "nan32c", "nan32d"] {
        eprintln!("NAN {n} = {:?}", call(&mut r, n, &[]));
    }
}

#[test]
fn reexported_import_and_reserved_names_via_call_export() {
    let e = env();
    // re-export the host import `log.write` as a user export and as the callback
    let f = raw()
        .body(r#"(export "relog" (func $log_write)) (export "cb:host:state@1.State.OnChange" (func $log_write))"#)
        .over("oxi_release", "(func (export \"oxi_release\") (param i32))");
    // duplicate callback name with the stub: remove stub by building raw_with
    let f2 = raw_with("", None).body(
        r#"(export "relog" (func $log_write)) (export "cb:host:state@1.State.OnChange" (func $log_write))"#);
    let _ = f;
    let mut r = match e.host.instantiate(&f2.bytes(), Grants::all()) {
        Ok(r) => r,
        Err(er) => {
            eprintln!("REEXPORT rejected: {er}");
            return;
        }
    };
    eprintln!("REEXPORT accepted; user_exports={:?}", r.user_exports());
    let c0 = call(&mut r, "cap0", &[])[0].clone();
    let res = r.call_export("relog", &[c0.clone(), i(1), i(3000), i(5)]);
    eprintln!("REEXPORT relog -> {res:?} logs={:?}", e.sink.lines().len());
    for n in [
        "cb:host:state@1.State.OnChange",
        "memory",
        "oxi_release",
        "oxi_init",
        "oxi_tick",
    ] {
        eprintln!(
            "RESERVED {n}: {:?}",
            r.call_export(n, &[]).map(|_| ()).map_err(|e| e.to_string())
        );
    }
    // pump delivers a callback that IS the host import
    let c1 = call(&mut r, "cap1", &[])[0].clone();
    let slot = call(&mut r, "slot", &[])[0].clone();
    call(&mut r, "r_watch", &[c1, slot, c0.clone()]); // closure = log cap handle
    e.store
        .set("k", oxiscript_runtime::caps::state::StateValue::I64(5))
        .unwrap();
    let before = e.sink.lines().len();
    let p = r.pump();
    eprintln!(
        "REEXPORT pump -> {p:?} log lines {} -> {}",
        before,
        e.sink.lines().len()
    );
}

/// Fuel vs wall time of bulk-memory / grow / div heavy turns (determines the
/// worst-case blocking time of one guest entry).
#[test]
fn worst_case_wall_time_per_entry() {
    let e = env();
    let f = raw().body(
        r#"
(func (export "grow_all") (result i32)
  (drop (memory.grow (i32.const 254))) (memory.size))
(func (export "fill_loop") (param $n i32)
  (loop $l
    (memory.fill (i32.const 1024) (local.get $n) (i32.const 15000000))
    (memory.copy (i32.const 8000000) (i32.const 1024) (i32.const 7000000))
    (br $l)))
(func (export "div_loop") (result i64) (local $a i64) (local $b i64)
  (local.set $a (i64.const 0x7fffffffffffffff)) (local.set $b (i64.const 3))
  (loop $l
    (local.set $a (i64.add (i64.rem_u (i64.mul (local.get $a) (i64.const 6364136223846793005)) (i64.const 1000000007)) (i64.const 12345678901)))
    (br $l))
  (local.get $a))
(func (export "f64_loop") (result f64) (local $a f64)
  (local.set $a (f64.const 1.5))
  (loop $l
    (local.set $a (f64.sqrt (f64.add (f64.mul (local.get $a) (local.get $a)) (f64.const 1.0))))
    (br $l))
  (local.get $a))
"#,
    );
    for (name, args) in [
        ("grow_all", vec![]),
        ("fill_loop", vec![i(7)]),
        ("div_loop", vec![]),
        ("f64_loop", vec![]),
    ] {
        let mut r = rt(&e, &f);
        if name == "fill_loop" {
            call(&mut r, "grow_all", &[]);
        }
        let t = Instant::now();
        let res = r.call_export(name, &args);
        eprintln!(
            "WALL {name}: {:?} wall={:?} fuel={} pages={}",
            res.as_ref().map(|_| ()).map_err(|e| e
                .to_string()
                .chars()
                .take(40)
                .collect::<String>()),
            t.elapsed(),
            r.last_fuel_used(),
            r.memory_pages()
        );
    }
}

#[test]
fn reload_storm_cost_and_rss() {
    let e = env();
    let f = raw();
    let b = f.bytes();
    let mut r = e.host.instantiate(&b, Grants::all()).unwrap();
    let rss0 = rss_kib();
    let t = Instant::now();
    let n = 3000;
    for _ in 0..n {
        r.reload(&b).unwrap();
    }
    let el = t.elapsed();
    eprintln!(
        "RELOAD {n} reloads of {} bytes: {:?} ({:?}/reload) rss {}KiB -> {}KiB live={}",
        b.len(),
        el,
        el / n,
        rss0,
        rss_kib(),
        e.host.live_instances()
    );
    // big-memory instance reload
    let big = raw().body(r#"(func (export "touch") (local $i i32) (loop $l (i32.store8 (local.get $i) (i32.const 1)) (local.set $i (i32.add (local.get $i) (i32.const 4096))) (br_if $l (i32.lt_u (local.get $i) (i32.const 16000000)))))"#);
    let _ = big;
}

// ---------------------------------------------------------------- F9
/// F9 the shared StateStore has no per-script quota and no delete: one script
/// (restarting to reset its 256-slot table) fills all 1024 keys forever.
#[test]
fn f9_one_script_exhausts_the_shared_state_store() {
    let e = env();
    let f = raw().body(
        r#"
(func (export "fill") (param $base i32) (result i32)
  (local $i i32) (local $st i32) (local $h i32) (local $ok i32)
  (loop $l
    (i32.store8 (i32.const 9000) (i32.add (i32.const 65) (i32.and (local.get $i) (i32.const 15))))
    (i32.store8 (i32.const 9001) (i32.add (i32.const 65) (i32.shr_u (local.get $i) (i32.const 4))))
    (i32.store8 (i32.const 9002) (i32.add (i32.const 65) (i32.and (local.get $base) (i32.const 15))))
    (i32.store8 (i32.const 9003) (i32.const 75))
    (call $s_open (global.get $cap1) (i32.const 9000) (i32.const 4))
    (local.set $h) (local.set $st)
    (if (i32.eqz (local.get $st))
      (then (if (i32.eqz (call $s_set (global.get $cap1) (local.get $h) (i64.const 1)))
               (then (local.set $ok (i32.add (local.get $ok) (i32.const 1)))))))
    (local.set $i (i32.add (local.get $i) (i32.const 1)))
    (br_if $l (i32.lt_u (local.get $i) (i32.const 256))))
  (local.get $ok))
"#,
    );
    let mut evil = rt(&e, &f);
    let mut total = 0;
    for round in 0..5 {
        let r = call(&mut evil, "fill", &[i(round)]);
        if let Value::I32(n) = r[0] {
            total += n
        }
        evil.restart().unwrap(); // fresh slot table, host state survives
    }
    eprintln!("F9 evil wrote {total} keys, store.len={}", e.store.len());
    // an innocent script now cannot create its first key
    let mut good = rt(&e, &raw());
    let c1 = call(&mut good, "cap1", &[])[0].clone();
    // key "zzzz" at 3000 ("hell")
    let h = call(&mut good, "r_open", &[c1.clone(), i(3000), i(4)]);
    let s = call(&mut good, "r_set", &[c1, h[1].clone(), Value::I64(1)]);
    assert_eq!(
        st(s),
        0,
        "F9: innocent script denied (status 18) because another script filled the shared store"
    );
}

// ---------------------------------------------------------------- F10
/// F10 log lines are stored raw: newline / ANSI escapes injected by the guest.
#[test]
fn f10_log_injection_is_not_sanitised() {
    let e = env();
    let f = raw().body(r#"(data (i32.const 9100) "ok\0a[ERROR] forged line\1b[2J\00x")"#);
    let mut r = rt(&e, &f);
    let c0 = call(&mut r, "cap0", &[])[0].clone();
    call(&mut r, "r_log", &[c0, i(1), i(9100), i(27)]);
    let l = e.sink.lines();
    let m = &l.last().unwrap().msg;
    eprintln!("F10 stored {m:?}");
    assert!(
        !m.chars().any(|c| c.is_control()),
        "F10: control chars survive in the sink: {m:?}"
    );
}
