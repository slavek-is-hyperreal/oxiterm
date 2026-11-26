#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.Std: adversarial tests for std/math, std/string, std/array, std/collections.
mod common;
use common::*;
use oxiscript_runtime::Value;

fn f(p: &mut Prog, name: &str, a: &[f64]) -> f64 {
    let args: Vec<Value> = a.iter().map(|x| Value::F64(*x)).collect();
    match p.call(name, &args)[0] { Value::F64(x) => x, ref v => panic!("{v:?}") }
}
fn ulp(a: f64, b: f64) -> u64 {
    if a.is_nan() && b.is_nan() { return 0; }
    if a.is_nan() != b.is_nan() { return u64::MAX; }
    if a == b { return 0; }
    if a.is_infinite() || b.is_infinite() { return u64::MAX; }
    let k = |x: f64| { let i = x.to_bits() as i64; if i < 0 { i64::MIN - i } else { i } };
    (k(a) as i128 - k(b) as i128).unsigned_abs() as u64
}

const MATH: &str = r#"import { Log } from "host:log";
import { sin, cos, tan, asin, acos, atan, atan2, exp, log, log2, log10, pow, cbrt, hypot } from "std/math";
@host export function fsin(x: f64): f64 { return sin(x); }
@host export function fcos(x: f64): f64 { return cos(x); }
@host export function ftan(x: f64): f64 { return tan(x); }
@host export function fexp(x: f64): f64 { return exp(x); }
@host export function flog(x: f64): f64 { return log(x); }
@host export function flog2(x: f64): f64 { return log2(x); }
@host export function flog10(x: f64): f64 { return log10(x); }
@host export function fcbrt(x: f64): f64 { return cbrt(x); }
@host export function fatan(x: f64): f64 { return atan(x); }
@host export function fasin(x: f64): f64 { return asin(x); }
@host export function facos(x: f64): f64 { return acos(x); }
@host export function fpow(x: f64, y: f64): f64 { return pow(x, y); }
@host export function fatan2(x: f64, y: f64): f64 { return atan2(x, y); }
@host export function fhypot(x: f64, y: f64): f64 { return hypot(x, y); }
export function main(log: Log): void { log.write(2, "up"); }
"#;

/// BS01: special values must follow IEEE/C99 (what Rust std gives), bit for bit incl. sign of zero.
#[test]
fn math_special_values_match_reference() {
    let mut p = Prog::new(MATH);
    let sub = f64::from_bits(1);
    let xs = [0.0, -0.0, 1.0, -1.0, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, sub, -sub, f64::MAX, f64::MIN_POSITIVE,
        1e-310, 710.0, -746.0, -745.2, 709.8, 1e22, 1e300, 1.7976931348623157e308, 6.02e23, std::f64::consts::PI, 0.5, -0.5, 2.0, 1e-300];
    let mut bad = vec![];
    type F1 = fn(f64) -> f64;
    let unary: [(&str, F1); 11] = [("fsin", f64::sin), ("fcos", f64::cos), ("ftan", f64::tan), ("fexp", f64::exp), ("flog", f64::ln),
        ("flog2", f64::log2), ("flog10", f64::log10), ("fcbrt", f64::cbrt), ("fatan", f64::atan), ("fasin", f64::asin), ("facos", f64::acos)];
    for (n, r) in unary {
        for &x in &xs {
            let g = f(&mut p, n, &[x]);
            let w = r(x);
            let zero_sign = g == 0.0 && w == 0.0 && g.is_sign_negative() != w.is_sign_negative();
            if ulp(g, w) > 2 || zero_sign { bad.push(format!("{n}({x:e}) = {g:e} want {w:e}")); }
        }
    }
    let binary: [(&str, fn(f64, f64) -> f64); 3] = [("fpow", f64::powf), ("fatan2", f64::atan2), ("fhypot", f64::hypot)];
    for (n, r) in binary {
        for &x in &xs { for &y in &xs {
            let g = f(&mut p, n, &[x, y]);
            let w = r(x, y);
            let zero_sign = g == 0.0 && w == 0.0 && g.is_sign_negative() != w.is_sign_negative();
            if ulp(g, w) > 2 || zero_sign { bad.push(format!("{n}({x:e},{y:e}) = {g:e} want {w:e}")); }
        }}
    }
    assert!(bad.is_empty(), "{} mismatches, first 25:\n{}", bad.len(), bad.iter().take(25).cloned().collect::<Vec<_>>().join("\n"));
}

