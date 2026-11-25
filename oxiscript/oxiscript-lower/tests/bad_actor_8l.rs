#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.L: targeted probes. Each `probe!` is a test; FAIL = finding (or probe error, see message).
//! Message prefixes: REJECT (checker/lowering diagnostic), PANIC (compiler panic), TRAP (main trapped),
//! LOG (wrong output), LEAK (live objects after deinit).
mod common;
use common::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

pub struct Out {
    pub log: Vec<String>,
    pub live: i32,
}

pub fn run_probe(src: &str) -> Result<Out, String> {
    let r = catch_unwind(AssertUnwindSafe(|| {
        let wasm = match oxiscript_lower::compile_source(src, &interfaces(), &oxiscript_lower::Options::default()) {
            Ok(w) => w,
            Err(e) => return Err(format!("REJECT: {e}")),
        };
        let env = env();
        let mut rt = match env.host.instantiate(&wasm, oxiscript_runtime::Grants::all()) {
            Ok(r) => r,
            Err(e) => {
                let log: Vec<String> = env.sink.lines().into_iter().map(|l| l.msg).collect();
                return Err(format!("TRAP: {e} log={log:?}"));
            }
        };
        let log: Vec<String> = env.sink.lines().into_iter().map(|l| l.msg).collect();
        let live = rt.deinit().map_err(|e| format!("TRAP in deinit: {e}"))?;
        Ok(Out { log, live })
    }));
    match r {
        Ok(x) => x,
        Err(p) => {
            let m = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
            Err(format!("PANIC: {m}"))
        }
    }
}

pub const HDR: &str = "import { Log } from \"host:log\";\n";

pub fn check(body: &str, want: &[&str]) {
    let src = format!("{HDR}{body}");
    match run_probe(&src) {
        Err(e) => panic!("{e}\n--- source ---\n{src}"),
        Ok(o) => {
            let want: Vec<String> = want.iter().map(|s| s.to_string()).collect();
            assert_eq!(o.log, want, "LOG mismatch\n--- source ---\n{src}");
            assert_eq!(o.live, 0, "LEAK: {} live objects\n--- source ---\n{src}", o.live);
        }
    }
}

macro_rules! probe {
    ($name:ident, $body:expr, $want:expr) => {
        #[test]
        fn $name() {
            check($body, &$want);
        }
    };
}

macro_rules! probe_ign {
    ($name:ident, $body:expr, $want:expr) => {
        #[test]
        #[ignore = "closure cycle leak: expected until the 8.M cycle collector exists (Stub has none); handoff L04"]
        fn $name() {
            check($body, &$want);
        }
    };
}

