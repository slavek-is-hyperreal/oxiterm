#![allow(clippy::pedantic)]
//! Closed programs: closures, generics, interfaces, enums, unions, switch, Result.
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_runtime::Value;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn prog(body: &str) -> String {
    format!("{HDR}{body}")
}

#[test]
fn p23_closures_immutable_and_mutated_captures() {
    run_log(
        &prog(
            r#"
function counter(start: i32): () => i32 {
  let n = start;
  return () => { n += 1; return n; };
}
function adder(k: i32): (x: i32) => i32 { return (x) => x + k; }
function compose(f: (x: i32) => i32, g: (x: i32) => i32): (x: i32) => i32 { return (x) => g(f(x)); }
export function main(log: Log): void {
  const c1 = counter(10);
  const c2 = counter(100);
  c1(); c1();
  log.write(2, `${c1()} ${c2()} ${c1()}`);
  const add5 = adder(5);
  const times = adder(1);
  const both = compose(add5, (x) => x * 2);
  log.write(2, `${add5(1)} ${times(1)} ${both(1)}`);
  let total = 0;
  const bump = (d: i32) => { total += d; };
  bump(3); bump(4);
  log.write(2, `${total}`);
  const prefix = "pre-";
  const label = (s: string) => prefix + s;
  log.write(2, label("fix"));
}
"#,
        ),
        &["13 101 14", "6 2 12", "7", "pre-fix"],
    );
}

#[test]
fn p24_closures_in_fields_and_arrays() {
    run_log(
        &prog(
            r#"
struct Handler { name: string; run: (x: i32) => i32; }
class Bus {
  handlers: ((x: i32) => i32)[] = [];
  add(h: (x: i32) => i32): void { this.handlers.push(h); }
  fire(x: i32): i32 { let s = 0; for (const h of this.handlers) s += h(x); return s; }
}
function twice(f: (x: i32) => i32, x: i32): i32 { return f(f(x)); }
export function main(log: Log): void {
  const h: Handler = { name: "dbl", run: (x) => x * 2 };
  log.write(2, `${h.name} ${h.run(21)} ${twice(h.run, 3)}`);
  const bus = new Bus();
  let calls = 0;
  bus.add((x) => { calls += 1; return x + 1; });
  bus.add((x) => x * 10);
  bus.add(h.run);
  log.write(2, `${bus.fire(5)} ${bus.fire(1)} ${calls}`);
  const fs: (() => string)[] = [() => "a", () => "b"];
  log.write(2, fs[0]() + fs[1]());
}
"#,
        ),
        &["dbl 42 12", "66 14 2", "ab"],
    );
}

#[test]
fn p25_generics() {
    run_log(
        &prog(
            r#"
struct Box<T> { v: T; }
class Stack<T> {
  items: T[] = [];
  push(x: T): void { this.items.push(x); }
  pop(): T | null { return this.items.pop(); }
  size(): i32 { return this.items.length; }
}
function ident<T>(x: T): T { return x; }
function first<T>(xs: T[]): T { return xs[0]; }
function wrap<T>(x: T): Box<T> { return { v: x }; }
function mapAll<T, U>(xs: T[], f: (x: T) => U): U[] { return xs.map(f); }
export function main(log: Log): void {
  log.write(2, `${ident(5)} ${ident("s")} ${ident(true)} ${first([7, 8])} ${first(["p", "q"])}`);
  const a = wrap(3);
  const b = wrap("str");
  const c = wrap(a);
  log.write(2, `${a.v} ${b.v} ${c.v.v}`);
  const s = new Stack<i32>();
  s.push(1); s.push(2); s.push(3);
  const t = new Stack<string>();
  t.push("x");
  log.write(2, `${s.size()} ${s.pop() ?? -1} ${s.pop() ?? -1} ${t.pop() ?? "none"} ${t.pop() ?? "none"}`);
  const lens = mapAll(["a", "bb", "ccc"], (w) => w.byteLength);
  const strs = mapAll([1, 2], (n) => `#${n}`);
  log.write(2, `${lens[2]} ${strs[1]}`);
}
"#,
        ),
        &["5 s true 7 p", "3 str 3", "3 3 2 x none", "3 #2"],
    );
}

