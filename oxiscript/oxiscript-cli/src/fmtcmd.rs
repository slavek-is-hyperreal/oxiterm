//! `oxi fmt`: the canonical formatter of `oxiscript-syntax` (no options).

use crate::check::{EXIT_FATAL, EXIT_FINDINGS, EXIT_OK};
use crate::project::{collect_oxs, SourceFile};
use std::io::{Read, Write};
use std::path::PathBuf;

pub struct Args {
    pub paths: Vec<PathBuf>,
    pub check: bool,
    pub stdin: bool,
}

fn syntax_errors(display: &str, src: &str, e: &oxiscript_syntax::FormatError) -> String {
    let f = SourceFile::new(display.to_string(), src.to_string());
    let mut s = String::new();
    for d in e.diagnostics.iter().filter(|d| d.is_error()).take(5) {
        let (l, c, _) = f.pos(d.span.lo);
        s.push_str(&format!("{display}:{l}:{c}: error[{}]: {}\n", d.code, d.message));
    }
    s.push_str(&format!("{display}: not formatted (fix the syntax errors first; `oxi check` shows them)\n"));
    s
}

pub fn run(a: &Args) -> i32 {
    if a.stdin {
        let mut src = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut src) {
            eprintln!("error: cannot read stdin: {e}");
            return EXIT_FATAL;
        }
        return match oxiscript_syntax::format(&src) {
            Ok(out) => {
                let _ = std::io::stdout().write_all(out.as_bytes());
                EXIT_OK
            }
            Err(e) => {
                eprint!("{}", syntax_errors("<stdin>", &src, &e));
                EXIT_FINDINGS
            }
        };
    }
    let paths = if a.paths.is_empty() { vec![PathBuf::from(".")] } else { a.paths.clone() };
    let mut files = Vec::new();
    for p in &paths {
        if !p.exists() {
            eprintln!("error: {}: no such file or directory", p.display());
            return EXIT_FATAL;
        }
        if let Err(e) = collect_oxs(p, &mut files) {
            eprintln!("error: {e}");
            return EXIT_FATAL;
        }
    }
    let mut changed = 0;
    let mut bad = 0;
    for f in &files {
        let display = f.strip_prefix("./").unwrap_or(f).to_string_lossy().replace('\\', "/");
        let src = match std::fs::read_to_string(f) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {display}: {e}");
                return EXIT_FATAL;
            }
        };
        match oxiscript_syntax::format(&src) {
            Ok(out) => {
                if out != src {
                    changed += 1;
                    if a.check {
                        println!("would reformat {display}");
                    } else if let Err(e) = std::fs::write(f, out) {
                        eprintln!("error: {display}: {e}");
                        return EXIT_FATAL;
                    } else {
                        println!("formatted {display}");
                    }
                }
            }
            Err(e) => {
                bad += 1;
                eprint!("{}", syntax_errors(&display, &src, &e));
            }
        }
    }
    if a.check {
        println!("{} file(s) checked, {changed} would change, {bad} with syntax errors", files.len());
    }
    if bad > 0 || (a.check && changed > 0) {
        EXIT_FINDINGS
    } else {
        EXIT_OK
    }
}
