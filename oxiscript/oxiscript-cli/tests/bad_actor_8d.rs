//! BAD ACTOR 8.D: each test asserts the SAFE / documented behaviour; a failing test is a finding
//! (see HANDOFF-badactor-8d.md). `held_up_*` tests pass and pin behaviour that survived attack.
//! The binary runs as a subprocess with a timeout so a hang or crash cannot hide the other repros.
#![allow(clippy::too_many_lines, clippy::format_collect, clippy::needless_pass_by_value)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

fn tmp(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("oxi-ba8d-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

struct Out {
    code: Option<i32>,
    stdout: String,
    stderr: String,
    secs: f64,
}

fn oxi(args: &[&str], dir: &Path, stdin: Option<&[u8]>, limit_s: u64) -> Out {
    let t0 = Instant::now();
    let mut c = Command::new(env!("CARGO_BIN_EXE_oxi"));
    c.args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    let mut si = child.stdin.take().unwrap();
    let data = stdin.map(<[u8]>::to_vec).unwrap_or_default();
    let w = std::thread::spawn(move || {
        let _ = si.write_all(&data);
    });
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let ro = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = std::io::Read::read_to_end(&mut so, &mut v);
        v
    });
    let re = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = std::io::Read::read_to_end(&mut se, &mut v);
        v
    });
    let code = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st.code();
        }
        if t0.elapsed() > Duration::from_secs(limit_s) {
            let _ = child.kill();
            let _ = child.wait();
            break Some(-999);
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let _ = w.join();
    Out {
        code,
        stdout: String::from_utf8_lossy(&ro.join().unwrap()).into_owned(),
        stderr: String::from_utf8_lossy(&re.join().unwrap()).into_owned(),
        secs: t0.elapsed().as_secs_f64(),
    }
}

fn project(tag: &str, needs: &str, main: &str) -> PathBuf {
    let d = tmp(tag);
    std::fs::write(d.join("oxi.toml"), format!("[package]\nname = \"p\"\nversion = \"0.1.0\"\nneeds = {needs}\n")).unwrap();
    std::fs::write(d.join("main.oxs"), main).unwrap();
    d
}

/// F1: an absolute import specifier escapes the project root (`root.join("/abs")` replaces the root).
#[test]
fn abs_import_escapes_root() {
    let outside = tmp("outside");
    std::fs::write(outside.join("secret.oxs"), "export const Z: i32 = 1 == 2;\n").unwrap();
    let d = project("abs", "[]", &format!("import {{ Z }} from \"{}/secret\";\nconst a: i32 = Z;\n", outside.display()));
    let o = oxi(&["check", "main.oxs", "--json"], &d, None, 20);
    let leaked = o.stdout.contains("secret.oxs") && !o.stdout.contains("cannot resolve import");
    assert!(!leaked, "a file outside the project root was loaded and checked:\n{}", o.stdout);
}

/// F2: `needs` is not enforced: a program may import and use an interface the manifest does not list.
#[test]
fn needs_not_enforced() {
    let d = project(
        "needs",
        "[\"host:log@1\"]",
        "import { Log } from \"host:log\";\nimport { State } from \"host:state\";\nexport function main(log: Log, state: State): void {\n  const s = state.open(\"k\");\n  log.write(2, \"x\");\n}\n",
    );
    let o = oxi(&["check", "main.oxs"], &d, None, 20);
    assert_ne!(o.code, Some(0), "program uses host:state although oxi.toml needs only host:log@1:\n{}", o.stdout);
}

/// F2b: the manifest lists an interface `main` does not take (order/arity of `needs` vs `main` is unchecked).
#[test]
fn needs_vs_main_signature_unchecked() {
    let d = project(
        "needs2",
        "[\"host:log@1\", \"host:state@1\"]",
        "import { Log } from \"host:log\";\nexport function main(log: Log): void {\n  log.write(2, \"x\");\n}\n",
    );
    let o = oxi(&["check", "main.oxs"], &d, None, 20);
    assert_ne!(o.code, Some(0), "`needs` lists host:state@1 but main takes only Log; nothing reported:\n{}", o.stdout);
}

