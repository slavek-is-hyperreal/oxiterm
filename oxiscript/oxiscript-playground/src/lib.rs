//! `OxiScript` playground: the compiler compiled to wasm32 for the website.
//!
//! The pure functions ([`check_json`], [`format_json`], [`apply_fix_json`], [`explain_json`],
//! [`hosts_json`]) work on native targets too (tests, CI). The `#[wasm_bindgen]` wrappers are
//! one-liners over them. All results are JSON strings (one boundary type, no struct glue).
//!
//! Offsets: the compiler counts UTF-8 bytes, a browser `<textarea>` counts UTF-16 code units.
//! Every range returned here is UTF-16 (`start`/`end`) plus 1-based line/column. Fixes are
//! applied in Rust ([`apply_fix_json`]), never by the page, so byte offsets never leave this crate.
//!
//! # Compile + run (8.L, not yet)
//! TODO(8.L): `compile(src) -> JSON { ok, diagnostics, wasm_len, manifest }` plus
//! `compile_bytes(src) -> Uint8Array` once `oxiscript-lower` lands; the page then instantiates the
//! module with the `log`/`state` host imports (see [`hosts_json`]) using the browser's own
//! `WebAssembly` engine. Only `sites/oxiscript/js/playground.js` changes (search `TODO(8.L)`).

use oxiscript_abi::codes;
use oxiscript_abi::idl::{self, Interface};
use oxiscript_check::{check_source, CheckOptions};
use oxiscript_syntax::{apply_edits, Diagnostic, FileId, LineIndex, TextEdit};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

/// Most source bytes the playground accepts (the page also caps its textarea).
pub const MAX_SOURCE_BYTES: usize = 256 * 1024;

fn host_interfaces() -> Vec<Interface> {
    let mut v = idl::parse(idl::EXAMPLE_LOG_IDL).unwrap_or_default();
    v.extend(idl::parse(idl::EXAMPLE_STATE_IDL).unwrap_or_default());
    v
}

fn options() -> CheckOptions {
    CheckOptions {
        hosts: host_interfaces(),
    }
}

/// Byte offset -> UTF-16 offset.
fn utf16_at(src: &str, byte: usize) -> usize {
    let mut b = byte.min(src.len());
    while !src.is_char_boundary(b) {
        b -= 1;
    }
    src[..b].encode_utf16().count()
}

fn too_big(src: &str) -> Option<Value> {
    (src.len() > MAX_SOURCE_BYTES).then(|| {
        json!({ "ok": false, "diagnostics": [{
            "code": "", "severity": "error", "title": "Source too large",
            "message": format!("the playground accepts at most {MAX_SOURCE_BYTES} bytes"),
            "start": 0, "end": 0, "line": 1, "col": 1, "endLine": 1, "endCol": 1, "fixes": [] }] })
    })
}

/// The diagnostics of the entry module (the prelude module is dropped).
fn entry_diagnostics(src: &str) -> Vec<Diagnostic> {
    let checked = check_source(src, &options());
    checked
        .diagnostics
        .into_iter()
        .filter(|d| d.span.file == FileId(0))
        .collect()
}

fn diag_json(src: &str, li: &LineIndex, d: &Diagnostic) -> Value {
    let def = codes::lookup(d.code);
    let severity = match &def {
        Some(c) => match c.severity {
            codes::Severity::Error => "error",
            codes::Severity::Warning => "warning",
            codes::Severity::Note => "note",
            codes::Severity::Internal => "internal",
        },
        None if d.is_error() => "error",
        None => "warning",
    };
    let lo = d.span.lo as usize;
    let hi = (d.span.hi as usize).max(lo);
    let (line, col) = li.line_col(src, d.span.lo);
    let (end_line, end_col) = li.line_col(src, d.span.hi.max(d.span.lo));
    let fixes: Vec<Value> = d
        .fixes
        .iter()
        .map(|f| {
            let edits: Vec<Value> = f
                .edits
                .iter()
                .map(|e| {
                    json!({ "start": utf16_at(src, e.span.lo as usize),
                            "end": utf16_at(src, e.span.hi as usize), "text": e.replacement })
                })
                .collect();
            json!({ "title": f.title, "edits": edits })
        })
        .collect();
    json!({
        "code": d.code,
        "severity": severity,
        "title": def.as_ref().map(|c| c.title.clone()).unwrap_or_default(),
        "message": d.message,
        "start": utf16_at(src, lo), "end": utf16_at(src, hi),
        "line": line, "col": col, "endLine": end_line, "endCol": end_col,
        "fixes": fixes,
    })
}

