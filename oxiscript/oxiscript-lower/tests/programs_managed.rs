#![allow(clippy::pedantic, clippy::nursery)]
//! Closed programs: arrays, structs (copy-on-write), classes, nullables, globals.
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::Value;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn prog(body: &str) -> String {
    format!("{HDR}{body}")
}

#[test]
fn p11_arrays_basic() {
    run_log(
        &prog(
            r#"
function sum(xs: i32[]): i32 {
  let s = 0;
  for (const x of xs) s += x;
  return s;
}
export function main(log: Log): void {
  const xs: i32[] = [1, 2, 3];
  xs.push(4);
  xs.push(5);
  log.write(2, `${xs.length} ${xs[0]} ${xs[4]} ${sum(xs)}`);
  xs[1] = 20;
  const p = xs.pop();
  const q = xs.pop();
  log.write(2, `${p ?? -1} ${q ?? -1} ${xs.length} ${sum(xs)}`);
  const e: i32[] = [];
  const r = e.pop();
  log.write(2, `${r ?? -1} ${e.length}`);
  const names: string[] = ["a", "bb", "ccc"];
  names.push("dddd");
  let total = 0;
  for (const n of names) total += n.byteLength;
  log.write(2, `${total} ${names[3]}`);
}
"#,
        ),
        &["5 1 5 15", "5 4 3 24", "-1 0", "10 dddd"],
    );
}

#[test]
fn p12_array_traps() {
    let mut p = Prog::new(&prog(
        r#"
@host export function get(i: i32): i32 { const xs: i32[] = [1, 2, 3]; return xs[i]; }
@host export function set(i: i32): void { const xs: i32[] = [1, 2, 3]; xs[i] = 9; }
export function main(log: Log): void { log.write(2, "up"); }
"#,
    ));
    assert_eq!(p.call("get", &[Value::I32(2)]), vec![Value::I32(3)]);
    let e = p.call_err("get", &[Value::I32(3)]);
    assert_eq!(trap_kind(&e), Some(TrapKind::Bounds));
    p = Prog::from_wasm(p.wasm.clone());
    let e = p.call_err("get", &[Value::I32(-1)]);
    assert_eq!(trap_kind(&e), Some(TrapKind::Bounds));
    p = Prog::from_wasm(p.wasm.clone());
    let e = p.call_err("set", &[Value::I32(7)]);
    assert_eq!(trap_kind(&e), Some(TrapKind::Bounds));
}

#[test]
fn p13_array_methods() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  const xs: i32[] = [1, 2, 3, 4, 5, 6];
  const ev = xs.filter((x) => x % 2 === 0);
  const sq = xs.map((x) => x * x);
  const total = xs.reduce((acc, x) => acc + x, 0);
  log.write(2, `${ev.length} ${ev[2]} ${sq[5]} ${total}`);
  let seen = 0;
  xs.forEach((x) => { seen += x; });
  log.write(2, `${seen} ${xs.some((x) => x > 5)} ${xs.every((x) => x > 1)} ${xs.some((x) => x > 6)}`);
  const sl = xs.slice(1, 4);
  const cc = sl.concat(ev);
  log.write(2, `${sl.length} ${sl[0]} ${cc.length} ${cc[3]} ${cc[5]}`);
  const words = ["x", "yy", "zzz"];
  const lens = words.map((w) => w.byteLength);
  const joined = words.reduce((acc, w) => acc + w, ">");
  log.write(2, `${lens[2]} ${joined}`);
  const strs = xs.map((x) => `n${x}`);
  log.write(2, strs[4]);
}
"#,
        ),
        &["3 6 36 21", "21 true false false", "3 2 6 2 6", "3 >xyyzzz", "n5"],
    );
}

#[test]
fn p14_struct_copy_on_write() {
    run_log(
        &prog(
            r#"
struct Point { x: i32; y: i32; }
struct Line { a: Point; b: Point; label: string; }
function bump(p: Point): Point { let q = p; q.x += 100; return q; }
export function main(log: Log): void {
  const p: Point = { x: 1, y: 2 };
  let q = p;
  q.x = 10;
  log.write(2, `${p.x} ${q.x} ${q.y}`);
  const r = bump(p);
  log.write(2, `${p.x} ${r.x}`);
  let l: Line = { a: p, b: q, label: "L" };
  const m = l;
  l.a.x = 7;
  l.b.y = 8;
  l.label = "M";
  log.write(2, `${l.a.x} ${l.b.y} ${l.label} | ${m.a.x} ${m.b.y} ${m.label} | ${p.x} ${q.y}`);
}
"#,
        ),
        &["1 10 2", "1 101", "7 8 M | 1 2 L | 1 2"],
    );
}

