//! `oxi fmt`: the canonical formatter of `oxiscript-syntax` (no options).

use crate::check::{EXIT_FATAL, EXIT_FINDINGS, EXIT_OK};
use crate::project::{self, collect_oxs, ReadError, SourceFile};
use oxiscript_abi::sanitize::sanitize_terminal;
use oxiscript_syntax::lexer::is_invisible;
use oxiscript_syntax::{parse, CommentKind, FileId};
use std::io::{Read, Write};
use std::path::PathBuf;

pub struct Args {
    pub paths: Vec<PathBuf>,
    pub check: bool,
    pub stdin: bool,
}

fn syntax_errors(display: &str, src: &str, e: &oxiscript_syntax::FormatError) -> String {
    let safe_display = sanitize_terminal(display);
    let f = SourceFile::new(display.to_string(), src.to_string());
    let mut s = String::new();
    for d in e.diagnostics.iter().filter(|d| d.is_error()).take(5) {
        let (l, c, _) = f.pos(d.span.lo);
        s.push_str(&format!(
            "{safe_display}:{l}:{c}: error[{}]: {}\n",
            d.code,
            sanitize_terminal(&d.message)
        ));
    }
    s.push_str(&format!("{safe_display}: not formatted (fix the syntax errors first: `oxi check --fix` inserts missing `;` and applies the other safe syntax fixes, `oxi check` shows the rest)\n"));
    s
}

/// Escape invisible and bidi control characters inside preserved line comments,
/// matching `oxiscript-syntax`'s block-comment hygiene so `oxi fmt` cannot
/// re-emit raw Trojan Source controls in `//` or `///` comments.
fn sanitize_line_comment(s: &str) -> String {
    if !s.chars().any(|c| is_invisible(c) && !matches!(c, '\n' | '\r' | '\t')) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if is_invisible(c) && !matches!(c, '\n' | '\r' | '\t') {
            out.push_str(&format!("\\u{{{:X}}}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// The canonical format of `src`, keeping what the formatter has no business changing: a leading
/// BOM, the file's line-ending style (CRLF stays CRLF) and the text of `//` and `///` line comments
/// (the formatter demotes a `///` that is not attached to an item to `//`).
///
/// Literals are different on purpose: the formatter writes raw control and invisible characters
/// (NUL, DEL, U+200B, U+2028, ...) inside string and template literals as escapes (a hygiene rule,
/// documented in AGENTS.md).
pub fn format_preserving(src: &str) -> Result<String, oxiscript_syntax::FormatError> {
    let (bom, body) = match src.strip_prefix('\u{feff}') {
        Some(rest) => (true, rest),
        None => (false, src),
    };
    let crlf = body.matches("\r\n").count();
    let bare_lf = body.matches('\n').count() - crlf;
    let mut out = oxiscript_syntax::format(body)?;
    out = restore_line_comments(body, &out);
    if crlf > bare_lf {
        out = out.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    if bom {
        out.insert(0, '\u{feff}');
    }
    Ok(out)
}

fn line_comment_texts(src: &str) -> Option<Vec<(usize, usize, bool)>> {
    let p = parse(src, FileId(0));
    if p.diagnostics.iter().any(oxiscript_syntax::Diagnostic::is_error) {
        return None;
    }
    Some(p.comments.iter().map(|c| (c.lo as usize, c.hi as usize, matches!(c.kind, CommentKind::Line | CommentKind::Doc))).collect())
}

/// Put the input text of every line comment back into the formatted output (comments keep their
/// order, so the n-th comment of the output is the n-th of the input). Skipped unless both have
/// the same number of comments of the same kinds.
fn restore_line_comments(input: &str, output: &str) -> String {
    let (Some(a), Some(b)) = (line_comment_texts(input), line_comment_texts(output)) else {
        return output.to_string();
    };
    if a.len() != b.len() || a.iter().zip(&b).any(|(x, y)| x.2 != y.2) {
        return output.to_string();
    }
    let mut res = String::with_capacity(output.len());
    let mut at = 0;
    for (x, y) in a.iter().zip(&b) {
        if !y.2 {
            continue;
        }
        let want = sanitize_line_comment(input[x.0..x.1].trim_end());
        res.push_str(&output[at..y.0]);
        res.push_str(&want);
        at = y.1;
    }
    res.push_str(&output[at..]);
    res
}

pub fn run(a: &Args) -> i32 {
    if a.stdin {
        let mut src = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut src) {
            eprintln!("error: cannot read stdin: {}", sanitize_terminal(&e.to_string()));
            return EXIT_FATAL;
        }
        return match format_preserving(&src) {
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
            eprintln!(
                "error: {}: no such file or directory",
                sanitize_terminal(&p.display().to_string())
            );
            return EXIT_FATAL;
        }
        if let Err(e) = collect_oxs(p, &mut files) {
            eprintln!("error: {}", sanitize_terminal(&e));
            return EXIT_FATAL;
        }
    }
    let mut changed = 0;
    let mut bad = 0;
    for f in &files {
        let display = f.strip_prefix("./").unwrap_or(f).to_string_lossy().replace('\\', "/");
        let safe_display = sanitize_terminal(&display);
        let src = match project::read_regular(f, project::MAX_SOURCE_BYTES) {
            Ok(s) => s,
            Err(ReadError::Skipped(why)) => {
                project::note(format!("{display}: skipped ({why})"));
                continue;
            }
            Err(ReadError::Failed(why) | ReadError::TooLarge(why) | ReadError::NotUtf8(why)) => {
                // one unreadable file does not stop the others
                eprintln!("error: {safe_display}: {}", sanitize_terminal(&why));
                bad += 1;
                continue;
            }
        };
        match format_preserving(&src) {
            Ok(out) => {
                if out != src {
                    changed += 1;
                    if a.check {
                        println!("would reformat {safe_display}");
                    } else if let Err(e) = project::write_atomic(f, &out) {
                        eprintln!("error: {safe_display}: {}", sanitize_terminal(&e));
                        bad += 1;
                    } else {
                        println!("formatted {safe_display}");
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
