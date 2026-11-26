//! Stage 8.Sec regression tests for `oxi` CLI terminal-escape sanitising (`[R3-206]`).

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

#[test]
fn cli_check_human_and_json_sanitise_esc_osc_csi_c1_cr_and_bidi() {
    // Hostile source containing ESC/CSI, OSC+BEL, 8-bit C1 CSI/OSC, CR and bidi controls
    // inside both a string literal (triggers OXS-W0003 + echoed snippet) and an invalid char (OXS-E0014).
    let src = concat!(
        "export function f(): i32 {\n",
        "  const s: string = \"\x1b[2J\x1b]0;pwn\x07\u{009b}31m\u{009d}0;t\u{009c}\u{202e}\u{2066}\u{2069}\";\n",
        "  return s == \"x\"; // \r\x1b[31m\u{202e}\n",
        "}\n"
    );

    // Human output (NO_COLOR=1)
    let human = oxi(&["check", "--stdin", "--stdin-path", "hostile.oxs"], &crate_dir(), Some(src));
    let out = String::from_utf8_lossy(&human.stdout);
    let err = String::from_utf8_lossy(&human.stderr);
    assert_no_raw_unsafe_except_lf("human stdout", &out);
    assert_no_raw_unsafe_except_lf("human stderr", &err);
    assert!(out.contains("\\x1b[2J"), "expected escaped CSI in echoed source: {out}");
    assert!(out.contains("\\u{202e}"), "expected escaped RLO in echoed source: {out}");

    // JSON output
    let js = oxi(&["check", "--json", "--stdin", "--stdin-path", "hostile.oxs"], &crate_dir(), Some(src));
    let doc: Value = serde_json::from_slice(&js.stdout).expect("valid json");
    let diags = doc["diagnostics"].as_array().expect("diagnostics array");
    assert!(!diags.is_empty());
    for d in diags {
        let msg = d["message"].as_str().unwrap_or("");
        assert!(!msg.chars().any(is_unsafe_terminal_char), "raw unsafe char in JSON message: {msg:?}");
        if let Some(fixes) = d["fixes"].as_array() {
            for f in fixes {
                let title = f["title"].as_str().unwrap_or("");
                assert!(!title.chars().any(is_unsafe_terminal_char), "raw unsafe char in JSON fix title: {title:?}");
            }
        }
    }
}

#[test]
fn cli_fatal_message_sanitises_hostile_path() {
    let hostile_path = "missing-\x1b[31m-\u{009b}2J-\u{202e}.oxs";
    let human = oxi(&["check", hostile_path], &crate_dir(), None);
    let err = String::from_utf8_lossy(&human.stderr);
    assert_no_raw_unsafe_except_lf("fatal stderr", &err);
    assert!(err.contains("\\x1b[31m"), "expected escaped ESC in fatal: {err}");

    let js = oxi(&["check", "--json", hostile_path], &crate_dir(), None);
    let doc: Value = serde_json::from_slice(&js.stdout).expect("valid json");
    let fatal = doc["fatal"].as_str().expect("fatal string");
    assert!(!fatal.chars().any(is_unsafe_terminal_char), "raw unsafe char in JSON fatal: {fatal:?}");
}