#[test]
fn p15_struct_methods_and_arrays_of_structs() {
    run_log(
        &prog(
            r#"
struct Counter {
  n: i32;
  mutating inc(by: i32): void { this.n += by; }
  mutating bumped(by: i32): i32 { this.n += by; return this.n; }
  get(): i32 { return this.n; }
}
struct P { x: f64; y: f64; }
export function main(log: Log): void {
  let c: Counter = { n: 5 };
  const d = c;
  c.inc(3);
  const r = c.bumped(2);
  log.write(2, `${c.get()} ${d.get()} ${r}`);
  const cs: Counter[] = [{ n: 1 }, { n: 2 }];
  cs[0].inc(10);
  cs[1].n = 7;
  const e = cs[0];
  cs[0].inc(1);
  log.write(2, `${cs[0].get()} ${cs[1].n} ${e.get()}`);
  const ps: P[] = [{ x: 1.0, y: 2.0 }, { x: 3.0, y: 4.0 }];
  for (let i = 0; i < ps.length; i++) {
    ps[i].x += 0.5;
    ps[i].y *= 2.0;
  }
  log.write(2, `${(ps[0].x * 10.0) as i32} ${(ps[1].y) as i32}`);
}
"#,
        ),
        &["10 5 10", "12 7 11", "15 8"],
    );
}

#[test]
fn p16_classes() {
    run_log(
        &prog(
            r#"
class Account {
  balance: i32;
  readonly owner: string;
  history: i32[] = [];
  static count: i32 = 0;
  constructor(owner: string, start: i32) {
    this.owner = owner;
    this.balance = start;
    Account.count += 1;
  }
  deposit(n: i32): void { this.balance += n; this.history.push(n); }
  describe(): string { return `${this.owner}:${this.balance}:${this.history.length}`; }
}
function pay(a: Account): void { a.deposit(5); }
export function main(log: Log): void {
  const a = new Account("ann", 10);
  const b = a;
  const c = new Account("bob", 1);
  pay(a);
  b.deposit(1);
  log.write(2, a.describe());
  log.write(2, `${a === b} ${a === c} ${a !== c} ${Account.count}`);
  log.write(2, c.describe());
}
"#,
        ),
        &["ann:16:2", "true false true 2", "bob:1:0"],
    );
}

#[test]
fn p17_nullables() {
    run_log(
        &prog(
            r#"
struct Node { v: i32; next: Node | null; }
function len(n: Node | null): i32 {
  if (n === null) return 0;
  return 1 + len(n.next);
}
function pick(a: i32 | null, b: i32): i32 { return a ?? b; }
function name(s: string | null): string { return s ?? "anon"; }
function isNull(a: i32 | null): bool { return a === null; }
function show(a: i32 | null): string { if (a !== null) { return `v${a + 1}`; } return "none"; }
function slen(s: string | null): i32 { return s?.byteLength ?? -1; }
export function main(log: Log): void {
  const list: Node = { v: 1, next: { v: 2, next: { v: 3, next: null } } };
  log.write(2, `${len(list)} ${len(null)}`);
  log.write(2, `${pick(5, 9)} ${pick(null, 9)}`);
  log.write(2, `${name("x")} ${name(null)}`);
  log.write(2, `${isNull(7)} ${isNull(null)} ${show(7)} ${show(null)}`);
  log.write(2, `got ${pick(11, 0) + 1}`);
  const tail = list.next;
  if (tail !== null) { log.write(2, `tail ${tail.v}`); }
  log.write(2, `${slen(null)} ${slen("abc")}`);
}
"#,
        ),
        &["3 0", "5 9", "x anon", "false true v8 none", "got 12", "tail 2", "-1 3"],
    );
}

