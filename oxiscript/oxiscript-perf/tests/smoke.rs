//! Smoke test: one tiny workload through the whole harness (wasmi and the native baseline, debug
//! build, a child process each). Heavy runs are not tests.

use std::process::Command;
use std::time::Instant;

fn oxiperf(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_oxiperf")).args(args).output().expect("spawn oxiperf")
}

#[test]
fn tiny_workload_on_wasmi_runs_end_to_end_within_ten_seconds() {
    let dir = std::env::temp_dir().join(format!("oxiperf-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let t = Instant::now();
    let o = oxiperf(&["run", "--quick", "--workloads", "matmul", "--engines", "oxiscript-wasmi,rust-native", "--iters", "3", "--out", dir.to_str().unwrap()]);
    let secs = t.elapsed().as_secs_f64();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    assert!(secs < 10.0, "smoke run took {secs:.1}s");

    for f in ["manifest.json", "raw.jsonl", "summary.json", "report.md"] {
        assert!(dir.join(f).exists(), "{f} missing");
    }
    let summary: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
    assert_eq!(summary["digests_ok"], true, "checksums must agree");
    let rows = summary["rows"].as_array().unwrap();
    let wasmi = rows.iter().find(|r| r["engine"] == "oxiscript-wasmi").expect("wasmi row");
    assert!(wasmi["fuel"].as_u64().unwrap() > 0, "wasmi reports deterministic fuel");
    assert_eq!(wasmi["runtime_mode"], "stub");
    assert!(wasmi["median_ms"].as_f64().unwrap() > 0.0);
    let native = rows.iter().find(|r| r["engine"] == "rust-native").expect("native row");
    assert_eq!(wasmi["checksum"], native["checksum"]);
    let report = std::fs::read_to_string(dir.join("report.md")).unwrap();
    assert!(report.contains("runtime mode measured: `stub`") && report.contains("w2c2"));

    // `analyze` rebuilds the report from the raw data alone, identically.
    let before = std::fs::read_to_string(dir.join("summary.json")).unwrap();
    let a = oxiperf(&["analyze", "--input", dir.to_str().unwrap()]);
    assert!(a.status.success());
    assert_eq!(before, std::fs::read_to_string(dir.join("summary.json")).unwrap(), "analysis is deterministic");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_engine_and_missing_args_fail_cleanly() {
    let o = oxiperf(&["exec", "--engine", "nope", "--workload", "sort"]);
    assert_eq!(o.status.code(), Some(2));
    assert_eq!(oxiperf(&[]).status.code(), Some(2));
}
