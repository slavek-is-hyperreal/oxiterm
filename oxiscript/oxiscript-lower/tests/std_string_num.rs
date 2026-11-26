//! Proving tests for Stage 8.Std BAD ACTOR fixes (BS01-BS08):
//! - BS01: 200,000+ differential tests of `parseFloat` vs Rust `str::parse::<f64>` (exact bits)
//! - BS02+BS03: exact decimal differential tests of `toFixed` vs Rust `format!("{:.N$}")` incl. `f64::MAX`
//! - BS04: `Map`/`Set` NaN & `-0.0` SameValueZero key semantics (f64 & f32, distinct NaN payloads)
//! - BS05: IR-level zero-allocation proof + 1,000,000 huge-arg trig calls on one instance (256 KiB mem)
//! - BS06: `TrapKind::Bounds` on invalid `toRadix*` radix and `toFixed` digits
//! - BS07: exact fuel measurements for sort and Map/Set operations
#![allow(clippy::pedantic)]

mod common;

use common::{compile, compile_full, trap_kind, Prog};
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::{Limits, Value};

struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range_usize(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next_u64() as usize) % (hi - lo + 1)
    }
    fn range_i32(&mut self, lo: i32, hi: i32) -> i32 {
        lo + (self.next_u64() % ((hi - lo + 1) as u64)) as i32
    }
}

/// Exact decimal representation of a positive dyadic rational `m * 2^e` for constructing
/// true halfway test strings (`(2*k + 1) * 2^(exp - 1)`).
fn dyadic_to_decimal(mut m: u128, e: i32) -> String {
    if m == 0 {
        return "0".into();
    }
    // Express m * 2^e = digits * 10^dec_exp
    let mut digits: Vec<u8> = Vec::new();
    if e >= 0 {
        // BigInt = m << e (up to ~1075 bits, use base 10^9 limbs)
        let mut limbs: Vec<u32> = vec![
            (m % 1_000_000_000) as u32,
            ((m / 1_000_000_000) % 1_000_000_000) as u32,
            ((m / 1_000_000_000_000_000_000) % 1_000_000_000) as u32,
        ];
        while limbs.last() == Some(&0) {
            limbs.pop();
        }
        for _ in 0..e {
            let mut carry = 0u64;
            for l in &mut limbs {
                let v = u64::from(*l) * 2 + carry;
                if v >= 1_000_000_000 {
                    *l = (v - 1_000_000_000) as u32;
                    carry = 1;
                } else {
                    *l = v as u32;
                    carry = 0;
                }
            }
            if carry > 0 {
                limbs.push(carry as u32);
            }
        }
        let mut s = String::new();
        for (idx, &l) in limbs.iter().rev().enumerate() {
            if idx == 0 {
                s.push_str(&l.to_string());
            } else {
                s.push_str(&format!("{l:09}"));
            }
        }
        s
    } else {
        // m * 5^(-e) * 10^e
        let p = (-e) as usize;
        let mut limbs: Vec<u32> = Vec::new();
        while m > 0 {
            limbs.push((m % 1_000_000_000) as u32);
            m /= 1_000_000_000;
        }
        for _ in 0..p {
            let mut carry = 0u64;
            for l in &mut limbs {
                let v = u64::from(*l) * 5 + carry;
                *l = (v % 1_000_000_000) as u32;
                carry = v / 1_000_000_000;
            }
            if carry > 0 {
                limbs.push(carry as u32);
            }
        }
        let mut s = String::new();
        for (idx, &l) in limbs.iter().rev().enumerate() {
            if idx == 0 {
                s.push_str(&l.to_string());
            } else {
                s.push_str(&format!("{l:09}"));
            }
        }
        let _ = &mut digits;
        format!("{s}e{e}")
    }
}