#[test]
fn p18_globals_and_init_order() {
    run_log(
        &prog(
            r#"
const BASE: i32 = 10;
let counter: i32 = BASE * 2;
let name: string = "g" + "lobal";
let items: i32[] = [BASE, BASE + 1];
struct Cfg { a: i32; b: string; }
let cfg: Cfg = { a: counter + 1, b: name };
function next(): i32 { counter += 1; return counter; }
export function main(log: Log): void {
  log.write(2, `${counter} ${name} ${items.length} ${cfg.a} ${cfg.b}`);
  next();
  next();
  items.push(next());
  cfg.a = 100;
  name = "changed";
  log.write(2, `${counter} ${name} ${items[2]} ${cfg.a} ${cfg.b}`);
}
"#,
        ),
        &["20 global 2 21 global", "23 changed 23 100 global"],
    );
}

#[test]
fn p19_borrow_from_field_across_call_convention_10() {
    // the callee overwrites the very field the caller loaded a borrow from
    run_log(
        &prog(
            r#"
class Box { s: string; items: string[] = []; constructor(s: string) { this.s = s; } }
let g: string = "first" + "-global";
let gb: Box = new Box("box" + "-one");
function clobber(b: Box): void { b.s = "second" + "-value"; }
function clobberGlobal(): void { g = "second" + "-global"; }
function use(a: string, b: Box): string { clobber(b); return a + "/" + b.s; }
function useG(a: string): string { clobberGlobal(); return a + "/" + g; }
export function main(log: Log): void {
  const b = new Box("one" + "-two");
  log.write(2, use(b.s, b));
  log.write(2, useG(g));
  gb.items.push("x" + "y");
  const first = gb.items[0];
  gb.items[0] = "z" + "z";
  log.write(2, `${first} ${gb.items[0]}`);
  let cur = gb.s;
  gb = new Box("replaced");
  log.write(2, `${cur} ${gb.s}`);
}
"#,
        ),
        &["one-two/second-value", "first-global/second-global", "xy zz", "box-one replaced"],
    );
}

#[test]
fn p20_recursion() {
    run_log(
        &prog(
            r#"
function fib(n: i32): i32 { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
function fact(n: i64): i64 { if (n <= 1i64) return 1i64; return n * fact(n - 1i64); }
function even(n: i32): bool { if (n === 0) return true; return odd(n - 1); }
function odd(n: i32): bool { if (n === 0) return false; return even(n - 1); }
function rev(s: string): string { return s.byteLength <= 1 ? s : rev(s.slice(1, s.byteLength)) + s.slice(0, 1); }
export function main(log: Log): void {
  log.write(2, `${fib(15)} ${fact(15i64)} ${even(10)} ${odd(7)} ${even(7)}`);
  log.write(2, rev("abcdef"));
}
"#,
        ),
        &["610 1307674368000 true true false", "fedcba"],
    );
}

#[test]
fn p21_loops_and_control_flow() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  let s = 0;
  for (let i = 0; i < 10; i++) { if (i === 3) continue; if (i === 8) break; s += i; }
  let j = 0;
  while (j < 5) { j++; }
  let k = 10;
  do { k -= 3; } while (k > 0);
  let t = 0;
  outer: for (let a = 0; a < 4; a++) {
    for (let b = 0; b < 4; b++) {
      if (b === 2) continue outer;
      if (a === 3) break outer;
      t += a * 10 + b;
    }
  }
  let r = 0;
  for (const x of range(0, 5)) r += x;
  log.write(2, `${s} ${j} ${k} ${t} ${r}`);
  let str = "";
  for (let i = 0; i < 3; i++) { str = str + `${i},`; }
  log.write(2, str);
}
"#,
        ),
        &["25 5 -2 63 10", "0,1,2,"],
    );
}

#[test]
fn p22_destructuring() {
    run_log(
        &prog(
            r#"
struct Q { z: i32; }
struct P { x: i32; y: string; inner: Q; }
struct AB { a: i32; b: i32; }
export function main(log: Log): void {
  const p: P = { x: 1, y: "why", inner: { z: 9 } };
  const { x, y, inner: { z } } = p;
  log.write(2, `${x} ${y} ${z}`);
  const pairs: AB[] = [{ a: 1, b: 2 }, { a: 3, b: 4 }];
  let s = 0;
  for (const { a, b: renamed } of pairs) { s += a * renamed; }
  log.write(2, `${s}`);
}
"#,
        ),
        &["1 why 9", "14"],
    );
}
