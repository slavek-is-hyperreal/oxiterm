//! BAD ACTOR round 2026-11-27-1, target 1 (8.Sec/8.Fix loader change in `load.rs`).
//!
//! The branch replaced `wasmi::EnforcedLimits::strict()` with `default()` plus its
//! own structural caps, dropping the min-average-bytes-per-function rule. These
//! tests craft small modules whose *eager compilation* or *instantiation* cost is
//! far out of proportion with their size. Failing tests are findings
//! (docs/stages/rounds/2026-11-27-1.md, R1-nn).
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::Fixture;
use oxiscript_runtime::{Grants, Host, Limits};
use std::time::{Duration, Instant};

/// Loading a module of at most `max_module_bytes` must stay cheap: the budget is
/// generous (a real 4 MiB program loads in well under a second in release).
const LOAD_BUDGET: Duration = Duration::from_secs(2);

fn time_load(wasm: &[u8]) -> (Duration, Result<(), String>) {
    let host = Host::new(Limits::default());
    let t = Instant::now();
    let r = host.instantiate(wasm, Grants::none()).map(|_| ()).map_err(|e| format!("{e:?}"));
    (t.elapsed(), r)
}

fn assert_cheap(name: &str, wasm: &[u8]) {
    let (dt, r) = time_load(wasm);
    eprintln!("{name}: {} bytes, load {:?}, result {:?}", wasm.len(), dt, r.as_ref().map_err(|e| &e[..e.len().min(160)]));
    assert!(
        dt < LOAD_BUDGET,
        "{name}: a {}-byte module took {dt:?} to load/reject (budget {LOAD_BUDGET:?})",
        wasm.len()
    );
}

/// R1-01: 10 000 functions, each with 50 000 declared locals in one compressed
/// local entry (about 8 bytes per body). `strict()` refused this shape (average
/// body < 40 bytes); `default()` + the new caps accept it. Measures how much
/// eager compilation and instantiation cost the shape buys.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn r1_01_many_functions_with_max_locals_tiny_bodies() {
    let mut body = String::new();
    for _ in 0..9_000 {
        body.push_str("(func (local i64 i64 i64 i64 i64 i64 i64 i64 i64 i64))\n");
    }
    // The text format has no compressed-locals syntax; patch locals after encoding
    // would be simpler, but a WAT `(local $x i64)` list of 50 000 entries per
    // function is too large. Use a smaller fan-out (10 locals) as the control and
    // the binary-patched module below as the attack.
    let control = Fixture::new(&[]).body(&body).bytes();
    assert_cheap("control: 9000 funcs x 10 locals", &control);

    // wasmi caps a function at 30 000 locals (TooManyFunctionParams above that).
    let attack = many_locals_module(9_900, 29_990);
    let (dt, r) = time_load(&attack);
    eprintln!("attack: 9900 funcs x 29990 locals: {} bytes, {dt:?}, {r:?}", attack.len());
    assert!(r.is_ok(), "accepted shape expected: {r:?}");
    // A legitimate module loads at roughly 80 ns/byte in release (600 KB of nested
    // blocks: ~48 ms). Allow 20x that before calling it an amplification.
    let budget = Duration::from_nanos(attack.len() as u64 * 80 * 20).max(Duration::from_millis(100));
    assert!(
        dt < budget,
        "R1-01: a {}-byte module costs {dt:?} of eager compilation (budget {budget:?}); strict() required >= 40 bytes per body",
        attack.len()
    );
}

/// Build the conforming fixture, then append `n` extra defined functions whose
/// bodies are `locals` x i64 declared in one compressed entry and `end`.
fn many_locals_module(n: u32, locals: u32) -> Vec<u8> {
    // Each extra function is defined in WAT with one local, then its local count
    // is rewritten in the code section. Simpler: generate WAT with a placeholder
    // body and rebuild the code section with wasm-encoder-like manual encoding.
    let mut body = String::new();
    for _ in 0..n {
        body.push_str("(func (local i64))\n");
    }
    let wasm = Fixture::new(&[]).body(&body).bytes();
    rewrite_single_local_counts(&wasm, locals)
}

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