#[test]
#[cfg_attr(debug_assertions, ignore = "heavy (200k differential): runs in release and in the GitHub nightly, see scripts/oxiscript-gate.sh")]
fn bs01_parse_float_differential_200k() {
    let src = r#"
import { parseFloat } from "std/string";
import { bitsOf } from "std/math";

@host export function pf_bits(s: string): u64 {
  const r = parseFloat(s);
  switch (r.kind) {
    case "ok":
      return bitsOf(r.value);
    case "err":
      return 0xffff_ffff_ffff_ffffu64;
  }
}
"#;
    let wasm = compile(src);
    let make_prog = || {
        Prog::from_wasm_limits(
            wasm.clone(),
            Limits {
                fuel_per_call: 50_000_000,
                ..Limits::default()
            },
        )
    };
    let mut p = make_prog();
    let mut tested: usize = 0;

    let check_one = |p: &mut Prog, s: &str, count: &mut usize| {
        if *count > 0 && (*count).is_multiple_of(8_000) {
            p.finish();
            *p = make_prog();
        }
        let got_bits = match p.call("pf_bits", &[Value::Str(s.to_string())])[0] {
            Value::I64(v) => v as u64,
            ref other => panic!("unexpected return: {other:?}"),
        };
        let want = s.trim().parse::<f64>();
        match want {
            Ok(w) => {
                let want_bits = if w.is_nan() {
                    0x7ff8_0000_0000_0000u64
                } else {
                    w.to_bits()
                };
                assert_eq!(
                    got_bits, want_bits,
                    "parseFloat({s:?}) mismatch: got {got_bits:#018x} ({}) want {want_bits:#018x} ({w:e})",
                    f64::from_bits(got_bits)
                );
            }
            Err(_) => {
                assert_eq!(
                    got_bits, 0xffff_ffff_ffff_ffffu64,
                    "parseFloat({s:?}) should Err, got {got_bits:#018x}"
                );
            }
        }
        *count += 1;
    };

    // 1. Hand-crafted edge & adversarial cases
    let edges = [
        "0", "-0", "+0", "0.0", "-0.0", "0e999999999", "-0e-999999999",
        "1.7976931348623157e308", "1.7976931348623158e308", "1.7976931348623159e308",
        "-1.7976931348623157e308", "-1.7976931348623159e308",
        "2.2250738585072014e-308", "2.2250738585072011e-308", "2.2250738585072009e-308",
        "5e-324", "4.9e-324", "2.4703282292062327e-324", "2.4703282292062328e-324",
        "-2.4703282292062327e-324", "-2.4703282292062328e-324",
        "0.000000000000000000000000000001", "1e-325", "1e-400", "1e400", "-1e400",
        "9007199254740991", "9007199254740992", "9007199254740993", "9007199254740992.5",
        "9007199254740993.5", "9007199254740994.5", "9007199254740995.5",
        "18446744073709551615", "18446744073709551616", "18446744073709551617",
        "123456789012345678901234567890",
        "0.1", "0.2", "0.3", "0.5", "0.045", "1e23", "1e22", "1e-22", "1e-23",
        "1.", ".5", "-.5e1", "+1.5", "1e+5", "1E-5", "  -12.345e2 ",
        "NaN", "nan", "inf", "-inf", "+inf", "Infinity", "-Infinity",
        "", " ", "+", "-", ".", ".e1", "1e", "1e+", "1e-", "abc", "1.2.3", "1 2", "1e2x",
    ];
    for s in edges {
        check_one(&mut p, s, &mut tested);
    }

    // 2. Exact halfway & near-halfway dyadic midpoints across normal & subnormal ranges
    let mut rng = Rng(0x5eed_cafe_beef_0001);
    let midpoint_groups: usize = if !cfg!(debug_assertions) { 400 } else { 60 };
    for i in 0..midpoint_groups {
        let (m, e) = if i < midpoint_groups / 3 {
            // Subnormal midpoints: (2*k + 1) * 2^-1075
            let k = if i < 15 {
                i as u128
            } else {
                u128::from(rng.next_u64() & 0x000f_ffff_ffff_ffff)
            };
            (2 * k + 1, -1075)
        } else {
            // Normal midpoints: (2*(2^52 + m) + 1) * 2^(e - 1)
            let mant = (1u128 << 52) | u128::from(rng.next_u64() & 0x000f_ffff_ffff_ffff);
            let exp = rng.range_i32(-1074, 971);
            (2 * mant + 1, exp - 1)
        };
        let exact = dyadic_to_decimal(m, e);
        check_one(&mut p, &exact, &mut tested);
        // Slightly above midpoint (append "0000000000000001" before 'e' or after decimal)
        let above = if let Some(pos) = exact.find('e') {
            format!("{}.0000000000000001{}", &exact[..pos], &exact[pos..])
        } else {
            format!("{exact}.0000000000000001")
        };
        check_one(&mut p, &above, &mut tested);
        // Negative version
        let neg_exact = format!("-{exact}");
        check_one(&mut p, &neg_exact, &mut tested);
    }

    // 3. 200,000 random decimal strings covering all paths (Clinger, Eisel-Lemire, BigUint)
    let big_stride: usize = if !cfg!(debug_assertions) { 20 } else { 400 };
    let mut buf = String::with_capacity(64);
    for i in 0..200_000usize {
        buf.clear();
        if i.is_multiple_of(4) {
            // Exact round-trip of random IEEE-754 f64 bit patterns
            let bits = rng.next_u64();
            let x = f64::from_bits(bits);
            if x.is_finite() {
                if i.is_multiple_of(8) {
                    buf.push_str(&format!("{x:e}"));
                } else {
                    buf.push_str(&format!("{x}"));
                }
            } else {
                buf.push_str("1.2345678901234567e-100");
            }
        } else {
            if rng.next_u64() & 1 != 0 {
                buf.push('-');
            }
            let ndig = if i % big_stride == 1 {
                rng.range_usize(20, 40) // forces BigUint fallback
            } else if i.is_multiple_of(10) {
                rng.range_usize(1, 7)
            } else {
                rng.range_usize(8, 19) // Eisel-Lemire fast path
            };
            let dot_pos = rng.range_usize(0, ndig);
            for d in 0..ndig {
                if d == dot_pos && dot_pos > 0 && dot_pos < ndig {
                    buf.push('.');
                }
                let ch = b'0' + (rng.next_u64() % 10) as u8;
                buf.push(ch as char);
            }
            if !rng.next_u64().is_multiple_of(5) {
                let exp = match i % 7 {
                    0 => rng.range_i32(-330, -300), // subnormal region
                    1 => rng.range_i32(295, 315),   // max/overflow region
                    2 => rng.range_i32(-25, 25),    // Clinger boundary
                    _ => rng.range_i32(-300, 300),
                };
                buf.push(if rng.next_u64() & 1 == 0 { 'e' } else { 'E' });
                buf.push_str(&exp.to_string());
            }
        }
        check_one(&mut p, &buf, &mut tested);
    }

    assert!(tested >= 200_000, "expected >= 200k inputs, ran {tested}");
    eprintln!("bs01_parse_float_differential_200k: verified {tested} inputs bit-for-bit against Rust str::parse::<f64>");
    p.finish();
}

