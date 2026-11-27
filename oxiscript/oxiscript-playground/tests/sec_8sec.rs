//! Stage 8.Sec regression tests for `oxiscript-playground` escape sanitising (`[R3-206]`).

use oxiscript_abi::sanitize::is_unsafe_terminal_char;
use oxiscript_playground::check_json;
#[cfg(feature = "run")]
use oxiscript_playground::run::run_json;
use serde_json::Value;

#[test]
fn playground_check_json_sanitises_esc_osc_csi_c1_cr_and_bidi() {
    let src = "export function f(): string { return \"\x1b[2J\x1b]0;pwn\x07\u{009b}31m\u{009d}t\u{009c}\u{202e}\u{2066}\u{2069}\"; }\n";
    let v: Value = serde_json::from_str(&check_json(src)).expect("valid json");
    let diags = v["diagnostics"].as_array().expect("diagnostics");
    assert!(!diags.is_empty());
    for d in diags {
        for field in ["message", "title"] {
            let s = d[field].as_str().unwrap_or("");
            assert!(
                !s.chars().any(is_unsafe_terminal_char),
                "playground check_json leaked unsafe char in {field}: {s:?}"
            );
        }
        for f in d["fixes"].as_array().into_iter().flatten() {
            let t = f["title"].as_str().unwrap_or("");
            assert!(
                !t.chars().any(is_unsafe_terminal_char),
                "playground check_json leaked unsafe char in fix title: {t:?}"
            );
        }
    }
}

#[cfg(feature = "run")]
#[test]
fn playground_run_json_sanitises_guest_log_output() {
    let src = r#"
import { Log } from "host:log";
export function main(log: Log): void {
  log.write(2, "\x1b[2J\x1b]0;pwn\x07\u{9b}31m\r\u{202e}\u{2066}hi\u{2069}");
}
"#;
    let v: Value = serde_json::from_str(&run_json(src, 1_000_000)).expect("valid json");
    assert_eq!(v["ok"], Value::Bool(true), "{v}");
    let lines = v["lines"].as_array().expect("lines");
    assert_eq!(lines.len(), 1);
    let msg = lines[0]["msg"].as_str().expect("msg");
    assert!(
        !msg.chars().any(is_unsafe_terminal_char),
        "playground run_json leaked unsafe char in log line: {msg:?}"
    );
    assert!(msg.contains("\\x1b[2J"), "expected escaped CSI: {msg}");
    assert!(msg.contains("\\x07"), "expected escaped BEL: {msg}");
    assert!(msg.contains("\\x9b31m"), "expected escaped C1 CSI: {msg}");
    assert!(msg.contains("\\x0d"), "expected escaped CR: {msg}");
    assert!(msg.contains("\\u{202e}"), "expected escaped RLO: {msg}");
}

#[cfg(feature = "run")]
#[test]
fn playground_run_json_preserves_zwj_and_zwnj_in_log_lines() {
    let src = r#"
import { Log } from "host:log";
export function main(log: Log): void {
  log.write(2, "family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
  log.write(2, "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}");
}
"#;
    let v: Value = serde_json::from_str(&run_json(src, 1_000_000)).expect("valid json");
    assert_eq!(v["ok"], Value::Bool(true), "{v}");
    let lines = v["lines"].as_array().expect("lines");
    assert_eq!(lines[0]["msg"].as_str(), Some("family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}"));
    assert_eq!(lines[1]["msg"].as_str(), Some("\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}"));
}
