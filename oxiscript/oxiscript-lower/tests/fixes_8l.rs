#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! Regression tests for the BAD ACTOR 8.L fixes (L01, L02, L03, L06, L08, L10) and the coverage the attack did not reach.
mod common;
use common::*;

const HDR: &str = "import { Log } from \"host:log\";\n";

fn run(body: &str, want: &[&str]) {
    run_log(&format!("{HDR}{body}"), want);
}

// ---- L02: per-iteration binding of a C-style `for (let ...)` (JS CreatePerIterationEnvironment) ----
#[test]
fn l02_plain_capture() {
    run(r#"export function main(log: Log): void {
  const fs: (() => i32)[] = [];
  for (let i = 0; i < 3; i++) { fs.push(() => i); }
  let s = ""; for (const f of fs) s += `${f()}`; log.write(2, s);
}"#, &["012"]);
}
#[test]
fn l02_body_mutation_carries_to_next_iteration() {
    // the body bumps i: the next iteration's binding starts from the (mutated) value, and the captured
    // closure of an earlier iteration sees the value of ITS iteration after the body ran, not the update.
    run(r#"export function main(log: Log): void {
  const fs: (() => i32)[] = [];
  for (let i = 0; i < 6; i++) { fs.push(() => i); if (i % 2 === 0) { i += 1; } }
  let s = ""; for (const f of fs) s += `${f()},`; log.write(2, s);
}"#, &["1,3,5,"]);
}
#[test]
fn l02_closure_mutates_its_iteration_binding() {
    run(r#"export function main(log: Log): void {
  const fs: (() => i32)[] = [];
  for (let i = 0; i < 3; i++) { fs.push(() => { i += 10; return i; }); }
  let s = ""; for (const f of fs) s += `${f()},`; for (const f of fs) s += `${f()},`;
  log.write(2, s);
}"#, &["10,11,12,20,21,22,"]);
}
#[test]
fn l02_managed_counter_continue_break() {
    run(r#"export function main(log: Log): void {
  const fs: (() => string)[] = [];
  for (let s = "a"; s.byteLength < 8; s += "b") {
    if (s.byteLength === 2) { continue; }
    fs.push(() => s);
    if (s.byteLength === 5) { break; }
  }
  let out = ""; for (const f of fs) out += `${f()},`; log.write(2, out);
}"#, &["a,abb,abbb,abbbb,"]);
}
#[test]
fn l02_nested_and_two_vars() {
    run(r#"export function main(log: Log): void {
  const fs: (() => i32)[] = [];
  let j = 10;
  for (let i = 0; i < 2; i++) {
    j += 5;
    for (let k = 0; k < 2; k++) { fs.push(() => i * 100 + j + k); }
  }
  let s = ""; for (const f of fs) s += `${f()},`; log.write(2, s);
}"#, &["20,21,120,121,"]);
}
#[test]
fn l02_return_inside_loop_no_leak() {
    run(r#"function find(): (() => i32) | null {
  for (let i = 0; i < 10; i++) { const f = () => i; if (i === 4) { return f; } }
  return null;
}
export function main(log: Log): void { const f = find(); log.write(2, f === null ? "none" : `${f()}`); }"#, &["4"]);
}
#[test]
fn l02_uncaptured_loops_unchanged() {
    run(r#"export function main(log: Log): void {
  let t = 0; for (let i = 0; i < 100; i++) { t += i; } log.write(2, `${t}`);
}"#, &["4950"]);
}

