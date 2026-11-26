//! BAD ACTOR: CLI surface of compilation targets.
#![allow(clippy::pedantic, dead_code)]

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};

const HELLO: &str = "import { Log } from \"host:log\";\nexport function main(log: Log): void {\n  log.write(2, \"hi\");\n}\n";

fn tmp(tag: &str) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!("oxi-ba-targets-{}-{}-{tag}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}
fn oxi(args: &[&str], dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_oxi")).args(args).current_dir(dir).env("NO_COLOR", "1").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).output().unwrap()
}
fn text(o: &Output) -> String {
    format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))
}
fn toml(target_line: &str) -> String {
    format!("[package]\nname = \"t\"\nversion = \"0.1.0\"\n{target_line}\nneeds = [\"host:log@1\"]\n")
}
fn manifest_target(f: &Path) -> String {
    let Ok(b) = std::fs::read(f) else { return "<none>".into() };
    String::from_utf8_lossy(&b).lines().filter(|l| l.starts_with("target ")).collect::<Vec<_>>().join(",")
}

#[test]
fn flag_values_are_strict() {
    let w = tmp("flag");
    std::fs::write(w.join("main.oxs"), HELLO).unwrap();
    for bad in ["WEB", "Web", " web", "web ", "", "device,web", "web\0", "wеb", "ｗｅｂ"] {
        if bad.contains('\0') {
            continue; // argv cannot carry NUL
        }
        for cmd in ["build", "run", "check"] {
            let o = oxi(&[cmd, "--target", bad], &w);
            assert_eq!(o.status.code(), Some(2), "{cmd} --target {bad:?}: {}", text(&o));
        }
    }
    // repeated flag: must not silently pick one
    let o = oxi(&["build", "--target", "web", "--target", "server", "-o", "x.wasm"], &w);
    println!("repeated --target: exit {:?} manifest {:?}\n{}", o.status.code(), manifest_target(&w.join("x.wasm")), text(&o));
    assert_ne!(o.status.code(), Some(0), "repeated --target accepted (last wins?)");
}

#[test]
fn oxi_toml_target_is_strict() {
    let w = tmp("toml");
    std::fs::write(w.join("main.oxs"), HELLO).unwrap();
    for (line, ok) in [
        ("target = \"web\"", true),
        ("target = \"Web\"", false),
        ("target = \" web\"", false),
        ("target = 5", false),
        ("target = [\"web\"]", false),
        ("target = true", false),
        ("target = \"\"", false),
        ("target = \"web\"\ntarget = \"server\"", false), // duplicate key = toml error
        ("target = 'web'", true),
    ] {
        std::fs::write(w.join("oxi.toml"), toml(line)).unwrap();
        let b = oxi(&["build", "-o", "o.wasm"], &w);
        let c = oxi(&["check", "--json"], &w);
        let r = oxi(&["run"], &w);
        println!("{line:?}: build {:?} check {:?} run {:?}", b.status.code(), c.status.code(), r.status.code());
        assert_eq!(b.status.code() == Some(0), ok, "build {line:?}: {}", text(&b));
        assert_eq!(r.status.code() == Some(0), ok, "run {line:?}: {}", text(&r));
        assert_eq!(c.status.code() == Some(0), ok, "check {line:?}: {}", text(&c));
    }
    // `target` in the wrong table must not be honoured
    std::fs::write(w.join("oxi.toml"), "[package]\nname = \"t\"\nversion = \"0.1.0\"\nneeds = [\"host:log@1\"]\n[other]\ntarget = \"web\"\n").unwrap();
    assert_eq!(oxi(&["build", "-o", "o.wasm"], &w).status.code(), Some(0));
    assert_eq!(manifest_target(&w.join("o.wasm")), "");
    // wrong-case key
    std::fs::write(w.join("oxi.toml"), toml("Target = \"web\"")).unwrap();
    let o = oxi(&["build", "-o", "o.wasm"], &w);
    println!("Target= key: exit {:?} manifest {:?}", o.status.code(), manifest_target(&w.join("o.wasm")));
}