// ---------------- convention 10: borrows across calls/stores ----------------
probe!(c10_this_field_held_across_reset, r#"
class H { arr: i32[] = [1, 2, 3]; reset(): void { this.arr = [9]; }
  f(): i32 { const x = this.arr; this.reset(); return x.length * 10 + x[0]; } }
export function main(log: Log): void { const h = new H(); log.write(2, `${h.f()} ${h.arr.length}`); }
"#, ["31 1"]);

probe!(c10_field_passed_as_arg_callee_replaces, r#"
class H { arr: string[] = []; constructor() { this.arr = ["a" + "b", "c" + "d"]; } reset(): void { this.arr = ["z"]; } }
function g(a: string[], h: H): string { h.reset(); return a[0] + a[1]; }
export function main(log: Log): void { const h = new H(); log.write(2, g(h.arr, h)); log.write(2, h.arr[0]); }
"#, ["abcd", "z"]);

probe!(c10_array_elem_overwritten_by_callee, r#"
function g(xs: string[]): string { xs[0] = "zz"; return "!"; }
export function main(log: Log): void {
  const xs: string[] = ["a" + "b", "c"];
  const s = xs[0] + g(xs);
  log.write(2, s); log.write(2, xs[0]);
}
"#, ["ab!", "zz"]);

probe!(c10_array_elem_popped_by_callee, r#"
function g(xs: string[]): string { xs.pop(); xs.pop(); return "!"; }
export function main(log: Log): void {
  const xs: string[] = ["a" + "b", "c" + "d"];
  const s = xs[0] + g(xs) + `${xs.length}`;
  log.write(2, s);
}
"#, ["ab!0"]);

probe!(c10_index_expr_swaps_items, r#"
class O { items: string[] = ["p" + "q", "r"]; bump(): i32 { this.items = ["new"]; return 0; } }
export function main(log: Log): void { const o = new O(); const s = o.items[o.bump()]; log.write(2, s); log.write(2, o.items[0]); }
"#, ["pq", "new"]);

probe!(c10_compound_assign_callee_mutates, r#"
class O { v: string = "a" + "b"; }
function rep(o: O): string { o.v = "zz"; return "!"; }
export function main(log: Log): void { const o = new O(); o.v += rep(o); log.write(2, o.v); }
"#, ["ab!"]);

probe!(c10_int_compound_assign, r#"
class O { v: i32 = 5; }
function rep(o: O): i32 { o.v = 100; return 1; }
export function main(log: Log): void { const o = new O(); o.v += rep(o); log.write(2, `${o.v}`); }
"#, ["6"]);

probe!(c10_global_replaced_by_callee, r#"
let G: string[] = [];
function init(): void { G = ["x" + "y", "z"]; }
function swap(): string { G = ["new"]; return "!"; }
function use(a: string[]): string { const s = swap(); return a[0] + s; }
export function main(log: Log): void { init(); log.write(2, use(G)); log.write(2, G[0]); }
"#, ["xy!", "new"]);

probe!(c10_closure_env_overwritten, r#"
function mk(): () => string {
  let s = "a" + "b";
  const f = () => { const old = s; s = "zz" + "z"; return old + s; };
  return f;
}
export function main(log: Log): void { const f = mk(); log.write(2, f()); log.write(2, f()); }
"#, ["abzzz", "zzzzzz"]);

probe!(c10_closure_calls_itself_replaces_captured, r#"
class Box { s: string = "q" + "r"; }
export function main(log: Log): void {
  const b = new Box();
  const f = (): string => { const x = b.s; b.s = "new" + "s"; return x; };
  log.write(2, f() + f());
}
"#, ["qrnews"]);

probe!(c10_two_refs_same_instance, r#"
class N { name: string = "n" + "0"; next: N | null = null; }
function relink(a: N, b: N): string { a.name = "A" + "A"; const old = b.name; b.name = "B" + "B"; return old; }
export function main(log: Log): void { const n = new N(); const m = n; log.write(2, relink(n, m)); log.write(2, n.name); }
"#, ["AA", "BB"]);

probe!(c10_array_swap_self_assign, r#"
export function main(log: Log): void {
  const xs: string[] = ["a" + "1", "b" + "2"];
  xs[0] = xs[1]; xs[1] = xs[0]; xs[0] = xs[0];
  log.write(2, xs[0] + xs[1]);
}
"#, ["b2b2"]);

probe!(c10_field_self_assign, r#"
class O { s: string = "a" + "b"; }
export function main(log: Log): void { const o = new O(); o.s = o.s; o.s = o.s + o.s; log.write(2, o.s); }
"#, ["abab"]);

// ---------------- copy-on-write ----------------
probe!(cow_nested_struct_in_array_in_class, r#"
struct In { v: i32; }
struct Mid { i: In; tag: string; }
class C { ps: Mid[] = []; }
export function main(log: Log): void {
  const c = new C();
  c.ps.push({ i: { v: 1 }, tag: "t" + "0" });
  let m = c.ps[0];
  m.i.v = 99;
  log.write(2, `${c.ps[0].i.v} ${m.i.v}`);
  c.ps[0].i.v = 7;
  log.write(2, `${c.ps[0].i.v} ${m.i.v}`);
  const m2 = c.ps[0];
  c.ps[0].tag = "chg";
  log.write(2, `${m2.tag} ${c.ps[0].tag}`);
}
"#, ["1 99", "7 99", "t0 chg"]);

probe!(cow_struct_in_struct_copy, r#"
struct A { x: i32; }
struct B { a: A; s: string; }
struct C3 { b: B; }
export function main(log: Log): void {
  let c: C3 = { b: { a: { x: 1 }, s: "s" + "s" } };
  const d = c;
  c.b.a.x = 2;
  c.b.s = "new";
  log.write(2, `${d.b.a.x} ${d.b.s} ${c.b.a.x} ${c.b.s}`);
  let e = d;
  e.b.a.x += 40;
  log.write(2, `${d.b.a.x} ${e.b.a.x} ${c.b.a.x}`);
}
"#, ["1 ss 2 new", "1 41 2"]);

probe!(cow_param_mutated_callee, r#"
struct V { n: i32; tags: string[]; mutating inc(): void { this.n += 1; } }
function f(p: V): i32 { p.n = 50; p.inc(); return p.n; }
function g(p: V): V { p.inc(); return p; }
export function main(log: Log): void {
  const v: V = { n: 1, tags: [] };
  const r = f(v);
  const w = g(v);
  log.write(2, `${v.n} ${r} ${w.n}`);
}
"#, ["1 51 2"]);

probe!(cow_mutating_method_on_elem_and_field, r#"
struct Ctr { n: i32; mutating inc(by: i32): void { this.n += by; } }
class H { c: Ctr = { n: 0 }; cs: Ctr[] = [{ n: 1 }, { n: 2 }]; }
export function main(log: Log): void {
  const h = new H();
  const snap = h.c;
  const snaps = h.cs[1];
  h.c.inc(5);
  h.cs[1].inc(10);
  h.cs[0].inc(1);
  log.write(2, `${snap.n} ${h.c.n} ${snaps.n} ${h.cs[1].n} ${h.cs[0].n}`);
}
"#, ["0 5 2 12 2"]);

probe!(cow_global_struct, r#"
struct Ctr { n: i32; mutating inc(): void { this.n += 1; } }
let G: Ctr = { n: 10 };
function bump(): void { G.inc(); }
export function main(log: Log): void {
  const before = G;
  bump(); bump();
  G.n += 100;
  log.write(2, `${before.n} ${G.n}`);
}
"#, ["10 112"]);

probe!(cow_shallow_array_field, r#"
struct S { xs: i32[]; n: i32; }
export function main(log: Log): void {
  let a: S = { xs: [1], n: 1 };
  let b = a;
  b.xs.push(2);
  b.n = 5;
  log.write(2, `${a.xs.length} ${b.xs.length} ${a.n} ${b.n}`);
  b.xs = [7, 8, 9];
  log.write(2, `${a.xs.length} ${b.xs.length}`);
}
"#, ["2 2 1 5", "2 3"]);

probe!(cow_struct_array_elem_copies, r#"
struct P { x: i32; }
export function main(log: Log): void {
  const ps: P[] = [{ x: 1 }, { x: 2 }];
  let q = ps[0];
  q.x = 10;
  ps[1] = q;
  q.x = 20;
  log.write(2, `${ps[0].x} ${ps[1].x} ${q.x}`);
  const qs = ps;           // arrays are references
  qs[0].x = 5;
  log.write(2, `${ps[0].x}`);
}
"#, ["1 10 20", "5"]);

probe!(cow_struct_captured_and_mutated, r#"
struct V { x: i32; }
export function main(log: Log): void {
  let v: V = { x: 1 };
  const f = () => v.x;
  const w = v;
  v.x = 5;
  const g = () => { v.x += 1; return v.x; };
  log.write(2, `${f()} ${w.x} ${g()} ${f()} ${v.x}`);
}
"#, ["5 1 6 6 6"]);

probe!(cow_struct_in_union_variant_and_nullable, r#"
struct P { x: i32; }
struct W { p: P | null; }
export function main(log: Log): void {
  let w: W = { p: { x: 1 } };
  const w2 = w;
  w.p = { x: 2 };
  const a = w2.p;
  log.write(2, `${(a ?? { x: -1 }).x} ${(w.p ?? { x: -1 }).x}`);
}
"#, ["1 2"]);

// ---------------- evaluation order ----------------
const TR: &str = r#"
let n: i32 = 0;
function t(k: i32): i32 { n = n * 10 + k; return k; }
function a3(x: i32, y: i32, z: i32): i32 { return x + y * z; }
"#;

#[test]
fn order_args_and_binary() {
    check(&format!("{TR}{}", r#"
export function main(log: Log): void {
  const r = a3(t(1), t(2), t(3));
  log.write(2, `${n} ${r}`);
  n = 0;
  const q = t(1) + t(2) * t(3) - t(4);
  log.write(2, `${n} ${q}`);
}"#), &["123 7", "1234 3"]);
}

#[test]
fn order_assign_index_compound() {
    check(&format!("{TR}{}", r#"
class O { v: i32 = 0; }
function pick(os: O[], i: i32): O { return os[i]; }
export function main(log: Log): void {
  const xs: i32[] = [0, 0, 0, 0];
  xs[t(1)] = t(2);
  log.write(2, `${n}`);
  n = 0;
  xs[t(1)] += t(2);
  log.write(2, `${n} ${xs[1]}`);
  n = 0;
  const os: O[] = [new O(), new O()];
  pick(os, t(1)).v = t(0);
  log.write(2, `${n}`);
  n = 0;
  pick(os, t(1)).v += t(3);
  log.write(2, `${n} ${os[1].v}`);
}"#), &["12", "12 4", "10", "13 3"]);
}

#[test]
fn order_short_circuit() {
    check(&format!("{TR}{}", r#"
function nn(k: i32): i32 | null { n = n * 10 + k; return k > 5 ? k : null; }
export function main(log: Log): void {
  let r = t(1) > 5 && t(2) > 0;
  log.write(2, `${n} ${r}`);
  n = 0;
  r = t(1) > 0 || t(2) > 0;
  log.write(2, `${n} ${r}`);
  n = 0;
  r = t(1) > 5 && t(2) > 0 || t(3) > 0;
  log.write(2, `${n} ${r}`);
  n = 0;
  const a = nn(7) ?? nn(8);
  log.write(2, `${n} ${a ?? -1}`);
  n = 0;
  const b = nn(1) ?? nn(9) ?? nn(2);
  log.write(2, `${n} ${b ?? -1}`);
  n = 0;
  const c = t(1) > 0 ? t(2) : t(3);
  const d = t(0) > 0 ? t(4) : t(5);
  log.write(2, `${n} ${c} ${d}`);
}"#), &["1 false", "1 true", "13 true", "7 7", "19 9", "1205 2 5"]);
}

#[test]
fn order_try_in_nested_positions() {
    check(&format!("{}", r#"
let n: i32 = 0;
function ok(s: string): Result<string, string> { n += 1; return Ok(s + "!"); }
function bad(): Result<string, string> { n += 1; return Err("bad"); }
function two(a: string, b: string): string { return a + b; }
function run(k: i32): Result<string, string> {
  const pre = "pre" + `${k}`;
  if (k === 0) { const arr = [pre, ok("a")?, ok("b")?]; return Ok(arr[0] + arr[1] + arr[2]); }
  if (k === 1) { const arr = [pre, ok("a")?, bad()?, ok("c")?]; return Ok(arr[0]); }
  if (k === 2) { return Ok(`${pre}-${ok("x")?}-${bad()?}-${ok("y")?}`); }
  if (k === 3) { return Ok(two(ok("p")?, bad()?)); }
  if (k === 4) { return Ok((ok("q")?).slice(0, 1) + (bad()?).slice(0, 1)); }
  if (k === 5) { const v = (ok("m")? + "x") === "m!x" ? bad()? : "no"; return Ok(v); }
  return Ok(two(pre, ok("z")?));
}
function show(r: Result<string, string>): string { return r.kind === "ok" ? r.value : "E:" + r.error; }
export function main(log: Log): void {
  for (let k = 0; k < 7; k++) { log.write(2, `${show(run(k))} ${n}`); n = 0; }
}"#), &["pre0a!b! 2", "E:bad 2", "E:bad 2", "E:bad 2", "E:bad 2", "E:bad 2", "pre6z! 1"]);
}

#[test]
fn order_void_coercion_keeps_effect_and_frees() {
    check(&format!("{}", r#"
let n: i32 = 0;
function bump(): string { n += 1; return "s" + `${n}`; }
function call(f: () => void): void { f(); f(); }
export function main(log: Log): void {
  const f: () => void = () => bump();
  call(f);
  const g: () => void = () => { const x = bump(); };
  call(g);
  log.write(2, `${n}`);
  const h: (x: i32) => void = (x) => [bump(), bump()];
  h(1);
  log.write(2, `${n}`);
}"#), &["4", "6"]);
}

#[test]
fn order_interp_and_concat_side_effects() {
    check(&format!("{TR}{}", r#"
export function main(log: Log): void {
  const s = `${t(1)}-${t(2)}` + `${t(3)}`;
  log.write(2, `${s} ${n}`);
  const arr: i32[] = [t(4), t(5), t(6)];
  log.write(2, `${n} ${arr[0]}${arr[2]}`);
}"#), &["1-23 123", "123456 46"]);
}

// ---------------- control flow ----------------
probe!(cf_labelled_loops_owned_locals, r#"
export function main(log: Log): void {
  let out = "";
  outer: for (let i = 0; i < 4; i++) {
    const a = "a" + `${i}`;
    for (let j = 0; j < 4; j++) {
      const b = a + "b" + `${j}`;
      const arr = [a, b];
      if (j === 1) continue;
      if (i === 1) continue outer;
      if (i === 3) break outer;
      out += arr[1] + ",";
      if (j === 2) break;
    }
    out += "|";
  }
  log.write(2, out);
}
"#, ["a0b0,a0b2,|a2b0,a2b2,|"]);

probe!(cf_while_break_in_switch_in_loop, r#"
export function main(log: Log): void {
  let out = "";
  let i = 0;
  while (true) {
    const s = "s" + `${i}`;
    switch (i) {
      case 0: out += s; break;
      case 1: i += 1; continue;
      case 3: out += "end"; break;
      default: out += "d" + s;
    }
    if (i >= 3) break;
    i += 1;
  }
  log.write(2, out);
}
"#, ["s0ds2end"]);

#[test]
fn cf_switch_nonconst_labels_order() {
    check(&format!("{TR}{}", r#"
export function main(log: Log): void {
  const x = 2;
  let r = "";
  switch (x) {
    case t(1): r = "one"; break;
    case t(2): case t(3): r = "two"; break;
    default: r = "def";
  }
  log.write(2, `${r} ${n}`);
  n = 0;
  switch (9) { case t(4), t(5): r = "x"; break; default: r = "dd"; }
  log.write(2, `${r} ${n}`);
}"#), &["two 12", "dd 45"]);
}

probe!(cf_switch_expression_strings, r#"
function f(s: string): i32 { return switch (s) { case "a": 1, case "b", "c": 2, default: 3, }; }
export function main(log: Log): void { log.write(2, `${f("a")}${f("c")}${f("zz" + "")}${f("b" + "")}`); }
"#, ["1232"]);

probe!(cf_union_switch_exhaustive, r#"
type Shape = { kind: "circle"; r: i32 } | { kind: "rect"; w: i32; h: i32 } | { kind: "tri"; b: i32; h: i32 };
function area(s: Shape): i32 {
  switch (s.kind) {
    case "circle": return 3 * s.r * s.r;
    case "rect": return s.w * s.h;
    case "tri": return s.b * s.h / 2;
  }
}
export function main(log: Log): void {
  const xs: Shape[] = [{ kind: "circle", r: 2 }, { kind: "rect", w: 3, h: 4 }, { kind: "tri", b: 5, h: 6 }];
  let t = 0;
  for (const s of xs) t += area(s);
  log.write(2, `${t} ${area(xs[2])}`);
}
"#, ["39 15"]);

probe!(cf_do_while_continue_owned, r#"
export function main(log: Log): void {
  let i = 0; let acc = "";
  do { const s = "x" + `${i}`; i += 1; if (i % 2 === 0) continue; acc += s; } while (i < 6);
  log.write(2, acc);
}
"#, ["x0x2x4"]);

// ---------------- closures ----------------
probe!(cl_loop_var_capture, r#"
export function main(log: Log): void {
  const fs: (() => i32)[] = [];
  for (let i = 0; i < 3; i++) { fs.push(() => i); }
  let s = "";
  for (const f of fs) s += `${f()}`;
  log.write(2, s);
  const gs: (() => i32)[] = [];
  for (const k of [10, 20, 30]) { gs.push(() => k); }
  s = "";
  for (const g of gs) s += `${g()},`;
  log.write(2, s);
  const hs: (() => i32)[] = [];
  let j = 0;
  while (j < 3) { const c = j * 2; hs.push(() => c); j += 1; }
  s = "";
  for (const h of hs) s += `${h()},`;
  log.write(2, s);
}
"#, ["012", "10,20,30,", "0,2,4,"]);

probe!(cl_shared_cell_two_closures, r#"
export function main(log: Log): void {
  let n = 0; let s = "a" + "b";
  const inc = () => { n += 1; s += "x"; return n; };
  const get = () => s + `${n}`;
  inc(); inc();
  log.write(2, get());
  n = 10;
  log.write(2, get());
  const inner = () => { const f = () => { n += 100; }; f(); f(); return n; };
  log.write(2, `${inner()} ${n}`);
}
"#, ["abxx2", "abxx10", "210 210"]);

probe_ign!(cl_class_field_closure_capturing_self, r#"
class Node { name: string; cb: (() => string) | null = null; constructor(name: string) { this.name = name; } }
export function main(log: Log): void {
  const n = new Node("a" + "b");
  n.cb = () => n.name + "!";
  log.write(2, n.cb !== null ? "set" : "unset");
  const f = n.cb;
  if (f !== null) { log.write(2, f()); }
}
"#, ["set", "ab!"]);

probe_ign!(cl_recursion_via_closure_var, r#"
export function main(log: Log): void {
  let fib: (n: i32) => i32 = (n) => n;
  fib = (n) => n < 2 ? n : fib(n - 1) + fib(n - 2);
  log.write(2, `${fib(10)}`);
}
"#, ["55"]);

probe!(cl_closure_replaced_while_running, r#"
export function main(log: Log): void {
  let f: () => string = () => "x";
  const cap = "cap" + "t";
  f = () => { f = () => "second"; return cap + "1"; };
  log.write(2, f() + f());
}
"#, ["capt1second"]);

probe!(cl_array_of_closures_capturing_arrays, r#"
export function main(log: Log): void {
  const acc: string[] = [];
  const fs: ((s: string) => void)[] = [];
  for (let i = 0; i < 3; i++) { const tag = "t" + `${i}`; fs.push((s) => { acc.push(tag + s); }); }
  for (const f of fs) f("!");
  fs.pop();
  let o = ""; for (const a of acc) o += a + ","; log.write(2, o + `${fs.length}`);
}
"#, ["t0!,t1!,t2!,2"]);

// bisect of order_try_in_nested_positions: one `?` shape per test
fn try_case(k: &str) -> String {
    format!(r#"
let n: i32 = 0;
function ok(s: string): Result<string, string> {{ n += 1; return Ok(s + "!"); }}
function bad(): Result<string, string> {{ n += 1; return Err("bad"); }}
function two(a: string, b: string): string {{ return a + b; }}
function run(): Result<string, string> {{
  const pre = "pre" + "x";
  {k}
}}
export function main(log: Log): void {{ const r = run(); log.write(2, r.kind === "ok" ? r.value : "E:" + r.error); }}
"#)
}
#[test]
fn try_array_lit_second_fails() { check(&try_case(r#"const arr = [pre, ok("a")?, bad()?, ok("c")?]; return Ok(arr[0]);"#), &["E:bad"]); }
#[test]
fn try_template_second_fails() { check(&try_case(r#"return Ok(`${pre}-${ok("x")?}-${bad()?}-${ok("y")?}`);"#), &["E:bad"]); }
#[test]
fn try_call_args_second_fails() { check(&try_case(r#"return Ok(two(ok("p")?, bad()?));"#), &["E:bad"]); }
#[test]
fn try_method_receiver_second_fails() { check(&try_case(r#"return Ok((ok("q")?).slice(0, 1) + (bad()?).slice(0, 1));"#), &["E:bad"]); }
#[test]
fn try_cond_branch_fails() { check(&try_case(r#"const v = (ok("m")? + "x") === "m!x" ? bad()? : "no"; return Ok(v);"#), &["E:bad"]); }
#[test]
fn try_first_fails_with_prior_temp() { check(&try_case(r#"return Ok(two(pre + "y", bad()?));"#), &["E:bad"]); }
#[test]
fn try_tpl_a() { check(&try_case(r#"return Ok(`${pre}-${bad()?}`);"#), &["E:bad"]); }
#[test]
fn try_tpl_b() { check(&try_case(r#"return Ok(`a${ok("x")?}-${bad()?}`);"#), &["E:bad"]); }
#[test]
fn try_tpl_c() { check(&try_case(r#"return Ok(`${ok("x")?}${bad()?}`);"#), &["E:bad"]); }
#[test]
fn try_tpl_d() { check(&try_case(r#"return Ok(`lit-${bad()?}`);"#), &["E:bad"]); }
#[test]
fn try_tpl_e() { check(&try_case(r#"const s = `${pre}${bad()?}`; return Ok(s);"#), &["E:bad"]); }
#[test]
fn try_concat_a() { check(&try_case(r#"return Ok(pre + "-" + bad()?);"#), &["E:bad"]); }
#[test]
fn try_concat_b() { check(&try_case(r#"return Ok((pre + "-") + (ok("z")? + bad()?));"#), &["E:bad"]); }

fn try_case2(k: &str) -> String {
    format!(r#"
let n: i32 = 0;
struct P {{ a: string; b: string; }}
class K {{ a: string; b: string; constructor(a: string, b: string) {{ this.a = a; this.b = b; }} m(x: string, y: string): string {{ return this.a + x + y; }} }}
function ok(s: string): Result<string, string> {{ n += 1; return Ok(s + "!"); }}
function bad(): Result<string, string> {{ n += 1; return Err("bad"); }}
function two(a: string, b: string): string {{ return a + b; }}
function run(): Result<string, string> {{
  const pre = "pre" + "x";
  {k}
}}
export function main(log: Log): void {{ const r = run(); log.write(2, r.kind === "ok" ? r.value : "E:" + r.error); }}
"#)
}
#[test]
fn try2_objlit() { check(&try_case2(r#"const p: P = { a: ok("a")?, b: bad()? }; return Ok(p.a);"#), &["E:bad"]); }
#[test]
fn try2_new_args() { check(&try_case2(r#"const k = new K(ok("a")?, bad()?); return Ok(k.a);"#), &["E:bad"]); }
#[test]
fn try2_method_args() { check(&try_case2(r#"const k = new K("a" + "b", "c"); return Ok(k.m(ok("a")?, bad()?));"#), &["E:bad"]); }
#[test]
fn try2_nested_call() { check(&try_case2(r#"return Ok(two(two(ok("a")?, pre), bad()?));"#), &["E:bad"]); }
#[test]
fn try2_array_index() { check(&try_case2(r#"const xs: string[] = [pre, "b" + "c"]; return Ok(xs[0] + xs[bad()? === "" ? 0 : 1]);"#), &["E:bad"]); }
#[test]
fn try2_struct_in_array_lit() { check(&try_case2(r#"const ps: P[] = [{ a: pre, b: ok("z")? }, { a: ok("y")?, b: bad()? }]; return Ok(ps[0].a);"#), &["E:bad"]); }
#[test]
fn try2_closure_creation_then_fail() { check(&try_case2(r#"const f = () => pre + "!"; const g = [f, f]; const s = two(g[0](), bad()?); return Ok(s);"#), &["E:bad"]); }
#[test]
fn try2_in_loop_with_locals() { check(&try_case2(r#"let acc = ""; for (let i = 0; i < 3; i++) { const t = pre + `${i}`; acc += two(t, i === 2 ? bad()? : ok("k")?); } return Ok(acc);"#), &["E:bad"]); }
#[test]
fn try2_switch_scrutinee() { check(&try_case2(r#"switch (ok("s")? + pre) { case "x": return Ok("1"); default: return Ok(two(pre, bad()?)); }"#), &["E:bad"]); }
#[test]
fn try2_ternary_cond() { check(&try_case2(r#"return Ok((ok("c")? === "c!") ? two(pre, bad()?) : "no");"#), &["E:bad"]); }
#[test]
fn try2_field_assign_rhs() { check(&try_case2(r#"const k = new K("a" + "b", "c"); k.a = two(pre, bad()?); return Ok(k.a);"#), &["E:bad"]); }
#[test]
fn try2_compound_assign_rhs() { check(&try_case2(r#"const k = new K("a" + "b", "c"); k.a += bad()?; return Ok(k.a);"#), &["E:bad"]); }
#[test]
fn try2_array_set_rhs() { check(&try_case2(r#"const xs: string[] = [pre]; xs[0] = two(xs[0], bad()?); return Ok(xs[0]);"#), &["E:bad"]); }
#[test]
fn try2_push_arg() { check(&try_case2(r#"const xs: string[] = [pre]; xs.push(two(pre, bad()?)); return Ok(xs[0]);"#), &["E:bad"]); }
#[test]
fn try2_logical_rhs() { check(&try_case2(r#"const b = pre === "pre" || bad()? === ""; return Ok(b ? "t" : "f");"#), &["E:bad"]); }
#[test]
fn try2_nullcoalesce_rhs() { check(&try_case2(r#"const s: string | null = null; const t = s ?? bad()?; return Ok(t);"#), &["E:bad"]); }

// ---------------- known-attack catalogue: 2.4 mutate while iterating, 2.5 drop during use, 2.12 deep drop/recursion ----------------
probe!(ka24_push_while_for_of, r#"
export function main(log: Log): void {
  const xs: string[] = ["a" + "1", "b" + "2"];
  let n = 0;
  for (const x of xs) { n += 1; if (n < 5) { xs.push(x + "!"); } }
  log.write(2, `${n} ${xs.length} ${xs[xs.length - 1]}`);
}
"#, ["6 6 b2!!"]);
probe!(ka24_pop_while_for_of, r#"
export function main(log: Log): void {
  const xs: string[] = ["a" + "1", "b" + "2", "c" + "3", "d" + "4"];
  let s = "";
  for (const x of xs) { s += x; xs.pop(); }
  log.write(2, `${s} ${xs.length}`);
}
"#, ["a1b2 2"]);
probe!(ka24_clear_by_reassign_in_foreach, r#"
class H { xs: string[] = ["a" + "1", "b" + "2", "c" + "3"]; }
export function main(log: Log): void {
  const h = new H();
  let s = "";
  h.xs.forEach((x) => { s += x; h.xs = ["z"]; });
  log.write(2, `${s} ${h.xs.length}`);
}
"#, ["a1b2c3 1"]);
probe!(ka24_map_callback_mutates_source, r#"
export function main(log: Log): void {
  const xs: string[] = ["a" + "1", "b" + "2", "c" + "3"];
  const ys = xs.map((x) => { xs[0] = "Q" + "Q"; return x + "'"; });
  log.write(2, `${ys.length} ${ys[0]}${ys[1]}${ys[2]} ${xs.length}`);
}
"#, ["3 a1'b2'c3' 3"]);
probe!(ka25_closure_clears_own_slot, r#"
class Slot { f: (() => string) | null = null; }
export function main(log: Log): void {
  const s = new Slot();
  const cap = "cap" + "t";
  s.f = () => { s.f = null; return cap + "!"; };
  const g = s.f;
  if (g !== null) { log.write(2, g()); }
  log.write(2, s.f === null ? "cleared" : "kept");
}
"#, ["capt!", "cleared"]);
probe!(ka25_closure_overwrites_own_field_while_running, r#"
class Slot { f: () => string; constructor() { this.f = () => "init"; } }
export function main(log: Log): void {
  const s = new Slot();
  const cap = "cap" + "t";
  s.f = () => { s.f = () => "second"; return cap + "!" + s.f(); };
  log.write(2, s.f());
  log.write(2, s.f());
}
"#, ["capt!second", "second"]);
probe!(ka212_deep_list_drop_3k, r#"
class N { v: i32; next: N | null; constructor(v: i32, next: N | null) { this.v = v; this.next = next; } }
export function main(log: Log): void {
  let head: N | null = null;
  for (let i = 0; i < 3000; i++) { head = new N(i, head); }
  let t = 0; let c = head;
  while (c !== null) { t += c.v % 7; c = c.next; }
  log.write(2, `${t}`);
}
"#, ["8994"]);
#[test]
fn ka212_infinite_recursion_traps_cleanly() {
    let mut p = Prog::new(&format!("{HDR}{}", r#"
function f(n: i32): i32 { return f(n + 1) + 1; }
export function go(): i32 { return f(0); }
export function main(log: Log): void { log.write(2, "up"); }
"#));
    let e = p.call_err("go", &[]);
    eprintln!("infinite recursion -> {e}");
}
probe!(ka26_generic_ref_vs_scalar_layout, r#"
struct Box<T> { v: T; }
function swap<T>(a: Box<T>, b: Box<T>): Box<T> { const t = a; return b; }
function dup<T>(x: T): T[] { const r: T[] = []; r.push(x); r.push(x); return r; }
export function main(log: Log): void {
  const a: Box<i32> = { v: 7 }; const b: Box<i32> = { v: 1 };
  const s1: Box<string> = { v: "x" + "y" }; const s2: Box<string> = { v: "z" + "w" };
  const c1: Box<Box<string>> = { v: s1 }; const c2: Box<Box<string>> = { v: s2 };
  log.write(2, `${swap(a, b).v} ${swap(s1, s2).v} ${swap(c1, c2).v.v} ${dup(5).length} ${dup("q" + "r")[1]} ${dup(s1)[0].v}`);
}
"#, ["1 zw zw 2 qr xy"]);

// ---------------- W0700: exported function with an ABI-unfit signature stays internal ----------------
#[test]
fn w0700_is_visible_and_export_absent() {
    let src = format!("{HDR}{}", r#"
struct P { x: i32; }
export function mk(x: i32): P { return { x: x }; }
export function arr(xs: i32[]): i32 { return xs.length; }
export function okf(x: i32): i32 { return x + 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#);
    let out = compile_full(&src);
    eprintln!("notes: {:?}", out.notes);
    let mut p = Prog::from_wasm(out.wasm.clone());
    for name in ["mk", "arr"] {
        let r = p.rt.call_export(name, &[]);
        eprintln!("export {name}: {:?}", r.as_ref().map(|_| ()).map_err(|e| e.to_string()));
        assert!(r.is_err(), "{name} must not be callable");
    }
    let joined = format!("{:?}", out.notes);
    for name in ["mk", "arr"] {
        assert!(joined.contains("W0700") && joined.contains(name), "no visible W0700 note for `{name}`: {joined}");
    }
    assert_eq!(p.call("okf", &[oxiscript_runtime::Value::I32(1)]), vec![oxiscript_runtime::Value::I32(2)]);
}

// ---------------- advisor round: mutating-method aliasing, multi-hop borrowed chains ----------------
probe!(alias_mut_method_self_as_arg_local, r#"
struct V { x: i32; mutating addTwice(o: V): void { this.x += o.x; this.x += o.x; } }
export function main(log: Log): void { let v: V = { x: 1 }; v.addTwice(v); log.write(2, `${v.x}`); }
"#, ["3"]);
probe!(alias_mut_method_self_as_arg_array_elem, r#"
struct V { x: i32; mutating addTwice(o: V): void { this.x += o.x; this.x += o.x; } }
export function main(log: Log): void { const vs: V[] = [{ x: 1 }]; vs[0].addTwice(vs[0]); log.write(2, `${vs[0].x}`); }
"#, ["3"]);
probe!(alias_mut_method_self_as_arg_class_field, r#"
struct V { x: i32; mutating addTwice(o: V): void { this.x += o.x; this.x += o.x; } }
class HV { v: V = { x: 1 }; }
export function main(log: Log): void { const h = new HV(); h.v.addTwice(h.v); log.write(2, `${h.v.x}`); }
"#, ["3"]);
probe!(alias_mut_method_self_as_arg_global, r#"
struct V { x: i32; mutating addTwice(o: V): void { this.x += o.x; this.x += o.x; } }
let G: V = { x: 1 };
export function main(log: Log): void { G.addTwice(G); log.write(2, `${G.x}`); }
"#, ["3"]);
probe!(alias_mut_method_copy_then_self, r#"
struct V { x: i32; mutating addTwice(o: V): void { this.x += o.x; this.x += o.x; } }
export function main(log: Log): void { let v: V = { x: 1 }; const w = v; v.addTwice(w); log.write(2, `${v.x} ${w.x}`); }
"#, ["3 1"]);
probe!(alias_param_copy_mutated, r#"
struct V { x: i32; }
function f(p: V): i32 { let q = p; q.x = 50; return q.x; }
export function main(log: Log): void { const v: V = { x: 1 }; const r = f(v); log.write(2, `${v.x} ${r}`); }
"#, ["1 50"]);
probe!(chain2_class_field_chain_callee_replaces_link, r#"
struct M { tag: string; }
class H { m: M; constructor() { this.m = { tag: "t" + "1" }; } }
function g(t: string, h: H): string { h.m = { tag: "z" + "z" }; const junk = ["q" + "q", "r" + "r"]; return t + "!" + junk[1]; }
export function main(log: Log): void { const h = new H(); log.write(2, g(h.m.tag, h)); log.write(2, h.m.tag); }
"#, ["t1!rr", "zz"]);
probe!(chain2_array_elem_field_callee_replaces_and_pops, r#"
struct P { s: string; }
function k(t: string, xs: P[]): string { xs[0] = { s: "n" + "w" }; xs.pop(); const junk = ["x" + "x", "y" + "y"]; return t + "!" + junk[0]; }
export function main(log: Log): void { const xs: P[] = [{ s: "o" + "ld" }]; log.write(2, k(xs[0].s, xs)); log.write(2, `${xs.length}`); }
"#, ["old!xx", "0"]);
probe!(chain3_deep_chain_arg, r#"
struct In { s: string; }
struct Mid { i: In; }
class Top { m: Mid; constructor() { this.m = { i: { s: "d" + "eep" } }; } }
function g(t: string, o: Top): string { o.m = { i: { s: "n" + "ew" } }; const junk = ["q" + "q", "r" + "r", "s" + "s"]; return t + junk[2]; }
export function main(log: Log): void { const o = new Top(); log.write(2, g(o.m.i.s, o)); log.write(2, o.m.i.s); }
"#, ["deepss", "new"]);
probe!(chain2_method_receiver_chain_replaced, r#"
struct M { tag: string; }
class H { m: M; constructor() { this.m = { tag: "t" + "1" }; } swap(): string { this.m = { tag: "z" + "z" }; const j = ["a" + "a", "b" + "b"]; return j[0]; } }
function len2(a: string, b: string): string { return a + b; }
export function main(log: Log): void { const h = new H(); log.write(2, len2(h.m.tag, h.swap()) + h.m.tag); }
"#, ["t1aazz"]);
probe!(leak_mut_method_local_with_other_struct_arg, r#"
struct V { x: i32; mutating add(o: V): void { this.x += o.x; } }
export function main(log: Log): void { let v: V = { x: 1 }; let w: V = { x: 5 }; v.add(w); log.write(2, `${v.x} ${w.x}`); }
"#, ["6 5"]);
probe!(leak_mut_method_local_no_struct_arg, r#"
struct V { x: i32; mutating add(k: i32): void { this.x += k; } }
export function main(log: Log): void { let v: V = { x: 1 }; v.add(5); const w = v; v.add(1); log.write(2, `${v.x} ${w.x}`); }
"#, ["7 6"]);
probe!(leak_mut_method_local_alias_arg_scalar_use, r#"
struct V { x: i32; mutating add(o: V): void { this.x += o.x; } }
export function main(log: Log): void { let v: V = { x: 1 }; v.add(v); log.write(2, `${v.x}`); }
"#, ["2"]);
probe!(leak_mut_method_param_receiver_alias_arg, r#"
struct V { x: i32; mutating add(o: V): void { this.x += o.x; } }
function f(p: V): i32 { let q = p; q.add(p); return q.x; }
export function main(log: Log): void { const v: V = { x: 2 }; log.write(2, `${f(v)} ${v.x}`); }
"#, ["4 2"]);
probe!(leak_mut_method_str_field_struct_alias_arg, r#"
struct V { s: string; mutating app(o: V): void { this.s += o.s; } }
export function main(log: Log): void { let v: V = { s: "a" + "b" }; v.app(v); log.write(2, v.s); let w: V = { s: "c" + "d" }; const u = w; w.app(u); log.write(2, w.s + u.s); }
"#, ["abab", "cdcdcd"]);

// L03 breadth: null-narrowed local used in other nullable contexts
probe!(l03_null_local_to_nullable_param, r#"
function f(s: string | null): i32 { return s === null ? -1 : s.byteLength; }
export function main(log: Log): void { const s: string | null = null; log.write(2, `${f(s)}`); }
"#, ["-1"]);
probe!(l03_null_local_returned, r#"
function g(): string | null { const s: string | null = null; return s; }
export function main(log: Log): void { log.write(2, `${(g() ?? "none")}`); }
"#, ["none"]);
probe!(l03_null_local_stored_in_field, r#"
class C { s: string | null = null; }
export function main(log: Log): void { const s: string | null = null; const c = new C(); c.s = s; log.write(2, `${(c.s ?? "none")}`); }
"#, ["none"]);
probe!(l03_null_local_in_array, r#"
export function main(log: Log): void { const s: string | null = null; const xs: (string | null)[] = [s, "a" + "b"]; log.write(2, `${(xs[0] ?? "none")}${(xs[1] ?? "none")}`); }
"#, ["noneab"]);
probe!(l03_null_local_compare_eq, r#"
export function main(log: Log): void { const s: string | null = null; log.write(2, s === null ? "null" : "set"); }
"#, ["null"]);
probe!(l03_let_null_then_assigned_in_loop, r#"
export function main(log: Log): void {
  let best: string | null = null;
  for (const w of ["aa", "b", "cccc"]) { if (best === null || w.byteLength > best.byteLength) { best = w; } }
  log.write(2, best ?? "none");
}
"#, ["cccc"]);
probe!(l03_let_null_used_before_assignment, r#"
export function main(log: Log): void {
  let x: string | null = null;
  log.write(2, x === null ? "null" : "set");
  x = "a" + "b";
  log.write(2, x);
}
"#, ["null", "ab"]);

#[test]
fn ka212_deep_list_threshold_probe() {
    for n in [200, 500, 1000, 1500, 2000, 2500] {
        let src = format!("{HDR}{}", format!(r#"
class N {{ v: i32; next: N | null; constructor(v: i32, next: N | null) {{ this.v = v; this.next = next; }} }}
export function main(log: Log): void {{
  let head: N | null = null;
  for (let i = 0; i < {n}; i++) {{ head = new N(i, head); }}
  log.write(2, "built");
}}
"#));
        let r = run_probe(&src);
        eprintln!("list of {n}: {}", match &r { Ok(o) => format!("ok live={}", o.live), Err(e) => e.chars().take(80).collect() });
    }
}
