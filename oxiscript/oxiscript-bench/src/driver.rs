//! `oxiperf run`: build the artifacts, run every (workload, executor, invocation) in a child
//! process, interleaved round-robin (R13), verify checksums, write the raw data and call `analyze`.

use crate::machine;
use crate::workloads::{self, Workload};
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

pub const RUNTIME_MODE: &str = "stub";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Engine {
    Wasmi,
    Wasmtime,
    Native,
    Node,
    C,
}

impl Engine {
    pub const ALL: [Engine; 5] = [Engine::Wasmi, Engine::Wasmtime, Engine::Native, Engine::Node, Engine::C];
    pub fn id(self) -> &'static str {
        match self {
            Engine::Wasmi => "oxiscript-wasmi",
            Engine::Wasmtime => "oxiscript-wasmtime",
            Engine::Native => "rust-native",
            Engine::Node => "js-node",
            Engine::C => "c-gcc",
        }
    }
    pub fn language(self) -> &'static str {
        match self {
            Engine::Wasmi | Engine::Wasmtime => "oxiscript",
            Engine::Native => "rust",
            Engine::Node => "javascript",
            Engine::C => "c",
        }
    }
}

pub struct Config {
    pub out: PathBuf,
    /// `Some((invocations, iterations))` = a fixed plan (quick runs, explicit flags); `None` = derive
    /// the counts per cell from a measured pilot (Kalibera & Jones), then freeze them.
    pub fixed: Option<(usize, usize)>,
    pub bounds: crate::stats::PlanBounds,
    pub pilot_invocations: usize,
    pub pilot_iters: usize,
    pub warmup: usize,
    pub min_iter_ms: f64,
    pub only_workloads: Vec<String>,
    pub only_engines: Vec<String>,
    pub quick: bool,
    pub pin_core: Option<usize>,
    pub tier: String,
}

fn n_for(w: &Workload, cfg: &Config) -> i32 {
    if cfg.quick {
        w.n_quick
    } else {
        w.n
    }
}

fn median_f(v: &mut [f64]) -> f64 {
    crate::stats::median(v)
}

/// Compile the `OxiScript` source `reps` times; returns the wasm and the compile times (ms).
pub fn compile_oxs(id: &str, reps: usize) -> Result<(Vec<u8>, Vec<f64>), String> {
    let src = workloads::oxs_source(id);
    let mut wasm = Vec::new();
    let mut times = Vec::new();
    for _ in 0..reps {
        let t = Instant::now();
        wasm = oxiscript_lower::compile_source(&src, &[], &oxiscript_lower::Options::default()).map_err(|e| format!("{id}: compile failed:\n{e}"))?;
        times.push(t.elapsed().as_secs_f64() * 1e3);
    }
    Ok((wasm, times))
}

struct Artifacts {
    wasm: Option<PathBuf>,
    c_bin: Option<PathBuf>,
}

fn which(cmd: &str, arg: &str) -> bool {
    machine::tool_version(cmd, &[arg]).is_some()
}

fn child_cmd(cfg: &Config, program: &str, args: Vec<String>) -> Command {
    if let Some(c) = cfg.pin_core {
        let mut cmd = Command::new("taskset");
        cmd.arg("-c").arg(c.to_string()).arg(program).args(args);
        cmd
    } else {
        let mut cmd = Command::new(program);
        cmd.args(args);
        cmd
    }
}

/// Deterministic pseudo-random padding length for invocation `i` (R14).
pub fn env_pad(i: usize) -> usize {
    let mut r = crate::stats::Rng::new(0x0A11_CE00 + i as u64);
    r.below(4097)
}

