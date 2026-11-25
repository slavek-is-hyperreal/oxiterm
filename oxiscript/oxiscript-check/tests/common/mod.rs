#![allow(dead_code, clippy::format_push_string)]
use oxiscript_abi::idl;
use oxiscript_check::*;
use oxiscript_syntax::LineIndex;

pub fn hosts() -> CheckOptions {
    let mut v = Vec::new();
    for src in [
        idl::EXAMPLE_LOG_IDL,
        idl::EXAMPLE_STATE_IDL,
        include_str!("../idl/ui.idl"),
        include_str!("../idl/canvas.idl"),
    ] {
        v.extend(idl::parse(src).expect("idl"));
    }
    CheckOptions { hosts: v }
}

/// (code, line) of every diagnostic, sorted.
pub fn codes(src: &str) -> Vec<(String, u32)> {
    let c = check_source(src, &hosts());
    let li = LineIndex::new(src);
    let mut v: Vec<(String, u32)> = c.diagnostics.iter().map(|d| (d.code.to_string(), li.line(d.span.lo))).collect();
    v.sort();
    v
}

pub fn show(src: &str) -> String {
    let c = check_source(src, &hosts());
    let li = LineIndex::new(src);
    c.diagnostics
        .iter()
        .map(|d| format!("{} line {}: {}", d.code, li.line(d.span.lo), d.message))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Expected `(code, line)` pairs from `//! E0100` markers.
pub fn expected(src: &str) -> Vec<(String, u32)> {
    let mut v = Vec::new();
    for (i, l) in src.lines().enumerate() {
        if let Some(at) = l.find("//! ") {
            for c in l[at + 4..].split_whitespace() {
                v.push((format!("OXS-{c}"), (i + 1) as u32));
            }
        }
    }
    v.sort();
    v
}

pub fn run_cases(cases: &[(&str, &str)]) {
    let mut bad = String::new();
    for (name, src) in cases {
        let want = expected(src);
        let got = codes(src);
        if got != want {
            bad.push_str(&format!("--- {name}\nwant {want:?}\n got {got:?}\n{}\n", show(src)));
        }
    }
    assert!(bad.is_empty(), "\n{bad}");
}

pub fn assert_ok(src: &str) {
    let c = check_source(src, &hosts());
    let errs: Vec<_> = c.errors().collect();
    assert!(errs.is_empty(), "unexpected errors:\n{}\n--- source ---\n{src}", show(src));
}
