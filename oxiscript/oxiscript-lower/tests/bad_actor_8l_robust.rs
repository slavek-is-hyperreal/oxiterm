#![allow(clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.L robustness: hostile shapes must neither panic, abort (stack), nor take unbounded time.
mod common;
use common::*;
use std::time::Instant;

fn go(name: &str, body: String, want_log: Option<&str>) {
    let src = format!("import {{ Log }} from \"host:log\";\n{body}");
    let t = Instant::now();
    let r = std::panic::catch_unwind(|| oxiscript_lower::compile_source(&src, &interfaces(), &oxiscript_lower::Options::default()));
    let dt = t.elapsed();
    eprintln!("{name}: {} bytes src, {:?}", src.len(), dt);
    assert!(dt.as_secs() < 20, "{name}: too slow {dt:?}");
    match r {
        Err(_) => panic!("{name}: lowering PANICKED"),
        Ok(Err(e)) => {
            let s = e.to_string();
            assert!(!s.contains("E0702"), "{name}: internal bug E0702: {}", &s[..s.len().min(400)]);
            eprintln!("{name}: rejected cleanly: {}", &s[..s.len().min(160)]);
            assert!(want_log.is_none(), "{name}: unexpectedly rejected: {}", &s[..s.len().min(400)]);
        }
        Ok(Ok(wasm)) => {
            if let Some(w) = want_log {
                let p = Prog::from_wasm(wasm);
                assert_eq!(p.log(), vec![w.to_string()], "{name}");
            }
        }
    }
}

