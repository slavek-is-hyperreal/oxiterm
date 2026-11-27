//! Stage 8.Sec regression tests for `oxiscript-runtime`:
//! - `[R3-206]` shared terminal-escape/bidi sanitising in `caps::log`
//! - `[R3-214]` wall-clock limit independent of fuel + proportional host-byte fuel
//! - `[R3-221]` hot reload cannot widen capabilities
//! - `[R3-213]` fuel exhaustion fault-injection sweep
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::*;
use oxiscript_abi::sanitize::is_unsafe_terminal_char;
use oxiscript_runtime::caps::log::sanitize as log_sanitize;
use oxiscript_runtime::{Error, Grants, Host, HostOptions, Limits, TrapCause, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[test]
fn log_sanitize_escapes_esc_osc_csi_c1_cr_and_bidi() {
    let raw = "\x1b[2J\x1b]0;pwn\x07\u{009b}31m\r\n\u{202e}\u{2066}hi\u{2069}";
    let clean = log_sanitize(raw, 512);
    assert!(!clean.chars().any(is_unsafe_terminal_char), "{clean:?}");
    assert!(clean.contains("\\x1b[2J"));
    assert!(clean.contains("\\x9b31m"));
    assert!(clean.contains("\\x0d"));
    assert!(clean.contains("\\u{202e}"));
}

#[test]
fn hot_reload_cannot_widen_capabilities() {
    let e = env();
    // v1 declares only host:log@1.
    let v1 = Fixture::new(&[LOG])
        .imports(IMPORT_LOG)
        .body(
            r#"
(data (i32.const 3000) "v1")
(func (export "ping") (result i32)
  (call $log_write (global.get $cap0) (i32.const 2) (i32.const 3000) (i32.const 2))
  (i32.const 1))
"#,
        )
        .bytes();

    // Even when Runtime holds Grants::all(), reloading from [LOG] to [LOG, STATE] must be rejected.
    let mut rt = e.host.instantiate(&v1, Grants::all()).expect("v1 loads");
    assert_eq!(rt.call_export("ping", &[]).unwrap(), vec![Value::I32(1)]);

    let wider = Fixture::new(&[LOG, STATE])
        .imports(IMPORT_LOG)
        .imports(IMPORT_STATE)
        .body(
            r#"
(data (i32.const 3100) "pwn")
(global $s (mut i32) (i32.const 0))
(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32))
(func (export "ping") (result i32) (i32.const 2))
"#,
        )
        .init(
            r#"
(call $s_open (global.get $cap1) (i32.const 3100) (i32.const 3))
(global.set $s) (drop)
(drop (call $s_set (global.get $cap1) (global.get $s) (i64.const 99)))
"#,
        )
        .bytes();
    let err = rt.reload(&wider).expect_err("widening reload must fail");
    assert_eq!(err.code(), "OXS-E0400", "expected OXS-E0400 on widening reload, got {err:?}");
    // Old instance stays alive and untouched; v2's oxi_init never wrote to state.
    assert_eq!(rt.call_export("ping", &[]).unwrap(), vec![Value::I32(1)]);
    assert_eq!(e.store.get_in(rt.namespace(), "pwn"), None);

    // Narrowing from [LOG, STATE] -> [LOG] is allowed, and then re-widening -> [LOG, STATE] is rejected.
    let mut rt2 = e.host.instantiate(&wider, Grants::all()).expect("wide loads");
    rt2.reload(&v1).expect("narrowing reload is allowed");
    let err2 = rt2.reload(&wider).expect_err("re-widening after narrowing must fail");
    assert_eq!(err2.code(), "OXS-E0400", "expected OXS-E0400 on re-widening reload, got {err2:?}");
}