#[allow(clippy::too_many_arguments)]
fn spawn(cfg: &Config, exe: &Path, e: Engine, w: &Workload, n: i32, art: &Artifacts, warmup: usize, iters: usize, pad: usize) -> Result<(Value, f64), String> {
    let nums = |v: &mut Vec<String>| {
        v.push(n.to_string());
        v.push(warmup.to_string());
        v.push(iters.to_string());
        v.push(cfg.min_iter_ms.to_string());
    };
    let mut cmd = match e {
        Engine::Wasmi | Engine::Wasmtime | Engine::Native => {
            let mut a = vec!["exec".to_string(), "--engine".into(), if e == Engine::Wasmi { "wasmi" } else if e == Engine::Wasmtime { "wasmtime" } else { "native" }.into()];
            a.push("--workload".into());
            a.push(w.id.into());
            if let Some(p) = &art.wasm {
                a.push("--wasm".into());
                a.push(p.display().to_string());
            }
            for (k, v) in [("--n", n.to_string()), ("--warmup", warmup.to_string()), ("--iters", iters.to_string()), ("--min-ms", cfg.min_iter_ms.to_string())] {
                a.push(k.into());
                a.push(v);
            }
            child_cmd(cfg, &exe.display().to_string(), a)
        }
        Engine::Node => {
            let mut a = vec![
                workloads::bench_root().join("drivers/node_runner.js").display().to_string(),
                workloads::suite_file(w.id, "main.js").display().to_string(),
            ];
            nums(&mut a);
            child_cmd(cfg, "node", a)
        }
        Engine::C => {
            let mut a = vec![];
            nums(&mut a);
            child_cmd(cfg, &art.c_bin.as_ref().ok_or("no C binary")?.display().to_string(), a)
        }
    };
    cmd.env("OXIPERF_ENV_PAD", "x".repeat(pad));
    let t = Instant::now();
    let out = cmd.output().map_err(|er| format!("spawn {}: {er}", e.id()))?;
    let wall_ms = t.elapsed().as_secs_f64() * 1e3;
    if !out.status.success() {
        return Err(format!("{} {} exited with {}: {}", e.id(), w.id, out.status, String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("")));
    }
    let line = String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("").to_string();
    let v: Value = serde_json::from_str(&line).map_err(|er| format!("{} {}: bad child output ({er}): {line:.200}", e.id(), w.id))?;
    Ok((v, wall_ms))
}

fn gcc_build(w: &Workload, dir: &Path) -> Result<(PathBuf, f64, u64), String> {
    let bin = dir.join(w.id);
    let mut best = f64::MAX;
    for _ in 0..3 {
        let t = Instant::now();
        let st = Command::new("gcc")
            .args(["-O2", "-ffp-contract=off", "-DNDEBUG", "-o"])
            .arg(&bin)
            .arg(workloads::suite_file(w.id, "main.c"))
            .status()
            .map_err(|e| format!("gcc: {e}"))?;
        if !st.success() {
            return Err(format!("gcc failed on {}", w.id));
        }
        best = best.min(t.elapsed().as_secs_f64() * 1e3);
    }
    let size = std::fs::metadata(&bin).map_or(0, |m| m.len());
    Ok((bin, best, size))
}

