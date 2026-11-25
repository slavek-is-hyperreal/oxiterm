#![allow(clippy::pedantic)]
//! Tricky ownership and control-flow shapes. Every program ends with zero live objects.
mod common;
use common::*;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn prog(body: &str) -> String {
    format!("{HDR}{body}")
}

#[test]
fn p40_early_returns_release_everything() {
    run_log(
        &prog(
            r#"
struct P { name: string; tags: string[]; }
function find(ps: P[], want: string): string {
  const prefix = "found:" + want;
  for (const p of ps) {
    const label = p.name + "!";
    if (p.name === want) {
      const extra = [label, prefix];
      if (extra.length > 1) { return extra[0] + extra[1]; }
    }
    for (const t of p.tags) {
      const tt = t + "?";
      if (tt === want + "?") { return "tag:" + tt; }
    }
  }
  return "none";
}
export function main(log: Log): void {
  const ps: P[] = [{ name: "a", tags: ["x", "y"] }, { name: "b", tags: ["z"] }];
  log.write(2, find(ps, "b"));
  log.write(2, find(ps, "y"));
  log.write(2, find(ps, "q"));
}
"#,
        ),
        &["b!found:b", "tag:y?", "none"],
    );
}

#[test]
fn p41_try_with_pending_temporaries() {
    run_log(
        &prog(
            r#"
function name(ok: bool): Result<string, string> { return ok ? Ok("n" + "ame") : Err("noname"); }
function age(ok: bool): Result<string, string> { return ok ? Ok("4" + "2") : Err("noage"); }
function join3(a: string, b: string, c: string): string { return a + "/" + b + "/" + c; }
function person(a: bool, b: bool): Result<string, string> {
  const pre = "p" + "-";
  const arr = [pre, "x" + "y"];
  return Ok(join3(name(a)?, age(b)?, arr[1]));
}
function show(r: Result<string, string>): string { return r.kind === "ok" ? r.value : "E:" + r.error; }
export function main(log: Log): void {
  log.write(2, `${show(person(true, true))} ${show(person(false, true))} ${show(person(true, false))}`);
}
"#,
        ),
        &["name/42/xy E:noname E:noage"],
    );
}

#[test]
fn p42_nested_closures() {
    run_log(
        &prog(
            r#"
function make(): () => () => i32 {
  let n = 0;
  const label = "l" + "bl";
  return () => {
    n += 10;
    const inner = () => { n += 1; return n + label.byteLength; };
    return inner;
  };
}
function apply(f: () => i32): i32 { return f() + f(); }
export function main(log: Log): void {
  const outer = make();
  const a = outer();
  const b = outer();
  log.write(2, `${a()} ${b()} ${a()} ${apply(b)}`);
  const total: i32[] = [0];
  const addAll = (xs: i32[]) => { xs.forEach((x) => { total[0] += x; }); };
  addAll([1, 2, 3]);
  addAll([10]);
  log.write(2, `${total[0]}`);
}
"#,
        ),
        &["24 25 26 55", "16"],
    );
}

#[test]
fn p43_closures_capturing_this() {
    run_log(
        &prog(
            r#"
class Counter {
  n: i32 = 0;
  step: i32;
  constructor(step: i32) { this.step = step; }
  incrementer(): () => i32 { return () => { this.n += this.step; return this.n; }; }
}
struct V {
  x: i32;
  mutating scale(k: i32): () => i32 { this.x *= k; return () => this.x; }
}
export function main(log: Log): void {
  const c = new Counter(3);
  const inc = c.incrementer();
  inc(); inc();
  log.write(2, `${c.n} ${inc()}`);
  let v: V = { x: 2 };
  const snap = v.scale(5);
  v.x = 100;
  log.write(2, `${snap()} ${v.x}`);
}
"#,
        ),
        &["6 9", "10 100"],
    );
}

#[test]
fn p44_break_continue_with_owned_locals() {
    run_log(
        &prog(
            r#"
export function main(log: Log): void {
  let out = "";
  const words = ["alpha", "beta", "gamma", "delta", "eps"];
  for (let i = 0; i < words.length; i++) {
    const w = words[i] + "-" + `${i}`;
    if (i === 1) continue;
    const parts = [w, w + w];
    if (parts[1].byteLength > 20) break;
    out += parts[0].slice(0, 2);
    while (true) {
      const tmp = w + "~";
      if (tmp.byteLength > 0) break;
    }
  }
  log.write(2, out);
  let k = 0;
  let acc = "";
  do {
    const s = "k" + `${k}`;
    k += 1;
    if (k === 2) continue;
    acc += s;
  } while (k < 4);
  log.write(2, acc);
}
"#,
        ),
        &["algadeep", "k0k2k3"],
    );
}

#[test]
fn p45_switch_with_owned_scrutinee_and_returns() {
    run_log(
        &prog(
            r#"
function classify(s: string): string {
  const key = s + "!";
  switch (key) {
    case "a!": { const r = "A" + key; return r; }
    case "b!": return "B";
    default: break;
  }
  const rest = [key, key];
  return rest[0] + rest[1];
}
function code(n: i32): string {
  const m = `${n}`;
  switch (n) {
    case 1: return m + "one";
    case 2:
      if (n > 1) { return m + "two"; }
      break;
    default: return "other";
  }
  return "fallthrough";
}
export function main(log: Log): void {
  log.write(2, `${classify("a")} ${classify("b")} ${classify("zz")}`);
  log.write(2, `${code(1)} ${code(2)} ${code(9)}`);
}
"#,
        ),
        &["Aa! B zz!zz! 1one 2two other"],
    );
}

