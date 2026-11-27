//! `oxiperf analyze`: raw.jsonl + manifest.json -> summary.json + report.md. Statistics are computed
//! only here, from the raw data (methodology section 5: execution and analysis are separate).

use crate::stats;
use crate::workloads;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const BOOTSTRAP_RESAMPLES: usize = 10_000;
pub const BASELINE: &str = "rust-native";
/// Minimum practical effect for an external cross-language claim (R18).
pub const MPES: f64 = 0.10;

pub fn read_raw(dir: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(dir.join("raw.jsonl")).map_err(|e| format!("raw.jsonl: {e}"))?;
    text.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).map_err(|e| format!("raw.jsonl: {e}"))).collect()
}

fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}

fn med(v: &[f64]) -> Option<f64> {
    if v.is_empty() {
        None
    } else {
        Some(stats::median(v))
    }
}

/// R18 verdict for `engine / baseline` given the ratio and its 95% interval.
pub fn verdict(ratio: f64, lo: f64, hi: f64) -> &'static str {
    let excludes_one = lo > 1.0 || hi < 1.0;
    if !excludes_one || (ratio - 1.0).abs() < MPES {
        "parity / within noise"
    } else if ratio > 1.0 {
        "slower"
    } else {
        "faster"
    }
}

type Key = (String, String); // (benchmark, engine)

