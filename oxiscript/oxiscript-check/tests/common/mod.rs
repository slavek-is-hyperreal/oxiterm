#![allow(dead_code)]
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

pub fn assert_ok(src: &str) {
    let c = check_source(src, &hosts());
    let errs: Vec<_> = c.errors().collect();
    assert!(errs.is_empty(), "unexpected errors:\n{}\n--- source ---\n{src}", show(src));
}
