//! BAD ACTOR round 2026-11-27-1, target 1 (8.Sec R3-206 terminal-escape sanitiser in `oxi`).
//! Failing tests are findings (R1-nn in docs/stages/rounds/2026-11-27-1.md).
#![allow(dead_code)]

use oxiscript_abi::sanitize::is_unsafe_terminal_char;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn oxi(args: &[&str], dir: &Path, stdin: Option<&str>) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_oxi"));
    c.args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    if let Some(s) = stdin {
        child.stdin.take().unwrap().write_all(s.as_bytes()).unwrap();
    }
    child.wait_with_output().unwrap()
}

fn assert_no_raw_unsafe_except_lf(label: &str, text: &str) {
    for ch in text.chars() {
        if ch == '\n' {
            continue;
        }
        assert!(
            !is_unsafe_terminal_char(ch),
            "{label} leaked unsafe char U+{:04X} ({ch:?}) in:\n{text:?}",
            ch as u32
        );
    }
}

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let d = base.join(format!("oxi-ba-r1-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// R1-03: `oxi fmt` prints file names and syntax-error lines without the shared
/// sanitiser (fmtcmd.rs: `would reformat`, `formatted`, `error: {display}`, and
/// `syntax_errors`). A file named with an OSC title sequence retitles the terminal.
#[test]
fn r1_03_fmt_prints_raw_escapes_from_file_names() {
    let d = scratch("fmt");
    let evil = "esc\x1b]0;PWN\x07.oxs";
    std::fs::write(d.join(evil), "export function f(a: i32): i32 {\n  return a\n}\n").unwrap();
    std::fs::write(d.join("ugly\u{9b}31m.oxs"), "let   x=1 ;\n").unwrap();
    for args in [&["fmt", "--check", "."][..], &["fmt", "."][..]] {
        let o = oxi(args, &d, None);
        assert_no_raw_unsafe_except_lf(&format!("oxi {args:?} stdout"), &String::from_utf8_lossy(&o.stdout));
        assert_no_raw_unsafe_except_lf(&format!("oxi {args:?} stderr"), &String::from_utf8_lossy(&o.stderr));
    }
}

/// R1-04: `oxi check --json` sanitises `diagnostics[].file` and `edits[].file` but not
/// `files[]` (nor `file_targets` keys), so a consumer cannot match a diagnostic to its
/// file (the VS Code extension drops all of them, see
/// editors/vscode-oxiscript/test/bad_actor_round1.test.mjs), and human output loses
/// the source snippet (looked up by the sanitised name).
#[test]
fn r1_04_json_file_names_are_inconsistent() {
    let d = scratch("json");
    let name = "soft\u{ad}hy.oxs";
    std::fs::write(d.join(name), "export function f(a: i32, b: i32): bool {\n  return a == b;\n}\n").unwrap();
    let o = oxi(&["check", "--json", name], &d, None);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let files: Vec<&str> = v["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    let dfile = v["diagnostics"][0]["file"].as_str().unwrap();
    let efile = v["diagnostics"][0]["fixes"][0]["edits"][0]["file"].as_str().unwrap();
    assert!(
        files.contains(&dfile) && files.contains(&efile),
        "files[] {files:?} does not contain the diagnostic's file {dfile:?} / edit file {efile:?}"
    );
    let h = oxi(&["check", name], &d, None);
    let out = String::from_utf8_lossy(&h.stdout);
    assert!(out.contains("return a == b;"), "human output has no source snippet:\n{out}");
}

/// R1-12: 8.Fix R2-133 says `oxi fmt` escapes bidi/invisible characters in comments so it
/// "cannot re-emit raw Trojan Source controls"; that holds for `/* */` but `format_preserving`
/// restores the original text of `//` line comments, so a raw U+202E survives `oxi fmt`.
#[test]
fn r1_12_fmt_keeps_raw_bidi_in_line_comments() {
    let d = scratch("bidi");
    std::fs::write(d.join("b.oxs"), "// a\u{202e}b\nexport   function g(): i32 {\n  return 1; /* c\u{202e}d */\n}\n").unwrap();
    let o = oxi(&["fmt", "b.oxs"], &d, None);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let out = std::fs::read_to_string(d.join("b.oxs")).unwrap();
    assert!(!out.contains('\u{202e}'), "formatted file still holds a raw U+202E:\n{out:?}");
}
