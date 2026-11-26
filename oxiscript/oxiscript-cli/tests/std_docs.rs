//! Integration test: every ```oxs block in docs/oxiscript/std.md passes `oxi check` and `oxi fmt`,
//! and `oxi run` works end-to-end with `std/...` imports.
#![allow(clippy::pedantic)]

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tmp(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "oxi-std-doc-test-{}-{}-{tag}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
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

#[test]
fn every_oxs_block_in_std_md_checks_and_is_formatted() {
    let path = repo_root().join("docs/oxiscript/std.md");
    let md = std::fs::read_to_string(&path).expect("docs/oxiscript/std.md");
    let mut n = 0;
    let mut lines = md.lines().enumerate();
    while let Some((ln, l)) = lines.next() {
        let Some(info) = l.trim_start().strip_prefix("```") else { continue };
        if info.trim() != "oxs" {
            if !info.trim().is_empty() || l.trim() != "```" {
                for (_, x) in lines.by_ref() {
                    if x.trim_start().starts_with("```") {
                        break;
                    }
                }
            }
            continue;
        }
        let mut body = String::new();
        for (_, x) in lines.by_ref() {
            if x.trim_start().starts_with("```") {
                break;
            }
            body.push_str(x);
            body.push('\n');
        }
        let o = oxi(&["check", "--json", "--stdin", "--stdin-path", "block.oxs"], &tmp("chk"), Some(&body));
        let v: Value = serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e})"));
        assert_eq!(v["ok"], true, "{}:{}: block failed oxi check:\n{v}\n{body}", path.display(), ln + 1);
        let f = oxi(&["fmt", "--stdin"], &tmp("fmt"), Some(&body));
        assert_eq!(
            String::from_utf8_lossy(&f.stdout),
            body,
            "{}:{}: block is not in canonical `oxi fmt` form",
            path.display(),
            ln + 1
        );
        n += 1;
    }
    assert!(n >= 7, "expected at least 7 checked examples in docs/oxiscript/std.md, found {n}");
}

#[test]
fn cli_runs_program_importing_std_and_user_module_with_correct_file_ids() {
    let w = tmp("cli_std");
    std::fs::write(
        w.join("oxi.toml"),
        "[package]\nname = \"stddemo\"\nversion = \"0.1.0\"\nneeds = [\"host:log@1\"]\n",
    )
    .unwrap();
    std::fs::write(
        w.join("util.oxs"),
        "import { sqrt } from \"std/math\";\nexport function hyp(a: f64, b: f64): f64 {\n  return sqrt(a * a + b * b);\n}\n",
    )
    .unwrap();
    std::fs::write(
        w.join("main.oxs"),
        "import { Log } from \"host:log\";\nimport { toFixed } from \"std/string\";\nimport { hyp } from \"./util\";\n\nexport function main(log: Log): void {\n  log.write(2, toFixed(hyp(3.0, 4.0), 1));\n}\n",
    )
    .unwrap();
    let o = oxi(&["run"], &w, None);
    assert_eq!(o.status.code(), Some(0), "{}", String::from_utf8_lossy(&o.stderr));
    assert_eq!(String::from_utf8_lossy(&o.stdout), "5.0\n");

    // Also verify diagnostics in a user file imported AFTER a std module report the right file name.
    std::fs::write(
        w.join("util.oxs"),
        "import { sqrt } from \"std/math\";\nexport function bad(a: i32): bool {\n  return a == 1;\n}\n",
    )
    .unwrap();
    let o2 = oxi(&["check", "--json", "main.oxs"], &w, None);
    let v: Value = serde_json::from_slice(&o2.stdout).unwrap();
    assert_eq!(v["ok"], false);
    assert_eq!(v["diagnostics"][0]["file"], "util.oxs");
    assert_eq!(v["diagnostics"][0]["range"]["start"]["line"], 3);
}