#[test]
fn bs02_bs03_to_fixed_differential_exact() {
    let src = r#"
import { Log } from "host:log";
import { toFixed } from "std/string";

@host export function run_one(x: f64, d: i32): i32 {
  const s = toFixed(x, d);
  return s.byteLength;
}
"#;
    let _ = src;

    // Batch check via host:log so we compare full string outputs directly.
    let mut xs: Vec<f64> = vec![
        0.0, -0.0, 0.5, 1.5, 2.5, 3.5, -0.5, -1.5, -2.5,
        0.1, 0.2, 0.3, 0.7, 0.045, 0.055, 0.125, 0.375, 1.005, 2.675,
        9.995, 99.5, 0.9999999, 1.0000000000000002, -0.4, -0.004,
        123_456_789.987_654_33, 4503599627370497.5, 1e15 + 0.3,
        1e-7, 1e-18, 1e-20, 1e-21, 5e-324, -5e-324, f64::MIN_POSITIVE,
        1e18, 1e19, 1e20, 1e21, 1e22, 1e50, 1e100, 1e300,
        f64::MAX, -f64::MAX, f64::INFINITY, f64::NEG_INFINITY, f64::NAN,
    ];
    let mut rng = Rng(0x1234_abcd_5678_ef01);
    for _ in 0..300 {
        let bits = rng.next_u64();
        let x = f64::from_bits(bits);
        if x.is_finite() {
            xs.push(x);
        }
    }

    // Verify in chunks of 400 calls per module so compile/run stays fast.
    let mut cases: Vec<(f64, usize)> = Vec::new();
    for (idx, &x) in xs.iter().enumerate() {
        let ds: &[usize] = if idx < 48 {
            &[0, 1, 2, 3, 6, 10, 15, 20]
        } else {
            &[0, 2, 7, 20]
        };
        for &d in ds {
            cases.push((x, d));
        }
    }

    let mut total = 0usize;
    for chunk in cases.chunks(300) {
        let mut prog_src = String::from(
            "import { Log } from \"host:log\";\nimport { toFixed } from \"std/string\";\nimport { fromBits } from \"std/math\";\nexport function main(log: Log): void {\n",
        );
        let mut wants: Vec<String> = Vec::with_capacity(chunk.len());
        for &(x, d) in chunk {
            prog_src.push_str(&format!(
                "  log.write(2, toFixed(fromBits({}u64), {d}));\n",
                x.to_bits()
            ));
            let mut w = if x.is_nan() {
                "NaN".to_string()
            } else if x == f64::INFINITY {
                "Infinity".to_string()
            } else if x == f64::NEG_INFINITY {
                "-Infinity".to_string()
            } else {
                format!("{x:.d$}")
            };
            if w.starts_with("-0") && w[1..].chars().all(|c| c == '0' || c == '.') {
                w.remove(0);
            }
            wants.push(w);
        }
        prog_src.push_str("}\n");
        let wasm = compile(&prog_src);
        let mut p = Prog::from_wasm_limits(
            wasm,
            Limits {
                fuel_per_call: 200_000_000,
                ..Limits::default()
            },
        );
        let got = p.log();
        assert_eq!(got.len(), wants.len());
        for ((&(x, d), w), g) in chunk.iter().zip(wants.iter()).zip(got.iter()) {
            assert_eq!(g, w, "toFixed({x:e} [{:#018x}], {d}) mismatch", x.to_bits());
        }
        p.finish();
        total += chunk.len();
    }
    eprintln!("bs02_bs03_to_fixed_differential_exact: verified {total} cases exactly against Rust format!");
}