/// Runs the whole plan. Returns the number of checksum mismatches (the run fails if > 0).
pub fn run(cfg: &Config) -> Result<usize, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(cfg.out.join("artifacts")).map_err(|e| e.to_string())?;
    let tmp = std::env::var("TMPDIR").map_or_else(|_| cfg.out.join("artifacts"), PathBuf::from);
    let cdir = tmp.join("oxiperf-c");
    std::fs::create_dir_all(&cdir).map_err(|e| e.to_string())?;

    let workloads: Vec<&Workload> = workloads::WORKLOADS.iter().filter(|w| cfg.only_workloads.is_empty() || cfg.only_workloads.iter().any(|x| x == w.id)).collect();
    if workloads.is_empty() {
        return Err("no workload selected".into());
    }
    // Engines available on this machine.
    let node_ok = which("node", "--version");
    let gcc_ok = which("gcc", "--version");
    let mut engines: Vec<Engine> = vec![];
    let mut notes: Vec<(String, String)> = vec![];
    for e in Engine::ALL {
        let (ok, note) = match e {
            Engine::Wasmi | Engine::Native => (true, "built in"),
            Engine::Wasmtime => (cfg!(feature = "wasmtime"), if cfg!(feature = "wasmtime") { "wasmtime crate" } else { "unavailable: built with --no-default-features" }),
            Engine::Node => (node_ok, if node_ok { "node on PATH" } else { "unavailable: node not found" }),
            Engine::C => (gcc_ok, if gcc_ok { "gcc on PATH" } else { "unavailable: gcc not found" }),
        };
        let wanted = cfg.only_engines.is_empty() || cfg.only_engines.iter().any(|x| x == e.id());
        if ok && wanted {
            engines.push(e);
        }
        notes.push((e.id().to_string(), if wanted { note.to_string() } else { "not selected".into() }));
    }
    notes.push(("oxiscript-w2c2".to_string(), "TODO: w2c2 is not installed on this machine; no row is produced".to_string()));

    let manifest_path = cfg.out.join("manifest.json");
    let info = machine::RunInfo {
        run_id: cfg.out.file_name().and_then(|s| s.to_str()).unwrap_or("run"),
        tier: &cfg.tier,
        runtime_mode: RUNTIME_MODE,
        plan: plan_json(cfg, None),
        pinned_core: cfg.pin_core,
        engines: &notes,
    };
    let mut manifest = machine::build_manifest(&info, &machine::snapshot());
    machine::validate_manifest(&manifest)?;
    write_json(&manifest_path, &manifest)?; // immutable-before-measurement copy; `end` is filled in afterwards

    let mut raw = std::fs::File::create(cfg.out.join("raw.jsonl")).map_err(|e| e.to_string())?;
    let mut emit = |v: Value| -> Result<(), String> { writeln!(raw, "{v}").map_err(|e| e.to_string()) };

    // Artifacts and compile latency.
    let mut arts: Vec<(&Workload, Artifacts)> = vec![];
    for w in &workloads {
        let (wasm, times) = compile_oxs(w.id, if cfg.quick { 3 } else { 7 })?;
        let path = cfg.out.join("artifacts").join(format!("{}.wasm", w.id));
        std::fs::write(&path, &wasm).map_err(|e| e.to_string())?;
        let t = times.clone();
        let steady = median_f(&mut t[1..].to_vec());
        for e in [Engine::Wasmi, Engine::Wasmtime] {
            emit(json!({ "kind": "artifact", "benchmark": w.id, "engine": e.id(), "artifact": "wasm", "artifact_bytes": wasm.len(), "compile_ms_first": times[0], "compile_ms_median": steady, "compile_note": "oxiscript compile_source in-process (check+lower+codegen+validate), first call and median of the rest" }))?;
        }
        let mut c_bin = None;
        if gcc_ok && w.has_c && engines.contains(&Engine::C) {
            let (bin, ms, size) = gcc_build(w, &cdir)?;
            emit(json!({ "kind": "artifact", "benchmark": w.id, "engine": "c-gcc", "artifact": "executable", "artifact_bytes": size, "compile_ms_median": ms, "compile_note": "gcc -O2, best of 3, includes link; size of the unstripped executable" }))?;
            c_bin = Some(bin);
        }
        let js = std::fs::metadata(workloads::suite_file(w.id, "main.js")).map_or(0, |m| m.len());
        emit(json!({ "kind": "artifact", "benchmark": w.id, "engine": "js-node", "artifact": "source", "artifact_bytes": js, "compile_note": "no ahead-of-time step; V8 compiles at load" }))?;
        arts.push((w, Artifacts { wasm: Some(path), c_bin }));
    }

    let mut results: Vec<(String, Vec<(String, u32)>)> = workloads.iter().map(|w| (w.id.to_string(), vec![])).collect();
    let mut invalid = 0usize;

    // One child run -> one raw record. Checksums of steady and pilot runs feed the agreement check.
    let mut one = |emit: &mut dyn FnMut(Value) -> Result<(), String>, kind: &str, inv: usize, wi: usize, w: &Workload, art: &Artifacts, e: Engine, warm: usize, it: usize| -> Result<Option<Value>, String> {
        let n = n_for(w, cfg);
        let pad = env_pad(inv);
        eprintln!("[{kind} {}] {:<16} {:<19}", inv + 1, w.id, e.id());
        let mut rec = json!({ "kind": kind, "benchmark": w.id, "origin": w.origin.as_str(), "language": e.language(), "engine": e.id(), "track": w.track.as_str(), "size_profile": "server", "n": n, "runtime_mode": RUNTIME_MODE, "invocation_idx": inv, "env_pad_bytes": pad });
        let mut ret = None;
        match spawn(cfg, &exe, e, w, n, art, warm, it, pad) {
            Ok((v, wall)) => {
                let ok = v["checksum_stable"].as_bool().unwrap_or(false) && v.get("fuel_stable").and_then(Value::as_bool).unwrap_or(true);
                for (k2, v2) in v.as_object().unwrap() {
                    rec[k2] = v2.clone();
                }
                rec["process_ms"] = json!(wall);
                rec["valid"] = json!(ok);
                if !ok {
                    rec["invalid_reason"] = json!("checksum or fuel not stable across iterations");
                    invalid += 1;
                }
                if kind != "cold" {
                    if let Some(c) = v["checksum"].as_u64() {
                        results[wi].1.push((format!("{} ({kind} {inv})", e.id()), c as u32));
                    }
                }
                ret = Some(rec.clone());
            }
            Err(msg) => {
                rec["valid"] = json!(false);
                rec["invalid_reason"] = json!(msg);
                invalid += 1;
                eprintln!("  INVALID: {msg}");
            }
        }
        emit(rec)?;
        Ok(ret)
    };

    // Phase 1: dimensioning. A pilot per cell (workload x engine); the counts are computed from its
    // variance components and cost, then frozen (R21) before any measured run.
    let mut plans: std::collections::BTreeMap<(usize, usize), (usize, usize)> = std::collections::BTreeMap::default();
    let mut plan_cells: Vec<Value> = vec![];
    for (wi, (w, art)) in arts.iter().enumerate() {
        for (ei, e) in engines.iter().enumerate() {
            if *e == Engine::C && art.c_bin.is_none() {
                continue;
            }
            let cell = if let Some((inv, it)) = cfg.fixed { (inv, it) } else {
                let mut groups: Vec<Vec<f64>> = vec![];
                let mut costs: Vec<f64> = vec![];
                for p in 0..cfg.pilot_invocations {
                    if let Some(r) = one(&mut emit, "pilot", p, wi, w, art, *e, cfg.warmup, cfg.pilot_iters)? {
                        if r["valid"] == json!(true) {
                            let its: Vec<f64> = r["iter_ns"].as_array().unwrap().iter().filter_map(Value::as_f64).collect();
                            let reps = r["inner_reps"].as_f64().unwrap_or(1.0);
                            let timed = (cfg.warmup + its.len()) as f64 * reps * crate::stats::mean(&its);
                            costs.push((r["process_ms"].as_f64().unwrap_or(0.0) * 1e6 - timed).max(1e5));
                            groups.push(its);
                        }
                    }
                }
                if groups.len() < 2 {
                    (cfg.bounds.min_invocations, cfg.bounds.min_iters)
                } else {
                    crate::stats::plan_counts(&groups, crate::stats::median(&costs), cfg.bounds)
                }
            };
            plans.insert((wi, ei), cell);
            plan_cells.push(json!({ "benchmark": w.id, "engine": e.id(), "invocations": cell.0, "iterations_per_invocation": cell.1 }));
            emit(json!({ "kind": "plan", "benchmark": w.id, "engine": e.id(), "invocations": cell.0, "iterations_per_invocation": cell.1, "derived_from_pilot": cfg.fixed.is_none() }))?;
        }
    }
    manifest["plan"] = plan_json(cfg, Some(&plan_cells));
    write_json(&manifest_path, &manifest)?; // the frozen plan, before the measured phase

    // Phase 2: measured runs, interleaved round-robin across engines (R13).
    let max_inv = plans.values().map(|p| p.0).max().unwrap_or(0);
    for inv in 0..max_inv {
        for (wi, (w, art)) in arts.iter().enumerate() {
            for k in 0..engines.len() {
                let ei = (k + inv) % engines.len();
                let e = engines[ei];
                let Some(&(planned_inv, it)) = plans.get(&(wi, ei)) else { continue };
                if inv >= planned_inv {
                    continue;
                }
                one(&mut emit, "cold", inv, wi, w, art, e, 0, 0)?;
                one(&mut emit, "steady", inv, wi, w, art, e, cfg.warmup, it)?;
            }
        }
    }

    // Checksums: every executor of a workload must agree (R01, section 4.3).
    let mut mismatches = 0usize;
    for (id, rs) in &results {
        match workloads::verify_checksums(rs) {
            Ok(c) => emit(json!({ "kind": "digest", "benchmark": id, "digest_ok": true, "checksum": c, "executors": rs.len() }))?,
            Err(m) => {
                mismatches += 1;
                eprintln!("CHECKSUM MISMATCH in {id}: {m}");
                emit(json!({ "kind": "digest", "benchmark": id, "digest_ok": false, "error": m.to_string() }))?;
            }
        }
    }
    drop(raw);
    manifest["host"]["end"] = machine::snapshot();
    manifest["invalid_invocations"] = json!(invalid);
    write_json(&manifest_path, &manifest)?;
    crate::report::analyze(&cfg.out)?;
    if invalid > 0 {
        eprintln!("{invalid} invalid child run(s); see raw.jsonl");
    }
    Ok(mismatches + invalid)
}

pub fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    std::fs::write(path, serde_json::to_string_pretty(v).unwrap() + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

fn plan_json(cfg: &Config, cells: Option<&[Value]>) -> Value {
    let b = cfg.bounds;
    json!({
        "description": match cfg.fixed {
            Some((i, n)) => format!("fixed: {i} invocations x {n} iterations per cell"),
            None => format!("derived per cell from a pilot ({} invocations x {} iterations) after Kalibera & Jones: iterations balance invocation cost against variance, invocations reach a {:.0}% half-width of the 95% interval of the mean; clamped to {}..{} invocations and {}..{} iterations (release tier would need at least 10 invocations)", cfg.pilot_invocations, cfg.pilot_iters, b.target_rel_halfwidth * 100.0, b.min_invocations, b.max_invocations, b.min_iters, b.max_iters),
        },
        "fixed": cfg.fixed.is_some(),
        "warmup_iterations": cfg.warmup,
        "min_iter_ms": cfg.min_iter_ms,
        "bounds": { "min_invocations": b.min_invocations, "max_invocations": b.max_invocations, "min_iters": b.min_iters, "max_iters": b.max_iters, "target_rel_halfwidth": b.target_rel_halfwidth },
        "cells": cells,
    })
}
