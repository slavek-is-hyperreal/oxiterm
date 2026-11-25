#![allow(clippy::doc_markdown)]
//! `oxs-fmt` (OxiScript formatter): format an OxiScript file, or just check it.
//!
//! ```text
//! cargo run -p oxiscript-syntax --example oxs_fmt -- file.oxs          # print formatted
//! cargo run -p oxiscript-syntax --example oxs_fmt -- --check file.oxs  # exit 1 if not formatted
//! cargo run -p oxiscript-syntax --example oxs_fmt -- --diag file.oxs   # list diagnostics only
//! ```
use oxiscript_syntax::{format, parse_module, LineIndex};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut check = false;
    let mut diag_only = false;
    let mut files = Vec::new();
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "--check" => check = true,
            "--diag" => diag_only = true,
            _ => files.push(a),
        }
    }
    if files.is_empty() {
        eprintln!("usage: oxs_fmt [--check|--diag] FILE.oxs...");
        return ExitCode::from(2);
    }
    let mut status = 0u8;
    for f in files {
        let src = match std::fs::read_to_string(&f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{f}: {e}");
                status = 2;
                continue;
            }
        };
        if diag_only {
            let (_, diags) = parse_module(&src);
            let li = LineIndex::new(&src);
            for d in &diags {
                let (l, c) = li.line_col(&src, d.span.lo);
                println!("{f}:{l}:{c}: {}: {}", d.code, d.message);
                for fx in &d.fixes {
                    println!("    fix: {}", fx.title);
                }
            }
            if !diags.is_empty() {
                status = 1;
            }
            continue;
        }
        match format(&src) {
            Ok(out) => {
                if check {
                    if out != src {
                        eprintln!("{f}: not formatted");
                        status = 1;
                    }
                } else {
                    print!("{out}");
                }
            }
            Err(e) => {
                let li = LineIndex::new(&src);
                for d in &e.diagnostics {
                    let (l, c) = li.line_col(&src, d.span.lo);
                    eprintln!("{f}:{l}:{c}: {}: {}", d.code, d.message);
                }
                status = 1;
            }
        }
    }
    ExitCode::from(status)
}