/// Type-check `src` against the built-in `log` and `state` host interfaces.
/// `{ "ok": bool, "diagnostics": [ { code, severity, title, message, start, end, line, col,
/// endLine, endCol, fixes: [ { title, edits: [ { start, end, text } ] } ] } ] }`.
/// `ok` is true when there is no error (warnings allowed).
pub fn check_json(src: &str) -> String {
    if let Some(v) = too_big(src) {
        return v.to_string();
    }
    let li = LineIndex::new(src);
    let diags = entry_diagnostics(src);
    let ok = !diags.iter().any(Diagnostic::is_error);
    let list: Vec<Value> = diags.iter().map(|d| diag_json(src, &li, d)).collect();
    json!({ "ok": ok, "diagnostics": list }).to_string()
}

/// Canonical formatting: `{ "ok": true, "text": "..." }` or `{ "ok": false, "diagnostics": [...] }`.
pub fn format_json(src: &str) -> String {
    if let Some(v) = too_big(src) {
        return v.to_string();
    }
    match oxiscript_syntax::format(src) {
        Ok(text) => json!({ "ok": true, "text": text }).to_string(),
        Err(e) => {
            let li = LineIndex::new(src);
            let list: Vec<Value> = e
                .diagnostics
                .iter()
                .map(|d| diag_json(src, &li, d))
                .collect();
            json!({ "ok": false, "diagnostics": list }).to_string()
        }
    }
}

/// Apply fix `fix` of diagnostic `diag` (indices into the list [`check_json`] returned for the same
/// `src`): `{ "ok": true, "text": new source }` or `{ "ok": false, "error": "..." }`.
pub fn apply_fix_json(src: &str, diag: usize, fix: usize) -> String {
    if src.len() > MAX_SOURCE_BYTES {
        return json!({ "ok": false, "error": "source too large" }).to_string();
    }
    let diags = entry_diagnostics(src);
    let Some(f) = diags.get(diag).and_then(|d| d.fixes.get(fix)) else {
        return json!({ "ok": false, "error": "no such fix (the source changed?)" }).to_string();
    };
    let edits: Vec<TextEdit> = f.edits.clone();
    match apply_edits(src, &edits) {
        Some(text) => json!({ "ok": true, "text": text }).to_string(),
        None => {
            json!({ "ok": false, "error": "the fix does not apply to this source" }).to_string()
        }
    }
}

/// The registry entry of a reason code, or `{ "ok": false }`.
pub fn explain_json(code: &str) -> String {
    match codes::lookup(code) {
        Some(c) => json!({
            "ok": true, "code": c.code, "title": c.title, "message": c.message,
            "severity": format!("{:?}", c.severity).to_lowercase(),
            "phase": c.phase, "fix": c.fix,
        })
        .to_string(),
        None => json!({ "ok": false }).to_string(),
    }
}

/// The host interfaces the playground checks against (their IDL source text).
pub fn hosts_json() -> String {
    json!({ "log": idl::EXAMPLE_LOG_IDL, "state": idl::EXAMPLE_STATE_IDL }).to_string()
}

/// Number of reason codes in the registry.
pub fn registry_size() -> usize {
    codes::seed().len()
}

