//! Diagnostics as data: one `Diag` per finding with resolved positions and
//! fixes with applicability, rendered either as rustc-style text or as the
//! stable JSON documented in `SCHEMA.md`.

use crate::project::{Run, SourceFile};
use oxiscript_abi::sanitize::sanitize_terminal;
use oxiscript_syntax::{Diagnostic, FileId, Span};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fmt::Write;

pub const SCHEMA_VERSION: u32 = 1;

pub const MACHINE_APPLICABLE: &str = "machine-applicable";
pub const MAYBE_INCORRECT: &str = "maybe-incorrect";
pub const ADVICE: &str = "advice";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pos {
    pub byte: u32,
    pub line: u32,
    pub col: u32,
    pub utf16: u32,
}

#[derive(Clone, Debug)]
pub struct Range {
    pub start: Pos,
    pub end: Pos,
}

#[derive(Clone, Debug)]
pub struct EditOut {
    pub file: String,
    pub range: Range,
    pub new_text: String,
}

#[derive(Clone, Debug)]
pub struct FixOut {
    pub title: String,
    pub applicability: &'static str,
    pub edits: Vec<EditOut>,
}

#[derive(Clone, Debug)]
pub struct Diag {
    pub code: String,
    pub severity: &'static str,
    pub message: String,
    pub file: String,
    pub range: Range,
    pub fixes: Vec<FixOut>,
}

pub fn severity_of(code: &str) -> &'static str {
    match code.as_bytes().get(4) {
        Some(b'E') => "error",
        Some(b'W') => "warning",
        Some(b'P') => "note",
        Some(b'L') => "error",
        _ => "internal",
    }
}

/// How safe is applying this fix without a human looking at it?
///
/// * no edits: `advice` (text only);
/// * more than one alternative with edits on the diagnostic, the ambiguous `?` (E0006) and
///   `for ... in` -> `of` (keys become values): `maybe-incorrect`;
/// * syntax-level codes (`OXS-E0001..E0099`, except the ambiguous-`?` E0006):
///   the edit restores the one reading the grammar allows: `machine-applicable`;
/// * everything else (type-level fixes such as coercions, stub switch cases):
///   `maybe-incorrect`.
pub fn applicability(code: &str, fix: &oxiscript_syntax::Fix, alternatives: usize) -> &'static str {
    if fix.edits.is_empty() {
        return ADVICE;
    }
    // `for (k in xs)` -> `of` turns keys into values: the program changes meaning.
    if alternatives > 1 || code == "OXS-E0006" || fix.title == "use `of`" {
        return MAYBE_INCORRECT;
    }
    let n: u32 = code.get(5..).and_then(|d| d.parse().ok()).unwrap_or(9999);
    if code.starts_with("OXS-E") && n < 100 {
        MACHINE_APPLICABLE
    } else {
        MAYBE_INCORRECT
    }
}

fn pos(f: &SourceFile, byte: u32) -> Pos {
    let (line, col, utf16) = f.pos(byte);
    Pos {
        byte,
        line,
        col,
        utf16,
    }
}

fn range(files: &[SourceFile], s: Span) -> Option<(usize, Range)> {
    let i = s.file.0 as usize;
    let f = files.get(i)?;
    let n = f.src.len() as u32;
    let (lo, hi) = (s.lo.min(n), s.hi.min(n).max(s.lo.min(n)));
    Some((
        i,
        Range {
            start: pos(f, lo),
            end: pos(f, hi),
        },
    ))
}

/// Resolve all diagnostics of a run. Diagnostics whose span points outside
/// the loaded files (the implicit prelude, `Span::DUMMY` of cycles) fall back to the entry file.
pub fn collect(run: &Run) -> Vec<Diag> {
    let mut out = Vec::new();
    let mut seen: HashSet<(usize, u32, u32, String)> = HashSet::new();
    for d in &run.checked.diagnostics {
        let span = if (d.span.file.0 as usize) < run.files.len() {
            d.span
        } else {
            Span::new(FileId(0), 0, 0)
        };
        let Some((fi, r)) = range(&run.files, span) else {
            continue;
        };
        if !seen.insert((fi, r.start.byte, r.end.byte, d.code.to_string())) {
            continue;
        }
        let mut diag = Diag {
            code: d.code.to_string(),
            severity: severity_of(d.code),
            message: sanitize_terminal(&d.message),
            file: run.files[fi].display.clone(),
            range: r,
            fixes: fixes_of(run, d),
        };
        crate::hints::apply(&mut diag, &run.files[fi]);
        out.push(diag);
    }
    out
}

/// A diagnostic made by the CLI itself (not the checker) at `span` of `files[span.file]`.
pub fn diag_at(srcs: &[SourceFile], span: Span, code: &str, message: String, fixes: Vec<FixOut>) -> Option<Diag> {
    let (fi, r) = range(srcs, span)?;
    Some(Diag {
        code: code.to_string(),
        severity: severity_of(code),
        message: sanitize_terminal(&message),
        file: srcs[fi].display.clone(),
        range: r,
        fixes: fixes
            .into_iter()
            .map(|mut f| {
                f.title = sanitize_terminal(&f.title);
                f
            })
            .collect(),
    })
}