/// F4 (docs route, test corrected): `fmt` refuses files with syntax errors; the docs must say that `oxi check --fix`
/// (not `fmt`) inserts the `;`. The original assertion (fmt exits 0 on a missing `;`) pinned the false docs claim.
#[test]
fn fmt_inserts_semicolon_as_docs_claim() {
    let d = tmp("fmt");
    let o = oxi(&["fmt", "--stdin"], &d, Some(b"const x: i32 = 1\n"), 20);
    assert_eq!(o.code, Some(1), "fmt must refuse a file with a syntax error: {}", o.stderr);
    assert!(o.stderr.contains("check --fix"), "fmt's refusal does not point at `oxi check --fix`: {}", o.stderr);
    std::fs::write(d.join("a.oxs"), "const x: i32 = 1\n").unwrap();
    let _ = oxi(&["check", "a.oxs", "--fix"], &d, None, 20);
    assert_eq!(std::fs::read_to_string(d.join("a.oxs")).unwrap(), "const x: i32 = 1;\n");
    let docs = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/templates/common/AGENTS.md")).unwrap();
    assert!(!docs.contains("`oxi fmt` and `oxi check --fix` insert"), "AGENTS.md still claims fmt inserts the `;`");
}

/// F10: fmt rewrites the text of a comment (`///` becomes `//`) when a plain comment sits between it and the item.
#[test]
fn fmt_keeps_comment_text() {
    let d = tmp("cmt");
    let src = "/// doc\n// x\nfunction f(): void {}\n";
    let o = oxi(&["fmt", "--stdin"], &d, Some(src.as_bytes()), 20);
    assert!(o.stdout.starts_with("/// doc"), "comment text changed:\n{}", o.stdout);
}

/// F6: with more than ~200 syntax errors `--fix` does nothing and says nothing (the diagnostic cap defeats the
/// convergence guard, which compares two capped counts).
#[test]
fn fix_many_errors_makes_progress() {
    let d = tmp("many");
    let src: String = (0..1000).map(|i| format!("const x{i}: i32 = 1\n")).collect();
    std::fs::write(d.join("m.oxs"), &src).unwrap();
    let o = oxi(&["check", "m.oxs", "--fix"], &d, None, 60);
    let after = std::fs::read_to_string(d.join("m.oxs")).unwrap();
    assert!(after != src, "--fix left a 1000-error file untouched without a message: {}{}", o.stdout, o.stderr);
}

/// F7: loading project IDL is quadratic in the number of interfaces (`hosts.retain` per interface).
#[test]
fn idl_load_is_linear() {
    let time_for = |n: usize| {
        let d = project("idl", "[]", "export function f(): i32 { return 1; }\n");
        std::fs::create_dir_all(d.join("idl")).unwrap();
        let idl: String = (0..n).map(|i| format!("interface A{i} \"host:m{i}\" @1 {{ fn f(); }}\n")).collect();
        std::fs::write(d.join("idl/x.idl"), idl).unwrap();
        oxi(&["check", "main.oxs"], &d, None, 120).secs
    };
    let a = time_for(10_000);
    let b = time_for(20_000);
    assert!(b < a * 3.0 + 0.2, "10k interfaces {a:.2}s, 20k interfaces {b:.2}s (x{:.1}): quadratic", b / a);
}

/// F8: a symlink loop (`loop -> .`) makes the walker visit the same file 41 times (and with a second link
/// it dies with ELOOP, exit 2); `fmt`/`--fix` would rewrite the same file repeatedly.
#[test]
#[cfg(unix)]
fn symlink_loop_does_not_duplicate_files() {
    let d = project("loop", "[]", "export function f(): i32 { return 1; }\n");
    std::os::unix::fs::symlink(".", d.join("loop")).unwrap();
    let o = oxi(&["check", ".", "--json"], &d, None, 30);
    let v: serde_json::Value = serde_json::from_str(&o.stdout).expect("json");
    assert_eq!(v["files"].as_array().unwrap().len(), 1, "files: {}", v["files"]);
}