#[test]
fn p46_struct_with_array_field_shares_the_array() {
    run_log(
        &prog(
            r#"
struct Bag { items: string[]; n: i32; }
export function main(log: Log): void {
  let a: Bag = { items: ["x"], n: 1 };
  const b = a;
  a.n = 2;
  a.items.push("y");
  log.write(2, `${a.n} ${b.n} ${a.items.length} ${b.items.length}`);
}
"#,
        ),
        &["2 1 2 2"],
    );
}

#[test]
fn p47_optional_calls_and_methods() {
    run_log(
        &prog(
            r#"
class Greeter { greeting: string; constructor(g: string) { this.greeting = g; } hello(n: string): string { return this.greeting + " " + n; } }
function maybe(g: Greeter | null, n: string): string { return g?.hello(n) ?? "nobody"; }
function call(cb: ((x: i32) => i32) | null, x: i32): i32 { return cb?.(x) ?? -1; }
export function main(log: Log): void {
  log.write(2, `${maybe(new Greeter("hi"), "bob")} ${maybe(null, "bob")}`);
  log.write(2, `${call((x) => x * 3, 4)} ${call(null, 4)}`);
}
"#,
        ),
        &["hi bob nobody 12 -1"],
    );
}

#[test]
fn p48_joins_with_many_assigned_variables() {
    run_log(
        &prog(
            r#"
struct S { a: i32; b: string; }
export function main(log: Log): void {
  let s = "";
  let t: S = { a: 0, b: "z" };
  let n = 0;
  const xs = [3, 1, 4, 1, 5, 9, 2, 6];
  for (const x of xs) {
    if (x % 2 === 0) {
      s += `e${x}`;
      t.a += x;
    } else if (x > 4) {
      s += `b${x}`;
      n += 100;
      t.b = t.b + "!";
    } else {
      n += 1;
      let u = t;
      u.a = -1;
    }
  }
  log.write(2, `${s} ${t.a} ${t.b} ${n}`);
}
"#,
        ),
        &["e4b5b9e2e6 12 z!! 203"],
    );
}

#[test]
fn p49_nested_struct_mutation_through_class_and_array() {
    run_log(
        &prog(
            r#"
struct V { x: i32; y: i32; mutating move(dx: i32, dy: i32): void { this.x += dx; this.y += dy; } }
class Sprite { pos: V; name: string; constructor(name: string) { this.name = name; this.pos = { x: 0, y: 0 }; } }
struct World { sprites: Sprite[]; origin: V; }
export function main(log: Log): void {
  const s = new Sprite("s");
  const before = s.pos;
  s.pos.move(1, 2);
  s.pos.move(10, 20);
  s.pos.x += 5;
  log.write(2, `${s.pos.x} ${s.pos.y} ${before.x}`);
  let w: World = { sprites: [s], origin: { x: 7, y: 8 } };
  const copy = w;
  w.origin.move(1, 1);
  w.sprites[0].pos.move(100, 100);
  log.write(2, `${w.origin.x} ${copy.origin.x} ${s.pos.x} ${copy.sprites[0].pos.y}`);
}
"#,
        ),
        &["16 22 0", "8 7 116 122"],
    );
}

#[test]
fn p50_determinism_same_source_same_bytes() {
    let src = prog(
        r#"
struct P { x: i32; y: string; }
function f<T>(x: T): T { return x; }
export function main(log: Log): void {
  const p: P = { x: f(1), y: f("s") };
  const g = (n: i32) => n + p.x;
  log.write(2, `${g(2)} ${p.y}`);
}
"#,
    );
    let a = compile_full(&src);
    let b = compile_full(&src);
    assert_eq!(a.wasm, b.wasm);
    assert_eq!(oxiscript_ir::print_module(&a.module), oxiscript_ir::print_module(&b.module));
}

#[test]
fn p51_convention_10_loads_are_retained_before_the_clobbering_call() {
    // `use(b.s, b)`: the field value must be retained at the load, before `clobber` can overwrite the field
    let out = compile_full(&prog(
        r#"
class Box { s: string; constructor(s: string) { this.s = s; } }
function clobber(b: Box): void { b.s = "second" + "-value"; }
function use(a: string, b: Box): string { clobber(b); return a + "/" + b.s; }
export function main(log: Log): void {
  const b = new Box("one" + "-two");
  log.write(2, use(b.s, b));
}
"#,
    ));
    let text = oxiscript_ir::print_module(&out.module);
    let main = text.split("func @main").nth(1).expect("main");
    let user_main_start = text.find("func @main").unwrap();
    let _ = (main, user_main_start);
    // find the lowered user main: the function that calls `use`
    let f = text
        .split("func @")
        .find(|f| f.contains("call @use."))
        .expect("caller of use");
    let lines: Vec<&str> = f.lines().collect();
    let load = lines
        .iter()
        .position(|l| l.contains("getfield Box_") && l.contains(".s "))
        .expect("field load");
    let name = lines[load].trim().split(':').next().unwrap().trim().to_string();
    let call = lines.iter().position(|l| l.contains("call @use.")).unwrap();
    assert!(
        lines[load + 1..call].iter().any(|l| l.trim() == format!("retain {name}")),
        "the loaded field {name} must be retained before the call:\n{f}"
    );
    let mut p = Prog::from_wasm(out.wasm);
    assert_eq!(p.log(), vec!["one-two/second-value"]);
    p.finish();
}