/// `check(src) -> JSON` (see [`check_json`]).
#[wasm_bindgen]
pub fn check(src: &str) -> String {
    check_json(src)
}
/// `format(src) -> JSON` (see [`format_json`]).
#[wasm_bindgen]
pub fn format(src: &str) -> String {
    format_json(src)
}
/// `apply_fix(src, diag, fix) -> JSON` (see [`apply_fix_json`]).
#[wasm_bindgen]
pub fn apply_fix(src: &str, diag: usize, fix: usize) -> String {
    apply_fix_json(src, diag, fix)
}
/// `explain(code) -> JSON` (see [`explain_json`]).
#[wasm_bindgen]
pub fn explain(code: &str) -> String {
    explain_json(code)
}
/// `hosts() -> JSON` (see [`hosts_json`]).
#[wasm_bindgen]
pub fn hosts() -> String {
    hosts_json()
}
/// Number of reason codes in the registry.
#[wasm_bindgen]
pub fn registry_count() -> usize {
    registry_size()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn good_program_is_ok() {
        let src = "import { Log } from \"host:log\";\nexport function main(log: Log): void {\n  log.write(2, \"hi\");\n}\n";
        let v = parse(&check_json(src));
        assert_eq!(v["ok"], true, "{v}");
    }

    #[test]
    fn error_has_registry_title_and_utf16_range() {
        // a multi-byte char before the error makes UTF-8 and UTF-16 offsets differ
        let src = "// za\u{17c}\u{f3}\u{142}\u{107}\nconst x: i32 = \"a\";\n";
        let v = parse(&check_json(src));
        assert_eq!(v["ok"], false, "{v}");
        let d = &v["diagnostics"][0];
        assert!(d["code"].as_str().unwrap().starts_with("OXS-E"));
        assert!(!d["title"].as_str().unwrap().is_empty());
        let start = d["start"].as_u64().unwrap() as usize;
        let units: Vec<u16> = src.encode_utf16().collect();
        let tail = String::from_utf16(&units[start..]).unwrap();
        assert!(tail.starts_with('"'), "range points at the literal, got {tail:?}");
        assert_eq!(d["line"], 2);
    }

    #[test]
    fn fix_is_applied_in_rust() {
        let src = "const a: i32 = 1;\nconst b: bool = a == 1;\n";
        let v = parse(&check_json(src));
        let (di, d) = v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .find(|(_, d)| !d["fixes"].as_array().unwrap().is_empty())
            .expect("== has a fix");
        assert_eq!(d["code"], "OXS-E0005");
        let r = parse(&apply_fix_json(src, di, 0));
        assert_eq!(r["ok"], true, "{r}");
        assert!(r["text"].as_str().unwrap().contains("==="));
        assert!(apply_fix_json(src, 99, 0).contains("\"ok\":false"));
    }

    #[test]
    fn format_roundtrip_and_errors() {
        let r = parse(&format_json("export   function f( ):void{return;}"));
        assert_eq!(r["ok"], true, "{r}");
        assert!(r["text"].as_str().unwrap().contains("function f()"));
        let e = parse(&format_json("function {"));
        assert_eq!(e["ok"], false);
    }

    #[test]
    fn explain_known_and_unknown() {
        assert_eq!(parse(&explain_json("OXS-E0005"))["ok"], true);
        assert_eq!(parse(&explain_json("OXS-E9999"))["ok"], false);
        assert!(registry_size() > 50);
    }

    #[test]
    fn oversize_source_is_refused() {
        let big = "a".repeat(MAX_SOURCE_BYTES + 1);
        assert_eq!(parse(&check_json(&big))["ok"], false);
    }

    #[test]
    fn hostile_inputs_do_not_panic() {
        for s in [
            "",
            "\u{feff}",
            "\0",
            "let",
            "\"unterminated",
            "/*",
            "((((((((((",
            "\u{1d518}\u{1d52b}",
            "export function main(): void {",
        ] {
            let _ = check_json(s);
            let _ = format_json(s);
            let _ = apply_fix_json(s, 0, 0);
        }
    }
}
