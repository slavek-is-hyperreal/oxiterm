//! BAD ACTOR round 2026-11-27-1, target 3 (`OxiPerf` v0): ways the harness can report a speedup
//! that is not real. A fake `node` on PATH stands in for a broken executor (wrong answer, very
//! fast). Failing tests are findings (R1-nn in docs/stages/rounds/2026-11-27-1.md).

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let d = base.join(format!("oxiperf-ba-r1-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A `node` that answers `--version` and otherwise prints one child-protocol line with the given
/// checksum JSON value and 1 ns iterations.
fn fake_node(dir: &Path, checksum_json: &str) -> PathBuf {
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo v99.0.0; exit 0; fi\necho '{{\"checksum\":{checksum_json},\"checksum_stable\":true,\"cold_ns\":1,\"warmup\":1,\"inner_reps\":1,\"peak_rss_kib\":1,\"iter_ns\":[1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0,1.0]}}'\n"
    );
    let p = bin.join("node");
    std::fs::write(&p, script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    bin
}

fn run_with_fake_node(name: &str, checksum_json: &str) -> (std::process::Output, Value) {
    let d = scratch(name);
    let bin = fake_node(&d, checksum_json);
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    let out = d.join("out");
    let o = Command::new(env!("CARGO_BIN_EXE_oxiperf"))
        .args(["run", "--quick", "--workloads", "matmul", "--engines", "js-node,rust-native", "--iters", "3", "--out", out.to_str().unwrap()])
        .env("PATH", path)
        .output()
        .expect("spawn oxiperf");
    let summary: Value = std::fs::read_to_string(out.join("summary.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    (o, summary)
}

fn js_row(summary: &Value) -> Option<&Value> {
    summary["rows"].as_array()?.iter().find(|r| r["engine"] == "js-node")
}

/// Control: a positive wrong checksum is caught (digest mismatch, run fails).
#[test]
fn probe_wrong_positive_checksum_is_caught() {
    let (o, s) = run_with_fake_node("pos", "12345");
    assert!(!o.status.success() || s["digests_ok"] != Value::Bool(true), "wrong checksum accepted: {s}");
}

/// R1-40: a checksum that is not a non-negative integer (negative, fractional, or a string)
/// is dropped by `v["checksum"].as_u64()` before the agreement check, so the executor is never
/// compared; with `checksum_stable: true` the run is valid, `digests_ok` is true, and the table
/// reports a fake "faster" verdict. Expected: such a run is invalid.
#[test]
fn r1_40_non_u64_checksum_skips_verification() {
    let mut fake = Vec::new();
    for (name, cs) in [("neg", "-7"), ("frac", "1.5"), ("str", "\"x\"")] {
        let (o, s) = run_with_fake_node(name, cs);
        let row = js_row(&s);
        let verdict = row.and_then(|r| r["ratio_vs_rust_native"]["verdict"].as_str()).unwrap_or("-").to_string();
        eprintln!("checksum {cs}: exit {:?}, digests_ok {}, js verdict {verdict}", o.status.code(), s["digests_ok"]);
        if o.status.success() && s["digests_ok"] == Value::Bool(true) && row.is_some() {
            fake.push(format!("checksum {cs} -> digests_ok, verdict {verdict}"));
        }
    }
    assert!(fake.is_empty(), "unverified executor results were reported: {fake:?}");
}
