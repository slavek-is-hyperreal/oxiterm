//! Integration tests: run the real `oxi` binary.
#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::format_push_string,
    clippy::doc_markdown,
    clippy::needless_pass_by_value
)]

use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    crate_dir().join("../..")
}

fn tmp(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("oxi-cli-test-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
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

fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}

fn json(o: &Output) -> Value {
    serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("not JSON ({e}): {}", text(o)))
}

fn codes(v: &Value) -> Vec<String> {
    v["diagnostics"].as_array().unwrap().iter().map(|d| d["code"].as_str().unwrap().to_string()).collect()
}

#[test]
fn version() {
    let o = oxi(&["--version"], &crate_dir(), None);
    assert!(o.status.success());
    assert!(text(&o).starts_with("oxi "), "{}", text(&o));
}

// ---------------------------------------------------------------- JSON schema

const BAD: &str = "const a: i32 = 1;\nexport function f(x: i32): bool {\n  return x == 2;\n}\nfunction g(a: i32): f64 {\n  return a;\n}\n";

#[test]
fn json_schema_snapshot() {
    let d = tmp("snap");
    std::fs::write(d.join("bad.oxs"), BAD).unwrap();
    let o = oxi(&["check", "--json", "bad.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(1));
    let actual = String::from_utf8(o.stdout).unwrap();
    // The snapshot is of the syntax-error wave; a second run after fixing shows the type error.
    let snap = crate_dir().join("tests/snapshots/check_bad_syntax.json");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&snap, &actual).unwrap();
    }
    let want = std::fs::read_to_string(&snap).expect("snapshot (run with UPDATE_SNAPSHOTS=1)");
    assert_eq!(actual.trim_end(), want.trim_end());

    std::fs::write(d.join("bad.oxs"), BAD.replace("==", "===")).unwrap();
    let o = oxi(&["check", "--json", "bad.oxs"], &d, None);
    let actual = String::from_utf8(o.stdout).unwrap();
    let snap = crate_dir().join("tests/snapshots/check_bad_types.json");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        std::fs::write(&snap, &actual).unwrap();
    }
    let want = std::fs::read_to_string(&snap).expect("snapshot");
    assert_eq!(actual.trim_end(), want.trim_end());
}

