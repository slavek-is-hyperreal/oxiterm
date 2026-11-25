#![allow(clippy::doc_markdown)]
//! Measures how much native stack the inline (no extra thread) parser/formatter needs.
//!
//! `cargo run --release -p oxiscript-syntax --example stack_probe -- KIND N [fmt]`
//! prints the smallest stack (KB, binary search over child processes) on which
//! parsing (or formatting with `fmt`) the generated input of nesting `N` succeeds.
//! KIND: paren, bracket, block, call, unary, arrow, ternary, type, switch, member, add.
use oxiscript_syntax::{format_with, parse_with, FileId, FormatOptions, ParseOptions};

fn gen(kind: &str, n: usize) -> String {
    match kind {
        "paren" => format!("let x = {}1{};", "(".repeat(n), ")".repeat(n)),
        "bracket" => format!("let x = {}1{};", "[".repeat(n), "]".repeat(n)),
        "block" => format!("function f(): void {}{}", "{".repeat(n), "}".repeat(n)),
        "call" => format!("let x = {}1{};", "f(".repeat(n), ")".repeat(n)),
        "unary" => format!("let x = {}1;", "-".repeat(n)),
        "arrow" => format!("let x = {}1;", "(a) => ".repeat(n)),
        "ternary" => format!("let x = {}1{};", "a ? ".repeat(n), " : 2".repeat(n)),
        "type" => format!("let x: {}i32{};", "Array<".repeat(n), ">".repeat(n)),
        "switch" => format!(
            "let x = {}1{};",
            "switch (a) { case 1: ".repeat(n),
            " }".repeat(n)
        ),
        "member" => format!("let x = a{};", ".b".repeat(n)),
        "add" => format!("let x = a{};", " + a".repeat(n)),
        _ => panic!("kind"),
    }
}

fn run(kind: &str, n: usize, fmt: bool, depth: usize) {
    let src = gen(kind, n);
    let opts = ParseOptions {
        max_depth: depth,
        use_thread: false,
        ..ParseOptions::default()
    };
    if fmt {
        let o = FormatOptions {
            parse: opts,
            ..FormatOptions::default()
        };
        let out = format_with(&src, &o);
        std::mem::forget(out.is_ok());
    } else {
        let p = parse_with(&src, FileId(0), &opts);
        let ok = p.diagnostics.is_empty();
        std::mem::forget(p);
        assert!(ok, "diagnostics");
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.get(1).map(String::as_str) == Some("--child") {
        let kb: usize = a[2].parse().unwrap();
        let n: usize = a[4].parse().unwrap();
        let kind = a[3].clone();
        let fmt = a.get(5).map(String::as_str) == Some("fmt");
        let depth: usize = a.get(6).and_then(|s| s.parse().ok()).unwrap_or(10_000);
        std::thread::Builder::new()
            .stack_size(kb * 1024)
            .spawn(move || run(&kind, n, fmt, depth))
            .unwrap()
            .join()
            .unwrap();
        return;
    }
    let kind = &a[1];
    let n: usize = a[2].parse().unwrap();
    let fmt = a.get(3).map(String::as_str) == Some("fmt");
    let exe = std::env::current_exe().unwrap();
    let ok = |kb: usize| {
        std::process::Command::new(&exe)
            .args(["--child", &kb.to_string(), kind, &n.to_string()])
            .args(if fmt { vec!["fmt"] } else { vec!["parse"] })
            .args(["10000"])
            .output()
            .unwrap()
            .status
            .success()
    };
    let (mut lo, mut hi) = (8usize, 4096usize);
    if !ok(hi) {
        println!("{kind} n={n}: does not fit even in {hi} KB");
        return;
    }
    while lo + 1 < hi {
        let mid = usize::midpoint(lo, hi);
        if ok(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    println!("{kind} n={n} {}: min stack {hi} KB ({} B/level)", if fmt { "format" } else { "parse" }, hi * 1024 / n.max(1));
}
