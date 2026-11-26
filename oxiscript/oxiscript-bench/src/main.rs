//! `oxiperf`: the `OxiScript` performance benchmark harness (stage 8.Bench, `OxiPerf` v0).
//!
//! ```text
//! oxiperf run [--out DIR] [--quick] [--invocations N --iters N: fixed plan instead of a derived one] [--max-invocations N] [--max-iters N] [--warmup N] [--min-iter-ms MS]
//!             [--workloads a,b] [--engines x,y] [--pin-core C] [--tier dev|ci]
//! oxiperf analyze --input DIR        # regenerate summary.json and report.md from raw.jsonl only
//! oxiperf exec --engine wasmi|wasmtime|native ...   # internal: one measured child process
//! ```

// Statistics and benchmark plumbing: single-letter names follow the maths, the report and driver
// functions are long straight-line code, and measurements are cast between integer and float types.
#![allow(clippy::many_single_char_names, clippy::too_many_lines)]

mod child;
mod driver;
mod machine;
mod native;
mod report;
mod stats;
mod workloads;

use std::path::PathBuf;

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn num<T: std::str::FromStr>(args: &[String], name: &str, default: T) -> Result<T, String> {
    match arg(args, name) {
        Some(v) => v.parse().map_err(|_| format!("{name}: bad number `{v}`")),
        None => Ok(default),
    }
}

fn list(args: &[String], name: &str) -> Vec<String> {
    arg(args, name).map_or_else(Vec::new, |v| v.split(',').map(str::to_string).collect())
}

fn today() -> String {
    machine::tool_version("date", &["+%F"]).unwrap_or_else(|| "unknown-date".into())
}

fn exec(args: &[String]) -> Result<(), String> {
    let engine = arg(args, "--engine").ok_or("--engine")?;
    let id = arg(args, "--workload").ok_or("--workload")?;
    let n: i32 = num(args, "--n", 0)?;
    let (warmup, iters): (usize, usize) = (num(args, "--warmup", 3)?, num(args, "--iters", 30)?);
    let min_ms: f64 = num(args, "--min-ms", 10.0)?;
    let wasm = || std::fs::read(arg(args, "--wasm").ok_or("--wasm")?).map_err(|e| e.to_string());
    let out = match engine.as_str() {
        "native" => child::measure(&mut child::Native { id, n }, warmup, iters, min_ms)?,
        "wasmi" => child::measure(&mut child::Wasmi::new(wasm()?, n), warmup, iters, min_ms)?,
        #[cfg(feature = "wasmtime")]
        "wasmtime" => {
            let mut wt = child::Wt::new(&wasm()?, n)?;
            let compile_ns = wt.compile_ns;
            let mut v = child::measure(&mut wt, warmup, iters, min_ms)?;
            v["compile_ns"] = serde_json::json!(compile_ns);
            // The module is compiled once, before `load_ns` (which then measures instantiation only).
            v["load_ns"] = serde_json::json!(v["load_ns"].as_f64().unwrap_or(0.0) + compile_ns);
            v
        }
        other => return Err(format!("unknown or disabled engine `{other}`")),
    };
    println!("{out}");
    Ok(())
}

fn real_main() -> Result<i32, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("exec") => exec(&args).map(|()| 0),
        Some("analyze") => {
            let dir = PathBuf::from(arg(&args, "--input").ok_or("analyze needs --input DIR")?);
            report::analyze(&dir)?;
            println!("wrote {}/summary.json and report.md", dir.display());
            Ok(0)
        }
        Some("run") => {
            let quick = args.iter().any(|a| a == "--quick");
            let out = match arg(&args, "--out") {
                Some(p) => PathBuf::from(p),
                None => workloads::bench_root().join("results").join(format!("{}-{}", today(), machine::machine_slug())),
            };
            let cfg = driver::Config {
                out,
                fixed: if quick || arg(&args, "--invocations").is_some() || arg(&args, "--iters").is_some() {
                    Some((num(&args, "--invocations", 1)?, num(&args, "--iters", if quick { 3 } else { 30 })?))
                } else {
                    None
                },
                bounds: stats::PlanBounds { min_invocations: 3, max_invocations: num(&args, "--max-invocations", 10)?, min_iters: 10, max_iters: num(&args, "--max-iters", 50)?, target_rel_halfwidth: 0.02 },
                pilot_invocations: 3,
                pilot_iters: 15,
                warmup: num(&args, "--warmup", if quick { 1 } else { 3 })?,
                min_iter_ms: num(&args, "--min-iter-ms", if quick { 1.0 } else { 10.0 })?,
                only_workloads: list(&args, "--workloads"),
                only_engines: list(&args, "--engines"),
                quick,
                pin_core: arg(&args, "--pin-core").map(|v| v.parse().map_err(|_| "--pin-core".to_string())).transpose()?,
                tier: arg(&args, "--tier").unwrap_or_else(|| "dev".into()),
            };
            let bad = driver::run(&cfg)?;
            println!("report: {}/report.md", cfg.out.display());
            Ok(i32::from(bad != 0))
        }
        _ => {
            eprintln!("usage: oxiperf run [--quick] [--out DIR] [...] | oxiperf analyze --input DIR");
            Ok(2)
        }
    }
}

fn main() {
    match real_main() {
        Ok(0) => {}
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("oxiperf: {e}");
            std::process::exit(2);
        }
    }
}