pub fn analyze(dir: &Path) -> Result<(), String> {
    let raw = read_raw(dir)?;
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    crate::machine::validate_manifest(&manifest)?;

    let mut groups: BTreeMap<Key, Vec<Vec<f64>>> = BTreeMap::new(); // per invocation iteration times (ns)
    let mut excluded: BTreeMap<Key, usize> = BTreeMap::new();
    let mut meta: BTreeMap<Key, Value> = BTreeMap::new();
    let mut rss: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    let mut cold_proc: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    let mut cold_call: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    let mut load: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    let mut drift: BTreeMap<Key, Vec<&'static str>> = BTreeMap::new();
    let mut inner: BTreeMap<Key, Vec<f64>> = BTreeMap::new();
    let mut art: BTreeMap<Key, Value> = BTreeMap::new();
    let mut digest: BTreeMap<String, Value> = BTreeMap::new();
    let mut checksum: BTreeMap<Key, u64> = BTreeMap::new();
    let mut raw_checksums: BTreeMap<String, Vec<workloads::ChecksumEntry>> = BTreeMap::new();
    let mut invalid_workloads: BTreeSet<String> = BTreeSet::new();

    for r in &raw {
        let kind = r["kind"].as_str().unwrap_or("");
        let bench = r["benchmark"].as_str().unwrap_or("").to_string();
        let eng = r["engine"].as_str().unwrap_or("").to_string();
        let key = (bench.clone(), eng);
        match kind {
            "artifact" => {
                art.insert(key, r.clone());
            }
            "digest" => {
                digest.insert(bench, r.clone());
            }
            "steady" | "cold" | "pilot" => {
                let parsed_cs = workloads::parse_u32_checksum(r);
                let stable = r["checksum_stable"].as_bool().unwrap_or(false)
                    && r.get("fuel_stable").and_then(Value::as_bool).unwrap_or(true);
                let run_ok = r["valid"] == json!(true) && parsed_cs.is_some() && stable;
                let label = format!("{} ({kind} {})", key.1, r["invocation_idx"]);
                if !run_ok {
                    if kind != "pilot" {
                        *excluded.entry(key).or_default() += 1;
                    }
                    invalid_workloads.insert(bench.clone());
                    let reason = r["invalid_reason"].as_str().map_or_else(
                        || {
                            if parsed_cs.is_none() {
                                format!("missing or malformed u32 checksum: {}", r.get("checksum").unwrap_or(&Value::Null))
                            } else {
                                "invalid run".to_string()
                            }
                        },
                        str::to_string,
                    );
                    raw_checksums.entry(bench).or_default().push((label, Err(reason)));
                    continue;
                }
                let cs = parsed_cs.expect("validated above");
                raw_checksums.entry(bench).or_default().push((label, Ok(cs)));
                if kind == "pilot" {
                    continue;
                }
                if kind == "cold" {
                    if let Some(x) = f(r, "process_ms") {
                        cold_proc.entry(key.clone()).or_default().push(x);
                    }
                    if let Some(x) = f(r, "cold_ns") {
                        cold_call.entry(key.clone()).or_default().push(x / 1e6);
                    }
                    if let Some(x) = f(r, "load_ns") {
                        load.entry(key).or_default().push(x / 1e6);
                    }
                    continue;
                }
                let its: Vec<f64> = r["iter_ns"].as_array().map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_f64).collect());
                if its.is_empty() {
                    continue;
                }
                drift.entry(key.clone()).or_default().push(stats::drift_status(&its));
                groups.entry(key.clone()).or_default().push(its);
                if let Some(x) = f(r, "peak_rss_kib") {
                    rss.entry(key.clone()).or_default().push(x);
                }
                if let Some(x) = f(r, "inner_reps") {
                    inner.entry(key.clone()).or_default().push(x);
                }
                checksum.insert(key.clone(), u64::from(cs));
                if let Some(x) = f(r, "load_ns") {
                    load.entry(key.clone()).or_default().push(x / 1e6);
                }
                meta.insert(key, r.clone());
            }
            _ => {}
        }
    }

    // Re-verify per-workload checksum agreement from raw records alone so `analyze`
    // catches malformed/missing/disagreeing checksums regardless of any `digest` record.
    let all_benches: BTreeSet<String> = digest.keys().chain(raw_checksums.keys()).cloned().collect();
    for bench in all_benches {
        let verified = raw_checksums
            .get(&bench)
            .map_or_else(|| Err("no checksums recorded".to_string()), |rs| workloads::verify_run_checksums(rs));
        let recorded_ok = digest.get(&bench).is_none_or(|d| d["digest_ok"] == json!(true));
        match (verified, recorded_ok, invalid_workloads.contains(&bench)) {
            (Ok(c), true, false) => {
                digest.entry(bench.clone()).or_insert_with(|| {
                    json!({ "kind": "digest", "benchmark": bench, "digest_ok": true, "checksum": c, "executors": raw_checksums.get(&bench).map_or(0, Vec::len) })
                });
            }
            (Err(err), _, _) => {
                invalid_workloads.insert(bench.clone());
                digest.insert(bench.clone(), json!({ "kind": "digest", "benchmark": bench, "digest_ok": false, "error": err }));
            }
            (_, false, _) | (_, _, true) => {
                invalid_workloads.insert(bench.clone());
                if digest.get(&bench).is_none_or(|d| d["digest_ok"] == json!(true)) {
                    digest.insert(bench.clone(), json!({ "kind": "digest", "benchmark": bench, "digest_ok": false, "error": "invalid run in workload" }));
                }
            }
        }
    }

    // Order: workloads in suite order, engines in a fixed order.
    let order: Vec<&str> = crate::workloads::WORKLOADS.iter().map(|w| w.id).collect();
    let eng_order = ["oxiscript-wasmi", "oxiscript-wasmtime", "rust-native", "c-gcc", "js-node"];
    let mut keys: Vec<Key> = groups.keys().cloned().collect();
    keys.sort_by_key(|(b, e)| (order.iter().position(|x| x == b).unwrap_or(99), eng_order.iter().position(|x| x == e).unwrap_or(99)));

    let mut rows = vec![];
    for key in &keys {
        let g = &groups[key];
        let pooled: Vec<f64> = g.iter().flatten().copied().collect();
        let (lo, hi) = stats::bootstrap_median_ci(g, BOOTSTRAP_RESAMPLES, 1);
        let m = &meta[key];
        let row_digest_ok = digest.get(&key.0).is_some_and(|d| d["digest_ok"] == json!(true)) && checksum.contains_key(key);
        let base = groups.get(&(key.0.clone(), BASELINE.to_string()));
        let ratio = if row_digest_ok {
            base.filter(|_| key.1 != BASELINE).map(|b| {
                let r = stats::median(&pooled) / stats::median(&b.iter().flatten().copied().collect::<Vec<_>>());
                let (rl, rh) = stats::bootstrap_ratio_ci(g, b, BOOTSTRAP_RESAMPLES, 2);
                json!({ "ratio": r, "ci95": [rl, rh], "verdict": verdict(r, rl, rh) })
            })
        } else {
            None
        };
        let mut statuses = drift[key].clone();
        statuses.sort_unstable();
        statuses.dedup();
        let a = art.get(key);
        let w = crate::workloads::find(&key.0);
        rows.push(json!({
            "benchmark": key.0, "origin": w.map(|w| w.origin.as_str()), "engine": key.1, "language": m["language"], "track": m["track"], "n": m["n"], "runtime_mode": m["runtime_mode"],
            "invocations": g.len(), "iterations_per_invocation": g[0].len(), "excluded_invalid": excluded.get(key).copied().unwrap_or(0),
            "median_ms": stats::median(&pooled) / 1e6, "ci95_ms": [lo / 1e6, hi / 1e6],
            "iqr_ms": [stats::percentile(&pooled, 25.0) / 1e6, stats::percentile(&pooled, 75.0) / 1e6],
            "p95_ms": stats::percentile(&pooled, 95.0) / 1e6,
            "min_ms": pooled.iter().copied().fold(f64::MAX, f64::min) / 1e6, "max_ms": pooled.iter().copied().fold(0.0, f64::max) / 1e6,
            "mean_ms": stats::mean(&pooled) / 1e6, "cv": stats::cv(&pooled),
            "warmup_status": statuses.join("+"),
            "inner_reps_median": med(inner.get(key).map_or(&[][..], |v| v)),
            "ratio_vs_rust_native": ratio,
            "fuel": m.get("fuel"), "wasm_pages": m.get("wasm_pages"),
            "peak_rss_mib_median": med(rss.get(key).map_or(&[][..], |v| v)).map(|k| k / 1024.0),
            "cold_process_ms_median": med(cold_proc.get(key).map_or(&[][..], |v| v)),
            "cold_first_call_ms_median": med(cold_call.get(key).map_or(&[][..], |v| v)),
            "load_ms_median": med(load.get(key).map_or(&[][..], |v| v)),
            "artifact_bytes": a.and_then(|x| x.get("artifact_bytes")), "artifact": a.and_then(|x| x.get("artifact")),
            "compile_ms_median": a.and_then(|x| x.get("compile_ms_median")), "compile_ms_first": a.and_then(|x| x.get("compile_ms_first")),
            "checksum": checksum.get(key), "digest_ok": digest.get(&key.0).map(|d| d["digest_ok"].clone()),
            "workload_title": w.map(|w| w.title),
        }));
    }

    // Suite geometric mean of the ratios per engine, always with the per-benchmark range (R16).
    let mut per_engine: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();
    for r in &rows {
        if let Some(x) = r["ratio_vs_rust_native"]["ratio"].as_f64() {
            per_engine.entry((r["origin"].as_str().unwrap_or("own").to_string(), r["engine"].as_str().unwrap().to_string())).or_default().push(x);
        }
    }
    let geomeans: Vec<Value> = per_engine
        .iter()
        .map(|((o, e), v)| json!({ "origin": o, "engine": e, "benchmarks": v.len(), "geomean": stats::geomean(v), "min": v.iter().copied().fold(f64::MAX, f64::min), "max": v.iter().copied().fold(0.0, f64::max) }))
        .collect();

    let digests_ok = !digest.is_empty() && invalid_workloads.is_empty() && digest.values().all(|d| d["digest_ok"] == json!(true));
    let summary = json!({
        "schema_version": 1,
        "run_id": manifest["run_id"], "runtime_mode": manifest["runtime_mode"], "baseline": BASELINE,
        "bootstrap": { "resamples": BOOTSTRAP_RESAMPLES, "method": "hierarchical (invocations, then iterations), splitmix64 seeded", "mpes_external": MPES },
        "digests_ok": digests_ok, "digests": digest.values().collect::<Vec<_>>(),
        "rows": rows, "geomean_vs_rust_native": geomeans,
    });
    crate::driver::write_json(&dir.join("summary.json"), &summary)?;
    std::fs::write(dir.join("report.md"), render(&manifest, &summary)).map_err(|e| e.to_string())
}