#[test]
fn json_schema_shape() {
    let d = tmp("shape");
    std::fs::write(d.join("bad.oxs"), BAD).unwrap();
    let v = json(&oxi(&["check", "--json", "bad.oxs"], &d, None));
    assert_eq!(v["schema"], 1);
    assert_eq!(v["tool"], "oxi");
    assert_eq!(v["ok"], false);
    assert!(v["fatal"].is_null());
    assert_eq!(v["summary"]["errors"], 1);
    let dg = &v["diagnostics"][0];
    assert_eq!(dg["code"], "OXS-E0005");
    assert_eq!(dg["severity"], "error");
    assert_eq!(dg["file"], "bad.oxs");
    for k in ["byte", "line", "col", "utf16"] {
        assert!(dg["range"]["start"][k].is_u64(), "{k}");
        assert!(dg["range"]["end"][k].is_u64(), "{k}");
    }
    assert_eq!(dg["range"]["start"]["line"], 3);
    let fix = &dg["fixes"][0];
    assert_eq!(fix["applicability"], "machine-applicable");
    assert_eq!(fix["edits"][0]["new_text"], "===");
    assert_eq!(fix["edits"][0]["file"], "bad.oxs");
    // clean program
    std::fs::write(d.join("ok.oxs"), "export function f(): i32 { return 1; }\n").unwrap();
    let o = oxi(&["check", "--json", "ok.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(json(&o)["ok"], true);
}

#[test]
fn utf16_columns_and_bytes() {
    // an emoji (4 bytes, 2 UTF-16 units, 1 char) before the error on the same line
    let src = "export function f(a: i32): bool { return \"\u{1F600}\" === \"x\" || a == 1; }\n";
    let o = oxi(&["check", "--json", "--stdin", "--stdin-path", "u.oxs"], &tmp("u16"), Some(src));
    let v = json(&o);
    let r = &v["diagnostics"][0]["range"]["start"];
    let byte = src.find("a == 1").unwrap() as u64 + 2;
    let (col, u16c) = {
        let before = &src[..byte as usize];
        (before.chars().count() as u64 + 1, before.encode_utf16().count() as u64 + 1)
    };
    assert_eq!(r["byte"], byte);
    assert_eq!(r["col"], col);
    assert_eq!(r["utf16"], u16c);
    assert_eq!(u16c, col + 1);
}

#[test]
fn check_exit_codes_and_missing_path() {
    let d = tmp("exit");
    let o = oxi(&["check", "nope.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(2));
    let o = oxi(&["check", "--json", "nope.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(json(&o)["ok"], false);
    assert!(json(&o)["fatal"].is_string());
    // warnings alone do not fail
    std::fs::write(d.join("w.oxs"), "export function f(): void { 1 + 2; }\n").unwrap();
    let o = oxi(&["check", "--json", "w.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(0));
    let v = json(&o);
    assert_eq!(v["summary"]["warnings"], 1, "{v}");
    assert_eq!(v["summary"]["errors"], 0);
    assert_eq!(v["ok"], true);
    assert_eq!(codes(&v), vec!["OXS-W0004".to_string()]);
}

#[test]
fn human_output_has_span_and_code() {
    let d = tmp("human");
    std::fs::write(d.join("bad.oxs"), BAD).unwrap();
    let o = oxi(&["check", "bad.oxs"], &d, None);
    let t = text(&o);
    assert!(t.contains("error[OXS-E0005]"), "{t}");
    assert!(t.contains("--> bad.oxs:3:12"), "{t}");
    assert!(t.contains("return x == 2;"), "{t}");
    assert!(t.contains("^^"), "{t}");
    assert!(t.contains("oxi explain OXS-E0005"), "{t}");
}

// ---------------------------------------------------------------- imports, IDL

#[test]
fn imported_module_diagnostics_use_their_own_file_and_lines() {
    let d = tmp("imports");
    std::fs::write(d.join("oxi.toml"), "[package]\nname = \"p\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(d.join("util.oxs"), "\n\n// pad\nexport function f(a: i32): bool { return a == 1; }\n").unwrap();
    std::fs::write(d.join("main.oxs"), "import { f } from \"./util\";\nexport function g(): bool { return f(1); }\n").unwrap();
    let o = oxi(&["check", "--json", "main.oxs"], &d, None);
    let v = json(&o);
    let dg = &v["diagnostics"][0];
    assert_eq!(dg["file"], "util.oxs");
    assert_eq!(dg["range"]["start"]["line"], 4);
    // a missing import is reported against the importing file
    std::fs::write(d.join("main.oxs"), "import { f } from \"./nope\";\n").unwrap();
    let v = json(&oxi(&["check", "--json", "main.oxs"], &d, None));
    assert!(codes(&v).contains(&"OXS-E0302".to_string()), "{v}");
}

#[test]
fn project_idl_is_resolved_and_errors_are_reported() {
    let d = tmp("idl");
    std::fs::write(d.join("oxi.toml"), "[package]\nname = \"p\"\nversion = \"0.1.0\"\nneeds = [\"host:log@1\", \"host:beep@1\"]\n").unwrap();
    std::fs::create_dir_all(d.join("idl")).unwrap();
    std::fs::write(d.join("idl/beep.idl"), "interface Beep \"host:beep\" @1 { fn beep(ms: u32); }\n").unwrap();
    std::fs::write(
        d.join("main.oxs"),
        "import { Log } from \"host:log\";\nimport { Beep } from \"host:beep\";\nexport function main(log: Log, b: Beep): void { b.beep(10u32); }\n",
    )
    .unwrap();
    let o = oxi(&["check", "main.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    // a broken IDL is a fatal error with a path and position, never a panic
    std::fs::write(d.join("idl/beep.idl"), "interface Beep \"host:beep\" @1 { fn beep(ms u32); }\n").unwrap();
    let o = oxi(&["check", "main.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).contains("beep.idl:1:"), "{}", text(&o));
    // needs naming an undeclared interface is a fatal error
    std::fs::write(d.join("idl/beep.idl"), "interface Beep \"host:beep\" @1 { fn beep(ms: u32); }\n").unwrap();
    std::fs::write(d.join("oxi.toml"), "[package]\nname = \"p\"\nversion = \"0.1.0\"\nneeds = [\"host:nothing@1\"]\n").unwrap();
    let o = oxi(&["check", "main.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).contains("host:nothing@1"), "{}", text(&o));
    // a project IDL replaces the built-in interface of the same module
    std::fs::write(d.join("oxi.toml"), "[package]\nname = \"p\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(d.join("idl/log.idl"), "interface Log \"host:log\" @1 { fn write(level: u8, msg: str); fn flush(); }\n").unwrap();
    std::fs::write(d.join("main.oxs"), "import { Log } from \"host:log\";\nexport function main(log: Log): void { log.flush(); }\n").unwrap();
    assert_eq!(oxi(&["check", "main.oxs"], &d, None).status.code(), Some(0));
}

// ---------------------------------------------------------------- fix

#[test]
fn machine_applicable_fixes_apply_together_and_clear_their_codes() {
    let d = tmp("fix");
    let cases = [
        "export function f(a: i32): bool { return a == 1; }\n",
        "const a: i32 = 1\nconst b: i32 = 2;\n",
        "export function f(a: i32, b: i32, c: i32): bool { return a & b === c; }\n",
        "export function f(a: u32, b: u32): u32 { return a << b + 1u32; }\n",
        "export function f(a: i32 | null, b: i32): i32 { return a ?? b || 0; }\n",
        "const a = 1f32;\n",
        "export function f(a: i32, b: i32): bool { return a == 1 && b == 2; }\n",
    ];
    for (i, c) in cases.iter().enumerate() {
        let f = d.join(format!("c{i}.oxs"));
        std::fs::write(&f, c).unwrap();
        let before = json(&oxi(&["check", "--json", f.to_str().unwrap()], &d, None));
        assert!(!codes(&before).is_empty(), "case {i} should fail: {c}");
        // 1: all machine-applicable edits of one run apply as one set without overlap
        let mut edits: Vec<(usize, usize, String)> = Vec::new();
        for dg in before["diagnostics"].as_array().unwrap() {
            for fx in dg["fixes"].as_array().unwrap() {
                if fx["applicability"] == "machine-applicable" {
                    for e in fx["edits"].as_array().unwrap() {
                        edits.push((e["range"]["start"]["byte"].as_u64().unwrap() as usize, e["range"]["end"]["byte"].as_u64().unwrap() as usize, e["new_text"].as_str().unwrap().to_string()));
                    }
                }
            }
        }
        edits.sort();
        for w in edits.windows(2) {
            assert!(w[0].1 <= w[1].0 || (w[0].0 == w[0].1 && w[1].0 == w[1].1 && w[0].0 != w[1].0), "case {i}: overlapping edits {edits:?}");
        }
        // 2: --fix leaves a program without syntax errors
        let o = oxi(&["check", "--fix", "--json", f.to_str().unwrap()], &d, None);
        let after = json(&o);
        for code in codes(&after) {
            let n: u32 = code[5..].parse().unwrap();
            assert!(n >= 100, "case {i}: syntax code {code} survived --fix: {after}");
        }
        // 3: idempotent
        let again = std::fs::read_to_string(&f).unwrap();
        oxi(&["check", "--fix", f.to_str().unwrap()], &d, None);
        assert_eq!(again, std::fs::read_to_string(&f).unwrap(), "case {i}");
    }
}

#[test]
fn alternatives_and_check_phase_fixes_are_maybe_incorrect() {
    let src = "export function f(a: i32): f64 { return a; }\nexport function g(x: u8): u8 { return x as u8 | 1; }\n";
    let v = json(&oxi(&["check", "--json", "--stdin", "--stdin-path", "a.oxs"], &tmp("alt"), Some(src)));
    for dg in v["diagnostics"].as_array().unwrap() {
        for fx in dg["fixes"].as_array().unwrap() {
            if !fx["edits"].as_array().unwrap().is_empty() {
                assert_eq!(fx["applicability"], "maybe-incorrect", "{dg}");
            }
        }
    }
    let src2 = "export function f(a: i32): f64 { return a; }\n";
    let v = json(&oxi(&["check", "--json", "--stdin", "--stdin-path", "a.oxs"], &tmp("alt2"), Some(src2)));
    assert_eq!(v["diagnostics"][0]["code"], "OXS-E0100");
    assert_eq!(v["diagnostics"][0]["fixes"][0]["applicability"], "maybe-incorrect");
}

#[test]
fn hints_for_typescript_habits() {
    let t = |src: &str| json(&oxi(&["check", "--json", "--stdin", "--stdin-path", "h.oxs"], &tmp("hint"), Some(src)));
    let v = t("export function f(x: number): i32 { return 1; }\n");
    assert_eq!(v["diagnostics"][0]["code"], "OXS-E0300");
    assert_eq!(v["diagnostics"][0]["fixes"].as_array().unwrap().len(), 2);
    assert_eq!(v["diagnostics"][0]["fixes"][0]["edits"][0]["new_text"], "i32");
    let v = t("export function f(s: string): i32 { return s.length; }\n");
    assert_eq!(v["diagnostics"][0]["code"], "OXS-E0108");
    assert_eq!(v["diagnostics"][0]["fixes"][0]["edits"][0]["new_text"], "byteLength");
    // exceptions must not get a machine-applicable ";" fix, wherever on the line the keyword is
    for src in [
        "export function f(): void {\n  throw \"x\";\n}\n",
        "export function f(a: i32): void {\n  if (a > 0) throw \"y\";\n}\n",
        "export function f(): void {\n  try {\n    const a = 1;\n  }\n  catch (e) {\n    const b = 2;\n  }\n}\n",
        "export function f(): void {\n  try { const a: i32 = 1; }\n  } catch (e) { const b: i32 = 2; }\n}\n",
    ] {
        let v = t(src);
        let mut saw_exception_error = false;
        for dg in v["diagnostics"].as_array().unwrap() {
            let line = dg["range"]["start"]["line"].as_u64().unwrap() as usize;
            let text_line = src.lines().nth(line - 1).unwrap();
            let on_exc = ["throw", "try", "catch", "finally"].iter().any(|w| text_line.contains(w));
            for fx in dg["fixes"].as_array().unwrap() {
                if on_exc && dg["code"] == "OXS-E0004" {
                    saw_exception_error = true;
                    assert_ne!(fx["applicability"], "machine-applicable", "{src}\n{dg}");
                }
            }
        }
        assert!(saw_exception_error, "expected an E0004 on an exception line: {src}\n{v}");
    }
    // `for...in` -> `of` changes meaning (keys become values): never machine-applicable
    let v = t("export function f(xs: i32[]): i32 {\n  let t: i32 = 0;\n  for (const i in xs) { t += xs[i]; }\n  return t;\n}\n");
    assert_eq!(v["diagnostics"][0]["code"], "OXS-E0001");
    assert_eq!(v["diagnostics"][0]["fixes"][0]["applicability"], "maybe-incorrect");
}

// ---------------------------------------------------------------- fmt

fn corpus_files() -> Vec<PathBuf> {
    let dir = crate_dir().join("../oxiscript-syntax/tests/corpus/valid");
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "oxs")).collect();
    v.sort();
    assert!(v.len() >= 40);
    v
}

#[test]
fn fmt_is_idempotent_on_the_corpus() {
    let d = tmp("fmt");
    for (i, f) in corpus_files().iter().enumerate() {
        std::fs::copy(f, d.join(format!("f{i}.oxs"))).unwrap();
    }
    let o = oxi(&["fmt", "."], &d, None);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    let snapshot: Vec<String> = (0..corpus_files().len()).map(|i| std::fs::read_to_string(d.join(format!("f{i}.oxs"))).unwrap()).collect();
    let o = oxi(&["fmt", "--check", "."], &d, None);
    assert_eq!(o.status.code(), Some(0), "second run must be clean: {}", text(&o));
    oxi(&["fmt", "."], &d, None);
    for (i, s) in snapshot.iter().enumerate() {
        assert_eq!(s, &std::fs::read_to_string(d.join(format!("f{i}.oxs"))).unwrap(), "file {i}");
    }
}

#[test]
fn fmt_check_and_stdin() {
    let d = tmp("fmt2");
    std::fs::write(d.join("a.oxs"), "let   x=1 ;").unwrap();
    let o = oxi(&["fmt", "--check", "a.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(d.join("a.oxs")).unwrap(), "let   x=1 ;", "--check must not write");
    let o = oxi(&["fmt", "--stdin"], &d, Some("let   x=1 ;"));
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(String::from_utf8(o.stdout).unwrap(), "let x = 1;\n");
    let o = oxi(&["fmt", "--stdin"], &d, Some("let x = ;"));
    assert_eq!(o.status.code(), Some(1));
    assert!(o.stdout.is_empty());
    let o = oxi(&["fmt", "a.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(0));
    assert_eq!(std::fs::read_to_string(d.join("a.oxs")).unwrap(), "let x = 1;\n");
}

#[test]
fn corpus_checks_clean_through_the_cli_except_known_imports() {
    // The corpus modules import `util`/`std/math` stubs the loader cannot see, so only
    // files without imports are asserted here; the rest is covered by oxiscript-check.
    let d = tmp("corp");
    let mut n = 0;
    for f in corpus_files() {
        let s = std::fs::read_to_string(&f).unwrap();
        if s.contains("import ") {
            continue;
        }
        std::fs::write(d.join("x.oxs"), &s).unwrap();
        let v = json(&oxi(&["check", "--json", "x.oxs"], &d, None));
        assert!(v["fatal"].is_null());
        n += 1;
    }
    assert!(n >= 10);
}

// ---------------------------------------------------------------- explain

#[test]
fn explain_every_registry_code() {
    let defs = oxiscript_abi::codes::seed();
    assert!(defs.len() >= 90);
    for d in defs {
        let o = oxi(&["explain", &d.code], &crate_dir(), None);
        assert_eq!(o.status.code(), Some(0), "{}", d.code);
        let t = text(&o);
        assert!(t.contains(&d.code) && t.contains(&d.title), "{t}");
    }
    let o = oxi(&["explain", "e0102"], &crate_dir(), None);
    assert!(text(&o).contains("OXS-E0102"));
    assert_eq!(oxi(&["explain", "OXS-E9999"], &crate_dir(), None).status.code(), Some(2));
}

// ---------------------------------------------------------------- new + templates

fn idl_interfaces(dir: &Path) -> Vec<oxiscript_abi::idl::Interface> {
    let mut v = Vec::new();
    for s in [oxiscript_abi::idl::EXAMPLE_LOG_IDL, oxiscript_abi::idl::EXAMPLE_STATE_IDL] {
        v.extend(oxiscript_abi::idl::parse(s).unwrap());
    }
    for e in std::fs::read_dir(dir.join("idl")).unwrap() {
        v.extend(oxiscript_abi::idl::parse(&std::fs::read_to_string(e.unwrap().path()).unwrap()).unwrap());
    }
    v
}

#[test]
fn every_template_creates_checks_formats_and_declares_its_needs() {
    for tpl in ["blink", "touch-button", "sensor-dashboard", "oxiui-page"] {
        let work = tmp(tpl);
        let name = format!("demo-{tpl}");
        let o = oxi(&["new", &name, "--template", tpl], &work, None);
        assert_eq!(o.status.code(), Some(0), "{tpl}: {}", text(&o));
        let proj = work.join(&name);
        for f in ["main.oxs", "oxi.toml", "AGENTS.md", ".vscode/extensions.json", ".vscode/settings.json"] {
            assert!(proj.join(f).is_file(), "{tpl}: {f}");
        }
        let o = oxi(&["check"], &proj, None);
        assert_eq!(o.status.code(), Some(0), "{tpl}: {}", text(&o));
        let o = oxi(&["fmt", "--check"], &proj, None);
        assert_eq!(o.status.code(), Some(0), "{tpl} is not canonically formatted: {}", text(&o));
        assert!(std::fs::read_to_string(proj.join("oxi.toml")).unwrap().contains(&format!("name = \"{name}\"")));
        // `needs` must equal the interfaces of `main`'s parameters, in order
        let toml: toml::Table = std::fs::read_to_string(proj.join("oxi.toml")).unwrap().parse().unwrap();
        let needs: Vec<String> = toml["package"]["needs"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect();
        let main_src = std::fs::read_to_string(proj.join("main.oxs")).unwrap();
        let sig = main_src.split("export function main(").nth(1).unwrap().split(')').next().unwrap();
        let ifaces = idl_interfaces(&proj);
        let want: Vec<String> = sig
            .split(',')
            .map(|p| {
                let ty = p.split(':').nth(1).unwrap().trim();
                let i = ifaces.iter().find(|i| i.name == ty).unwrap_or_else(|| panic!("{tpl}: unknown host type {ty}"));
                format!("{}@{}", i.module, i.version)
            })
            .collect();
        assert_eq!(needs, want, "{tpl}");
    }
}

#[test]
fn new_refuses_bad_input() {
    let w = tmp("newbad");
    assert_eq!(oxi(&["new", "Bad Name"], &w, None).status.code(), Some(2));
    assert_eq!(oxi(&["new", "ok", "--template", "nope"], &w, None).status.code(), Some(2));
    assert_eq!(oxi(&["new", "ok"], &w, None).status.code(), Some(0));
    assert_eq!(oxi(&["new", "ok"], &w, None).status.code(), Some(2), "must not overwrite");
}

// ---------------------------------------------------------------- run / build

#[test]
fn run_and_build_check_first_then_report_not_available() {
    let w = tmp("run");
    // no entry
    assert_eq!(oxi(&["run"], &w, None).status.code(), Some(2));
    std::fs::write(w.join("main.oxs"), "export function f(): i32 { return 1; }\n").unwrap();
    for c in ["run", "build"] {
        let o = oxi(&[c], &w, None);
        assert_eq!(o.status.code(), Some(3));
        assert!(text(&o).contains("available after 8.L (lowering) is merged"), "{}", text(&o));
    }
    // errors are reported like `check` and exit 1, not 3
    std::fs::write(w.join("main.oxs"), "export function f(a: i32): bool { return a == 1; }\n").unwrap();
    let o = oxi(&["run"], &w, None);
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o).contains("OXS-E0005"), "{}", text(&o));
}

// ---------------------------------------------------------------- docs: every ```oxs block passes

fn check_blocks(path: &Path) -> usize {
    let md = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut n = 0;
    let mut lines = md.lines().enumerate();
    while let Some((ln, l)) = lines.next() {
        let Some(info) = l.trim_start().strip_prefix("```") else { continue };
        let info = info.trim();
        if !(info == "oxs" || info.starts_with("oxs-err")) {
            // skip a non-checked fence
            if !info.is_empty() || l.trim() != "```" {
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
        let o = oxi(&["check", "--json", "--stdin", "--stdin-path", "block.oxs"], &tmp("blk"), Some(&body));
        let v = json(&o);
        if let Some(want) = info.strip_prefix("oxs-err") {
            let want = want.trim();
            assert!(codes(&v).iter().any(|c| c == want), "{}:{}: expected {want}, got {:?}\n{body}", path.display(), ln + 1, codes(&v));
        } else {
            assert_eq!(v["ok"], true, "{}:{}: block does not pass oxi check: {v}\n{body}", path.display(), ln + 1);
            let f = oxi(&["fmt", "--stdin"], &tmp("blkfmt"), Some(&body));
            assert_eq!(String::from_utf8_lossy(&f.stdout), body, "{}:{}: block is not in `oxi fmt` form", path.display(), ln + 1);
        }
        n += 1;
    }
    n
}

#[test]
fn every_oxs_block_in_agent_docs_is_checked() {
    let root = repo_root();
    let files = [
        crate_dir().join("templates/common/AGENTS.md"),
        root.join("AGENTS.md"),
        root.join(".claude/skills/oxiscript-authoring/SKILL.md"),
        root.join("docs/oxiscript/llms.txt"),
    ];
    let mut total = 0;
    for f in files {
        let n = check_blocks(&f);
        assert!(n >= 3, "{} has only {n} checked blocks", f.display());
        total += n;
    }
    assert!(total >= 20);
}

#[test]
fn root_agents_md_embeds_the_template_core() {
    let core = |s: &str| s.split("<!-- oxs-core:begin -->").nth(1).and_then(|x| x.split("<!-- oxs-core:end -->").next()).map(str::to_string);
    let tpl = std::fs::read_to_string(crate_dir().join("templates/common/AGENTS.md")).unwrap();
    let root = std::fs::read_to_string(repo_root().join("AGENTS.md")).unwrap();
    assert_eq!(core(&tpl).unwrap(), core(&root).expect("root AGENTS.md needs the core markers"));
}

// ---------------------------------------------------------------- agent test record

#[test]
fn agent_test_record_still_holds() {
    let dir = crate_dir().join("tests/agent_test");
    let run = |sub: &str| -> Vec<(String, Vec<String>)> {
        let mut v = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join(sub)).unwrap().map(|e| e.unwrap().path()).collect();
        files.sort();
        for f in files {
            let src = std::fs::read_to_string(&f).unwrap();
            let r = json(&oxi(&["check", "--json", "--stdin", "--stdin-path", "a.oxs"], &tmp("agent"), Some(&src)));
            v.push((f.file_name().unwrap().to_string_lossy().to_string(), codes(&r)));
        }
        v
    };
    let fin = run("final");
    assert_eq!(fin.len(), 10);
    assert!(fin.iter().all(|(_, c)| c.is_empty()), "{fin:?}");
    let first: Vec<_> = run("first_try").into_iter().filter(|(_, c)| !c.is_empty()).collect();
    assert_eq!(
        first,
        vec![
            ("02_lerp.oxs".to_string(), vec!["OXS-E0100".to_string()]),
            ("05_parse_digits.oxs".to_string(), vec!["OXS-E0110".to_string()]),
            ("08_stats.oxs".to_string(), vec!["OXS-E0100".to_string()]),
        ]
    );
    let extra: Vec<_> = run("extra_first_try").into_iter().filter(|(_, c)| !c.is_empty()).collect();
    assert_eq!(
        extra,
        vec![
            ("12_generic.oxs".to_string(), vec!["OXS-E0121".to_string()]),
            ("16_err_types.oxs".to_string(), vec!["OXS-E0104".to_string()]),
        ]
    );
}

/// TypeScript-habit and malformed inputs (`tests/fixtures/ts_habits.txt`, cases split by `=====`):
/// `check --fix` must terminate with no runaway growth, be idempotent, and never touch a line
/// that holds an exception keyword or `for ... in`.
#[test]
fn fix_terminates_and_leaves_exceptions_alone() {
    let all = std::fs::read_to_string(crate_dir().join("tests/fixtures/ts_habits.txt")).unwrap();
    let cases: Vec<&str> = all.split("\n=====\n").filter(|c| !c.trim().is_empty()).collect();
    assert!(cases.len() >= 60, "{}", cases.len());
    let d = tmp("habits");
    for (i, c) in cases.iter().enumerate() {
        let f = d.join(format!("h{i}.oxs"));
        std::fs::write(&f, c).unwrap();
        let o = oxi(&["check", "--fix", f.to_str().unwrap()], &d, None);
        assert!(matches!(o.status.code(), Some(0 | 1)), "case {i}: {}", text(&o));
        let after = std::fs::read_to_string(&f).unwrap();
        assert!(after.len() <= c.len() + 40, "case {i} grew from {} to {} bytes:\n{after}", c.len(), after.len());
        for (a, b) in c.lines().zip(after.lines()) {
            let guarded = ["throw", "try", "catch", "finally"].iter().any(|w| a.contains(w)) || a.contains("for (const k in");
            if guarded {
                assert_eq!(a, b, "case {i}: --fix edited a guarded line");
            }
        }
        // idempotent: a second run changes nothing
        oxi(&["check", "--fix", f.to_str().unwrap()], &d, None);
        assert_eq!(after, std::fs::read_to_string(&f).unwrap(), "case {i} is not idempotent");
    }
}

#[test]
fn fix_reverts_a_round_that_does_not_reduce_syntax_errors() {
    let d = tmp("conv");
    // the `;` inserted for the missing terminator lands inside the unterminated string
    let f = d.join("a.oxs");
    std::fs::write(&f, "const s = \"abc;\n").unwrap();
    let o = oxi(&["check", "--fix", "a.oxs"], &d, None);
    assert_eq!(o.status.code(), Some(1));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "const s = \"abc;\n", "file must be left as written");
}