/// BS02: huge trig arguments (Payne-Hanek) and subnormal/near-zero cases, dense sweep.
#[test]
fn math_trig_huge_arguments() {
    let mut p = Prog::new(MATH);
    let mut bad = vec![];
    let mut s: u64 = 0x1234_5678_9abc_def1;
    for i in 0..30000 {
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let e = 1 + (s % 2040) as u64; // exponent range, incl. near-max and tiny
        let x = f64::from_bits((e << 52) | (s >> 12));
        let x = if i % 2 == 0 { -x } else { x };
        for (n, r) in [("fsin", f64::sin as fn(f64) -> f64), ("fcos", f64::cos), ("ftan", f64::tan)] {
            let g = f(&mut p, n, &[x]); let w = r(x);
            if ulp(g, w) > 2 { bad.push(format!("{n}({x:e}) = {g:e} want {w:e}")); }
        }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

/// BS03: exp/log/pow tails (overflow, underflow into subnormals) over a full exponent sweep.
#[test]
fn math_exp_log_pow_tails() {
    let mut p = Prog::new(MATH);
    let mut bad = vec![];
    let mut s: u64 = 99;
    for _ in 0..20000 {
        s ^= s << 13; s ^= s >> 7; s ^= s << 17;
        let x = f64::from_bits(s);
        if x.is_nan() { continue; }
        for (n, r) in [("flog", f64::ln as fn(f64) -> f64), ("flog2", f64::log2), ("flog10", f64::log10), ("fcbrt", f64::cbrt)] {
            let g = f(&mut p, n, &[x]); let w = r(x);
            if ulp(g, w) > 2 { bad.push(format!("{n}({x:e}) = {g:e} want {w:e}")); }
        }
        let y = (x % 800.0) - 400.0;
        let g = f(&mut p, "fexp", &[y]); let w = y.exp();
        if ulp(g, w) > 2 { bad.push(format!("exp({y:e}) = {g:e} want {w:e}")); }
        let b = (s >> 3) as f64 / 1e15;
        let e = ((s >> 7) % 400) as f64 / 7.0 - 28.0;
        let g = f(&mut p, "fpow", &[b, e]); let w = b.powf(e);
        if ulp(g, w) > 2 { bad.push(format!("pow({b:e},{e:e}) = {g:e} want {w:e}")); }
    }
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
}

/// BS04: cross-run determinism (two fresh instances, same bits).
#[test]
fn math_cross_run_determinism() {
    let mut a = Prog::new(MATH); let mut b = Prog::new(MATH);
    for i in 0..3000 {
        let x = (i as f64) * 0.37 - 400.0;
        for n in ["fsin", "fcos", "fexp", "flog", "fatan"] {
            assert_eq!(f(&mut a, n, &[x]).to_bits(), f(&mut b, n, &[x]).to_bits(), "{n}({x})");
        }
    }
}

/// Compile a program whose main logs one line per expression; return the log.
fn logs(imports: &str, exprs: &[String]) -> Vec<String> {
    let mut src = format!("import {{ Log }} from \"host:log\";\n{imports}\nexport function main(log: Log): void {{\n");
    for e in exprs { src.push_str(&format!("  log.write(2, {e});\n")); }
    src.push_str("}\n");
    Prog::new(&src).log()
}
fn q(s: &str) -> String { format!("\"{s}\"") }
const SIMP: &str = "import { parseInt, parseInt32, parseFloat, toFixed, toRadix, split, replace, replaceAll, padStart, trim, repeat } from \"std/string\";\nimport { bitsOf } from \"std/math\";\nfunction pi(s: string): string { const r = parseInt(s); switch (r.kind) { case \"ok\": return r.value.toString(); case \"err\": return \"E\"; } }\nfunction pi32(s: string): string { const r = parseInt32(s); switch (r.kind) { case \"ok\": return r.value.toString(); case \"err\": return \"E\"; } }\nfunction pf(s: string): string { const r = parseFloat(s); switch (r.kind) { case \"ok\": return bitsOf(r.value).toString(); case \"err\": return \"E\"; } }\nfunction sp(s: string, sep: string): string { const p = split(s, sep); let o = \"\"; for (const x of p) { o = o + \"[\" + x + \"]\"; } return o; }\n";

/// BS05: integer parsing edge cases.
#[test]
fn string_parse_int_edges() {
    let cases: &[(&str, &str)] = &[("+5", "5"), ("-0", "0"), ("+", "E"), ("-", "E"), ("", "E"), ("٣", "E"), ("1_000", "E"), ("0x10", "E"),
        ("9223372036854775807", "9223372036854775807"), ("9223372036854775808", "E"), ("-9223372036854775808", "-9223372036854775808"), ("-9223372036854775809", "E"),
        ("00000000000000000000000000000000000000012", "12"), ("1 2", "E"), ("\u{a0}1", "E")];
    let c32: &[(&str, &str)] = &[("2147483647", "2147483647"), ("2147483648", "E"), ("-2147483649", "E"), ("+0", "0")];
    let mut ex: Vec<String> = cases.iter().map(|(i, _)| format!("pi({})", q(i))).collect();
    ex.extend(c32.iter().map(|(i, _)| format!("pi32({})", q(i))));
    let got = logs(SIMP, &ex);
    let mut bad = vec![];
    for (k, (i, w)) in cases.iter().chain(c32.iter()).enumerate() {
        if got[k] != *w { bad.push(format!("parse({i:?}) = {} want {w}", got[k])); }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// BS06: parseFloat versus Rust str::parse (correct rounding, overflow, subnormals, hard halfway cases).
#[test]
fn string_parse_float_correct_rounding() {
    let cases = ["1e400", "-1e400", "1e-400", "4.9e-324", "2.4703282292062327e-324", "2.4703282292062328e-324", "0.1", "0.3", "123456789012345678901234567890",
        "9007199254740993", "9007199254740992.5", "1.7976931348623157e308", "1.7976931348623159e308", "5e-324", "2.2250738585072011e-308", "0.000000000000000000000000000001",
        "1e23", "8.5e-5", "3.14159265358979323846264338327950288", "1.", ".5", "-.5e1", "+1.5", "1e+5", "1E5", "-0", "-0.0",
        "0.1000000000000000055511151231257827021181583404541015625", "7.4109846876186981626485318930233205854758970392148714663837852375101326090531312779794975454245398856969484704316857659638998506553390969459816219401617281718945106978546710679176872575177347315553307795408549809608457500958111373034747658096871009590975442271004757307809711118935784838675653998783503015228055934046593739791790738723868299395818481660169122019456499931289798411362062484498678713572180352209017023903285791732520220528974020802906854021606612375549983402671300035812486479041385743401875520901590172592547146296175134159774938718574737870961645638908718119841271673056017045493004705269590165763776884908267986972573366521765681389051285573e-324"];
    let mut ex: Vec<String> = cases.iter().map(|c| format!("pf({})", q(c))).collect();
    let info = [" 1", "1 ", "1e", "e5", "--1", "0x1p3", "1,5", "١", "inf", "Infinity", "NaN", "nan"];
    ex.extend(info.iter().map(|c| format!("pf({})", q(c))));
    let got = logs(SIMP, &ex);
    let mut bad = vec![];
    for (k, c) in cases.iter().enumerate() {
        match c.parse::<f64>() {
            Ok(w) => { let ws = w.to_bits().to_string(); if got[k] != ws { bad.push(format!("parseFloat({c:?}) bits {} want {ws} ({w:e})", got[k])); } }
            Err(_) => if got[k] != "E" { bad.push(format!("parseFloat({c:?}) = {} want Err", got[k])); }
        }
    }
    for (k, c) in info.iter().enumerate() { eprintln!("info parseFloat({c:?}) = {}", got[cases.len() + k]); }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// BS07: toFixed must equal exact-decimal rounding (Rust format!("{:.N}")) incl. ties, huge, tiny, negative zero.
#[test]
fn string_to_fixed_matches_exact_decimal() {
    let xs = [0.5, 1.5, 2.5, 0.125, 0.375, 1.005, 2.675, 1e21, 1e22, 123456789.987654321, 1e-7, 5e-324, -0.0, -0.4, -0.004,
        0.045, 9.995, 99.5, 0.9999999, 1.0000000000000002, 4503599627370497.5, 1e15 + 0.3, 0.1, 0.7];
    let mut ex = vec![]; let mut want = vec![];
    for &x in &xs { for d in [0usize, 1, 2, 3, 10, 20] {
        // JS-style: ties away from zero and no "-0": only exact-decimal digits are compared, so skip exact ties
        if x > 1e14 || [0.5, 1.5, 2.5, 0.125, 0.375, 99.5, 4503599627370497.5, 1e15 + 0.3].contains(&x) && d <= 3 && (d == 0 || x == 0.125 || x == 0.375 || x > 1e14) { continue; }
        let mut w = format!("{:.*}", d, x);
        if w.starts_with("-0") && w[1..].chars().all(|c| c == '0' || c == '.') { w.remove(0); }
        ex.push(format!("toFixed({:?}, {d})", x)); want.push((x, d, w));
    }}
    let got = logs(SIMP, &ex);
    let bad: Vec<String> = want.iter().zip(got.iter()).filter(|(w, g)| w.2 != **g).map(|(w, g)| format!("toFixed({:e}, {}) = {g:?} want {:?}", w.0, w.1, w.2)).collect();
    assert!(bad.is_empty(), "{} mismatches:\n{}", bad.len(), bad.join("\n"));
}

/// BS08: toRadix bounds; invalid radix must not silently produce garbage.
#[test]
fn string_to_radix_bounds() {
    let ex: Vec<String> = ["-9223372036854775807i64 - 1i64, 2", "-9223372036854775807i64 - 1i64, 36", "0i64, 10", "35i64, 36"].iter().map(|a| format!("toRadix({a})")).collect();
    let got = logs(SIMP, &ex);
    let want = ["-1000000000000000000000000000000000000000000000000000000000000000", "-1y2p0ij32e8e8", "0", "z"];
    for (g, w) in got.iter().zip(want) { assert_eq!(g, w); }
}

/// BS09: split/replace UTF-8 and empty-pattern semantics; pad with multibyte.
#[test]
fn string_utf8_and_degenerate() {
    let cases: Vec<(String, &str)> = vec![
        (format!("sp({}, {})", q("a€b€"), q("€")), "[a][b][]"),
        (format!("sp({}, {})", q(""), q(",")), "[]"),
        (format!("sp({}, {})", q("😀😀"), q("")), "[😀][😀]"),
        (format!("sp({}, {})", q("aaa"), q("aa")), "[][a]"),
        (format!("replaceAll({}, {}, {})", q("aaa"), q("aa"), q("b")), "ba"),
        (format!("trim({})", q("\u{a0}x\u{a0}")), "\u{a0}x\u{a0}"),
        (format!("padStart({}, 3, {})", q("é"), q("€")), "€€é"),
        (format!("padStart({}, 10, {})", q("abc"), q("")), "abc"),
        (format!("repeat({}, 1000000000)", q("")), ""),
    ];
    let ex: Vec<String> = cases.iter().map(|c| c.0.clone()).collect();
    let got = logs(SIMP, &ex);
    let bad: Vec<String> = cases.iter().zip(got.iter()).filter(|(c, g)| c.1 != **g).map(|(c, g)| format!("{} = {g:?} want {:?}", c.0, c.1)).collect();
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// BS10: repeat / padStart with an absurd count must trap (memory/fuel), never return Ok with garbage or hang.
#[test]
fn string_repeat_huge_is_bounded() {
    for body in ["repeat(\"abcdefgh\", 2147483647)", "padStart(\"x\", 2147483647, \"ab\")"] {
        let src = format!("import {{ Log }} from \"host:log\";\nimport {{ repeat, padStart }} from \"std/string\";\n@host export function go(): i32 {{ return {body}.byteLength; }}\nexport function main(log: Log): void {{ log.write(2, \"up\"); }}\n");
        let mut p = Prog::new(&src);
        assert!(p.rt.call_export("go", &[]).is_err(), "{body} returned Ok");
    }
}

fn big() -> oxiscript_runtime::Limits { let mut l = oxiscript_runtime::Limits::default(); l.fuel_per_call = 4_000_000_000; l }

const ARR: &str = r#"import { Log } from "host:log";
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
fn array_sort_stable_and_total() {
    let mut p = Prog::new(ARR);
    assert_eq!(p.call("stable", &[Value::I32(1000)]), vec![Value::I32(0)]);
    assert_eq!(p.call("evil", &[Value::I32(500)]), vec![Value::I32(500)]);
    assert_eq!(p.call("emptyOne", &[]), vec![Value::I32(5)]);
    // -inf(1) -1(1) -0(2) 0(3) 1/3 ... expected order: -inf,-1,-0,+0,3,+inf,NaN,NaN
    assert_eq!(p.call("nanSort", &[]), vec![Value::I32(11234499)]);
}
#[test]
fn array_sort_comparator_trap_propagates_and_no_leak() {
    let mut p = Prog::new(ARR);
    let r = p.rt.call_export("trapper", &[Value::I32(50)]);
    assert!(r.is_err());
    let _ = p.rt.deinit();
}
#[test]
fn array_sort_30k_within_default_fuel() {
    let mut p = Prog::new(ARR);
    let r = p.rt.call_export("big", &[Value::I32(30_000)]);
    assert_eq!(r.expect("sort of 30k i32 exceeds default limits"), vec![Value::I32(30_000)]);
}
#[test]
fn array_sort_string_order() {
    let mut p = Prog::new(ARR);
    // order "", a, z, é, €, 😀 -> byte lengths 0,1,1,2,3,4
    assert_eq!(p.call("strs", &[]), vec![Value::I32(((((0*7+1)*7+1)*7+2)*7+3)*7+4)]);
}

const COL: &str = r#"import { Log } from "host:log";
import { Map, Set, HashEq } from "std/collections";
struct K implements HashEq<K> { a: i32; hash(): u64 { return 42u64; } equals(o: K): bool { return this.a === o.a; } }
class Node { next: Node | null = null; m: Map<i32, Node> = new Map<i32, Node>(); }
@host export function collide(n: i32): i32 {
  const m = new Map<K, i32>();
  for (const i of range(0, n)) { m.set({ a: i }, i); }
  let t = 0;
  for (const i of range(0, n)) { t = t + (m.get({ a: i }) ?? -100000); }
  for (const i of range(0, n)) { if (i % 2 === 0) { m.remove({ a: i }); } }
  for (const i of range(0, n)) { m.set({ a: i + n }, 1); }
  return t + m.size() * 0;
}
@host export function churn(n: i32): i32 {
  const m = new Map<i32, i32>();
  for (const i of range(0, n)) { m.set(i, i); m.remove(i); }
  m.set(7, 1);
  return m.size() * 1000 + (m.get(7) ?? -1);
}
@host export function order(): i64 {
  const m = new Map<i32, i32>();
  for (const i of range(0, 40)) { m.set(i, i); }
  for (const i of range(0, 40)) { if (i % 3 !== 0) { m.remove(i); } }
  m.set(1, 1); m.set(0, 99);
  let o = 0i64;
  for (const k of m.keys()) { o = (o *% 31i64) +% (k as i64); }
  return o;
}
@host export function cycle(): i32 {
  const a = new Node(); const b = new Node();
  a.next = b; b.next = a;
  a.m.set(1, b); b.m.set(1, a);
  return 1;
}
@host export function bigSet(n: i32): i32 {
  const s = new Set<i32>();
  for (const i of range(0, n)) { s.add(i * 3); }
  let c = 0;
  for (const i of range(0, n)) { if (s.has(i * 3)) { c = c + 1; } if (s.has(i * 3 + 1)) { c = c + 100000000; } }
  return c;
}
@host export function nanKey(): i32 {
  const m = new Map<f64, i32>();
  const nan = 0.0 / 0.0;
  m.set(nan, 1); m.set(nan, 2);
  return m.size() * 10 + (m.get(nan) ?? -1);
}
export function main(log: Log): void { log.write(2, "up"); }
"#;

#[test]
#[ignore = "info: user-supplied constant hash is O(n^2) in any hash table"]
fn map_equal_hash_keys_correct() {
    let mut p = Prog::new(COL);
    let r = p.rt.call_export("collide", &[Value::I32(600)]);
    assert_eq!(r.expect("600 same-hash keys exceed default fuel (O(n^2) probing)"), vec![Value::I32(179700)]);
}
#[test]
fn map_set_remove_churn_stays_correct_and_bounded() {
    let mut p = Prog::from_wasm_limits(compile(COL), big());
    assert_eq!(p.call("churn", &[Value::I32(50_000)]), vec![Value::I32(1001)]);
}
#[test]
fn map_order_after_remove_reinsert() {
    let mut p = Prog::new(COL);
    // survivors 0,3,6..39 in insertion order; 1 appended after; 0 keeps its slot
    let mut w = 0i64; for i in (0..40).filter(|i| i % 3 == 0) { w = w.wrapping_mul(31).wrapping_add(i); } w = w.wrapping_mul(31).wrapping_add(1);
    assert_eq!(p.call("order", &[]), vec![Value::I64(w)]);
}
#[test]
fn set_large_and_miss_lookups() {
    let mut p = Prog::from_wasm_limits(compile(COL), big());
    assert_eq!(p.call("bigSet", &[Value::I32(20_000)]), vec![Value::I32(20_000)]);
}
#[test]
fn map_nan_key_single_entry() {
    let mut p = Prog::new(COL);
    // NaN !== NaN under ===, docs claim hash/equals consistent with ===; either one entry (canonical) or a leak of unreachable entries. Expect 1 entry.
    assert_eq!(p.call("nanKey", &[]), vec![Value::I32(12)]);
}

/// BS11: toFixed of huge finite values must not trap (it traps `overflow` for f64::MAX).
#[test]
fn string_to_fixed_f64_max_does_not_trap() {
    let src = "import { Log } from \"host:log\";\nimport { toFixed } from \"std/string\";\n@host export function fx(x: f64, d: i32): i32 { return toFixed(x, d).byteLength; }\nexport function main(log: Log): void { log.write(2, \"up\"); }\n";
    let mut p = Prog::new(src);
    let r = p.rt.call_export("fx", &[Value::F64(f64::MAX), Value::I32(2)]);
    assert_eq!(r.expect("toFixed(f64::MAX, 2) trapped"), vec![Value::I32(format!("{:.2}", f64::MAX).len() as i32)]);
}

const COL2: &str = r#"import { Log } from "host:log";
import { Map, Set } from "std/collections";
import { indexOf, includes } from "std/array";
@host export function prov(): i32 {
  const m = new Map<string, i32>();
  m.set("héllo", 1);
  const a = "hé" + "llo";
  const b = ["h", "é", "l", "l", "o"].reduce((x: string, y: string): string => x + y, "");
  m.set(a, 2); m.set(b, 3);
  const c = "héllo!".slice(0, 6);
  return m.size() * 100 + (m.get(c) ?? -1);
}
@host export function f32zero(): i32 {
  const m = new Map<f32, i32>();
  m.set(-0.0f32, 1); m.set(0.0f32, 2);
  return m.size() * 10 + (m.get(-0.0f32) ?? -1);
}
@host export function extremes(): i32 {
  const s = new Set<u64>(); const t = new Set<i8>(); const u = new Set<i64>();
  s.add(0xffff_ffff_ffff_ffffu64); s.add(0u64); s.add(0xffff_ffff_ffff_ffffu64);
  t.add(-128i8); t.add(127i8); t.add(-128i8); t.add(-1i8);
  u.add(-9223372036854775807i64 - 1i64); u.add(9223372036854775807i64);
  return s.size() * 100 + t.size() * 10 + u.size();
}
@host export function mutateInForEach(): i32 {
  const m = new Map<i32, i32>();
  for (const i of range(0, 10)) { m.set(i, i); }
  let seen = 0;
  m.forEach((k: i32, v: i32): void => { seen = seen + 1; m.remove(k + 1); m.set(k + 100, v); });
  return seen * 1000 + m.size();
}
@host export function arrayIncludesStrings(): i32 {
  const xs: string[] = ["a", "b" + "c", "é"];
  return (includes(xs, "bc") ? 1 : 0) + indexOf(xs, "é") * 10 + indexOf(xs, "zz") * 100;
}
@host export function clearThenReuse(): i32 {
  const m = new Map<i32, string>();
  for (const i of range(0, 100)) { m.set(i, "v" + i.toString()); }
  m.clear();
  m.set(5, "x");
  return m.size() * 10 + (m.get(5) ?? "").byteLength + (m.get(50) === null ? 100 : 0);
}
export function main(log: Log): void { log.write(2, "up"); }
"#;

#[test]
fn map_string_keys_equal_bytes_different_provenance() {
    let mut p = Prog::new(COL2);
    assert_eq!(p.call("prov", &[]), vec![Value::I32(103)]);
    p.finish();
}
#[test]
fn map_f32_negative_zero_is_same_key() {
    let mut p = Prog::new(COL2);
    assert_eq!(p.call("f32zero", &[]), vec![Value::I32(12)]);
}
#[test]
fn set_integer_extremes() {
    let mut p = Prog::new(COL2);
    assert_eq!(p.call("extremes", &[]), vec![Value::I32(232)]);
}
#[test]
fn map_for_each_with_mutation_is_defined_and_leak_free() {
    let mut p = Prog::new(COL2);
    let r = p.rt.call_export("mutateInForEach", &[]).expect("trap");
    eprintln!("info mutateInForEach = {r:?}");
    p.finish();
}
#[test]
fn array_include_index_strings() {
    let mut p = Prog::new(COL2);
    assert_eq!(p.call("arrayIncludesStrings", &[]), vec![Value::I32(1 + 20 - 100)]);
}
#[test]
fn map_clear_then_reuse_no_leak() {
    let mut p = Prog::new(COL2);
    assert_eq!(p.call("clearThenReuse", &[]), vec![Value::I32(10 + 1 + 100 + 0)]);
    p.finish();
}

/// BS12: out-of-range radix / digits are undocumented-or-documented-as-bounded; record what happens (must not return garbage silently).
#[test]
fn string_invalid_radix_and_digits_do_not_return_garbage() {
    let mut out = vec![];
    for (call, label) in [("toRadix(10i64, 0)", "radix0"), ("toRadix(10i64, 1)", "radix1"), ("toRadix(10i64, 37)", "radix37"), ("toRadix(10i64, -5)", "radix-5"),
                          ("toFixed(1.5, -1)", "digits-1"), ("toFixed(1.5, 21)", "digits21"), ("toFixed(1.5, 100)", "digits100")] {
        let src = format!("import {{ Log }} from \"host:log\";\nimport {{ toRadix, toFixed }} from \"std/string\";\nexport function main(log: Log): void {{ log.write(2, {call}); }}\n");
        let r = std::panic::catch_unwind(|| Prog::new(&src).log());
        out.push(format!("{label}: {}", match r { Ok(l) => format!("{l:?}"), Err(_) => "trap".into() }));
    }
    eprintln!("{}", out.join("\n"));
    // acceptable: a trap. Unacceptable: a plausible-looking string for radix 0/1/37/-5.
    for o in &out[..4] { assert!(o.ends_with("trap"), "invalid radix gave a value: {o}"); }
}