#[test]
fn p26_interfaces_and_bounds() {
    run_log(
        &prog(
            r#"
interface Shape { area(): f64; name(): string; }
struct Rect { w: f64; h: f64; area(): f64 { return this.w * this.h; } name(): string { return "rect"; } }
class Circle implements Shape {
  r: f64;
  constructor(r: f64) { this.r = r; }
  area(): f64 { return 3.0 * this.r * this.r; }
  name(): string { return "circle"; }
}
function describe<T extends Shape>(s: T): string { return `${s.name()}:${s.area() as i32}`; }
function total<T extends Shape>(xs: T[]): f64 { let t = 0.0; for (const x of xs) t += x.area(); return t; }
export function main(log: Log): void {
  const r: Rect = { w: 2.0, h: 5.0 };
  const c = new Circle(2.0);
  log.write(2, `${describe(r)} ${describe(c)}`);
  const rs: Rect[] = [{ w: 1.0, h: 2.0 }, { w: 3.0, h: 4.0 }];
  log.write(2, `${total(rs) as i32} ${total([c, new Circle(1.0)]) as i32}`);
}
"#,
        ),
        &["rect:10 circle:12", "14 15"],
    );
}

#[test]
fn p27_enums_and_switch() {
    run_log(
        &prog(
            r#"
enum Color { Red, Green, Blue }
enum Level: u8 { Low = 1, Mid = 5, High = 10 }
function name(c: Color): string {
  switch (c) {
    case Color.Red: return "red";
    case Color.Green: return "green";
    case Color.Blue: return "blue";
  }
}
function cls(n: i32): string {
  switch (n) {
    case 0: return "zero";
    case 1, 2: return "small";
    case 100: return "hundred";
    default: return "many";
  }
}
function lvl(l: Level): i32 {
  return switch (l) { case Level.Low: 1, case Level.Mid: 50, case Level.High: 100 };
}
function word(s: string): i32 {
  switch (s) {
    case "one": return 1;
    case "two": return 2;
    default: return 0;
  }
}
export function main(log: Log): void {
  log.write(2, `${name(Color.Red)} ${name(Color.Blue)} ${Color.Green as i32} ${Level.Mid as i32}`);
  log.write(2, `${cls(0)} ${cls(2)} ${cls(100)} ${cls(7)}`);
  log.write(2, `${lvl(Level.Low)} ${lvl(Level.High)} ${word("two")} ${word("zzz")}`);
  const lim = 3;
  let r = "";
  for (let i = 0; i < 6; i++) {
    switch (i) {
      case 0: r += "a"; break;
      case lim: r += "L"; break;
      case lim + 1: r += "M";
      default: r += ".";
    }
  }
  log.write(2, r);
}
"#,
        ),
        &["red blue 1 5", "zero small hundred many", "1 100 2 0", "a..LM."],
    );
}

#[test]
fn p28_unions_and_exhaustive_switch() {
    run_log(
        &prog(
            r#"
type Shape =
  | { kind: "circle"; r: f64 }
  | { kind: "rect"; w: f64; h: f64 }
  | { kind: "dot" };
function area(s: Shape): f64 {
  switch (s.kind) {
    case "circle": return 3.0 * s.r * s.r;
    case "rect": return s.w * s.h;
    case "dot": return 0.0;
  }
}
function label(s: Shape): string {
  return switch (s.kind) { case "circle": "round", case "rect": "boxy", case "dot": "point" };
}
function isRect(s: Shape): bool { return s.kind === "rect"; }
type Msg = { kind: 1; n: i32 } | { kind: 2; name: string };
function msg(m: Msg): string {
  if (m.kind === 1) { return `n${m.n}`; }
  return `s${m.name}`;
}
export function main(log: Log): void {
  const shapes: Shape[] = [{ kind: "circle", r: 2.0 }, { kind: "rect", w: 2.0, h: 4.5 }, { kind: "dot" }];
  let t = 0.0;
  let names = "";
  for (const s of shapes) { t += area(s); names += label(s) + ","; }
  log.write(2, `${t as i32} ${names} ${isRect(shapes[1])} ${isRect(shapes[0])}`);
  const m1: Msg = { kind: 1, n: 5 };
  const m2: Msg = { kind: 2, name: "bob" };
  log.write(2, `${msg(m1)} ${msg(m2)} ${m1.kind} ${m2.kind} ${shapes[0].kind}`);
}
"#,
        ),
        &["21 round,boxy,point, true false", "n5 sbob 1 2 circle"],
    );
}