fn fixes_of(run: &Run, d: &Diagnostic) -> Vec<FixOut> {
    let alts = d.fixes.iter().filter(|f| !f.edits.is_empty()).count();
    d.fixes
        .iter()
        .map(|f| FixOut {
            title: sanitize_terminal(&f.title),
            applicability: applicability(d.code, f, alts),
            edits: f
                .edits
                .iter()
                .filter_map(|e| {
                    let (fi, r) = range(&run.files, e.span)?;
                    Some(EditOut {
                        file: run.files[fi].display.clone(),
                        range: r,
                        new_text: e.replacement.clone(),
                    })
                })
                .collect(),
        })
        .collect()
}

fn pos_json(p: &Pos) -> Value {
    json!({"byte": p.byte, "line": p.line, "col": p.col, "utf16": p.utf16})
}

fn range_json(r: &Range) -> Value {
    json!({"start": pos_json(&r.start), "end": pos_json(&r.end)})
}

pub fn diag_json(d: &Diag) -> Value {
    let mut obj = json!({
        "code": d.code,
        "severity": d.severity,
        "message": d.message,
        "file": d.file,
        "range": range_json(&d.range),
        "fixes": d.fixes.iter().map(|f| json!({
            "title": f.title,
            "applicability": f.applicability,
            "edits": f.edits.iter().map(|e| json!({
                "file": e.file,
                "range": range_json(&e.range),
                "new_text": e.new_text,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    });
    let safe = sanitize_terminal(&d.file);
    if safe != d.file {
        if let Some(m) = obj.as_object_mut() {
            m.insert("file_display".to_string(), json!(safe));
        }
    }
    obj
}

/// The whole `oxi check --json` document.
pub fn document(files: &[String], diags: &[Diag], fatal: Option<&str>, notes: &[String], fix: Option<Value>, target: Option<&str>) -> Value {
    let errors = diags.iter().filter(|d| d.severity == "error" || d.severity == "internal").count();
    let warnings = diags.iter().filter(|d| d.severity == "warning").count();
    let mut doc = json!({
        "schema": SCHEMA_VERSION,
        "tool": "oxi",
        "version": env!("CARGO_PKG_VERSION"),
        "ok": errors == 0 && fatal.is_none(),
        "fatal": fatal,
        "files": files,
        "summary": {"errors": errors, "warnings": warnings},
        "diagnostics": diags.iter().map(diag_json).collect::<Vec<_>>(),
    });
    // additive fields, present only when there is something to say (schema stays 1)
    if let Some(o) = doc.as_object_mut() {
        if !notes.is_empty() {
            o.insert("notes".to_string(), json!(notes));
        }
        if let Some(t) = target {
            o.insert("target".to_string(), json!(t));
        }
        if let Some(f) = fix {
            o.insert("fix".to_string(), f);
        }
    }
    doc
}

/// rustc-style text. `sources` maps display path to source text.
pub fn render_human(d: &Diag, sources: &dyn Fn(&str) -> Option<String>, color: bool) -> String {
    let (red, yel, cyan, bold, off) = if color {
        ("\x1b[31m", "\x1b[33m", "\x1b[36m", "\x1b[1m", "\x1b[0m")
    } else {
        ("", "", "", "", "")
    };
    let sev_color = if d.severity == "warning" { yel } else { red };
    let msg = sanitize_terminal(&d.message);
    let file = sanitize_terminal(&d.file);
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{bold}{sev_color}{}[{}]{off}{bold}: {msg}{off}",
        d.severity, d.code
    );
    let line = d.range.start.line;
    let w = line.to_string().len();
    let _ = writeln!(
        s,
        "{:w$}{cyan}-->{off} {file}:{}:{}",
        "",
        line,
        d.range.start.col
    );
    if let Some(src) = sources(&d.file) {
        if let Some(text) = src.lines().nth(line as usize - 1) {
            let start_chars = (d.range.start.col as usize).saturating_sub(1);
            let span_chars = if d.range.end.line == line {
                (d.range.end.col as usize).saturating_sub(d.range.start.col as usize).max(1)
            } else {
                text.chars().count().saturating_sub(start_chars).max(1)
            };
            let prefix_raw: String = text.chars().take(start_chars).collect();
            let span_raw: String = text.chars().skip(start_chars).take(span_chars).collect();
            let safe_line = sanitize_terminal(text);
            let start = sanitize_terminal(&prefix_raw).chars().count();
            let len = sanitize_terminal(&span_raw).chars().count().max(1);
            let _ = writeln!(s, "{:w$} {cyan}|{off}", "");
            let _ = writeln!(s, "{cyan}{line:>w$} |{off} {safe_line}");
            let _ = writeln!(
                s,
                "{:w$} {cyan}|{off} {}{sev_color}{}{off}",
                "",
                " ".repeat(start),
                "^".repeat(len)
            );
        }
    }
    for f in &d.fixes {
        let tag = match f.applicability {
            MACHINE_APPLICABLE => "fix",
            MAYBE_INCORRECT => "fix, check it",
            _ => "help",
        };
        let _ = writeln!(s, "{:w$} {cyan}={off} {bold}{tag}{off}: {}", "", sanitize_terminal(&f.title));
    }
    let _ = writeln!(s, "{:w$} {cyan}={off} explain: oxi explain {}", "", d.code);
    s
}
