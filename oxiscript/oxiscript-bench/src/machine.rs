//! Machine description and run manifest (methodology R07, R13, section 5.3). Everything is read
//! from /proc, /sys and the pinned tools; nothing is typed in by hand. The host name is NOT recorded
//! (the machine is identified by its CPU).

use serde_json::{json, Value};
use std::process::Command;

pub const SCHEMA_VERSION: u64 = 1;
pub const METHODOLOGY: &str = "oxiperf-v0 (harness; methodology draft v0.1 of docs/research/cross-language-benchmark-methodology.md)";

fn read(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Output of `cmd args` (stdout, falling back to stderr), first line.
pub fn tool_version(cmd: &str, args: &[&str]) -> Option<String> {
    let o = Command::new(cmd).args(args).output().ok()?;
    if !o.status.success() {
        return None;
    }
    let s = if o.stdout.is_empty() { o.stderr } else { o.stdout };
    String::from_utf8_lossy(&s).lines().next().map(|l| l.trim().to_string())
}

fn cpu_model() -> String {
    read("/proc/cpuinfo")
        .and_then(|s| s.lines().find(|l| l.starts_with("model name")).and_then(|l| l.split(':').nth(1)).map(|v| v.trim().to_string()))
        .unwrap_or_else(|| "unknown".into())
}

/// `Intel(R) Core(TM) i5-3450 CPU @ 3.10GHz` -> `i5-3450`; used in the result directory name.
pub fn machine_slug() -> String {
    let m = cpu_model();
    let tok = m.split_whitespace().find(|t| t.chars().any(|c| c.is_ascii_digit()) && t.contains('-') && !t.starts_with("(R)"));
    let s = tok.map_or_else(|| m.replace(' ', "-"), str::to_string);
    s.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>().to_lowercase()
}

fn mem_total_mib() -> u64 {
    read("/proc/meminfo")
        .and_then(|s| s.lines().find(|l| l.starts_with("MemTotal:")).and_then(|l| l.split_whitespace().nth(1)).and_then(|v| v.parse::<u64>().ok()))
        .map_or(0, |kb| kb / 1024)
}

pub fn loadavg() -> f64 {
    read("/proc/loadavg").and_then(|s| s.split_whitespace().next().and_then(|v| v.parse().ok())).unwrap_or(f64::NAN)
}

fn cpu_mhz_now() -> Option<f64> {
    read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq").and_then(|v| v.parse::<f64>().ok()).map(|k| k / 1000.0)
}

fn temp_c() -> Option<f64> {
    for i in 0..8 {
        let base = format!("/sys/class/thermal/thermal_zone{i}");
        if read(&format!("{base}/type")).is_some_and(|t| t.contains("x86_pkg") || t.contains("cpu") || t.contains("acpitz")) {
            if let Some(v) = read(&format!("{base}/temp")).and_then(|v| v.parse::<f64>().ok()) {
                return Some(v / 1000.0);
            }
        }
    }
    None
}

/// Version of a crate as pinned in the repository `Cargo.lock`.
pub fn locked_version(name: &str) -> Option<String> {
    let lock = std::fs::read_to_string(crate::workloads::bench_root().join("../../Cargo.lock")).ok()?;
    let mut lines = lock.lines();
    while let Some(l) = lines.next() {
        if l == format!("name = \"{name}\"") {
            return lines.next().and_then(|v| v.strip_prefix("version = \"")).map(|v| v.trim_end_matches('"').to_string());
        }
    }
    None
}

/// A short snapshot taken before and after the measurements (governor drift, load, temperature).
pub fn snapshot() -> Value {
    json!({ "loadavg_1m": loadavg(), "cpu0_mhz": cpu_mhz_now(), "temp_c": temp_c() })
}

pub struct RunInfo<'a> {
    pub run_id: &'a str,
    pub tier: &'a str,
    pub runtime_mode: &'a str,
    pub plan: Value,
    pub pinned_core: Option<usize>,
    pub engines: &'a [(String, String)], // (engine id, availability note)
}