#[test]
fn bs04_map_set_nan_same_value_zero_f64_and_f32() {
    let src = r#"
import { Log } from "host:log";
import { Map, Set } from "std/collections";
import { fromBits, fromBitsF32 } from "std/math";
import { includes, indexOf } from "std/array";

@host export function test_nan(): i32 {
  const n1 = fromBits(0x7ff8_0000_0000_0000u64);
  const n2 = fromBits(0x7ff8_0000_0000_0001u64);
  const n3 = fromBits(0xfff8_0000_0000_1234u64);

  const m = new Map<f64, i32>();
  m.set(n1, 10);
  m.set(n2, 20);
  m.set(n3, 30);
  m.set(-0.0, 40);
  m.set(0.0, 50);
  if (m.size() !== 2) { return -1; }
  if ((m.get(n1) ?? -1) !== 30) { return -2; }
  if ((m.get(n2) ?? -1) !== 30) { return -3; }
  if ((m.get(0.0) ?? -1) !== 50) { return -4; }
  if ((m.get(-0.0) ?? -1) !== 50) { return -5; }
  if (!m.remove(n2)) { return -6; }
  if (m.has(n1) || m.size() !== 1) { return -7; }

  const s = new Set<f64>();
  if (!s.add(n1)) { return -8; }
  if (s.add(n3)) { return -9; }
  if (!s.has(n2) || s.size() !== 1) { return -10; }
  if (!s.remove(n3) || !s.isEmpty()) { return -11; }

  // f32 SameValueZero
  const fn1 = fromBitsF32(0x7fc0_0000u32);
  const fn2 = fromBitsF32(0xffc0_0123u32);
  const fm = new Map<f32, i32>();
  fm.set(fn1, 7);
  fm.set(fn2, 8);
  fm.set(-0.0f32, 9);
  fm.set(0.0f32, 11);
  if (fm.size() !== 2) { return -12; }
  if ((fm.get(fn1) ?? -1) !== 8) { return -13; }
  if ((fm.get(-0.0f32) ?? -1) !== 11) { return -14; }
  if (!fm.remove(fn2) || fm.size() !== 1) { return -15; }

  // Direct .equals() and .hash() consistency + array includes/indexOf
  if (!n1.equals(n3) || n1.hash() !== n3.hash()) { return -16; }
  if (!fn1.equals(fn2) || fn1.hash() !== fn2.hash()) { return -17; }
  if (!includes([1.0, n1, 2.0], n3) || indexOf([1.0, n1, 2.0], n2) !== 1) { return -18; }
  if (n1.equals(1.0) || fn1.equals(1.0f32)) { return -19; }
  return 1;
}
export function main(log: Log): void { log.write(2, "ok"); }
"#;
    let mut p = Prog::new(src);
    assert_eq!(p.call("test_nan", &[]), vec![Value::I32(1)]);
    p.finish();
}

