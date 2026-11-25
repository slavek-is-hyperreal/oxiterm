//! `oxi run` / `oxi build`: type-check the entry exactly like `oxi check`, then stop at
//! the point where slice 8.L (lowering) plugs in.
//!
//! Wiring plan (a few lines, at the marked spot): `run.checked` is a clean
//! [`oxiscript_check::Checked`] there and `entry` is the project-relative entry path.
//!
//! ```text
//! // TODO(8.L): expected signature, adjust to the real one when merged (the 8.L crate
//! // `oxiscript-lower` was an empty skeleton when this was written, so this is a guess):
//! // oxiscript_lower::compile_source(checked: &Checked, entry_name: &str)
//! //     -> Result<Vec<u8> /* wasm module */, Vec<oxiscript_syntax::Diagnostic>>
//! ```
//!
//! `build` then writes the bytes to `<package name>.wasm`; `run` instantiates them with
//! `oxiscript-runtime`, handing it the capabilities named by `oxi.toml` `needs` in order.

use crate::check::{hosts_for, HostCache, EXIT_FATAL, EXIT_FINDINGS};
use crate::project::{self, Entry};
use crate::report;
use std::collections::HashMap;
use std::path::PathBuf;

/// The command exists but its backend is not merged yet.
pub const EXIT_NOT_AVAILABLE: i32 = 3;

pub fn run(cmd: &str, paths: &[PathBuf]) -> i32 {
    let entry = paths.first().cloned().unwrap_or_else(|| PathBuf::from("main.oxs"));
    let src = match std::fs::read_to_string(&entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {}: {e}", entry.display());
            return EXIT_FATAL;
        }
    };
    let mut cache: HostCache = HashMap::new();
    let hosts = match hosts_for(&mut cache, &entry) {
        Ok(h) => h.clone(),
        Err(e) => {
            eprintln!("error: {e}");
            return EXIT_FATAL;
        }
    };
    let checked = project::check_entry(&Entry { path: &entry, src }, &hosts);
    let diags = report::collect(&checked);
    if !diags.is_empty() {
        let sources: HashMap<String, String> = checked.files.iter().map(|f| (f.display.clone(), f.src.clone())).collect();
        for d in &diags {
            eprintln!("{}", report::render_human(d, &|f| sources.get(f).cloned(), false));
        }
    }
    if diags.iter().any(|d| d.severity == "error" || d.severity == "internal") {
        eprintln!("oxi {cmd}: not run: the program has errors");
        return EXIT_FINDINGS;
    }
    // TODO(8.L): lower `checked.checked` here (see the module docs for the expected signature).
    eprintln!("oxi {cmd}: not available yet; available after 8.L (lowering) is merged.");
    eprintln!("The program type-checks. Use `oxi check` meanwhile.");
    EXIT_NOT_AVAILABLE
}
