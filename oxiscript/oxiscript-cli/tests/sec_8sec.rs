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

#[test]
fn cli_explain_and_new_sanitise_hostile_arguments() {
    let hostile = "OXS-\x1b]0;PWN\x07\u{009b}31m\u{202e}";
    let ex = oxi(&["explain", hostile], &crate_dir(), None);
    assert_eq!(ex.status.code(), Some(2));
    assert_no_raw_unsafe_except_lf("explain stdout", &String::from_utf8_lossy(&ex.stdout));
    assert_no_raw_unsafe_except_lf("explain stderr", &String::from_utf8_lossy(&ex.stderr));

    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let d = base.join(format!("oxi-sec-new-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    for args in [
        &["new", hostile][..],
        &["new", "okproj", "--template", hostile][..],
    ] {
        let o = oxi(args, &d, None);
        assert_eq!(o.status.code(), Some(2));
        assert_no_raw_unsafe_except_lf(&format!("new {args:?} stdout"), &String::from_utf8_lossy(&o.stdout));
        assert_no_raw_unsafe_except_lf(&format!("new {args:?} stderr"), &String::from_utf8_lossy(&o.stderr));
    }
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cli_check_json_emits_file_display_and_fix_works_on_special_filename() {
    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let d = base.join(format!("oxi-sec-r104-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let name = "soft\u{ad}hy.oxs";
    std::fs::write(d.join(name), "export function f(a: i32, b: i32): bool {\n  return a == b;\n}\n").unwrap();

    let js = oxi(&["check", "--json", name], &d, None);
    let doc: Value = serde_json::from_slice(&js.stdout).expect("valid json");
    assert_eq!(doc["files"][0].as_str(), Some(name));
    assert_eq!(doc["diagnostics"][0]["file"].as_str(), Some(name));
    assert_eq!(doc["diagnostics"][0]["file_display"].as_str(), Some("soft\\xadhy.oxs"));
    assert_eq!(doc["diagnostics"][0]["fixes"][0]["edits"][0]["file"].as_str(), Some(name));

    // Human output escapes the file name while still printing the source snippet.
    let human = oxi(&["check", name], &d, None);
    let hout = String::from_utf8_lossy(&human.stdout);
    assert_no_raw_unsafe_except_lf("human stdout", &hout);
    assert!(hout.contains("soft\\xadhy.oxs"), "{hout}");
    assert!(hout.contains("return a == b;"), "{hout}");

    // `--fix` must match `e.file == own` and repair the file.
    let fx = oxi(&["check", "--fix", name], &d, None);
    assert_eq!(fx.status.code(), Some(0), "{}", String::from_utf8_lossy(&fx.stdout));
    assert!(std::fs::read_to_string(d.join(name)).unwrap().contains("return a === b;"));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cli_run_preserves_zwj_zwnj_while_escaping_controls() {
    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let d = base.join(format!("oxi-sec-r105-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let src = "import { Log } from \"host:log\";\nexport function main(log: Log): void {\n  log.write(2, \"family \\u{1F468}\\u{200D}\\u{1F469}\\u{200D}\\u{1F467} \\u{0645}\\u{06CC}\\u{200C}\\u{062E}\\u{0648}\\u{0627}\\u{0647}\\u{0645}\");\n  log.write(2, \"esc \\x1b[31m \\u{202e}\");\n}\n";
    std::fs::write(d.join("main.oxs"), src).unwrap();
    let o = oxi(&["run", "main.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"), "{out:?}");
    assert!(out.contains("\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}"), "{out:?}");
    assert!(out.contains("\\x1b[31m"), "{out:?}");
    assert!(out.contains("\\u{202e}"), "{out:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn cli_fix_only_semicolons_skips_line_before_template_literal() {
    let src = "function f(): void {\n  const a = b\n  `t`;\n  const c = 1\n}\n";
    let o = oxi(&["check", "--stdin", "--fix-only", "semicolons"], &crate_dir(), Some(src));
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("const a = b\n  `t`;"), "must not insert `;` before template literal:\n{out}");
    assert!(out.contains("const c = 1;\n"), "must still fix safe missing `;`:\n{out}");
}