#[test]
fn bs05_math_transcendentals_zero_alloc_ir_and_1m_calls() {
    let src = r#"
import {
  sin, cos, tan, asin, acos, atan, atan2,
  exp, log, log2, log10, pow, cbrt, hypot, scalbn, fromBits
} from "std/math";

@host export function f_sin(x: f64): f64 { return sin(x); }
@host export function f_cos(x: f64): f64 { return cos(x); }
@host export function f_tan(x: f64): f64 { return tan(x); }
@host export function f_asin(x: f64): f64 { return asin(x); }
@host export function f_acos(x: f64): f64 { return acos(x); }
@host export function f_atan(x: f64): f64 { return atan(x); }
@host export function f_atan2(y: f64, x: f64): f64 { return atan2(y, x); }
@host export function f_exp(x: f64): f64 { return exp(x); }
@host export function f_log(x: f64): f64 { return log(x); }
@host export function f_log2(x: f64): f64 { return log2(x); }
@host export function f_log10(x: f64): f64 { return log10(x); }
@host export function f_pow(x: f64, y: f64): f64 { return pow(x, y); }
@host export function f_cbrt(x: f64): f64 { return cbrt(x); }
@host export function f_hypot(x: f64, y: f64): f64 { return hypot(x, y); }
@host export function f_scalbn(x: f64, n: i32): f64 { return scalbn(x, n); }

@host export function run_huge_trig(n: i32, seedIn: u64): f64 {
  let s: u64 = seedIn;
  let acc: f64 = 0.0;
  let i: i32 = 0;
  while (i < n) {
    s = s ^ (s << 13u64);
    s = s ^ (s >> 7u64);
    s = s ^ (s << 17u64);
    // Exponents in [1045, 2040] => |x| >= 2^22 > 823549.6 (always hits Payne-Hanek remPio2Large).
    const e: u64 = 1045u64 + (s % 995u64);
    let x = fromBits((e << 52u64) | (s >> 12u64));
    if ((i & 1) === 0) {
      x = -x;
    }
    const m = i % 3;
    if (m === 0) {
      acc = acc + sin(x);
    } else if (m === 1) {
      acc = acc + cos(x);
    } else {
      acc = acc + tan(x) * 1e-6;
    }
    i = i +% 1;
  }
  return acc;
}
"#;
    // 1. Static IR-level proof: zero allocating instructions in the entire compiled module!
    let out = compile_full(src);
    let mut alloc_sites = Vec::new();
    for func in &out.module.funcs {
        for blk in &func.blocks {
            for inst in &blk.insts {
                if inst.op.allocates() {
                    alloc_sites.push(format!("{}: {:?}", func.name, inst.op));
                }
            }
        }
    }
    assert!(
        alloc_sites.is_empty(),
        "transcendental math module contains allocating IR ops:\n{}",
        alloc_sites.join("\n")
    );

    // 2. Runtime proof: 1,000,000 huge-argument Payne-Hanek trig calls in release (50,000 in debug,
    // >4x the 11,796 Stub OOM crash threshold) on ONE Stub instance with a 4-page (256 KiB) ceiling.
    let mut p = Prog::from_wasm_limits(
        out.wasm,
        Limits {
            fuel_per_call: 4_000_000_000,
            max_memory_pages: 4,
            ..Limits::default()
        },
    );
    let (batches, per_batch): (u64, i32) = if !cfg!(debug_assertions) {
        (10, 100_000)
    } else {
        (1, 50_000)
    };
    for batch in 0..batches {
        let seed = 0x1234_5678_9abc_def1u64.wrapping_add(batch.wrapping_mul(0x9e37_79b9_7f4a_7c15));
        let res = p.call("run_huge_trig", &[Value::I32(per_batch), Value::I64(seed as i64)]);
        match res[0] {
            Value::F64(v) => assert!(v.is_finite(), "batch {batch}: expected finite sum, got {v}"),
            ref other => panic!("unexpected {other:?}"),
        }
    }
    p.finish();
}

