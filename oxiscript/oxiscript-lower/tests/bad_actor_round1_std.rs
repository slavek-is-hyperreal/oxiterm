//! BAD ACTOR round 2026-11-27-1, target 2 (8.Std v0, Opus pass): the new global
//! `hash()`/`equals()` surface, cross-target determinism of std code, Map/Set with
//! re-entrant keys, and parseFloat inputs the first round's random generator does
//! not reach (exponent overflow, 768-digit sticky tail, very long inputs).
//! Failing tests are findings (R1-nn in docs/stages/rounds/2026-11-27-1.md).
#![allow(clippy::pedantic)]

mod common;

use common::{compile, interfaces, Prog};
use oxiscript_lower::{compile_source, Options};
use oxiscript_runtime::Value;

fn try_compile(src: &str) -> Result<Vec<u8>, String> {
    compile_source(src, &interfaces(), &Options { target: common::test_target(), ..Options::default() }).map_err(|e| e.to_string())
}

/// Probe: a user interface with the same shape as `Hash` / `Eq` used as a generic bound must
/// either accept primitives consistently (check + lower + run) or reject them in the checker,
/// never pass the checker and then fail in lowering.
#[test]
fn probe_user_interface_matching_builtin_hash_eq() {
    let src = r#"
interface Hashy {
  hash(): u64;
}
interface Same<T> {
  equals(other: T): bool;
}
function h<T extends Hashy>(x: T): u64 {
  return x.hash();
}
function same<T extends Same<T>>(a: T, b: T): bool {
  return a.equals(b);
}
@host export function run(): i32 {
  let n = 0;
  if (h(5) === (5).hash()) {
    n += 1;
  }
  if (h("a") === "a".hash()) {
    n += 2;
  }
  if (same(0.0 / 0.0, 0.0 / 0.0)) {
    n += 4;
  }
  if (same(true, true)) {
    n += 8;
  }
  return n;
}
"#;
    let ck = oxiscript_check::check_source(src, &oxiscript_check::CheckOptions { hosts: interfaces() });
    let checked = ck.ok();
    match try_compile(src) {
        Ok(w) => {
            let mut p = Prog::from_wasm(w);
            assert_eq!(p.call("run", &[]), vec![Value::I32(15)]);
        }
        Err(e) => assert!(!checked, "checker accepted the program but lowering failed:\n{e}"),
    }
}

/// Probe: float hash/equals SameValueZero invariants with distinct NaN payloads, both widths.
#[test]
fn probe_float_hash_equals_invariants() {
    let src = r#"
import { fromBits, fromBitsF32 } from "std/math";
@host export function run(): i32 {
  let bad = 0;
  const n1 = fromBits(0x7ff8000000000001u64);
  const n2 = fromBits(0xfff0000000000abcu64);
  if (!n1.equals(n2) || n1.hash() !== n2.hash()) {
    bad += 1;
  }
  if (!(-0.0).equals(0.0) || (-0.0).hash() !== (0.0).hash()) {
    bad += 2;
  }
  const m1 = fromBitsF32(0x7fc00001u32);
  const m2 = fromBitsF32(0xff800abcu32);
  if (!m1.equals(m2) || m1.hash() !== m2.hash()) {
    bad += 4;
  }
  const z: f32 = -0.0;
  const pz: f32 = 0.0;
  if (!z.equals(pz) || z.hash() !== pz.hash()) {
    bad += 8;
  }
  return bad;
}
"#;
    let mut p = Prog::new(src);
    assert_eq!(p.call("run", &[]), vec![Value::I32(0)]);
}

