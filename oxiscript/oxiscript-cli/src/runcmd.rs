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
use oxiscript_abi::target::Target;
use oxiscript_playground::run::{run_wasm_on, DEFAULT_FUEL};
use std::collections::HashMap;
use std::path::PathBuf;

/// The program ran and stopped on a trap (overflow, bounds, out of fuel, ...).
pub const EXIT_TRAP: i32 = 4;

pub struct Args {
    pub file: Option<PathBuf>,
    pub fuel: Option<u64>,
    pub output: Option<PathBuf>,
    pub build: bool,
    pub target: Option<String>,
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
        "host_denied" => "the program used a capability it was not given",
        "host_bad_argument" => "a host function rejected an argument (for example a log message or state key over 64 KiB)",
        "unreachable" => "the program reached unreachable code",
        "out_of_fuel" => "the program used up its instruction budget (an endless loop? raise it with --fuel)",
        "stack_overflow" => "the call stack is too deep (endless recursion?)",
        "memory_bounds" => "a memory access was out of range",
        _ => "the runtime stopped the program",
    }
}

/// `eprintln!` that never panics: a closed or full stderr is ignored (the exit code still says what happened).
macro_rules! errln {
    ($($a:tt)*) => {{
        use std::io::Write;
        let _ = writeln!(std::io::stderr().lock(), $($a)*);
    }};
}

/// A package name that is safe to use as the default output file name.
fn valid_package_name(n: &str) -> bool {
    (1..=64).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Output sink for `run`: a closed pipe ends quietly (exit 0, like `cat | head`), any other write error is exit 2.
enum Sink {
    Ok,
    Closed,
    Failed(String),
}

fn emit(to_err: bool, line: &str) -> Sink {
    use std::io::Write;
    let r = if to_err { writeln!(std::io::stderr().lock(), "{line}") } else { writeln!(std::io::stdout().lock(), "{line}") };
    match r {
        Ok(()) => Sink::Ok,
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Sink::Closed,
        Err(e) => Sink::Failed(e.to_string()),
    }
}

pub fn run(a: &Args) -> i32 {
    let cmd = if a.build { "build" } else { "run" };
    let entry = a.file.clone().unwrap_or_else(|| PathBuf::from("main.oxs"));
    let src = match project::read_regular(&entry, project::MAX_SOURCE_BYTES) {
        Ok(s) => s,
        Err(project::ReadError::Skipped(e) | project::ReadError::Failed(e) | project::ReadError::TooLarge(e) | project::ReadError::NotUtf8(e)) => {
            errln!("error: {}: {e}", entry.display());
            return EXIT_FATAL;
        }
    };
    let mut cache: HostCache = HashMap::new();
    let prj = match project_for(&mut cache, &entry) {
        Ok(h) => h,
        Err(e) => {
            errln!("error: {e}");
            return EXIT_FATAL;
        }
    };
    let target = a.target.as_deref().and_then(Target::parse).or_else(|| prj.manifest.as_ref().and_then(|m| m.target)).unwrap_or_default();
    let ent = Entry { path: &entry, src };
    let checked = project::check_entry(&ent, &prj.opts);
    let mut diags = report::collect(&checked);
    if let Some(m) = &prj.manifest {
        diags.extend(needs::check(&checked, m, &prj.opts.hosts));
    }
    if !diags.is_empty() {
        let sources: HashMap<String, String> = checked.files.iter().map(|f| (f.display.clone(), f.src.clone())).collect();
        for d in &diags {
            errln!("{}", report::render_human(d, &|f| sources.get(f).cloned(), false));
        }
    }
    if diags.iter().any(|d| d.severity == "error" || d.severity == "internal") {
        errln!("oxi {cmd}: not {}: the program has errors", if a.build { "built" } else { "run" });
        return EXIT_FINDINGS;
    }
    let wasm = match project::compile_entry(&ent, &prj.opts, target) {
        Ok(w) => w,
        Err(ds) => {
            for d in &ds.0 {
                errln!("error[{}]: {}", d.code, d.message);
            }
            errln!("oxi {cmd}: the program could not be compiled");
            return EXIT_FINDINGS;
        }
    };
    if a.build {
        let out = if let Some(o) = a.output.clone() {
            o
        } else {
            let name = prj.manifest.as_ref().map_or_else(|| entry.file_stem().map_or_else(|| "out".to_string(), |s| s.to_string_lossy().into_owned()), |m| m.name.clone());
            if !valid_package_name(&name) {
                errln!("error: [package] name {name:?} is not usable as a file name (allowed: A-Z a-z 0-9 _ -, 1 to 64 characters); fix oxi.toml or pass -o FILE");
                return EXIT_FATAL;
            }
            PathBuf::from(format!("{name}.wasm"))
        };
        if let Err(e) = project::write_atomic_bytes(&out, &wasm, true) {
            errln!("error: {}: {e}", out.display());
            return EXIT_FATAL;
        }
        errln!("oxi build: wrote {} ({} bytes, target {target})", out.display(), wasm.len());
        return 0;
    }
    let fuel = match a.fuel {
        None | Some(0) => DEFAULT_FUEL,
        Some(n) => n,
    };
    let report = run_wasm_on(&wasm, fuel, target);
    let mut pipe_closed = false;
    for l in &report.lines {
        let r = if l.level >= 3 { emit(true, &format!("{}: {}", if l.level == 3 { "warn" } else { "error" }, l.msg)) } else { emit(false, &l.msg) };
        match r {
            Sink::Ok => {}
            Sink::Closed => {
                pipe_closed = true;
                break;
            }
            Sink::Failed(e) => {
                let _ = emit(true, &format!("oxi run: cannot write output: {e}"));
                return EXIT_FATAL;
            }
        }
    }
    if pipe_closed {
        return 0;
    }
    if report.dropped > 0 {
        let _ = emit(true, &format!("oxi run: {} log line(s) dropped (output cap)", report.dropped));
    }
    match report.stop {
        None => 0,
        Some(s) if s.reason == "rejected" => {
            let _ = emit(true, &format!("error: {} (`oxi run` provides host:log@1 and host:state@1; use `oxi build` for other interfaces)", s.detail));
            EXIT_FATAL
        }
        Some(s) => {
            let _ = emit(true, &format!("oxi run: stopped: {} [{}]", explain(s.reason), s.code));
            EXIT_TRAP
        }
    }
}