pub fn build_manifest(info: &RunInfo, start: &Value) -> Value {
    let governor = read("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor");
    let turbo_disabled = read("/sys/devices/system/cpu/intel_pstate/no_turbo").map(|v| v == "1");
    // Without root the governor cannot be set; wall times from such a run are not publishable (R13).
    let uncontrolled = governor.as_deref() != Some("performance") || turbo_disabled != Some(true) || info.pinned_core.is_none();
    let dirty = tool_version("git", &["status", "--porcelain", "--", ".", ":!bench/oxiperf/results"]).is_some();
    json!({
        "schema_version": SCHEMA_VERSION,
        "methodology_version": METHODOLOGY,
        "harness_version": env!("CARGO_PKG_VERSION"),
        "run_id": info.run_id,
        "tier": info.tier,
        "suite": "8.Bench part 2 (cross-language suite), v0 subset: unfrozen dev report, gated on owner approval Q-OP-1",
        "publishable": false,
        "publishable_reason": "dev tier: fewer invocations than R15 (10), no controlled DVFS, no isolated core; harness output, not a frozen report",
        "runtime_mode": info.runtime_mode,
        "size_profile": "server",
        "track": "base (+ one stdlib workload, labelled)",
        "plan": info.plan.clone(),
        "git": { "commit": tool_version("git", &["rev-parse", "HEAD"]), "dirty": dirty },
        "host": {
            "arch": std::env::consts::ARCH,
            "cpu_model": cpu_model(),
            "cpus_logical": std::thread::available_parallelism().map_or(0, std::num::NonZero::get),
            "ram_total_mib": mem_total_mib(),
            "os": read("/proc/version").map(|v| v.split(" (").next().unwrap_or("").to_string()),
            "governor": governor,
            "turbo_disabled": turbo_disabled,
            "aslr_mode": read("/proc/sys/kernel/randomize_va_space").and_then(|v| v.parse::<u32>().ok()),
            "pinned_core": info.pinned_core,
            "uncontrolled_dvfs": uncontrolled,
            "start": start.clone(),
            "end": Value::Null,
        },
        "toolchains": {
            "rustc": { "version": tool_version("rustc", &["--version"]), "flags": "cargo release profile: opt-level=3, codegen-units=16, no LTO, panic=unwind" },
            "oxiscript": { "version": env!("CARGO_PKG_VERSION"), "runtime_mode": info.runtime_mode, "target": "device" },
            "wasmi": { "crate_version": locked_version("wasmi"), "fuel": true, "features": "oxiscript-runtime defaults (validate, deterministic)" },
            "wasmtime": {
                "crate_version": if cfg!(feature = "wasmtime") { locked_version("wasmtime") } else { None },
                "cli_version": tool_version("wasmtime", &["--version"]),
                "flags": "Engine::default() (Cranelift, opt_level speed), module compiled once per process",
                "enabled": cfg!(feature = "wasmtime"),
            },
            "node": { "version": tool_version("node", &["--version"]), "v8": tool_version("node", &["-p", "process.versions.v8"]), "flags": "" },
            "gcc": { "version": tool_version("gcc", &["--version"]), "flags": "-O2 -ffp-contract=off -DNDEBUG" },
            "w2c2": { "status": "TODO: not installed on this machine, no rows are produced" },
        },
        "engines": info.engines.iter().map(|(id, note)| json!({ "id": id, "note": note })).collect::<Vec<_>>(),
    })
}

/// Fields every manifest must have (checked by `validate_manifest`, by the tests and by `analyze`).
pub const MANIFEST_REQUIRED: &[&str] = &[
    "schema_version", "methodology_version", "harness_version", "run_id", "tier", "publishable", "runtime_mode", "plan", "git", "host", "toolchains", "engines",
];
pub const HOST_REQUIRED: &[&str] = &["arch", "cpu_model", "cpus_logical", "ram_total_mib", "os", "governor", "turbo_disabled", "uncontrolled_dvfs", "start", "end"];

pub fn validate_manifest(m: &Value) -> Result<(), String> {
    for k in MANIFEST_REQUIRED {
        if m.get(k).is_none() {
            return Err(format!("manifest: missing `{k}`"));
        }
    }
    for k in HOST_REQUIRED {
        if m["host"].get(k).is_none() {
            return Err(format!("manifest.host: missing `{k}`"));
        }
    }
    if m["schema_version"] != json!(SCHEMA_VERSION) {
        return Err("manifest: unknown schema_version".into());
    }
    if !m["toolchains"].is_object() || m["toolchains"]["rustc"].is_null() {
        return Err("manifest.toolchains: rustc missing".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        let engines = vec![("rust-native".to_string(), "ok".to_string())];
        build_manifest(
            &RunInfo { run_id: "t", tier: "dev", runtime_mode: "stub", plan: json!({ "description": "test" }), pinned_core: None, engines: &engines },
            &snapshot(),
        )
    }

    #[test]
    fn manifest_has_the_required_schema() {
        let m = sample();
        validate_manifest(&m).unwrap();
        assert_eq!(m["publishable"], json!(false));
        assert_eq!(m["host"]["uncontrolled_dvfs"], json!(true)); // no pinned core in this sample
        assert!(m["host"]["cpu_model"].as_str().unwrap().len() > 2);
        assert!(m["toolchains"]["w2c2"]["status"].as_str().unwrap().starts_with("TODO"));
    }

    #[test]
    fn manifest_without_a_required_field_is_rejected() {
        for k in MANIFEST_REQUIRED {
            let mut m = sample();
            m.as_object_mut().unwrap().remove(*k);
            assert!(validate_manifest(&m).unwrap_err().contains(k), "{k}");
        }
        let mut m = sample();
        m["host"].as_object_mut().unwrap().remove("cpu_model");
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn manifest_does_not_record_the_host_name() {
        let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
        let host = host.trim();
        if host.len() > 3 {
            assert!(!sample().to_string().contains(host));
        }
    }

    #[test]
    fn slug_is_a_safe_path_component() {
        let s = machine_slug();
        assert!(!s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'), "{s}");
    }
}