#[test]
fn bs06_invalid_radix_and_digits_trap_bounds() {
    let src = r#"
import { toRadix, toRadixU64, toRadixI32, toFixed } from "std/string";
import { NAN, INF } from "std/math";

@host export function r64(n: i64, r: i32): i32 { return toRadix(n, r).byteLength; }
@host export function ru64(n: u64, r: i32): i32 { return toRadixU64(n, r).byteLength; }
@host export function r32(n: i32, r: i32): i32 { return toRadixI32(n, r).byteLength; }
@host export function fx(x: f64, d: i32): i32 { return toFixed(x, d).byteLength; }
@host export function fx_nan(d: i32): i32 { return toFixed(NAN, d).byteLength; }
@host export function fx_inf(d: i32): i32 { return toFixed(INF, d).byteLength; }
"#;
    let wasm = compile(src);
    for bad_r in [-5, 0, 1, 37, 100] {
        let mut p = Prog::from_wasm(wasm.clone());
        let e1 = p.call_err("r64", &[Value::I64(10), Value::I32(bad_r)]);
        assert_eq!(trap_kind(&e1), Some(TrapKind::Bounds), "toRadix(10, {bad_r}): {e1}");

        let mut p2 = Prog::from_wasm(wasm.clone());
        let e2 = p2.call_err("ru64", &[Value::I64(10), Value::I32(bad_r)]);
        assert_eq!(trap_kind(&e2), Some(TrapKind::Bounds), "toRadixU64(10, {bad_r}): {e2}");

        let mut p3 = Prog::from_wasm(wasm.clone());
        let e3 = p3.call_err("r32", &[Value::I32(10), Value::I32(bad_r)]);
        assert_eq!(trap_kind(&e3), Some(TrapKind::Bounds), "toRadixI32(10, {bad_r}): {e3}");
    }
    for bad_d in [-100, -1, 21, 100] {
        let mut p = Prog::from_wasm(wasm.clone());
        let e = p.call_err("fx", &[Value::F64(1.5), Value::I32(bad_d)]);
        assert_eq!(trap_kind(&e), Some(TrapKind::Bounds), "toFixed(1.5, {bad_d}): {e}");

        let mut p2 = Prog::from_wasm(wasm.clone());
        let e2 = p2.call_err("fx_nan", &[Value::I32(bad_d)]);
        assert_eq!(trap_kind(&e2), Some(TrapKind::Bounds), "toFixed(NaN, {bad_d}): {e2}");

        let mut p3 = Prog::from_wasm(wasm.clone());
        let e3 = p3.call_err("fx_inf", &[Value::I32(bad_d)]);
        assert_eq!(trap_kind(&e3), Some(TrapKind::Bounds), "toFixed(INF, {bad_d}): {e3}");
    }
}