#[test]
fn nested_parens_2k() {
    let n = 50;
    go("parens", format!("export function main(log: Log): void {{ const x = {}1{}; log.write(2, `${{x}}`); }}", "(".repeat(n), ")".repeat(n)), Some("1"));
}
#[test]
fn nested_ifs_500() {
    let n = 12;
    go("ifs", format!("export function main(log: Log): void {{ let c = 0; {} c += 1; {} log.write(2, `${{c}}`); }}", "if (c === 0) {".repeat(n), "}".repeat(n)), Some("1"));
}
#[test]
fn else_if_chain_5k() {
    let mut s = String::from("function f(x: i32): i32 {\n if (x === 0) { return 0; }\n");
    for i in 1..900 { s.push_str(&format!(" else if (x === {i}) {{ return {i}; }}\n")); }
    s.push_str(" else { return -1; }\n}\nexport function main(log: Log): void { log.write(2, `${f(899)} ${f(77)} ${f(9999)}`); }\n");
    go("else-if", s, Some("899 77 -1"));
}
#[test]
fn ten_k_statements() {
    let mut s = String::from("export function main(log: Log): void {\n let a = 0;\n");
    for i in 0..4000 { s.push_str(&format!(" a += {};\n", i % 7)); }
    s.push_str(" log.write(2, `${a}`);\n}\n");
    let want: i32 = (0..4000).map(|i| i % 7).sum();
    go("10k-stmts", s, Some(&want.to_string()));
}
#[test]
fn many_locals_70k() {
    let mut s = String::from("export function main(log: Log): void {\n");
    for i in 0..70000 { s.push_str(&format!(" const v{i} = {i};\n")); }
    s.push_str(" log.write(2, `${v1 + v69999}`);\n}\n");
    go("70k-locals", s, None);
}
#[test]
fn many_string_locals_5k_leak_free() {
    let mut s = String::from("export function main(log: Log): void {\n let t = 0;\n");
    for i in 0..5000 { s.push_str(&format!(" const v{i} = \"s\" + `${{t}}`; t += v{i}.byteLength;\n")); }
    s.push_str(" log.write(2, `${t}`);\n}\n");
    go("5k-str-locals", s, None);
}
#[test]
fn long_binary_chain_20k() {
    let mut e = String::from("1");
    for _ in 0..900 { e.push_str(" + 1"); }
    go("chain", format!("export function main(log: Log): void {{ const x = {e}; log.write(2, `${{x}}`); }}"), Some("901"));
}
#[test]
fn long_concat_chain_5k() {
    let mut e = String::from("\"a\"");
    for _ in 0..900 { e.push_str(" + \"b\""); }
    go("concat-chain", format!("export function main(log: Log): void {{ const x = {e}; log.write(2, `${{x.byteLength}}`); }}"), Some("901"));
}
#[test]
fn nested_array_literals_500() {
    let n = 50;
    let ty = format!("{}i32{}", "", "[]".repeat(n));
    go("nested-arrays", format!("export function main(log: Log): void {{ const x: {ty} = {}{}; log.write(2, \"ok\"); }}", "[".repeat(n), "]".repeat(n)), None);
}
#[test]
fn nested_closures_300() {
    let n = 50;
    let mut s = String::from("export function main(log: Log): void { let c = 0;\n const f0 = () => {\n");
    for i in 1..n { s.push_str(&format!(" c += {i}; const f{i} = () => {{\n")); }
    s.push_str(" c += 1;\n");
    for _ in 1..n { s.push_str(" };\n"); }
    s.push_str(" };\n f0(); log.write(2, `${c}`); }\n");
    go("nested-closures", s, None);
}
#[test]
fn nested_calls_1000() {
    let n = 50;
    let s = format!("function id(x: i32): i32 {{ return x; }}\nexport function main(log: Log): void {{ const x = {}1{}; log.write(2, `${{x}}`); }}", "id(".repeat(n), ")".repeat(n));
    go("nested-calls", s, Some("1"));
}
#[test]
fn nested_ternary_1000() {
    let n = 900;
    let s = format!("export function main(log: Log): void {{ const c = true; const x = {}0{}; log.write(2, `${{x}}`); }}", "c ? 1 : ".repeat(n), "");
    go("nested-ternary", s, None);
}
#[test]
fn deep_struct_nesting_200() {
    let mut s = String::new();
    s.push_str("struct S0 { v: i32; }\n");
    for i in 1..200 { s.push_str(&format!("struct S{i} {{ c: S{}; }}\n", i - 1)); }
    s.push_str("export function main(log: Log): void { let a: S199 = ");
    let mut lit = String::from("{ v: 1 }");
    for _ in 1..200 { lit = format!("{{ c: {lit} }}"); }
    s.push_str(&lit);
    s.push_str("; const b = a; a.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.v = 5; log.write(2, `${a.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.v} ${b.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.c.v}`); }\n");
    go("deep-struct", s, None);
}
/// Generic fan-out: f<T> calls f<Box<T>> up to a bounded depth via a chain of distinct functions, 6000 distinct instantiations.
#[test]
fn generic_fanout_over_budget() {
    let mut s = String::from("struct Box<T> { v: T; }\nfunction id<T>(x: T): T { return x; }\n");
    for i in 0..6000 { s.push_str(&format!("struct W{i} {{ a: i32; }}\n")); }
    s.push_str("export function main(log: Log): void { let t = 0;\n");
    for i in 0..6000 { s.push_str(&format!(" const w{i}: W{i} = {{ a: {i} }}; t += id(w{i}).a;\n")); }
    s.push_str(" log.write(2, `${t}`); }\n");
    let src = format!("import {{ Log }} from \"host:log\";\n{s}");
    let t = Instant::now();
    let r = oxiscript_lower::compile_source(&src, &interfaces(), &oxiscript_lower::Options::default());
    eprintln!("fanout: {:?} -> {}", t.elapsed(), match &r { Ok(w) => format!("ok {} bytes", w.len()), Err(e) => e.to_string().chars().take(200).collect() });
    assert!(t.elapsed().as_secs() < 30);
    if let Err(e) = &r { assert!(e.to_string().contains("E0701") || e.to_string().contains("E06"), "{e}"); }
}
