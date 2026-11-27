//! BAD ACTOR round 2026-11-27-2 (pre-0.0.3 small round): variants of the R1-03 / R1-04 fixes.
//! `probe_*` tests are attacks that held; `r2_*` tests fail until the finding is fixed.
#![allow(dead_code)]

use oxiscript_abi::sanitize::is_unsafe_terminal_char;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn oxi(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oxi"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap()
}

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let d = base.join(format!("oxi-ba-r2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn leaks(text: &str) -> Option<char> {
    text.chars().find(|&c| c != '\n' && is_unsafe_terminal_char(c))
}

const HOSTILE: [&str; 6] = [
    "esc\x1b]0;PWN\x07.oxs",
    "c1\u{9b}31m.oxs",
    "bidi\u{202e}x.oxs",
    "soft\u{ad}hy.oxs",
    "zw\u{200d}j.oxs",
    "line\u{2028}sep.oxs",
];
const BAD: &str = "export function f(a: i32, b: i32): bool {\n  return a == b\n}\n";
const UGLY: &str = "export function f(a:i32,b:i32):bool { return a===b; }\n";
const GOOD: &str = "export function main(): void {}\n";

/// Probe: every echo path of check / fmt / build with hostile file names is clean
/// (JSON output is excluded: there the path is data, see R1-04).
#[test]
fn probe_hostile_file_names_never_reach_the_terminal() {
    let d = scratch("names");
    for n in HOSTILE {
        std::fs::write(d.join(n), BAD).unwrap();
    }
    for args in [
        &["check", "."][..],
        &["check", "--fix", "."],
        &["check", "--fix-only", "semicolons", "."],
        &["fmt", "--check", "."],
        &["fmt", "."],
        &["build", "."],
    ] {
        let o = oxi(args, &d);
        for (w, b) in [("stdout", &o.stdout), ("stderr", &o.stderr)] {
            let t = String::from_utf8_lossy(b);
            if let Some(c) = leaks(&t) {
                panic!("{args:?} {w} leaked U+{:04X}\n{t:?}", c as u32);
            }
        }
    }
    let d2 = scratch("names2");
    for n in HOSTILE {
        std::fs::write(d2.join(n), UGLY).unwrap();
    }
    for args in [&["fmt", "--check", "."][..], &["fmt", "."]] {
        let o = oxi(args, &d2);
        for b in [&o.stdout, &o.stderr] {
            assert_eq!(leaks(&String::from_utf8_lossy(b)), None, "{args:?}");
        }
    }
}

/// Probe: hostile names via explicit file arguments, missing files, `run`, `explain`, `new`.
#[test]
fn probe_hostile_paths_in_run_and_errors() {
    let d = scratch("run");
    let evil = "r\x1b[2Jun\u{9d}.oxs";
    std::fs::write(d.join(evil), GOOD).unwrap();
    for args in [
        vec!["run", evil],
        vec!["check", evil],
        vec!["fmt", "--check", evil],
        vec!["check", "no\x1b[31msuch.oxs"],
        vec!["fmt", "no\x1b[31msuch.oxs"],
        vec!["build", "no\u{9b}such.oxs"],
        vec!["run", "no\u{9b}such.oxs"],
        vec!["explain", "\x1b[31m"],
        vec!["new", "bad\x1b]0;x\x07name"],
    ] {
        let o = oxi(&args, &d);
        for b in [&o.stdout, &o.stderr] {
            let t = String::from_utf8_lossy(b);
            assert_eq!(leaks(&t), None, "{args:?}:\n{t:?}");
        }
    }
}

/// R1-04 variant. On Linux `a\b.oxs` (one file, literal backslash) and `a/b.oxs` get the same display
/// name `a/b.oxs` (`replace('\\', "/")`), so the diagnostic dedup key `(file, range, code)` and the
/// `sources` / `files[]` keys collide: one file's error is dropped and attributed to the other.
#[test]
fn r2_01_backslash_file_name_collides_with_subdirectory_file() {
    let d = scratch("bslash");
    std::fs::create_dir_all(d.join("a")).unwrap();
    std::fs::write(d.join("a/b.oxs"), BAD).unwrap();
    std::fs::write(d.join("a\\b.oxs"), BAD).unwrap();
    let o = oxi(&["check", "--json", "."], &d);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let files = v["files"].as_array().unwrap();
    let diag_files: std::collections::HashSet<&str> =
        v["diagnostics"].as_array().unwrap().iter().map(|x| x["file"].as_str().unwrap()).collect();
    assert_eq!(files.len(), 2, "two files checked");
    assert_eq!(
        diag_files.len(),
        2,
        "both files have an OXS-E0005 but their display names collide: files={files:?} diag files={diag_files:?}"
    );
}

/// R1-04 probe: for hostile names `files[]` holds the raw name, every `diagnostics[].file` and
/// `edits[].file` equals an entry of `files[]`, each file has its OXS-E0005 and `file_display` is present
/// exactly when sanitising changes the name (and is itself clean).
#[test]
fn probe_check_json_hostile_names_are_consistent() {
    let d = scratch("json");
    for n in HOSTILE {
        std::fs::write(d.join(n), BAD).unwrap();
    }
    let o = oxi(&["check", "--json", "."], &d);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    let files: Vec<&str> = v["files"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
    for n in HOSTILE {
        assert!(files.contains(&n), "files[] lacks the raw name {n:?}: {files:?}");
        let ds: Vec<&Value> = v["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|x| x["file"].as_str() == Some(n) && x["code"] == "OXS-E0005")
            .collect();
        assert_eq!(ds.len(), 1, "{n:?}: expected one OXS-E0005, diagnostics = {}", v["diagnostics"]);
        let clean = oxiscript_abi::sanitize::sanitize_terminal(n);
        match ds[0].get("file_display").and_then(Value::as_str) {
            Some(fd) => {
                assert_ne!(clean, n);
                assert_eq!(fd, clean);
                assert_eq!(leaks(fd), None);
            }
            None => assert_eq!(clean, n, "{n:?}: file_display missing although the name is unsafe"),
        }
    }
    for x in v["diagnostics"].as_array().unwrap() {
        assert!(files.contains(&x["file"].as_str().unwrap()), "diagnostic file not in files[]: {x}");
        for f in x["fixes"].as_array().into_iter().flatten() {
            for e in f["edits"].as_array().into_iter().flatten() {
                assert!(files.contains(&e["file"].as_str().unwrap()), "edit file not in files[]: {e}");
            }
        }
    }
}

/// R2-01 probe: `--fix` with the colliding names writes each file's own fix only.
#[test]
fn probe_fix_with_colliding_names_touches_only_own_file() {
    let d = scratch("bslash-fix");
    std::fs::create_dir_all(d.join("a")).unwrap();
    // a/b.oxs: `==` (fixable to `===`); a\b.oxs: a missing semicolon (fixable)
    let sub = "export function f(a: i32, b: i32): bool {\n  return a == b;\n}\n";
    let back = "export function g(a: i32): i32 {\n  return a\n}\n";
    std::fs::write(d.join("a/b.oxs"), sub).unwrap();
    std::fs::write(d.join("a\\b.oxs"), back).unwrap();
    let _ = oxi(&["check", "--fix", "."], &d);
    let s = std::fs::read_to_string(d.join("a/b.oxs")).unwrap();
    let b = std::fs::read_to_string(d.join("a\\b.oxs")).unwrap();
    assert!(!s.contains("return a;") && !s.contains("g("), "a/b.oxs got a foreign edit: {s:?}");
    assert!(!b.contains("==") && !b.contains("f("), "a\\b.oxs got a foreign edit: {b:?}");
    assert!(b.contains("return a;"), "own fix missing in a\\b.oxs: {b:?}");
}