#[test]
fn p29_result_and_try() {
    run_log(
        &prog(
            r#"
function parse(s: string): Result<i32, string> {
  if (s.byteLength === 0) return Err("empty");
  let n = 0;
  for (const b of s.bytes()) {
    if (b < 48u8 || b > 57u8) return Err("bad digit");
    n = n * 10 + ((b - 48u8) as i32);
  }
  return Ok(n);
}
function sum(a: string, b: string): Result<i32, string> {
  const x = parse(a)?;
  const y = parse(b)?;
  return Ok(x + y);
}
function chain(a: string): Result<string, string> {
  const n = parse(a)?;
  return Ok(`<${n * 2}>`);
}
function unit(ok: bool): Result<void, string> { if (ok) return Ok(); return Err("no"); }
function useUnit(ok: bool): Result<i32, string> { unit(ok)?; return Ok(1); }
function show(r: Result<i32, string>): string {
  switch (r.kind) {
    case "ok": return `ok ${r.value}`;
    case "err": return `err ${r.error}`;
  }
}
export function main(log: Log): void {
  log.write(2, `${show(sum("12", "30"))} | ${show(sum("1x", "2"))} | ${show(sum("5", ""))}`);
  const c = chain("21");
  log.write(2, c.kind === "ok" ? "chain ok" : "chain err");
  log.write(2, `${show(useUnit(true))} ${show(useUnit(false))}`);
}
"#,
        ),
        &["ok 42 | err bad digit | err empty", "chain ok", "ok 1 err no"],
    );
}

#[test]
fn p30_laws_are_ignored_and_nested_items() {
    run_log(
        &prog(
            r#"
function add(a: i32, b: i32): i32 { return a + b; }
law add_commutes(a: i32, b: i32) when a > -1000 && b > -1000 { add(a, b) === add(b, a) }
export function main(log: Log): void {
  function helper(x: i32): i32 { return x * 3; }
  struct Local { v: i32; }
  const l: Local = { v: helper(4) };
  log.write(2, `${l.v} ${add(1, 2)}`);
}
"#,
        ),
        &["12 3"],
    );
}

#[test]
fn p31_default_params_and_statics() {
    run_log(
        &prog(
            r#"
function scale(x: i32, by: i32 = 3, off: i32 = 1): i32 { return x * by + off; }
class Ids {
  static next: i32 = 100;
  static take(): i32 { const v = Ids.next; Ids.next += 1; return v; }
}
export function main(log: Log): void {
  log.write(2, `${scale(2)} ${scale(2, 4)} ${scale(2, 4, 0)} ${Ids.take()} ${Ids.take()} ${Ids.next}`);
}
"#,
        ),
        &["7 9 8 100 101 102"],
    );
}

#[test]
fn p32_never_function_and_traps_in_exports() {
    let mut p = Prog::new(&prog(
        r#"
function fail(): never { const z: i32 = 0; const q = 1 / z; return fail(); }
export function ok(): i32 { return 5; }
export function boom(c: bool): i32 { if (c) { fail(); } return 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#,
    ));
    assert_eq!(p.call("boom", &[Value::I32(0)]), vec![Value::I32(1)]);
    let e = p.call_err("boom", &[Value::I32(1)]);
    assert_eq!(trap_kind(&e), Some(TrapKind::DivByZero));
}