fn sig(x: f64) -> String {
    if !x.is_finite() {
        "n/a".into()
    } else if x >= 100.0 {
        format!("{x:.0}")
    } else if x >= 10.0 {
        format!("{x:.1}")
    } else if x >= 1.0 {
        format!("{x:.2}")
    } else {
        format!("{x:.3}")
    }
}

fn opt(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_f64).map_or("-".into(), sig)
}

fn bytes(v: &Value) -> String {
    v.as_u64().map_or("-".into(), |b| if b >= 10_000 { format!("{:.1} KiB", b as f64 / 1024.0) } else { format!("{b} B") })
}

pub fn render(m: &Value, s: &Value) -> String {
    use std::fmt::Write;
    let mut o = String::new();
    let h = &m["host"];
    let _ = writeln!(o, "# OxiPerf report: {}\n", m["run_id"].as_str().unwrap_or(""));
    let _ = writeln!(o, "**Suite: {}.**\n", m["suite"].as_str().unwrap_or(""));
    let _ = writeln!(o, "**Not publishable.** {} Tier `{}`; harness version {}.\n", m["publishable_reason"].as_str().unwrap_or(""), m["tier"].as_str().unwrap_or(""), m["harness_version"].as_str().unwrap_or(""));
    let _ = writeln!(o, "**OxiScript runtime mode measured: `{}`** (test-stub runtime with a bump allocator that never reuses memory; the real runtime and f64-to-text arrive with 8.M-link, re-run after that merge). Allocation-heavy rows (`tree`, `string_template`, `json_tokenize`, `map_set`) are flattered by bump allocation, and every iteration uses a fresh instance.\n", m["runtime_mode"].as_str().unwrap_or(""));
    if let (Some(cells), Some(max)) = (m["plan"]["cells"].as_array(), m["plan"]["bounds"]["max_invocations"].as_u64()) {
        let capped = cells.iter().filter(|c| c["invocations"].as_u64() == Some(max)).count();
        if m["plan"]["fixed"] != json!(true) && capped * 2 > cells.len() {
            let _ = writeln!(o, "**Precision target not reached:** {capped} of {} cells hit the cap of {max} invocations, so the wanted 2% half-width of the interval was limited by the time budget (and by a loaded, uncontrolled machine), not by the measured noise. Read the intervals, not the medians alone.\n", cells.len());
        }
    }
    let _ = writeln!(o, "Checksums: {}.\n", if s["digests_ok"] == json!(true) { "**all executors agree on every workload**" } else { "**MISMATCH, this run is invalid**" });
    let _ = writeln!(o, "## Machine\n\n| | |\n|---|---|");
    for (k, v) in [
        ("CPU", h["cpu_model"].to_string()),
        ("Logical CPUs / RAM", format!("{} / {} MiB", h["cpus_logical"], h["ram_total_mib"])),
        ("OS", h["os"].to_string()),
        ("Governor / turbo disabled", format!("{} / {}", h["governor"], h["turbo_disabled"])),
        ("Pinned core", h["pinned_core"].to_string()),
        ("Uncontrolled DVFS", h["uncontrolled_dvfs"].to_string()),
        ("Load average at start / end", format!("{} / {}", opt(&h["start"], "loadavg_1m"), opt(&h["end"], "loadavg_1m"))),
        ("Git commit", format!("{} (dirty: {})", m["git"]["commit"], m["git"]["dirty"])),
        ("Plan", format!("{}; {} warm-up calls, at least {} ms per timed iteration; per-cell counts in manifest.json", m["plan"]["description"].as_str().unwrap_or(""), m["plan"]["warmup_iterations"], m["plan"]["min_iter_ms"])),
    ] {
        let _ = writeln!(o, "| {k} | {} |", v.trim_matches('"'));
    }
    let _ = writeln!(o, "\n## Workloads\n\n| id | origin | what | track | n | n means |\n|---|---|---|---|---:|---|");
    for w in crate::workloads::WORKLOADS {
        let _ = writeln!(o, "| {} | {} | {} | {} | {} | {} |", w.id, w.origin.as_str(), w.title, w.track.as_str(), w.n, w.n_meaning);
    }
    let _ = writeln!(o, "\n`origin awfy` = Are We Fast Yet: the JavaScript twin is the **upstream file, vendored verbatim at a pinned commit** (`upstream/PIN`); the Rust, C and OxiScript twins are our ports of the same algorithm because no upstream port exists (so they are not independent of the OxiScript author). `origin own` = our own workload, every twin written here, **reported separately and never a headline**. `base` = identical algorithm in every language (sources in `bench/oxiperf/suites/<id>/`); `stdlib` = each language's built-in hash map/set (the C row is absent: C has none). Every executor returns a u32 checksum and all must agree.\n## Tools\n\n| Tool | Version | Flags |\n|---|---|---|");
    for (name, t) in m["toolchains"].as_object().into_iter().flatten() {
        let ver = ["version", "crate_version", "status"].iter().find_map(|k| t.get(*k).filter(|v| !v.is_null())).map_or("-".to_string(), |v| v.to_string().trim_matches('"').to_string());
        let _ = writeln!(o, "| {name} | {ver} | {} |", t.get("flags").map_or(String::new(), |v| v.to_string().trim_matches('"').to_string()));
    }
    let _ = writeln!(o, "\n## Wall time (warm, steady state)\n");
    let _ = writeln!(o, "Median over all measured iterations of all invocations, 95% hierarchical bootstrap interval of the median ({} resamples), p95, and the ratio to `{BASELINE}` with its interval. `verdict` follows R18: a difference counts only if the interval excludes 1.0 and the point estimate is at least {:.0}% away. `n` is the problem size. Fuel is wasmi's deterministic metering and is **not** comparable across engines.\n", BOOTSTRAP_RESAMPLES, MPES * 100.0);
    let _ = writeln!(o, "| workload (origin, track, n) | engine | median ms | 95% CI ms | p95 ms | vs rust-native | verdict | fuel | warm-up drift |\n|---|---|---:|---|---:|---|---|---:|---|");
    let rows = s["rows"].as_array().cloned().unwrap_or_default();
    for r in &rows {
        let ci = &r["ci95_ms"];
        let rv = &r["ratio_vs_rust_native"];
        let ratio = if rv.is_null() {
            if r["engine"].as_str() == Some(BASELINE) {
                "baseline".to_string()
            } else {
                "-".to_string()
            }
        } else {
            format!("{:.2}x [{:.2}, {:.2}]", rv["ratio"].as_f64().unwrap_or(f64::NAN), rv["ci95"][0].as_f64().unwrap_or(f64::NAN), rv["ci95"][1].as_f64().unwrap_or(f64::NAN))
        };
        let _ = writeln!(
            o,
            "| {} ({}, {}, {}) | {} | {} | {} to {} | {} | {} | {} | {} | {} |",
            r["benchmark"].as_str().unwrap_or(""), r["origin"].as_str().unwrap_or(""), r["track"].as_str().unwrap_or(""), r["n"], r["engine"].as_str().unwrap_or(""),
            sig(r["median_ms"].as_f64().unwrap_or(f64::NAN)), sig(ci[0].as_f64().unwrap_or(f64::NAN)), sig(ci[1].as_f64().unwrap_or(f64::NAN)),
            sig(r["p95_ms"].as_f64().unwrap_or(f64::NAN)), ratio, rv["verdict"].as_str().unwrap_or("-"),
            r["fuel"].as_u64().map_or("-".into(), |x| x.to_string()), r["warmup_status"].as_str().unwrap_or(""),
        );
    }
    let _ = writeln!(o, "\nGeometric mean of the ratio to `{BASELINE}` **per origin group** (`awfy` is the only candidate for a headline, `own` is a separate labelled number; both only mean something next to the table above; min and max are the extremes):\n");
    let _ = writeln!(o, "| origin | engine | benchmarks | geomean | min | max |\n|---|---|---:|---:|---:|---:|");
    for g in s["geomean_vs_rust_native"].as_array().into_iter().flatten() {
        let _ = writeln!(o, "| {} | {} | {} | {}x | {}x | {}x |", g["origin"].as_str().unwrap_or(""), g["engine"].as_str().unwrap_or(""), g["benchmarks"], sig(g["geomean"].as_f64().unwrap_or(f64::NAN)), sig(g["min"].as_f64().unwrap_or(f64::NAN)), sig(g["max"].as_f64().unwrap_or(f64::NAN)));
    }
    let _ = writeln!(o, "\n## Startup and compile latency\n");
    let _ = writeln!(o, "`cold process` = spawn to exit of a child that only runs the workload once (process start, engine start, load, first call). `load` = module load, validation/translation or compile, and instantiation of the first instance (Wasm engines). `first call` = the cold first execution. `compile` = source to artifact (OxiScript: in-process check, lowering, codegen and validation, median of 6 after a first call; C: gcc best of 3 with link; JS and the Rust baseline have no ahead-of-time step here).\n");
    let _ = writeln!(o, "| workload | engine | cold process ms | load ms | first call ms | compile ms (first / median) |\n|---|---|---:|---:|---:|---|");
    for r in &rows {
        let comp = match (r["compile_ms_first"].as_f64(), r["compile_ms_median"].as_f64()) {
            (Some(a), Some(b)) => format!("{} / {}", sig(a), sig(b)),
            (None, Some(b)) => sig(b),
            _ => "-".into(),
        };
        let _ = writeln!(o, "| {} | {} | {} | {} | {} | {} |", r["benchmark"].as_str().unwrap_or(""), r["engine"].as_str().unwrap_or(""), opt(r, "cold_process_ms_median"), opt(r, "load_ms_median"), opt(r, "cold_first_call_ms_median"), comp);
    }
    let _ = writeln!(o, "\n## Memory and size\n");
    let _ = writeln!(o, "`peak RSS` = `VmHWM` of the child process (median of invocations; includes the engine itself, so compare Wasm engines with each other and read the others as ceilings). `pages` = Wasm linear memory in 64 KiB pages after one call (stub runtime: bump allocation, no reuse). `artifact` = `.wasm` file, C executable (unstripped) or JS source.\n");
    let _ = writeln!(o, "| workload | engine | peak RSS MiB | Wasm pages | artifact |\n|---|---|---:|---:|---|");
    for r in &rows {
        let _ = writeln!(o, "| {} | {} | {} | {} | {} {} |", r["benchmark"].as_str().unwrap_or(""), r["engine"].as_str().unwrap_or(""), opt(r, "peak_rss_mib_median"), r["wasm_pages"].as_u64().map_or("-".into(), |x| x.to_string()), r["artifact"].as_str().unwrap_or(""), bytes(&r["artifact_bytes"]));
    }
    let _ = writeln!(o, "\n## Not measured yet (TODO, nothing here is faked)\n");
    for t in [
        "`oxiscript-w2c2` (wasm to C to gcc): w2c2 is not installed on this machine; no row.",
        "wasmi executed-op count (diagnostic mode of R11) and the `Linked` runtime mode: after 8.M-link merges, re-run and compare `runtime_mode`.",
        "Warm-up changepoint classification (Barrett et al.): only a first-third versus last-third drift check is done.",
        "R15 release sample sizes (10 invocations), controlled DVFS, isolated core, live-object count after cycle collection, PSS.",
        "Rust, Go, AssemblyScript, Python and Lua twins beyond the Rust baseline; browser and boards; the `peak` track.",
        "Per-language verification by a second reviewer (section 4.3 of the methodology); the twins here are written by one author.",
    ] {
        let _ = writeln!(o, "- {t}");
    }
    let _ = writeln!(o, "\nSee `docs/bench/oxiperf.md` for what each column means and for the claims policy (no website claims from a harness run).");
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdicts_follow_r18() {
        assert_eq!(verdict(1.03, 1.01, 1.05), "parity / within noise"); // excludes 1.0 but under 10%
        assert_eq!(verdict(1.5, 0.9, 2.0), "parity / within noise"); // CI straddles 1.0
        assert_eq!(verdict(2.0, 1.8, 2.2), "slower");
        assert_eq!(verdict(0.5, 0.45, 0.55), "faster");
    }

    #[test]
    fn sig_digits() {
        assert_eq!(sig(123.4), "123");
        assert_eq!(sig(12.34), "12.3");
        assert_eq!(sig(1.234), "1.23");
        assert_eq!(sig(0.1234), "0.123");
        assert_eq!(sig(f64::NAN), "n/a");
    }

    #[test]
    fn analyze_rejects_malformed_or_missing_checksums_even_if_marked_valid() {
        for (tag, cs) in [
            ("missing", None),
            ("null", Some(json!(null))),
            ("neg", Some(json!(-7))),
            ("frac", Some(json!(1.5))),
            ("str", Some(json!("42"))),
            ("overflow", Some(json!(u64::from(u32::MAX) + 1))),
        ] {
            let dir = std::env::temp_dir().join(format!("oxiperf-report-r1-40-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            let info = crate::machine::RunInfo {
                run_id: "test",
                tier: "dev",
                runtime_mode: "stub",
                plan: json!({ "description": "fixed", "fixed": true, "warmup_iterations": 1, "min_iter_ms": 1.0, "bounds": {}, "cells": [] }),
                pinned_core: None,
                engines: &[],
            };
            let manifest = crate::machine::build_manifest(&info, &crate::machine::snapshot());
            crate::driver::write_json(&dir.join("manifest.json"), &manifest).unwrap();
            let mut bad_row = json!({
                "kind": "steady", "benchmark": "matmul", "origin": "own", "language": "javascript",
                "engine": "js-node", "track": "base", "n": 12, "runtime_mode": "stub", "invocation_idx": 0,
                "checksum_stable": true, "valid": true, "iter_ns": [1.0, 1.0, 1.0]
            });
            if let Some(v) = cs {
                bad_row["checksum"] = v;
            }
            let good_row = json!({
                "kind": "steady", "benchmark": "matmul", "origin": "own", "language": "rust",
                "engine": "rust-native", "track": "base", "n": 12, "runtime_mode": "stub", "invocation_idx": 0,
                "checksum": 42, "checksum_stable": true, "valid": true, "iter_ns": [100.0, 100.0, 100.0]
            });
            let fake_digest = json!({ "kind": "digest", "benchmark": "matmul", "digest_ok": true, "checksum": 42, "executors": 1 });
            std::fs::write(dir.join("raw.jsonl"), format!("{bad_row}\n{good_row}\n{fake_digest}\n")).unwrap();
            analyze(&dir).unwrap();
            let summary: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("summary.json")).unwrap()).unwrap();
            assert_eq!(summary["digests_ok"], json!(false), "{tag}: digests_ok must be false");
            let rows = summary["rows"].as_array().unwrap();
            assert!(rows.iter().all(|r| r["engine"] != "js-node"), "{tag}: invalid js-node row must be excluded");
            assert!(rows.iter().all(|r| r["ratio_vs_rust_native"].is_null()), "{tag}: no verdict when digest failed");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