/// Rewrite every code-section body that is exactly `01 01 7e 0b` (one i64 local,
/// `end`) into `01 <locals> 7e 0b`.
fn rewrite_single_local_counts(wasm: &[u8], locals: u32) -> Vec<u8> {
    let mut out = wasm[..8].to_vec();
    let mut p = 8;
    while p < wasm.len() {
        let id = wasm[p];
        p += 1;
        let size = read_leb(wasm, &mut p) as usize;
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
        for _ in 0..count {
            let bsz = read_leb(sec, &mut q) as usize;
            let fb = &sec[q..q + bsz];
            q += bsz;
            if fb == [1, 1, 0x7e, 0x0b] {
                let mut nb = vec![1];
                leb_u32(locals, &mut nb);
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

/// Probe (passes): one function body filling most of the 4 MiB cap with
/// `local.get 0` pushes followed by `local.set 0` and drops: the classic
/// "preserve every stack copy of a local on write" translation pattern.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn probe_local_preservation_pattern() {
    let k = 60_000;
    let mut f = String::from("(func (export \"lp\") (param i32) (result i32)\n");
    for _ in 0..k {
        f.push_str("local.get 0\n");
    }
    for _ in 0..200 {
        f.push_str("i32.const 1\nlocal.set 0\n");
    }
    for _ in 0..k - 1 {
        f.push_str("i32.add\n");
    }
    f.push_str(")\n");
    let wasm = Fixture::new(&[]).body(&f).bytes();
    assert_cheap("local preservation", &wasm);
}

/// Probe (passes): deep block nesting inside one function.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn probe_deep_block_nesting() {
    let n = 200_000;
    let mut f = String::from("(func (export \"deep\")\n");
    f.push_str(&"block\n".repeat(n));
    f.push_str(&"end\n".repeat(n));
    f.push_str(")\n");
    let wasm = Fixture::new(&[]).body(&f).bytes();
    assert_cheap("deep blocks", &wasm);
}

/// Probe (passes): one `br_table` with a million targets, repeated.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn probe_huge_br_table() {
    let mut f = String::from("(func (export \"bt\") (param i32)\n block\n");
    for _ in 0..3 {
        f.push_str("local.get 0\nbr_table ");
        f.push_str(&"0 ".repeat(900_000));
        f.push_str("0\n");
    }
    f.push_str(" end)\n");
    let wasm = Fixture::new(&[]).body(&f).bytes();
    assert_cheap("huge br_table", &wasm);
}

/// Probe (passes): passive element segments are not counted per item: 1000 segments x
/// ~1000 `ref.func` items is ~1M stored references, a memory/instantiation
/// amplification the per-segment cap does not see. Also 1000 passive data
/// segments are fine (bounded by module bytes).
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn probe_element_items_amplification() {
    let mut body = String::from("(func $z)\n");
    let items = " $z".repeat(1_300);
    for _ in 0..1_000 {
        body.push_str(&format!("(elem func{items})\n"));
    }
    let wasm = Fixture::new(&[]).body(&body).bytes();
    assert_cheap("element items", &wasm);
}

/// R1-02: `Limits::call_wall_micros` is documented in 8.Sec/closed.md as "checked
/// independently of fuel" and its test is named `..._stops_call_independent_of_fuel`,
/// but it is only checked at host-call boundaries and after the call returns. A
/// guest loop that makes no host call runs its whole fuel budget regardless of
/// the wall cap: the cap reports, it does not stop.
#[test]
fn r1_02_wall_cap_does_not_stop_a_compute_only_loop() {
    use oxiscript_runtime::{HostOptions, Value};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;
    let tick = Arc::new(AtomicU64::new(1000));
    let t2 = Arc::clone(&tick);
    let clock = Arc::new(move || t2.fetch_add(500, Ordering::SeqCst));
    let limits = Limits {
        fuel_per_call: 5_000_000,
        call_wall_micros: 1_000,
        ..Limits::default()
    };
    let host = Host::with_options(limits, HostOptions { clock: Some(clock), ..HostOptions::default() });
    let wasm = Fixture::new(&[])
        .body(
            r#"
(func (export "spin") (param $n i32)
  (loop $l
    (local.set $n (i32.sub (local.get $n) (i32.const 1)))
    (br_if $l (local.get $n))))
"#,
        )
        .bytes();
    let mut rt = host.instantiate(&wasm, Grants::none()).expect("loads");
    let r = rt.call_export("spin", &[Value::I32(i32::MAX)]);
    eprintln!("result {r:?}, fuel used {}", rt.last_fuel_used());
    assert!(
        rt.last_fuel_used() < 5_000_000 / 2,
        "R1-02: the wall cap (1 ms, fake clock +500 us per read) did not stop a compute-only loop: it ran {} fuel ({r:?})",
        rt.last_fuel_used()
    );
}

/// R1-05: the shared sanitiser escapes U+200C ZWNJ and U+200D ZWJ, which are
/// ordinary text (emoji ZWJ sequences, Persian and Indic orthography), not
/// terminal controls or bidi overrides. Guest log lines (and so `oxi run` and the
/// playground) are mangled: main only escaped control characters.
#[test]
fn r1_05_log_sanitiser_mangles_zwj_and_zwnj_text() {
    use oxiscript_runtime::caps::log::sanitize;
    for text in ["family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}", "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}", "\u{0915}\u{094D}\u{200D}\u{0937}"] {
        assert_eq!(sanitize(text, 512), text, "ordinary text was escaped");
    }
}
