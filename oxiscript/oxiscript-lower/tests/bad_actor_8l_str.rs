#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.L: string/array IR ops end to end (bounds, UTF-8 boundaries, empties, leaks).
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::Value;
use Value::I32;

const SRC: &str = r#"import { Log } from "host:log";
const S: string = "aé€😀z";
@host export function sl(a: i32, b: i32): i32 { return S.slice(a, b).byteLength; }
export function slTxt(a: i32, b: i32): string { return S.slice(a, b); }
@host export function slOf(s: string, a: i32, b: i32): i32 { return s.slice(a, b).byteLength; }
@host export function bytesOf(s: string): i32 { const bs = s.bytes(); let t = 0; for (const b of bs) t = t * 3 + (b as i32); return t % 1000003 + bs.length; }
@host export function charsOf(s: string): i32 { const cs = s.chars(); return cs.length * 1000 + (cs.length > 0 ? (cs[cs.length - 1] as i32) % 1000 : 0); }
@host export function idx(s: string): i32 { return s.indexOf("é"); }
@host export function idxE(s: string): i32 { return s.indexOf(""); }
@host export function starts(s: string): i32 { return (s.startsWith("a") ? 1 : 0) + (s.startsWith("") ? 2 : 0) + (s.startsWith(s + "x") ? 4 : 0) + (s.endsWith("z") ? 8 : 0) + (s.includes("€") ? 16 : 0) + (s.endsWith("") ? 32 : 0); }
@host export function cmp(s: string): i32 { return (s < "b" ? 1 : 0) + (s > "a" ? 2 : 0) + (s === "a" ? 4 : 0) + (s !== "" ? 8 : 0) + (s <= "a\0" ? 16 : 0) + (s >= "é" ? 32 : 0); }
@host export function eqPrefix(s: string): i32 { return (s === s.slice(0, 1) ? 1 : 0) + (s + "" === s ? 2 : 0) + ("ab" === s + "b" ? 4 : 0); }
@host export function popStr(): i32 {
  const xs: string[] = ["a" + "b", "c" + "d", "e" + "f"];
  const p = xs.pop();
  const q = xs.pop();
  const r = xs.pop();
  const s = xs.pop();
  return (p ?? "").byteLength + (q ?? "").byteLength * 10 + (r ?? "").byteLength * 100 + (s === null ? 1000 : 0) + xs.length * 10000;
}
@host export function emptyArr(): i32 { const e: string[] = []; const f: i32[] = []; e.push("x" + "y"); f.push(1); return e.length + f.length + (e.pop() ?? "").byteLength; }
@host export function popEmptyStructs(): i32 { const e: P[] = []; const p = e.pop(); return p === null ? 7 : 8; }
@host export function popThenPush(): i32 { const xs: P[] = [{ s: "a" + "b" }]; const p = xs.pop(); xs.push({ s: "z" }); xs.push({ s: "w" + "w" }); const q = xs.pop(); return xs.length * 100 + (p ?? { s: "" }).s.byteLength * 10 + (q ?? { s: "" }).s.byteLength; }
struct P { s: string; }
export function main(log: Log): void { log.write(2, "up"); }
"#;

fn run(wasm: &[u8], name: &str, args: &[Value]) -> Result<Vec<Value>, Option<TrapKind>> {
    let mut p = Prog::from_wasm(wasm.to_vec());
    let r = p.rt.call_export(name, args);
    match r {
        Ok(v) => {
            let n = p.rt.deinit().unwrap_or(-99);
            if n != 0 { eprintln!("LEAK {n} after {name}({args:?})"); return Err(None); }
            Ok(v)
        }
        Err(e) => { eprintln!("{name}: {e}"); Err(trap_kind(&e)) }
    }
}
fn s(x: &str) -> Value { Value::Str(x.to_string()) }