// ---- L03: a nullable local narrowed to exactly `null` ----
#[test]
fn l03_more_contexts() {
    run(r#"class C { s: string | null; constructor(s: string | null) { this.s = s; } }
function id(s: string | null): string | null { return s; }
export function main(log: Log): void {
  const s: string | null = null;
  const t = s ?? "dflt";
  const c = new C(s);
  const xs: (string | null)[] = [];
  xs.push(s);
  const n: i32 | null = null;
  log.write(2, `${t} ${(c.s ?? "cnull")} ${xs.length} ${(id(s) ?? "inull")} ${(n ?? 7)} ${(s === null)} ${(s !== null)}`);
}"#, &["dflt cnull 1 inull 7 true false"]);
}

// ---- L10: mutating method with the receiver aliased as an argument ----
#[test]
fn l10_variants() {
    run(r#"struct V { x: i32; s: string; mutating add(o: V): void { this.x += o.x; this.s += o.s; } mutating add2(o: V, p: V): i32 { this.x += o.x + p.x; return this.x; } }
export function main(log: Log): void {
  let v: V = { x: 2, s: "a" + "b" };
  v.add(v);
  v.add2(v, v);
  const w = v;
  v.add(w);
  log.write(2, `${v.x} ${v.s} ${w.x} ${w.s}`);
}"#, &["24 abababab 12 abab"]);
}

// ---- L01: template accumulator across `?` ----
#[test]
fn l01_template_shapes() {
    run(r#"function bad(): Result<string, string> { return Err("bad"); }
function ok(): Result<string, string> { return Ok("ok" + "!"); }
function a(): Result<string, string> { const p = "p" + "x"; return Ok(`${p}-${bad()?}-${p}`); }
function b(): Result<string, string> { const p = "p" + "x"; return Ok(`${p}-${ok()?}-${bad()?}`); }
function c(): Result<string, string> { return Ok(`${"x" + "y"}${ok()?}${"z" + "w"}${ok()?}`); }
export function main(log: Log): void {
  const ra = a(); const rb = b(); const rc = c();
  log.write(2, `${ra.kind === "ok" ? ra.value : ra.error} ${rb.kind === "ok" ? rb.value : rb.error} ${rc.kind === "ok" ? rc.value : rc.error}`);
}"#, &["bad bad xyok!zwok!"]);
}

// ---- L08: unfit exports are errors in 8.T ----
#[test]
fn l08_checker_message() {
    let src = format!("{HDR}type U = {{ kind: \"a\" }} | {{ kind: \"b\" }};\n@host export function area(u: U): f64 {{ return 1.0; }}\nexport function main(log: Log): void {{ }}\n");
    let e = oxiscript_lower::compile_source(&src, &interfaces(), &oxiscript_lower::Options::default()).expect_err("must be rejected");
    let t = e.to_string();
    assert!(t.contains("OXS-E0121") && t.contains("area") && t.contains("ABI 1.0"), "{t}");
}

// ---- L06: the Stub runtime's free is iterative ----
fn run_big(body: &str, want: &[&str]) {
    let wasm = compile(&format!("{HDR}{body}"));
    let l = oxiscript_runtime::Limits { fuel_per_call: u64::MAX / 4, ..oxiscript_runtime::Limits::default() };
    let mut p = Prog::from_wasm_limits(wasm, l);
    assert_eq!(p.log(), want.iter().map(|s| s.to_string()).collect::<Vec<_>>());
    p.finish();
}
#[test]
fn l06_deep_list_100k() {
    run_big(r#"class N { v: i32; next: N | null; constructor(v: i32, next: N | null) { this.v = v; this.next = next; } }
export function main(log: Log): void {
  let head: N | null = null;
  for (let i = 0; i < 100000; i++) { head = new N(i, head); }
  log.write(2, `${head === null ? 0 : head.v}`);
}"#, &["99999"]);
}
#[test]
fn l06_deep_via_arrays_and_strings() {
    // each node owns an array whose single element is the next node, plus a string: array element and field paths
    run_big(r#"class N { s: string; kids: N[]; constructor(s: string, kids: N[]) { this.s = s; this.kids = kids; } }
export function main(log: Log): void {
  let cur = new N("leaf" + "!", []);
  for (let i = 0; i < 20000; i++) { const k: N[] = []; k.push(cur); cur = new N("n" + "x", k); }
  log.write(2, cur.s);
}"#, &["nx"]);
}

// ---- L03 extra shapes: must never panic the compiler ----
fn probe_l03(body: &str) -> String {
    let src = format!("{HDR}{body}");
    match std::panic::catch_unwind(|| oxiscript_lower::compile_source(&src, &interfaces(), &oxiscript_lower::Options::default())) {
        Ok(Ok(_)) => "OK".to_string(),
        Ok(Err(e)) => format!("REJECT {e}"),
        Err(_) => "PANIC".to_string(),
    }
}
#[test]
fn l03_no_compiler_panic_on_null_narrowed_shapes() {
    let cases = [
        "export function main(log: Log): void { const s: string | null = null; log.write(2, `${s}`); }",
        "export function main(log: Log): void { const s: string | null = null; const t = s; log.write(2, t === null ? \"n\" : \"s\"); }",
        "export function main(log: Log): void { const s: string | null = null; const xs = [s]; log.write(2, `${xs.length}`); }",
        "export function main(log: Log): void { const s: string | null = null; const c = log !== null; const t = c ? s : \"a\"; log.write(2, t ?? \"z\"); }",
        "let g: string | null = null;\nexport function main(log: Log): void { if (g === null) { log.write(2, g ?? \"z\"); } }",
        "export function main(log: Log): void { const s: string | null = null; const f = () => s; log.write(2, f() ?? \"z\"); }",
        "export function main(log: Log): void { const s: string | null = null; const n = s?.byteLength; log.write(2, `${n ?? 7}`); }",
        "export function main(log: Log): void { let s: string | null = null; s = s; log.write(2, s ?? \"z\"); }",
        "export function main(log: Log): void { const s: string | null = null; const r: Result<string | null, string> = Ok(s); log.write(2, r.kind); }",
        "export function main(log: Log): void { const n: i32 | null = null; const m: i32 | null = n; log.write(2, `${m ?? 1}`); }",
    ];
    let mut bad = Vec::new();
    for c in cases {
        let r = probe_l03(c);
        eprintln!("{r:.120} <= {c}");
        if r == "PANIC" {
            bad.push(c);
        }
    }
    assert!(bad.is_empty(), "compiler panics: {bad:#?}");
}

// L06 budget isolation: with the DEFAULT fuel, build in one export and drop in another (fresh fuel per entry).
#[test]
fn l06_drop_alone_under_default_fuel() {
    let src = format!("{HDR}{}", r#"class N { v: i32; next: N | null; constructor(v: i32, next: N | null) { this.v = v; this.next = next; } }
let head: N | null = null;
@host export function build(n: i32): i32 { for (let i = 0; i < n; i++) { head = new N(i, head); } return n; }
@host export function drop(): i32 { head = null; return 1; }
export function main(log: Log): void { log.write(2, "up"); }
"#);
    let mut p = Prog::new(&src);
    // build in chunks (each entry gets fresh fuel), then drop everything in ONE entry
    let mut total = 0;
    for chunk in [20000, 20000, 20000, 20000, 20000, 20000] {
        p.rt.call_export("build", &[oxiscript_runtime::Value::I32(chunk)]).unwrap();
        total += chunk;
    }
    let d = p.rt.call_export("drop", &[]);
    eprintln!("drop of {total} nodes under default fuel: {:?}", d.as_ref().map(|_| ()).map_err(|e| e.to_string()));
    // iterative free removes the stack trap; whether the default per-entry fuel suffices is a budget question
    if let Err(e) = &d {
        assert!(e.to_string().contains("OutOfFuel"), "only a fuel limit is acceptable, got {e}");
    } else {
        p.finish();
    }
}