#[test]
fn wall_clock_limit_stops_call_independent_of_fuel() {
    // Fake clock that advances by 500 us on every read.
    let tick = Arc::new(AtomicU64::new(1000));
    let clk_tick = Arc::clone(&tick);
    let clock = Arc::new(move || clk_tick.fetch_add(500, Ordering::SeqCst));

    let limits = Limits {
        fuel_per_call: 100_000_000, // huge fuel: must not be the reason we stop
        call_wall_micros: 1_000,    // 1 ms wall-clock cap
        ..Limits::default()
    };
    let host = Host::with_options(
        limits,
        HostOptions {
            clock: Some(clock),
            ..HostOptions::default()
        },
    );
    let sink = oxiscript_runtime::caps::log::LogSink::new(64);
    host.register(Arc::new(oxiscript_runtime::caps::log::LogProvider::new(
        Arc::clone(&sink),
        oxiscript_runtime::caps::log::LogLimits::default(),
    )))
    .unwrap();

    let wasm = Fixture::new(&[LOG])
        .imports(IMPORT_LOG)
        .body(
            r#"
(data (i32.const 3000) "tick")
(func (export "chatty") (param $n i32)
  (loop $l
    (call $log_write (global.get $cap0) (i32.const 2) (i32.const 3000) (i32.const 4))
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
"#,
        )
        .bytes();

    let mut rt = host.instantiate(&wasm, Grants::all()).expect("loads");
    let err = rt.call_export("chatty", &[Value::I32(10)]).expect_err("wall-clock cap must trip");
    match err {
        Error::Trap(info) => match info.cause {
            TrapCause::HostFailure(m) => assert!(m.contains("wall-clock"), "unexpected message: {m}"),
            other => panic!("expected HostFailure(wall-clock), got {other:?}"),
        },
        other => panic!("expected Trap, got {other:?}"),
    }
    assert!(rt.is_poisoned(), "wall-clock trap must poison the instance");
    assert!(rt.last_fuel_used() < 100_000_000, "must stop on wall-clock before exhausting fuel");
}

#[test]
fn host_bytes_charge_proportional_fuel_including_buffer_too_small() {
    let e = env_with(Limits {
        bytes_per_fuel: 16,
        host_call_fuel: 100,
        ..Limits::default()
    });
    let wasm = Fixture::new(&[LOG, STATE])
        .imports(IMPORT_LOG)
        .imports(IMPORT_STATE)
        .body(
            r#"
(data (i32.const 3100) "k")
(global $slot (mut i32) (i32.const 0))
(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32))
(func (export "write_n") (param $len i32) (result i32)
  (call $s_write (global.get $cap1) (global.get $slot) (i32.const 4096) (local.get $len)))
(func (export "read_n") (param $cap i32) (result i32 i32)
  (call $s_read (global.get $cap1) (global.get $slot) (i32.const 4096) (local.get $cap)))
"#,
        )
        .init(
            r#"
(call $s_open (global.get $cap1) (i32.const 3100) (i32.const 1))
(global.set $slot) (drop)
"#,
        )
        .bytes();

    let mut rt = e.host.instantiate(&wasm, Grants::all()).unwrap();
    // Write 16 bytes vs 4096 bytes: fuel scales proportionally with payload bytes.
    rt.call_export("write_n", &[Value::I32(16)]).unwrap();
    let f_small_w = rt.last_fuel_used();
    rt.call_export("write_n", &[Value::I32(4096)]).unwrap();
    let f_big_w = rt.last_fuel_used();
    assert!(
        f_big_w >= f_small_w + 200,
        "input bytes must charge proportional fuel: small={f_small_w}, big={f_big_w}"
    );

    // Read with out_cap=0 (STATUS_BUFFER_TOO_SMALL) still charges proportional fuel for the 4096-byte value.
    let res = rt.call_export("read_n", &[Value::I32(0)]).unwrap();
    assert_eq!(res, vec![Value::I32(1), Value::I32(4096)]);
    let f_too_small = rt.last_fuel_used();
    assert!(
        f_too_small >= 100 + 4096 / 16,
        "STATUS_BUFFER_TOO_SMALL must charge proportional fuel, got {f_too_small}"
    );
}

#[test]
fn fuel_fault_injection_sweep_poisons_cleanly_and_recovers_on_restart() {
    let wasm = Fixture::new(&[LOG, STATE])
        .imports(IMPORT_LOG)
        .imports(IMPORT_STATE)
        .body(
            r#"
(data (i32.const 3000) "hello")
(data (i32.const 3100) "k")
(global $slot (mut i32) (i32.const 0))
(func (export "cb:host:state@1.State.OnChange") (param i32 i32 i32 i32))
(func (export "work") (result i32)
  (call $log_write (global.get $cap0) (i32.const 2) (i32.const 3000) (i32.const 5))
  (drop (call $s_set (global.get $cap1) (global.get $slot) (i64.const 7)))
  (drop (call $s_write (global.get $cap1) (global.get $slot) (i32.const 3000) (i32.const 5)))
  (i32.const 42))
"#,
        )
        .init(
            r#"
(call $s_open (global.get $cap1) (i32.const 3100) (i32.const 1))
(global.set $slot) (drop)
"#,
        )
        .bytes();

    // Find full completion fuel first.
    let e0 = env();
    let mut rt0 = e0.host.instantiate(&wasm, Grants::all()).unwrap();
    assert_eq!(rt0.call_export("work", &[]).unwrap(), vec![Value::I32(42)]);
    let needed = rt0.last_fuel_used();

    let mut saw_trap = false;
    let mut saw_ok = false;
    for step in (1..=needed + 20).step_by(17) {
        let e = env_with(Limits {
            fuel_per_call: step,
            ..Limits::default()
        });
        match e.host.instantiate(&wasm, Grants::all()) {
            Err(Error::Trap(info)) => {
                assert_eq!(info.cause, TrapCause::OutOfFuel);
                saw_trap = true;
            }
            Ok(mut rt) => match rt.call_export("work", &[]) {
                Ok(v) => {
                    assert_eq!(v, vec![Value::I32(42)]);
                    saw_ok = true;
                }
                Err(Error::Trap(info)) => {
                    assert_eq!(info.cause, TrapCause::OutOfFuel);
                    assert!(rt.is_poisoned());
                    assert!(matches!(rt.call_export("work", &[]), Err(Error::Poisoned)));
                    saw_trap = true;
                }
                Err(other) => panic!("unexpected error at fuel={step}: {other:?}"),
            },
            Err(other) => panic!("unexpected init error at fuel={step}: {other:?}"),
        }
    }
    assert!(saw_trap && saw_ok, "sweep must cover both trap and ok paths");
}

/// R1-01 boundary: a module right at the 1 000 000 total declared locals cap loads
/// cheaply, while exceeding it by even 1 local is rejected with `OXS-E0408`.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn total_declared_locals_cap_accepts_at_limit_and_rejects_above() {
    use std::time::Instant;

    fn leb_u32(mut v: u32, out: &mut Vec<u8>) {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
    }

    fn read_leb(b: &[u8], p: &mut usize) -> u32 {
        let mut r = 0u32;
        let mut s = 0;
        loop {
            let x = b[*p];
            *p += 1;
            r |= u32::from(x & 0x7f) << s;
            s += 7;
            if x & 0x80 == 0 {
                return r;
            }
        }
    }

    fn module_with_locals(funcs: u32, locals_per_func: u32, extra_in_last: u32) -> Vec<u8> {
        let mut body = String::new();
        for _ in 0..funcs {
            body.push_str("(func (local i64))\n");
        }
        let wasm = Fixture::new(&[]).body(&body).bytes();
        let mut out = wasm[..8].to_vec();
        let mut p = 8;
        while p < wasm.len() {
            let id = wasm[p];
            p += 1;
            let size = read_leb(&wasm, &mut p) as usize;
            let sec = &wasm[p..p + size];
            p += size;
            if id != 10 {
                out.push(id);
                leb_u32(size as u32, &mut out);
                out.extend_from_slice(sec);
                continue;
            }
            let mut q = 0;
            let count = read_leb(sec, &mut q);
            let mut new = Vec::new();
            leb_u32(count, &mut new);
            let mut matched = 0u32;
            for _ in 0..count {
                let bsz = read_leb(sec, &mut q) as usize;
                let fb = &sec[q..q + bsz];
                q += bsz;
                if fb == [1, 1, 0x7e, 0x0b] {
                    matched += 1;
                    let n = if matched == funcs {
                        locals_per_func + extra_in_last
                    } else {
                        locals_per_func
                    };
                    let mut nb = vec![1];
                    leb_u32(n, &mut nb);
                    nb.extend_from_slice(&[0x7e, 0x0b]);
                    leb_u32(nb.len() as u32, &mut new);
                    new.extend_from_slice(&nb);
                } else {
                    leb_u32(bsz as u32, &mut new);
                    new.extend_from_slice(fb);
                }
            }
            out.push(10);
            leb_u32(new.len() as u32, &mut out);
            out.extend_from_slice(&new);
        }
        out
    }

    let host = Host::new(Limits::default());
    // Exactly 1 000 000 locals (50 functions x 20 000 locals): accepted and cheap.
    let at_cap = module_with_locals(50, 20_000, 0);
    let t0 = Instant::now();
    host.instantiate(&at_cap, Grants::none())
        .expect("1 000 000 total locals must be accepted");
    let dt_ok = t0.elapsed();
    eprintln!(
        "R1-01 at cap (50 x 20 000 = 1 000 000 locals): {} bytes, load {dt_ok:?}",
        at_cap.len()
    );
    assert!(dt_ok.as_millis() < 200, "at-cap load took {dt_ok:?}");

    // 1 000 001 locals: rejected immediately with OXS-E0408.
    let over_cap = module_with_locals(50, 20_000, 1);
    let t1 = Instant::now();
    let err = host
        .instantiate(&over_cap, Grants::none())
        .err()
        .expect("1 000 001 total locals must be rejected");
    let dt_err = t1.elapsed();
    eprintln!(
        "R1-01 over cap (1 000 001 locals): {} bytes, reject {dt_err:?}, err {err:?}",
        over_cap.len()
    );
    assert_eq!(err.code(), "OXS-E0408");
    assert!(dt_err.as_millis() < 50, "over-cap rejection took {dt_err:?}");
}
