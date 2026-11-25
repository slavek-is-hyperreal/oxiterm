//! Native stack needed for the deepest parseable nestings.
//! `cargo run -p oxiscript-check --example stack_probe -- <case> <stack_kb>` exits 0 when the
//! check completes on a thread with that stack (a stack overflow aborts the process).
use oxiscript_check::{check_source, CheckOptions};

fn case(name: &str) -> String {
    let rep = |s: &str, n: usize| s.repeat(n);
    match name {
        "parens" => format!("function f(a: i32): i32 {{ return {}a{}; }}", rep("(", 50), rep(")", 50)),
        "calls" => format!("function g(a: i32): i32 {{ return a; }}\nfunction f(a: i32): i32 {{ return {}a{}; }}", rep("g(", 50), rep(")", 50)),
        "arrows" => format!("function f(): void {{ const g = {}0; }}", rep("() => ", 25)),
        "closures" => format!("function f(): void {{ {}{} }}", rep("const g = () => { ", 25), rep("};", 25)),
        "blocks" => format!("function f(): void {{ {}{} }}", rep("{ ", 27), rep("}", 27)),
        "ifs" => format!("function f(a: bool): void {{ {}{} }}", rep("if (a) { ", 27), rep("}", 27)),
        "objects" => format!("struct N {{ c: N | null; }}\nfunction f(): N {{ return {}null{}; }}", rep("{ c: ", 25), rep(" }", 25)),
        "switches" => format!("function f(a: i32): i32 {{ return {}0{}; }}", rep("switch (a) { default: ", 25), rep(" }", 25)),
        "chain" => format!("struct B {{ f(): B {{ return this; }} }}\nfunction f(b: B): B {{ return b{}; }}", rep(".f()", 400)),
        "sum" => format!("function f(a: i32): i32 {{ return a{}; }}", rep(" + a", 3000)),
        _ => String::new(),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = args.get(1).cloned().unwrap_or_default();
    let kb: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1024);
    let src = case(&name);
    let t = std::thread::Builder::new()
        .stack_size(kb * 1024)
        .spawn(move || {
            let c = check_source(&src, &CheckOptions::default());
            (c.ok(), c.diagnostics.first().map(|d| d.code.to_string()))
        })
        .unwrap();
    let (ok, first) = t.join().unwrap();
    println!("{name} at {kb} KB: ok={ok} first={first:?}");
}