#[test]
fn invalid_utf8_file_does_not_abort_project() {
    let d = project("utf8", "[]", "export function f(): i32 { return 1; }\n");
    std::fs::write(d.join("bad.oxs"), b"const x = \"\xff\";\n").unwrap();
    let o = oxi(&["check", "."], &d, None, 30);
    assert_ne!(o.code, Some(2), "{}", o.stderr);
}

/// F5: `--fix` writes through a symlink whose NAME ends in `.oxs` to ANY file (here `victim.txt`).
#[test]
#[cfg(unix)]
fn fix_does_not_write_through_symlink_outside_project() {
    let outside = tmp("symout");
    std::fs::write(outside.join("victim.txt"), "const a = 1\nconst b = 2\n").unwrap();
    let d = project("symin", "[]", "export function f(): i32 { return 1; }\n");
    std::os::unix::fs::symlink(outside.join("victim.txt"), d.join("evil.oxs")).unwrap();
    let _ = oxi(&["check", ".", "--fix"], &d, None, 30);
    let after = std::fs::read_to_string(outside.join("victim.txt")).unwrap();
    assert_eq!(after, "const a = 1\nconst b = 2\n", "--fix modified a non-.oxs file outside the project through a symlink");
}

/// F1b: a package-style specifier is not normalised, so `idl2/../../outside/x` leaves the root.
#[test]
fn pkg_spec_dotdot_escapes_root() {
    let base = tmp("pkgdots");
    let outside = base.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("bad2.oxs"), "export const Z: i32 = 1 == 2;\n").unwrap();
    let d = base.join("proj");
    std::fs::create_dir_all(d.join("sub")).unwrap();
    std::fs::write(d.join("oxi.toml"), "[package]\nname = \"p\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(d.join("main.oxs"), "import { Z } from \"sub/../../outside/bad2\";\nconst a: i32 = Z;\n").unwrap();
    let o = oxi(&["check", "main.oxs"], &d, None, 20);
    assert!(o.stdout.contains("cannot resolve import"), "package spec with `..` escaped the root:\n{}", o.stdout);
}

