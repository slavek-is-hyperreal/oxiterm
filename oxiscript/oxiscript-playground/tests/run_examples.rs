//! Every site example (except the deliberately broken `mistakes.oxs`) compiles and runs natively in the
//! same runtime the browser worker uses, and prints what the page promises. Also the trap and limit paths.

use oxiscript_playground::run::{compile, run_json, run_wasm, DEFAULT_FUEL, MAX_LINES};
use serde_json::Value;
use std::path::PathBuf;

fn example(name: &str) -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples").join(format!("{name}.oxs"));
    std::fs::read_to_string(p).unwrap()
}

fn lines(src: &str) -> Vec<String> {
    let wasm = compile(src).unwrap_or_else(|e| panic!("compile: {e}"));
    let r = run_wasm(&wasm, DEFAULT_FUEL);
    assert_eq!(r.stop, None, "stopped: {:?}", r.stop);
    r.lines.into_iter().map(|l| l.msg).collect()
}

fn main_only(body: &str) -> String {
    format!("import {{ Log }} from \"host:log\";\nexport function main(log: Log): void {{\n{body}\n}}\n")
}

fn stop_of(src: &str, fuel: u64) -> (String, Vec<String>) {
    let v: Value = serde_json::from_str(&run_json(src, fuel)).unwrap();
    assert_eq!(v["ok"], true, "{v}");
    let reason = v["stop"]["reason"].as_str().unwrap_or("").to_string();
    let lines = v["lines"].as_array().unwrap().iter().map(|l| l["msg"].as_str().unwrap().to_string()).collect();
    (reason, lines)
}

#[test]
fn every_example_runs_with_expected_output() {
    let want: &[(&str, &[&str])] = &[
        ("hello", &["Hello from OxiScript"]),
        ("counter", &["counter = 1", "counter = 2"]),
        ("errors", &["port ok", "port out of range"]),
        ("laws", &["add(2, 3) = 5", "add(3, 2) = 5"]),
        ("shapes", &["circle area x100 = 1256", "rect area x100 = 1350"]),
        ("structs", &["walk(4) x10 = 80"]),
    ];
    for (name, expect) in want {
        assert_eq!(lines(&example(name)), *expect, "{name}");
    }
    // nothing was forgotten: every example file is either listed or the known failing one
    let n = std::fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples"))
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "oxs"))
        .count();
    assert_eq!(n, want.len() + 1, "a new example needs an entry here");
}

#[test]
fn broken_example_reports_check_diagnostics() {
    let v: Value = serde_json::from_str(&run_json(&example("mistakes"), DEFAULT_FUEL)).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["stage"], "check");
    assert_eq!(v["diagnostics"][0]["code"], "OXS-E0005");
}

#[test]
fn infinite_loop_runs_out_of_fuel() {
    let (reason, _) = stop_of(&main_only("let i: i32 = 0;\nwhile (true) { i = i +% 1; }"), 100_000);
    assert_eq!(reason, "out_of_fuel");
}

#[test]
fn overflow_and_recursion_are_named() {
    let (r, l) = stop_of(&main_only("const k = [1];\nlog.write(2, \"before\");\nconst i: i32 = 2147483647 + k[0];\nlog.write(2, `${i}`);"), DEFAULT_FUEL);
    assert_eq!((r.as_str(), l), ("overflow", vec!["before".to_string()]));
    let src = "import { Log } from \"host:log\";\nfunction f(n: i32): i32 { return f(n + 1) + 1; }\nexport function main(log: Log): void { log.write(2, `${f(0)}`); }\n";
    assert_eq!(stop_of(src, DEFAULT_FUEL).0, "stack_overflow");
}

#[test]
fn array_bounds_trap() {
    let (r, _) = stop_of(&main_only("const a: i32[] = [1, 2];\nlet i: i32 = 5;\nlog.write(2, `${a[i]}`);"), DEFAULT_FUEL);
    assert_eq!(r, "bounds");
}

#[test]
fn f64_to_text_works() {
    assert_eq!(
        lines(&main_only(
            "const x = 1.5;\nconst z = 0.0;\nlog.write(2, `${x} ${(1.0).toString()} ${(0.1).toString()} ${(-0.0).toString()} ${(z / z).toString()} ${(1.0 / z).toString()} ${(-1.0 / z).toString()}`);"
        )),
        ["1.5 1 0.1 -0 NaN Infinity -Infinity"]
    );
    // float arithmetic and conversion to an integer work
    assert_eq!(lines(&main_only("const x = 1.5;\nlog.write(2, `${(x * 4.0) as i32}`);")), ["6"]);
}

#[test]
fn output_is_capped_and_counted() {
    let wasm = compile(&main_only("let i: i32 = 0;\nwhile (i < 1500) { log.write(2, \"x\"); i++; }")).unwrap();
    let r = run_wasm(&wasm, 100_000_000);
    assert_eq!(r.lines.len(), MAX_LINES);
    assert_eq!(r.dropped, 500);
    assert_eq!(r.stop, None);
}

#[test]
fn state_capability_works_and_is_per_run() {
    let src = example("counter");
    assert_eq!(lines(&src), ["counter = 1", "counter = 2"]);
    assert_eq!(lines(&src), ["counter = 1", "counter = 2"], "state does not leak between runs");
}

#[test]
fn a_module_without_main_says_nothing_ran() {
    let v: Value = serde_json::from_str(&run_json(
        "export function same(a: i32, b: i32): bool {\n  return a === b;\n}\n",
        DEFAULT_FUEL,
    ))
    .unwrap();
    assert_eq!(v["ok"], Value::Bool(true));
    assert_eq!(v["noMain"], Value::Bool(true));
    assert_eq!(v["exported"], serde_json::json!(["same"]));
    let hello: Value = serde_json::from_str(&run_json(
        "import { Log } from \"host:log\";\nexport function main(log: Log): void {\n  log.write(2, \"hi\");\n}\n",
        DEFAULT_FUEL,
    ))
    .unwrap();
    assert_eq!(hello["noMain"], Value::Bool(false));
}