#[test]
fn bs07_fuel_measurements_sort_and_collections() {
    let src = r#"
import { sortI32, sortBy, sortF64, sortString, isSortedBy } from "std/array";
import { Map, Set } from "std/collections";

@host export function bench_big30k(n: i32): i32 {
  const xs: i32[] = [];
  for (const i of range(0, n)) { xs.push(((i *% 1103515245) +% 12345) % 100003); }
  const ys = sortI32(xs);
  return isSortedBy(ys, (a: i32, b: i32): i32 => a - b) ? ys.length : -1;
}
@host export function bench_sort_i32_only(n: i32): i32 {
  const xs: i32[] = [];
  let i: i32 = 0;
  while (i < n) {
    xs.push(((i *% 1103515245) +% 12345));
    i = i +% 1;
  }
  const ys = sortI32(xs);
  return ys.length;
}
@host export function bench_sort_by(n: i32): i32 {
  const xs: i32[] = [];
  let i: i32 = 0;
  while (i < n) {
    xs.push(((i *% 1103515245) +% 12345) % 100003);
    i = i +% 1;
  }
  const ys = sortBy(xs, (a: i32, b: i32): i32 => a - b);
  return ys.length;
}
@host export function bench_harness_only(n: i32): i32 {
  const xs: i32[] = [];
  for (const i of range(0, n)) { xs.push(((i *% 1103515245) +% 12345) % 100003); }
  return isSortedBy(xs, (a: i32, b: i32): i32 => 0) ? xs.length : -1;
}
@host export function bench_sort_f64(n: i32): i32 {
  const xs: f64[] = [];
  let i: i32 = 0;
  while (i < n) {
    xs.push((((i *% 1103515245) +% 12345) % 100003) as f64);
    i = i +% 1;
  }
  const ys = sortF64(xs);
  return ys.length;
}
@host export function bench_map_ops(n: i32): i32 {
  const m = new Map<i32, i32>();
  let i: i32 = 0;
  while (i < n) {
    m.set(i, i *% 3);
    i = i +% 1;
  }
  let acc: i32 = 0;
  i = 0;
  while (i < n) {
    acc = acc +% (m.get(i) ?? 0);
    i = i +% 1;
  }
  return acc + m.size() * 0;
}
@host export function bench_set_ops(n: i32): i32 {
  const s = new Set<i32>();
  let i: i32 = 0;
  while (i < n) {
    s.add(i *% 3);
    i = i +% 1;
  }
  let c: i32 = 0;
  i = 0;
  while (i < n) {
    if (s.has(i *% 3)) { c = c +% 1; }
    i = i +% 1;
  }
  return c;
}
"#;
    let wasm = compile(src);
    let mut p = Prog::from_wasm_limits(
        wasm,
        Limits {
            fuel_per_call: 200_000_000,
            ..Limits::default()
        },
    );
    let r = p.call("bench_big30k", &[Value::I32(30_000)]);
    assert_eq!(r, vec![Value::I32(30_000)]);
    eprintln!("FUEL bench_big30k(30_000) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_harness_only", &[Value::I32(30_000)]);
    eprintln!("FUEL bench_harness_only(30_000) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_sort_i32_only", &[Value::I32(10_000)]);
    eprintln!("FUEL bench_sort_i32_only(10_000, full 32-bit) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_sort_i32_only", &[Value::I32(30_000)]);
    eprintln!("FUEL bench_sort_i32_only(30_000, full 32-bit) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_sort_f64", &[Value::I32(10_000)]);
    eprintln!("FUEL bench_sort_f64(10_000) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_sort_by", &[Value::I32(10_000)]);
    eprintln!("FUEL bench_sort_by(10_000) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_map_ops", &[Value::I32(10_000)]);
    eprintln!("FUEL bench_map_ops(10_000 set+get) = {}", p.rt.last_fuel_used());

    let _ = p.call("bench_set_ops", &[Value::I32(10_000)]);
    eprintln!("FUEL bench_set_ops(10_000 add+has) = {}", p.rt.last_fuel_used());

    p.finish();
}
