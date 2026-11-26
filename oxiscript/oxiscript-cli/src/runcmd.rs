//! `oxi run` / `oxi build`: type-check the entry exactly like `oxi check`, lower it to Wasm and
//! either write the module (`build`) or run it (`run`) in `oxiscript-runtime` with the same
//! capabilities and limits as the browser playground (`oxiscript_playground::run`).
//!
//! Exit codes: 0 ok, 1 the program has errors, 2 usage / I/O / configuration (including a program that
//! needs a host interface this tool does not provide), 4 the program trapped (see SCHEMA.md).

use crate::check::{project_for, HostCache, EXIT_FATAL, EXIT_FINDINGS};
use crate::needs;
use crate::project::{self, Entry};
use crate::report;
use oxiscript_playground::run::{run_wasm, DEFAULT_FUEL};
use std::collections::HashMap;
use std::path::PathBuf;

/// The program ran and stopped on a trap (overflow, bounds, out of fuel, ...).
pub const EXIT_TRAP: i32 = 4;

pub struct Args {
    pub file: Option<PathBuf>,
    pub fuel: Option<u64>,
    pub output: Option<PathBuf>,
    pub build: bool,
}

/// Plain-English reason for a stop id from the runtime.
fn explain(reason: &str) -> &'static str {
    match reason {
        "overflow" => "an integer operation overflowed (use `+%` to wrap or `+|` to saturate)",
        "div_by_zero" => "division by zero",
        "bounds" => "an array or string index was out of bounds",
        "bad_cast" => "a checked downcast failed",
        "null_deref" => "a null value was used",
        "out_of_memory" => "the program ran out of memory",
        "assert" => "an assertion failed",
        "bad_utf8" => "invalid UTF-8 text",
        "panic" => "the program called panic",
        "unreachable" => "the program reached code that cannot run yet (for example, an f64 converted to text is not supported by the current runtime)",
        "out_of_fuel" => "the program used up its instruction budget (an endless loop? raise it with --fuel)",
        "stack_overflow" => "the call stack is too deep (endless recursion?)",
        "memory_bounds" => "a memory access was out of range",
        _ => "the runtime stopped the program",
    }
}

pub fn run(a: &Args) -> i32 {
    let cmd = if a.build { "build" } else { "run" };
    let entry = a.file.clone().unwrap_or_else(|| PathBuf::from("main.oxs"));
    let src = match project::read_regular(&entry, project::MAX_SOURCE_BYTES) {
        Ok(s) => s,
        Err(project::ReadError::Skipped(e) | project::ReadError::Failed(e) | project::ReadError::TooLarge(e) | project::ReadError::NotUtf8(e)) => {
            eprintln!("error: {}: {e}", entry.display());
            return EXIT_FATAL;
        }
    };
    let mut cache: HostCache = HashMap::new();
    let prj = match project_for(&mut cache, &entry) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_FATAL;
        }
    };
    let ent = Entry { path: &entry, src };
    let checked = project::check_entry(&ent, &prj.opts);
    let mut diags = report::collect(&checked);
    if let Some(m) = &prj.manifest {
        diags.extend(needs::check(&checked, m, &prj.opts.hosts));
    }
    if !diags.is_empty() {
        let sources: HashMap<String, String> = checked.files.iter().map(|f| (f.display.clone(), f.src.clone())).collect();
        for d in &diags {
            eprintln!("{}", report::render_human(d, &|f| sources.get(f).cloned(), false));
        }
    }
    if diags.iter().any(|d| d.severity == "error" || d.severity == "internal") {
        eprintln!("oxi {cmd}: not {}: the program has errors", if a.build { "built" } else { "run" });
        return EXIT_FINDINGS;
    }
    let wasm = match project::compile_entry(&ent, &prj.opts) {
        Ok(w) => w,
        Err(ds) => {
            for d in &ds.0 {
                eprintln!("error[{}]: {}", d.code, d.message);
            }
            eprintln!("oxi {cmd}: the program could not be compiled");
            return EXIT_FINDINGS;
        }
    };
    if a.build {
        let out = a.output.clone().unwrap_or_else(|| {
            let name = prj.manifest.as_ref().map_or_else(|| entry.file_stem().map_or_else(|| "out".to_string(), |s| s.to_string_lossy().into_owned()), |m| m.name.clone());
            PathBuf::from(format!("{name}.wasm"))
        });
        if let Err(e) = std::fs::write(&out, &wasm) {
            eprintln!("error: {}: {e}", out.display());
            return EXIT_FATAL;
        }
        eprintln!("oxi build: wrote {} ({} bytes)", out.display(), wasm.len());
        return 0;
    }
    let report = run_wasm(&wasm, a.fuel.unwrap_or(DEFAULT_FUEL));
    for l in &report.lines {
        if l.level >= 3 {
            eprintln!("{}: {}", if l.level == 3 { "warn" } else { "error" }, l.msg);
        } else {
            println!("{}", l.msg);
        }
    }
    if report.dropped > 0 {
        eprintln!("oxi run: {} log line(s) dropped (output cap)", report.dropped);
    }
    match report.stop {
        None => 0,
        Some(s) if s.reason == "rejected" => {
            eprintln!("error: {} (`oxi run` provides host:log@1 and host:state@1; use `oxi build` for other interfaces)", s.detail);
            EXIT_FATAL
        }
        Some(s) => {
            eprintln!("oxi run: stopped: {} [{}]", explain(s.reason), s.code);
            EXIT_TRAP
        }
    }
}
