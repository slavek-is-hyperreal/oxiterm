#![allow(clippy::doc_markdown)]
//! BAD ACTOR (run stage): `oxi run` / `oxi build` reproducers. Each test asserts the CORRECT behaviour and therefore FAILS today
//! (#[ignore]d so the suite stays green; run with `cargo test -p oxiscript-cli --test bad_actor_run -- --ignored`).
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxi-ba-run-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn oxi(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_oxi"));
    c.current_dir(dir);
    c
}
const HELLO: &str = "import { Log } from \"host:log\";\nexport function main(log: Log): void { log.write(2, \"hello\"); }\n";

/// R1: the default output name comes from `[package] name` unvalidated: `name = "../../x"` writes outside the project.
#[test]
#[ignore = "reproduces an open BAD ACTOR finding"]
fn build_default_output_stays_inside_cwd() {
    let root = tmp("name");
    let proj = root.join("a/b");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("main.oxs"), HELLO).unwrap();
    std::fs::write(proj.join("oxi.toml"), "[package]\nname = \"../../escape\"\nversion = \"0.1.0\"\nneeds = [\"host:log@1\"]\n").unwrap();
    let st = oxi(&proj).arg("build").status().unwrap();
    assert!(!root.join("escape.wasm").exists(), "oxi build wrote outside the project dir ({st:?})");
}

/// R2: `-o` follows a symlink and overwrites its target (outside the project). `oxi fmt` refuses symlinks; `build` does not.
#[test]
#[ignore = "reproduces an open BAD ACTOR finding"]
fn build_output_does_not_follow_symlink() {
    let root = tmp("sym");
    std::fs::write(root.join("main.oxs"), HELLO).unwrap();
    std::fs::write(root.join("victim"), "KEEP").unwrap();
    std::os::unix::fs::symlink("victim", root.join("out.wasm")).unwrap();
    let _ = oxi(&root).args(["build", "-o", "out.wasm"]).status().unwrap();
    assert_eq!(std::fs::read(root.join("victim")).unwrap(), b"KEEP", "symlink target was overwritten");
}

/// R3: `oxi run` into a closed pipe (`oxi run x | head -1`) or a full device panics (Rust `println!`), exit 101 plus a panic message;
/// the documented exit codes are 0/1/2/4.
#[test]
#[ignore = "reproduces an open BAD ACTOR finding"]
fn run_closed_stdout_is_not_a_panic() {
    let root = tmp("pipe");
    let body: String = (0..300).map(|_| "log.write(2, \"line\");").collect();
    std::fs::write(root.join("main.oxs"), format!("import {{ Log }} from \"host:log\";\nexport function main(log: Log): void {{ {body} }}\n")).unwrap();
    let mut child = oxi(&root).arg("run").stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    drop(child.stdout.take()); // reader goes away at once
    let out = child.wait_with_output().unwrap();
    assert_ne!(out.status.code(), Some(101), "panicked: {}", String::from_utf8_lossy(&out.stderr));
}

/// R4: `--fuel 0` means "default budget" in the browser runner (`fuel >= 1 ? fuel : DEFAULT`) but `oxi run --fuel 0` clamps to 1
/// and traps with out_of_fuel on any program.
#[test]
#[ignore = "reproduces an open BAD ACTOR finding"]
fn run_fuel_zero_matches_browser_meaning() {
    let root = tmp("fuel0");
    std::fs::write(root.join("main.oxs"), HELLO).unwrap();
    let st = oxi(&root).args(["run", "--fuel", "0"]).stdout(Stdio::null()).stderr(Stdio::null()).status().unwrap();
    assert!(st.success() || st.code() == Some(2), "--fuel 0 trapped: {st:?}");
}

/// R1b (strongest form of R1+R3): a repo can ship a symlink at the DEFAULT output name; a plain `oxi build` (no `-o`) overwrites the target.
#[test]
#[ignore = "reproduces an open BAD ACTOR finding"]
fn plain_build_does_not_write_through_a_shipped_symlink() {
    let root = tmp("shipped");
    let proj = root.join("proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("main.oxs"), HELLO).unwrap();
    std::fs::write(proj.join("oxi.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nneeds = [\"host:log@1\"]\n").unwrap();
    std::fs::write(root.join("victim"), "KEEP").unwrap();
    std::os::unix::fs::symlink("../victim", proj.join("demo.wasm")).unwrap();
    let _ = oxi(&proj).arg("build").status().unwrap();
    assert_eq!(std::fs::read(root.join("victim")).unwrap(), b"KEEP", "plain `oxi build` overwrote a file outside the project through demo.wasm");
}
