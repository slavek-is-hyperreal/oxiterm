//! `oxi check`.

use crate::project::{self, Entry};
use crate::report::{self, Diag, MACHINE_APPLICABLE};
use oxiscript_syntax::{apply_edits, FileId, Span, TextEdit};
use std::collections::{HashMap, HashSet};
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FINDINGS: i32 = 1;
pub const EXIT_FATAL: i32 = 2;

pub struct Args {
    pub paths: Vec<PathBuf>,
    pub json: bool,
    pub stdin: bool,
    pub stdin_path: Option<PathBuf>,
    pub fix: bool,
}

pub type HostCache = HashMap<Option<PathBuf>, Result<Vec<oxiscript_abi::idl::Interface>, String>>;

pub fn hosts_for<'c>(
    cache: &'c mut HostCache,
    file: &Path,
) -> Result<&'c Vec<oxiscript_abi::idl::Interface>, String> {
    let root = project::find_root(file);
    cache
        .entry(root.clone())
        .or_insert_with(|| {
            if let Some(r) = &root {
                let mp = r.join(project::MANIFEST);
                let text = std::fs::read_to_string(&mp).map_err(|e| format!("{}: {e}", mp.display()))?;
                let m = project::parse_manifest(&text).map_err(|e| format!("{}: {e}", mp.display()))?;
                let hosts = project::load_hosts(root.as_deref())?;
                for need in &m.needs {
                    let known = need.split_once('@').is_some_and(|(module, ver)| {
                        hosts.iter().any(|h| h.module == module && ver.parse() == Ok(h.version))
                    });
                    if !known {
                        return Err(format!(
                            "{}: `needs` lists `{need}` but no host interface declares it (built-in: host:log@1, host:state@1; others come from idl/*.idl)",
                            mp.display()
                        ));
                    }
                }
                return Ok(hosts);
            }
            project::load_hosts(root.as_deref())
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn fatal(json: bool, msg: &str) -> i32 {
    if json {
        let doc = report::document(&[], &[], Some(msg));
        println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    } else {
        eprintln!("error: {msg}");
    }
    EXIT_FATAL
}

pub fn run(a: &Args) -> i32 {
    let mut cache: HostCache = HashMap::new();
    let mut entries: Vec<(PathBuf, String)> = Vec::new();
    if a.stdin {
        let mut s = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut s) {
            return fatal(a.json, &format!("cannot read stdin: {e}"));
        }
        let p = a.stdin_path.clone().unwrap_or_else(|| PathBuf::from("stdin.oxs"));
        entries.push((p, s));
    } else {
        let paths = if a.paths.is_empty() { vec![PathBuf::from(".")] } else { a.paths.clone() };
        let mut files = Vec::new();
        for p in &paths {
            if !p.exists() {
                return fatal(a.json, &format!("{}: no such file or directory", p.display()));
            }
            if let Err(e) = project::collect_oxs(p, &mut files) {
                return fatal(a.json, &e);
            }
        }
        for f in files {
            match std::fs::read_to_string(&f) {
                Ok(s) => entries.push((f, s)),
                Err(e) => return fatal(a.json, &format!("{}: {e}", f.display())),
            }
        }
    }

    if a.fix && !a.stdin {
        let mut applied = 0;
        for (path, src) in &mut entries {
            match fix_file(path, src, &mut cache) {
                Ok(n) => applied += n,
                Err(e) => return fatal(a.json, &e),
            }
        }
        if applied > 0 {
            eprintln!("applied {applied} machine-applicable fix(es)");
        }
    }

    let mut diags: Vec<Diag> = Vec::new();
    let mut seen: HashSet<(String, u32, u32, String)> = HashSet::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    let mut names = Vec::new();
    for (path, src) in &entries {
        let hosts = match hosts_for(&mut cache, path) {
            Ok(h) => h.clone(),
            Err(e) => return fatal(a.json, &e),
        };
        let run = project::check_entry(&Entry { path, src: src.clone() }, &hosts);
        for f in &run.files {
            sources.entry(f.display.clone()).or_insert_with(|| f.src.clone());
        }
        names.push(run.files[0].display.clone());
        for d in report::collect(&run) {
            if seen.insert((d.file.clone(), d.range.start.byte, d.range.end.byte, d.code.clone())) {
                diags.push(d);
            }
        }
    }
    diags.sort_by(|x, y| (&x.file, x.range.start.byte).cmp(&(&y.file, y.range.start.byte)));

    let errors = diags.iter().filter(|d| d.severity == "error" || d.severity == "internal").count();
    if a.json {
        let doc = report::document(&names, &diags, None);
        println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    } else {
        let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        for d in &diags {
            println!("{}", report::render_human(d, &|f| sources.get(f).cloned(), color));
        }
        let warnings = diags.len() - errors;
        if errors > 0 {
            println!("error: could not check: {errors} error(s), {warnings} warning(s) in {} file(s)", names.len());
        } else {
            println!("ok: {} file(s) checked, {warnings} warning(s)", names.len());
        }
    }
    if errors > 0 {
        EXIT_FINDINGS
    } else {
        EXIT_OK
    }
}

fn syntax_errors(diags: &[Diag]) -> usize {
    diags
        .iter()
        .filter(|d| d.code.starts_with("OXS-E") && d.code[5..].parse::<u32>().is_ok_and(|n| n < 100))
        .count()
}

/// Apply machine-applicable fixes of `path` until none remain (syntax errors hide
/// type errors, so one run can uncover a second wave). Returns how many fixes were applied.
///
/// Convergence guard: after a round the number of syntax diagnostics (`OXS-E0001..E0099`)
/// must have strictly dropped, otherwise the round is undone and the loop stops. (A fix such
/// as the `;` for `"abc;` lands inside the still unterminated string and would repeat forever.)
fn fix_file(path: &Path, src: &mut String, cache: &mut HostCache) -> Result<usize, String> {
    let mut total = 0;
    // (source before the last round, syntax diagnostics before it, fixes it applied)
    let mut last: Option<(String, usize, usize)> = None;
    for _ in 0..10 {
        let hosts = hosts_for(cache, path)?.clone();
        let run = project::check_entry(&Entry { path, src: src.clone() }, &hosts);
        let own = run.files[0].display.clone();
        let diags = report::collect(&run);
        let syntax = syntax_errors(&diags);
        if let Some((prev, prev_syntax, applied)) = last.take() {
            if syntax >= prev_syntax {
                std::fs::write(path, &prev).map_err(|e| format!("{}: {e}", path.display()))?;
                *src = prev;
                total -= applied;
                break;
            }
        }
        let mut taken: Vec<(u32, u32)> = Vec::new();
        let mut edits: Vec<TextEdit> = Vec::new();
        let mut n = 0;
        for d in &diags {
            for f in &d.fixes {
                if f.applicability != MACHINE_APPLICABLE || f.edits.is_empty() || f.edits.iter().any(|e| e.file != own) {
                    continue;
                }
                let spans: Vec<(u32, u32)> = f.edits.iter().map(|e| (e.range.start.byte, e.range.end.byte)).collect();
                let clash = spans.iter().any(|(lo, hi)| taken.iter().any(|(tl, th)| lo <= th && tl <= hi))
                    || spans.iter().enumerate().any(|(i, (lo, hi))| spans.iter().skip(i + 1).any(|(l2, h2)| lo <= h2 && l2 <= hi));
                if clash {
                    continue;
                }
                taken.extend(spans);
                for e in &f.edits {
                    edits.push(TextEdit {
                        span: Span::new(FileId(0), e.range.start.byte, e.range.end.byte),
                        replacement: e.new_text.clone(),
                    });
                }
                n += 1;
            }
        }
        if n == 0 {
            break;
        }
        let Some(new) = apply_edits(src, &edits) else { break };
        if new == *src {
            break;
        }
        std::fs::write(path, &new).map_err(|e| format!("{}: {e}", path.display()))?;
        last = Some((src.clone(), syntax, n));
        *src = new;
        total += n;
    }
    // the last round was never judged (loop ended by the round limit): judge it now
    if let Some((prev, prev_syntax, applied)) = last.take() {
        let hosts = hosts_for(cache, path)?.clone();
        let run = project::check_entry(&Entry { path, src: src.clone() }, &hosts);
        if syntax_errors(&report::collect(&run)) >= prev_syntax {
            std::fs::write(path, &prev).map_err(|e| format!("{}: {e}", path.display()))?;
            *src = prev;
            total -= applied;
        }
    }
    Ok(total)
}