/// Strip custom sections so only code/data are compared.
fn code_only(wasm: &[u8]) -> Vec<u8> {
    let mut out = wasm[..8].to_vec();
    let mut p = 8;
    while p < wasm.len() {
        let id = wasm[p];
        let start = p;
        p += 1;
        let mut size = 0usize;
        let mut s = 0;
        loop {
            let b = wasm[p];
            p += 1;
            size |= ((b & 0x7f) as usize) << s;
            s += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        p += size;
        if id != 0 {
            out.extend_from_slice(&wasm[start..p]);
        }
    }
    out
}

/// Probe: std math/string code is target-independent: the code of one program compiled for
/// device, web and server is identical once the manifest (custom sections) is removed.
#[test]
fn probe_std_code_is_target_independent() {
    let src = r#"
import { sin, cos, tan, exp, log, pow, atan2, sqrt, cbrt, hypot } from "std/math";
import { parseFloat, toFixed } from "std/string";
@host export function run(x: f64, y: f64): f64 {
  const p = parseFloat(toFixed(x, 6));
  let v = 0.0;
  switch (p.kind) {
    case "ok":
      v = p.value;
    case "err":
      v = 0.0;
  }
  return sin(x) + cos(y) + tan(x * y) + exp(x) + log(y) + pow(x, y) + atan2(y, x) + sqrt(y) + cbrt(x) + hypot(x, y) + v;
}
"#;
    let mut outs = Vec::new();
    for t in oxiscript_abi::target::Target::ALL {
        let w = compile_source(src, &interfaces(), &Options { target: t, ..Options::default() }).expect("compile");
        outs.push((t, code_only(&w)));
    }
    for (t, w) in &outs[1..] {
        assert!(w == &outs[0].1, "code for {t} differs from {}", outs[0].0);
    }
}

/// R1-30: a Map key whose `equals()` inserts into the same Map while `set` is probing.
/// `set` keeps its `cap`, `slot` and `probed` from before the call; afterwards the table may
/// have been rehashed, so the stale probe can insert a duplicate of an existing key (or
/// lose one). Expected: either a defined trap, or a consistent map (size == distinct keys,
/// every key found).
#[test]
fn r1_30_map_reentrant_equals_keeps_map_consistent() {
    let src = r#"
import { Map } from "std/collections";
class K {
  v: i32;
  host: Map<K, i32> | null = null;
  constructor(v: i32) {
    this.v = v;
  }
  hash(): u64 {
    return 7u64;
  }
  equals(o: K): bool {
    const m = this.host;
    if (m !== null) {
      this.host = null;
      for (const i of range(100, 140)) {
        m.set(new K(i), i);
      }
    }
    return this.v === o.v;
  }
}
@host export function run(): i32 {
  const m = new Map<K, i32>();
  const a = new K(1);
  m.set(a, 1);
  m.set(new K(2), 2);
  a.host = m;
  // probes over `a` (all keys hash to 7): a.equals runs the re-entrant inserts
  m.set(new K(2), 22);
  // distinct keys: 1, 2, 100..139 = 42
  let found = 0;
  for (const i of range(100, 140)) {
    if (m.has(new K(i))) {
      found += 1;
    }
  }
  const two = m.get(new K(2));
  const twoOk = two !== null && two === 22;
  if (m.size() !== 42 || found !== 40 || !twoOk || !m.has(new K(1))) {
    return m.size() * 1000 + found * 10 + (twoOk ? 1 : 0);
  }
  return 0;
}
"#;
    let mut p = Prog::new(src);
    let r = p.rt.call_export("run", &[]);
    eprintln!("re-entrant map: {r:?}");
    if let Ok(v) = r {
        assert_eq!(v, vec![Value::I32(0)], "Map inconsistent after re-entrant equals (size*1000 + found*10 + twoOk)");
    }
}

fn pf_prog() -> Prog {
    let src = r#"
import { parseFloat } from "std/string";
import { bitsOf } from "std/math";
import { repeat } from "std/string";
@host export function pfr(n: i32, kind0: i32): u64 {
  const kind = kind0 % 10;
  let s = "";
  if (kind === 0) {
    s = `0.${repeat("0", n)}1e${n + 1}`;
  } else if (kind === 1) {
    s = `1${repeat("0", n)}e-${n}`;
  } else if (kind === 2) {
    s = `0.${repeat("1", n)}`;
  } else {
    s = `${repeat("9", n)}.5`;
  }
  if (kind0 >= 10) {
    return s.byteLength as u64;
  }
  return pf(s);
}
@host export function blen(n: i32, kind: i32): u64 {
  return pfr(n, kind + 10);
}
@host export function pf(s: string): u64 {
  const r = parseFloat(s);
  switch (r.kind) {
    case "ok":
      return bitsOf(r.value);
    case "err":
      return 0xdeadu64;
  }
}
"#;
    Prog::new(src)
}

fn pf(p: &mut Prog, s: &str) -> Result<u64, String> {
    // Values go through the scratch payload as a string argument.
    match p.rt.call_export("pf", &[Value::Str(s.to_string())]) {
        Ok(v) => match v.first() {
            Some(Value::I64(b)) => Ok(*b as u64),
            other => Err(format!("{other:?}")),
        },
        Err(e) => Err(e.to_string()),
    }
}

/// Exact decimal expansion of 2^-1075 (half of the smallest subnormal).
fn half_min_subnormal() -> String {
    // 5^1075 has 752 digits; 2^-1075 = 5^1075 * 10^-1075.
    let mut d: Vec<u32> = vec![1];
    for _ in 0..1075 {
        let mut c = 0u32;
        for x in &mut d {
            let v = *x * 5 + c;
            *x = v % 10;
            c = v / 10;
        }
        while c > 0 {
            d.push(c % 10);
            c /= 10;
        }
    }
    let digits: String = d.iter().rev().map(|x| char::from(b'0' + *x as u8)).collect();
    // value = digits * 10^-1075 = 0.<1075-len zeros><digits>
    format!("0.{}{}", "0".repeat(1075 - digits.len()), digits)
}

/// R1-31: parseFloat on inputs outside the first round's generator, compared with Rust. All the
/// short adversarial cases (exponent overflow, 768-digit sticky tail, halfway, f64::MAX
/// boundary) are correct; the finding is cost: ~1000-2800 fuel per input digit, also past the
/// 768-digit truncation point, so a 20 000-digit string (a third of max_host_payload) traps
/// OutOfFuel under the default 10M fuel.
#[test]
#[cfg_attr(debug_assertions, ignore = "heavy: run with --release")]
fn r1_31_parse_float_long_input_exhausts_fuel() {
    let half = half_min_subnormal();
    let cases: Vec<String> = vec![
        "1e4294967296".into(),
        "1e-4294967296".into(),
        "1e2147483648".into(),
        "1e-2147483649".into(),
        "0e99999999999999999999".into(),
        "1e99999999999999999999".into(),
        "9007199254740993".into(),
        "9007199254740993.000000000000000000000000000001".into(),
        "9007199254740992.999999999999999999999999999999".into(),
        half.clone(),
        format!("{half}1"),
        format!("{half}{}1", "0".repeat(2000)),
        format!("{}{}", "1".repeat(800), "e-800"),
        "4.9406564584124654e-324".into(),
        "2.2250738585072011e-308".into(),
        "2.2250738585072012e-308".into(),
        "1.7976931348623158e308".into(),
        "1.7976931348623159e308".into(),
        "179769313486231580793728971405303415079934132710037826936173778980444968292764750946649017977587207096330286416692887910946555547851940402630657488671505820681908902000708383676273854845817711531764475730270069855571366959622842914819860834936475292719074168444365510704342711559699508093042880177904174497791.9999999".into(),
    ];
    let mut p = pf_prog();
    let mut bad = Vec::new();
    for s in &cases {
        let want = s.parse::<f64>().map(f64::to_bits).ok();
        let got = pf(&mut p, s);
        let short = if s.len() > 60 { format!("{}...({} bytes)", &s[..40], s.len()) } else { s.clone() };
        match (want, &got) {
            (Some(w), Ok(g)) if w == *g => {}
            _ => bad.push(format!("{short}: want {want:x?}, got {got:x?} (fuel {})", p.rt.last_fuel_used())),
        }
        if got.is_err() {
            p = pf_prog();
        }
    }
    // Long inputs built inside the guest (the host payload is capped).
    let mut per_digit = Vec::new();
    for kind in 0..4 {
        for n in [1_000usize, 2_000, 4_000] {
            let mut q = pf_prog();
            let _ = q.rt.call_export("blen", &[Value::I32(n as i32), Value::I32(kind)]);
            let build = q.rt.last_fuel_used();
            let mut q = pf_prog();
            let r = q.rt.call_export("pfr", &[Value::I32(n as i32), Value::I32(kind)]);
            let total = q.rt.last_fuel_used();
            let parse = total.saturating_sub(build);
            eprintln!("kind {kind} n {n}: build {build}, parse {parse} fuel ({} per digit) {r:?}", parse / n as u64);
            per_digit.push((kind, n, parse / n as u64));
        }
    }
    for (kind, n) in [(0, 20_000), (1, 20_000), (2, 20_000), (3, 20_000)] {
        let s = match kind {
            0 => format!("0.{}1e{}", "0".repeat(n), n + 1),
            1 => format!("1{}e-{}", "0".repeat(n), n),
            2 => format!("0.{}", "1".repeat(n)),
            _ => format!("{}.5", "9".repeat(n)),
        };
        let want = s.parse::<f64>().unwrap().to_bits();
        let r = p.rt.call_export("pfr", &[Value::I32(n as i32), Value::I32(kind)]);
        let fuel = p.rt.last_fuel_used();
        eprintln!("long kind {kind} n {n}: {r:?} fuel {fuel}");
        match r {
            Ok(v) if v.first() == Some(&Value::I64(want as i64)) => {}
            other => bad.push(format!("long kind {kind} ({n} digits): want {want:x}, got {other:?} (fuel {fuel})")),
        }
        p = pf_prog();
    }
    assert!(bad.is_empty(), "parseFloat differs from Rust:\n{}", bad.join("\n"));
}

/// Probe: `compile` is used so the import list above stays honest.
#[test]
fn probe_compile_smoke() {
    let _ = compile("export function f(): i32 {\n  return 1;\n}\n");
}

/// Probe: fuel cost of building strings (repeat and template concatenation) vs length.
#[test]
fn probe_string_build_fuel_scaling() {
    let src = r#"
import { repeat } from "std/string";
@host export function rep(n: i32): i32 {
  return repeat("0", n).byteLength;
}
@host export function tpl(n: i32): i32 {
  const s = repeat("0", n);
  return `0.${s}1e5`.byteLength;
}
"#;
    for f in ["rep", "tpl"] {
        for n in [1_000, 2_000, 4_000, 8_000] {
            let mut p = Prog::new(src);
            let r = p.rt.call_export(f, &[Value::I32(n)]);
            eprintln!("{f}({n}): {r:?} fuel {}", p.rt.last_fuel_used());
        }
    }
}