#[test]
fn string_ops_table() {
    let wasm = compile(SRC);
    let mut bad = Vec::new();
    let mut n = 0;
    // S = a(0) é(1,2) €(3..5) 😀(6..9) z(10); byteLength 11
    let want_sl = |a: i32, b: i32| -> Result<Vec<Value>, Option<TrapKind>> {
        let st = "aé€😀z";
        let (a2, b2) = (a as usize, b as usize);
        if a < 0 || b < 0 || a > b || b2 > st.len() { return Err(Some(TrapKind::Bounds)); }
        if !st.is_char_boundary(a2) || !st.is_char_boundary(b2) { return Err(Some(TrapKind::BadUtf8)); }
        Ok(vec![I32((b2 - a2) as i32)])
    };
    for a in -2..=13 { for b in -2..=13 {
        n += 1;
        let got = run(&wasm, "sl", &[I32(a), I32(b)]);
        let want = want_sl(a, b);
        // accept Bounds or BadUtf8 for either error flavour but never a value for an invalid slice
        let same = match (&got, &want) { (Ok(_), Ok(_)) => got == want, (Err(Some(_)), Err(Some(_))) => true, _ => false };
        if !same { bad.push(format!("S.slice({a},{b}) = {got:?} want {want:?}")); }
    } }
    n += 1;
    if run(&wasm, "slOf", &[s("héllo"), I32(0), I32(0)]) != Ok(vec![I32(0)]) { bad.push("slOf empty".into()); }
    n += 1;
    if run(&wasm, "slOf", &[s(""), I32(0), I32(0)]) != Ok(vec![I32(0)]) { bad.push("slOf on empty string".into()); }
    n += 1;
    if run(&wasm, "slOf", &[s(""), I32(0), I32(1)]) != Err(Some(TrapKind::Bounds)) { bad.push("slOf empty (0,1) must trap bounds".into()); }
    n += 1;
    if run(&wasm, "slOf", &[s("abc"), I32(i32::MAX), I32(i32::MAX)]) != Err(Some(TrapKind::Bounds)) { bad.push("slOf (MAX,MAX) must trap bounds".into()); }
    n += 1;
    if run(&wasm, "slOf", &[s("abc"), I32(3), I32(3)]) != Ok(vec![I32(0)]) { bad.push("slOf (len,len) must be empty".into()); }
    n += 1;
    if run(&wasm, "slOf", &[s("abc"), I32(2), I32(i32::MIN)]) != Err(Some(TrapKind::Bounds)) { bad.push("slOf (2,MIN) must trap bounds".into()); }
    for (name, arg) in [("idx", "héllo é"), ("idx", ""), ("idx", "e"), ("idxE", ""), ("idxE", "abc"), ("starts", "a€z"), ("starts", ""), ("starts", "z"), ("cmp", "a"), ("cmp", ""), ("cmp", "a\0"), ("cmp", "é"), ("cmp", "b"), ("cmp", "aé"), ("eqPrefix", "a"), ("eqPrefix", "ab"), ("charsOf", "a€😀"), ("charsOf", ""), ("bytesOf", "é€"), ("bytesOf", "")] {
        n += 1;
        let got = run(&wasm, name, &[s(arg)]);
        let want = oracle(name, arg);
        if got != Ok(vec![I32(want)]) { bad.push(format!("{name}({arg:?}) = {got:?} want {want}")); }
    }
    for (name, want) in [("popStr", 2 + 20 + 200 + 1000), ("emptyArr", 2 + 1 + 1), ("popEmptyStructs", 7), ("popThenPush", 100 + 20 + 2)] {
        n += 1;
        let got = run(&wasm, name, &[]);
        if got != Ok(vec![I32(want)]) { bad.push(format!("{name}() = {got:?} want {want}")); }
    }
    eprintln!("string/array cases: {n}, mismatches {}", bad.len());
    assert!(bad.is_empty(), "{} of {n} differ:\n{}", bad.len(), bad.join("\n"));
}

fn oracle(name: &str, a: &str) -> i32 {
    let b = |c: bool, v: i32| if c { v } else { 0 };
    match name {
        "idx" => a.find('é').map(|x| x as i32).unwrap_or(-1),
        "idxE" => 0,
        "starts" => b(a.starts_with('a'), 1) + 2 + b(false, 4) + b(a.ends_with('z'), 8) + b(a.contains('€'), 16) + 32,
        "cmp" => b(a.as_bytes() < b"b", 1) + b(a.as_bytes() > b"a", 2) + b(a == "a", 4) + b(!a.is_empty(), 8) + b(a.as_bytes() <= b"a\0", 16) + b(a.as_bytes() >= "é".as_bytes(), 32),
        "eqPrefix" => b(a == a.chars().next().map(|c| c.to_string()).unwrap_or_default(), 1) + 2 + b("ab" == format!("{a}b"), 4),
        "charsOf" => { let cs: Vec<char> = a.chars().collect(); cs.len() as i32 * 1000 + cs.last().map(|c| (*c as i32) % 1000).unwrap_or(0) }
        "bytesOf" => { let mut t: i64 = 0; for x in a.bytes() { t = t * 3 + x as i64; } (t % 1000003) as i32 + a.len() as i32 }
        _ => unreachable!(),
    }
}