/// Run `oxi check .` under a 1 GiB address-space cap and a 10 s limit.
fn capped_check(d: &Path) -> Out {
    let t0 = Instant::now();
    let mut child = Command::new("sh")
        .args(["-c", "ulimit -v 1048576; exec \"$0\" check .", env!("CARGO_BIN_EXE_oxi")])
        .current_dir(d)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let code = loop {
        if let Some(st) = child.try_wait().unwrap() {
            break st.code();
        }
        if t0.elapsed() > Duration::from_secs(10) {
            let _ = child.kill();
            let _ = child.wait();
            break Some(-999);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Out { code, stdout: String::new(), stderr: String::new(), secs: t0.elapsed().as_secs_f64() }
}

/// F15: a symlink named `*.oxs` / `*.idl` to `/dev/zero` is read with an unbounded `read_to_string`
/// (memory grows until the cap or the OOM killer; capped here, the run ends in "out of memory").
#[test]
#[cfg(unix)]
fn special_file_behind_symlink_is_not_slurped() {
    for (rel, dir) in [("z.oxs", ""), ("idl/z.idl", "idl")] {
        let d = project("zero", "[]", "export function f(): i32 { return 1; }\n");
        if !dir.is_empty() {
            std::fs::create_dir_all(d.join(dir)).unwrap();
        }
        std::os::unix::fs::symlink("/dev/zero", d.join(rel)).unwrap();
        let o = capped_check(&d);
        // safe behaviour: skip non-regular files (exit 0) or refuse with a message that is not an allocation failure
        assert!(o.code == Some(0), "{rel}: exit {:?} after {:.1}s (expected the special file to be skipped)", o.code, o.secs);
    }
}

/// F15b: a FIFO named `x.oxs` makes `oxi check .` block forever (and the editor's check until its 10 s kill).
#[test]
#[cfg(unix)]
fn fifo_named_oxs_does_not_hang() {
    let d = project("fifo", "[]", "export function f(): i32 { return 1; }\n");
    let st = Command::new("mkfifo").arg(d.join("x.oxs")).status().unwrap();
    assert!(st.success());
    let o = capped_check(&d);
    assert_ne!(o.code, Some(-999), "hung for {:.0}s on a FIFO", o.secs);
}

/// F16: `oxi.toml` discovery has no boundary: a broken (or planted) `oxi.toml` / `idl/` in any ancestor, e.g. a
/// world-writable `/tmp` or `$HOME`, governs a nested project that has its own `.git`.
#[test]
fn ancestor_manifest_does_not_cross_a_repo_boundary() {
    let base = tmp("anc");
    std::fs::write(base.join("oxi.toml"), "this is = not toml [\n").unwrap();
    let repo = base.join("repo");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join("x.oxs"), "export function f(): i32 { return 1; }\n").unwrap();
    let o = oxi(&["check", "x.oxs"], &repo, None, 20);
    assert_eq!(o.code, Some(0), "ancestor manifest above the repo root was used: {}", o.stderr);
}

/// F17 (info): `--stdin --fix` accepts `--fix` and silently ignores it.
#[test]
fn stdin_fix_is_not_silently_ignored() {
    let d = tmp("stdinfix");
    let o = oxi(&["check", "--stdin", "--fix"], &d, Some(b"const x: i32 = 1\n"), 20);
    assert!(o.stderr.contains("--fix") || o.code == Some(2), "no notice that --fix does nothing with --stdin: {}{}", o.stdout, o.stderr);
}

// ---- held up ----

#[test]
fn held_up_deep_nesting_is_a_diagnostic_not_a_crash() {
    let d = tmp("deep");
    for src in [
        format!("const x = {}1{};\n", "(".repeat(200_000), ")".repeat(200_000)),
        format!("const x = {}1;\n", "-".repeat(500_000)),
        format!("function f(): void {}{}\n", "{".repeat(100_000), "}".repeat(100_000)),
        format!("type T = {}i32{};\n", "Array<".repeat(100_000), ">".repeat(100_000)),
    ] {
        for cmd in [&["check", "--stdin", "--json"][..], &["fmt", "--stdin"][..]] {
            let o = oxi(cmd, &d, Some(src.as_bytes()), 20);
            assert!(matches!(o.code, Some(1)), "{cmd:?}: code {:?} after {:.1}s", o.code, o.secs);
        }
    }
}

#[test]
fn held_up_new_rejects_hostile_names() {
    let d = tmp("new");
    for n in ["../x", ".", "a/b", "A", "a b", "", "-x"] {
        let o = oxi(&["new", n], &d, None, 20);
        assert_eq!(o.code, Some(2), "name {n:?}");
    }
    std::fs::create_dir(d.join("exist")).unwrap();
    std::fs::write(d.join("exist/main.oxs"), "keep").unwrap();
    assert_eq!(oxi(&["new", "exist"], &d, None, 20).code, Some(2));
    assert_eq!(std::fs::read_to_string(d.join("exist/main.oxs")).unwrap(), "keep");
}

#[test]
fn held_up_json_valid_for_invalid_utf8_nul_and_astral() {
    let d = tmp("json");
    for input in [&b"const x = \"\xff\";\n"[..], b"const x: i32 = 1;\0\n", "const s = \"😀e\u{301}\"; const x: i32 = 1 == 2;\n".as_bytes()] {
        let o = oxi(&["check", "--stdin", "--json"], &d, Some(input), 20);
        let v: serde_json::Value = serde_json::from_str(&o.stdout).expect("valid JSON");
        assert_eq!(v["schema"], 1);
    }
    let o = oxi(&["check", "--stdin", "--json"], &d, Some("const s = \"😀\"; const x: i32 = 1 == 2;\n".as_bytes()), 20);
    let v: serde_json::Value = serde_json::from_str(&o.stdout).unwrap();
    let st = &v["diagnostics"][0]["range"]["start"];
    assert_eq!(st["utf16"].as_u64().unwrap(), st["col"].as_u64().unwrap() + 1, "one astral char = +1 utf16 unit");
}
