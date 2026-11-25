//! The semicolon assistant: `oxi check --fix-only semicolons` (what the VS Code extension runs on save).

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn tmp(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("oxi-semi-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn oxi(args: &[&str], dir: &std::path::Path, stdin: &str) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_oxi"))
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(stdin.as_bytes()).unwrap();
    c.wait_with_output().unwrap()
}

fn fix_stdin(d: &std::path::Path, src: &str) -> String {
    let o = oxi(&["check", "--stdin", "--fix-only", "semicolons"], d, src);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8(o.stdout).unwrap()
}

const TS_STYLE: &str = "import { Log } from \"host:log\"
const a: i32 = 1
let b = a +
  2
function f(x: i32): i32 {
  const y = x * (
    2 + 3
  )
  let i = 0
  i++
  if (y > 1) { return y }
  return y
}
const g = (n: i32): i32 => {
  return n
}
const s = `a
b`
type T = { kind: \"a\" } | { kind: \"b\" }
export function main(log: Log): void {
  log.write(2, \"x\")
}
";

#[test]
fn ts_style_code_is_valid_after_one_pass() {
    let d = tmp("ts");
    let out = fix_stdin(&d, TS_STYLE);
    std::fs::write(d.join("out.oxs"), &out).unwrap();
    let o = oxi(&["check", "out.oxs"], &d, "");
    assert_eq!(o.status.code(), Some(0), "{}\n{out}", String::from_utf8_lossy(&o.stdout));
    // multi-line expressions: the `;` goes at the end of the statement, never inside it
    assert!(out.contains("let b = a +\n  2;\n"), "{out}");
    assert!(out.contains("x * (\n    2 + 3\n  );\n"), "{out}");
    assert!(!out.contains(" +;") && !out.contains("(;"), "{out}");
    // a second pass changes nothing
    assert_eq!(fix_stdin(&d, &out), out);
}

#[test]
fn valid_files_are_not_touched() {
    let d = tmp("valid");
    let src = "export function f(): i32 {\r\n  return 1;\r\n}\r\n";
    assert_eq!(fix_stdin(&d, src), src);
    std::fs::write(d.join("v.oxs"), src).unwrap();
    let before = std::fs::metadata(d.join("v.oxs")).unwrap().modified().unwrap();
    let o = oxi(&["check", "v.oxs", "--fix-only", "semicolons"], &d, "");
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(d.join("v.oxs")).unwrap(), src);
    assert_eq!(std::fs::metadata(d.join("v.oxs")).unwrap().modified().unwrap(), before);
}

#[test]
fn only_semicolons_and_never_where_it_changes_meaning() {
    let d = tmp("only");
    // type error stays; `a\n[1]` reads as `a[1]` (no `;` after `[1]`); throw lines stay
    let src = "const c: i32 = 1\n[1]\nconst z: i32 = \"a\"\nfunction f(): void { throw 1 }\nlet k = 2 +\n";
    let out = fix_stdin(&d, src);
    assert!(out.contains("const z: i32 = \"a\";"), "{out}");
    assert!(out.contains("1\n[1]\n"), "{out}");
    assert!(out.contains("{ throw 1 }"), "{out}");
    // non-semicolon problems are not fixed silently: `===` is a machine-applicable fix for `--fix`, not here
    let out = fix_stdin(&d, "const a: i32 = 1\nconst b: bool = a == 1\n");
    assert!(out.contains("a == 1"), "{out}");
    assert!(out.contains("= 1;\n"), "{out}");
}

#[test]
fn file_mode_writes_atomically_and_flag_conflicts_are_usage_errors() {
    let d = tmp("file");
    std::fs::write(d.join("m.oxs"), "const a: i32 = 1\nconst b: i32 = 2\n").unwrap();
    let o = oxi(&["check", "m.oxs", "--fix-only", "semicolons"], &d, "");
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stdout));
    assert_eq!(std::fs::read_to_string(d.join("m.oxs")).unwrap(), "const a: i32 = 1;\nconst b: i32 = 2;\n");
    for args in [&["check", "--fix-only", "semicolons", "--fix"][..], &["check", "--fix-only", "semicolons", "--json"], &["check", "--fix-only", "bogus"]] {
        assert_eq!(oxi(args, &d, "").status.code(), Some(2), "{args:?}");
    }
}