/// `--target` given and a broken oxi.toml: the config error must still surface, and check --json
/// must not claim a target for a broken configuration.
#[test]
fn check_json_target_with_broken_toml_and_mixed_projects() {
    let w = tmp("json");
    std::fs::write(w.join("main.oxs"), HELLO).unwrap();
    std::fs::write(w.join("oxi.toml"), toml("target = \"moon\"")).unwrap();
    let o = oxi(&["check", "--json"], &w);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap_or(Value::Null);
    println!("broken toml: exit {:?} target {:?}", o.status.code(), v["target"]);
    assert_ne!(o.status.code(), Some(0));
    assert!(v["target"] != "device", "check --json reports target `device` although oxi.toml says `moon`");
    // two projects, first is device (no target), second is web: the one JSON document has ONE target
    let a = w.join("a");
    let b = w.join("b");
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    std::fs::write(w.join("oxi.toml"), toml("")).unwrap();
    std::fs::write(a.join("main.oxs"), HELLO).unwrap();
    std::fs::write(a.join("oxi.toml"), toml("")).unwrap();
    std::fs::write(b.join("main.oxs"), HELLO).unwrap();
    std::fs::write(b.join("oxi.toml"), toml("target = \"web\"")).unwrap();
    let o = oxi(&["check", "--json", "a/main.oxs", "b/main.oxs"], &w);
    let v: Value = serde_json::from_slice(&o.stdout).unwrap();
    println!("mixed projects target: {:?}", v["target"]);
    assert!(v["target"] == "device" || v["target"].is_null() || v["target"] == "mixed", "{}", v["target"]);
    let o = oxi(&["check", "--json", "b/main.oxs", "a/main.oxs"], &w);
    let v2: Value = serde_json::from_slice(&o.stdout).unwrap();
    println!("mixed projects reversed: {:?}", v2["target"]);
    assert_eq!(v["target"], v2["target"], "check --json target depends on file order for files of different projects");
}

#[test]
fn explain_e0411_and_target_flag_in_help() {
    let w = tmp("explain");
    let o = oxi(&["explain", "OXS-E0411"], &w);
    assert_eq!(o.status.code(), Some(0), "{}", text(&o));
    assert!(text(&o).to_lowercase().contains("target"), "{}", text(&o));
    for c in ["build", "run", "check"] {
        assert!(text(&oxi(&[c, "--help"], &w)).contains("--target"), "{c}");
    }
}

/// `oxi run --target T` really builds a module for T, runs it on a host with max_target = T, and
/// the module runs the same on all targets.
#[test]
fn run_output_identical_on_every_target() {
    let w = tmp("run");
    std::fs::write(w.join("main.oxs"), HELLO).unwrap();
    let base = oxi(&["run"], &w);
    for t in ["device", "web", "server"] {
        let o = oxi(&["run", "--target", t], &w);
        assert_eq!(o.status.code(), base.status.code(), "{t}");
        assert_eq!(o.stdout, base.stdout, "{t}");
    }
}

/// Build output differs between targets only in the manifest section (byte compare).
#[test]
fn build_outputs_differ_only_in_manifest() {
    let w = tmp("bytes");
    std::fs::write(w.join("main.oxs"), HELLO).unwrap();
    let strip = |f: &str| -> Vec<u8> {
        let b = std::fs::read(w.join(f)).unwrap();
        let mut out = b[..8].to_vec();
        let mut i = 8;
        while i < b.len() {
            let id = b[i];
            let (mut size, mut shift, mut j) = (0usize, 0, i + 1);
            loop {
                let x = b[j];
                j += 1;
                size |= ((x & 0x7f) as usize) << shift;
                shift += 7;
                if x & 0x80 == 0 {
                    break;
                }
            }
            let body = &b[j..j + size];
            let is_manifest = id == 0 && body.windows(12).any(|w| w == b"oxi.manifest");
            if !is_manifest {
                out.extend_from_slice(&b[i..j + size]);
            }
            i = j + size;
        }
        out
    };
    for t in ["device", "web", "server"] {
        assert_eq!(oxi(&["build", "--target", t, "-o", &format!("{t}.wasm")], &w).status.code(), Some(0));
    }
    assert_eq!(strip("device.wasm"), strip("web.wasm"));
    assert_eq!(strip("device.wasm"), strip("server.wasm"));
}
