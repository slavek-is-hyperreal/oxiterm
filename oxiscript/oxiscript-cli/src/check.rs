//! `oxi check`.

use crate::needs;
use crate::project::{self, Entry, ReadError};
use crate::report::{self, Diag, MACHINE_APPLICABLE};
use oxiscript_check::CheckOptions;
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
    /// `--fix-only semicolons`: apply only the machine-applicable `;` insertions.
    pub fix_only: Option<String>,
    /// `--target`: reported in the JSON document (the checker itself is target-independent).
    pub target: Option<String>,
}

/// Which machine-applicable fixes `--fix` may apply.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FixMode {
    All,
    /// Only the insertion of a missing `;` (`OXS-E0004`), nothing else.
    Semicolons,
}

/// Everything known about one project root (or the root-less case), loaded once per run.
pub struct Project {
    pub opts: CheckOptions,
    pub manifest: Option<project::Manifest>,
}

pub type HostCache = HashMap<Option<PathBuf>, Result<Project, String>>;

fn load_project(root: Option<&Path>) -> Result<Project, String> {
    if let Some(r) = root {
        let mp = r.join(project::MANIFEST);
        let text = project::read_regular(&mp, project::MAX_MANIFEST_BYTES)
            .map_err(|e| match e {
                ReadError::Skipped(w) | ReadError::Failed(w) | ReadError::TooLarge(w) | ReadError::NotUtf8(w) => format!("{}: {w}", mp.display()),
            })?;
        let m = project::parse_manifest(&text).map_err(|e| format!("{}: {e}", mp.display()))?;
        let hosts = project::load_hosts(root)?;
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
        return Ok(Project { opts: CheckOptions { hosts }, manifest: Some(m) });
    }
    Ok(Project { opts: CheckOptions { hosts: project::load_hosts(None)? }, manifest: None })
}

pub fn project_for<'c>(cache: &'c mut HostCache, file: &Path) -> Result<&'c Project, String> {
    let root = project::find_root(file);
    cache
        .entry(root.clone())
        .or_insert_with(|| load_project(root.as_deref()))
        .as_ref()
        .map_err(Clone::clone)
}

fn fatal(json: bool, msg: &str) -> i32 {
    let clean = oxiscript_abi::sanitize::sanitize_terminal(msg);
    if json {
        let doc = report::document(&[], &[], Some(&clean), &project::notes(), None, None);
        println!("{}", serde_json::to_string_pretty(&doc).unwrap_or_default());
    } else {
        eprintln!("error: {clean}");
    }
    EXIT_FATAL
}

/// A file-level problem (unreadable, not UTF-8, too large) reported as a diagnostic of that file.
fn file_diag(path: &Path, e: &ReadError) -> Diag {
    let (code, why) = match e {
        ReadError::TooLarge(w) => ("OXS-E0012", w),
        ReadError::NotUtf8(w) | ReadError::Failed(w) | ReadError::Skipped(w) => ("OXS-E0014", w),
    };
    let p = report::Pos { byte: 0, line: 1, col: 1, utf16: 1 };
    Diag {
        code: code.to_string(),
        severity: report::severity_of(code),
        message: oxiscript_abi::sanitize::sanitize_terminal(&format!("cannot read the file: {why}")),
        file: path.strip_prefix("./").unwrap_or(path).to_string_lossy().replace('\\', "/"),
        range: report::Range { start: p.clone(), end: p },
        fixes: Vec::new(),
    }
}

/// What `--fix` did, for the stderr notes and the JSON `fix` field.
#[derive(Default)]
pub struct FixStats {
    pub applied: usize,
    /// Files with more syntax errors than one run reports (the first ones were fixed, run again).
    pub capped: Vec<String>,
    /// Files where a round was undone because it did not reduce the syntax errors.
    pub reverted: Vec<String>,
    /// Files where the round limit stopped the loop with fixes still pending.
    pub round_limit: Vec<String>,
}

pub fn run(a: &Args) -> i32 {
    let mut cache: HostCache = HashMap::new();
    let mut entries: Vec<(PathBuf, String)> = Vec::new();
    let mut diags: Vec<Diag> = Vec::new();
    let mut names: Vec<String> = Vec::new();
    if a.stdin {
        let mut s = String::new();
        if let Err(e) = std::io::stdin().read_to_string(&mut s) {
            return fatal(a.json, &format!("cannot read stdin: {e}"));
        }
        let p = a.stdin_path.clone().unwrap_or_else(|| PathBuf::from("stdin.oxs"));
        if a.fix_only.is_some() {
            // `oxi check --stdin --fix-only semicolons`: the fixed source on stdout (like `oxi fmt --stdin`)
            let prj = match project_for(&mut cache, &p) {
                Ok(h) => h,
                Err(e) => return fatal(false, &e),
            };
            let mut st = FixStats::default();
            let out = fix_source(&p, &s, &prj.opts, FixMode::Semicolons, &mut st);
            let _ = std::io::Write::write_all(&mut std::io::stdout(), out.as_bytes());
            return EXIT_OK;
        }
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
            match project::read_regular(&f, project::MAX_SOURCE_BYTES) {
                Ok(s) => entries.push((f, s)),
                Err(ReadError::Skipped(why)) => project::note(format!("{}: skipped ({why})", f.display())),
                Err(ReadError::Failed(why)) => return fatal(a.json, &format!("{}: {why}", f.display())),
                Err(e @ (ReadError::TooLarge(_) | ReadError::NotUtf8(_))) => {
                    names.push(f.strip_prefix("./").unwrap_or(&f).to_string_lossy().replace('\\', "/"));
                    diags.push(file_diag(&f, &e));
                }
            }
        }
    }

    let mut stats: Option<FixStats> = None;
    if (a.fix || a.fix_only.is_some()) && !a.stdin {
        let mut st = FixStats::default();
        for (path, src) in &mut entries {
            if std::fs::symlink_metadata(&*path).is_ok_and(|m| m.file_type().is_symlink()) {
                project::note(format!("{}: not fixed (refusing to write through a symlink)", path.display()));
                continue;
            }
            let mode = if a.fix_only.is_some() { FixMode::Semicolons } else { FixMode::All };
            if let Err(e) = fix_file(path, src, &mut cache, mode, &mut st) {
                return fatal(a.json, &e);
            }
        }
        if st.applied > 0 {
            eprintln!("applied {} machine-applicable fix(es)", st.applied);
        }
        stats = Some(st);
    }

    let mut file_targets: Vec<(String, oxiscript_abi::target::Target)> = Vec::new();
    let mut seen: HashSet<(String, u32, u32, String)> = HashSet::new();
    let mut sources: HashMap<String, String> = HashMap::new();
    for (path, src) in &entries {
        let prj = match project_for(&mut cache, path) {
            Ok(h) => h,
            Err(e) => return fatal(a.json, &e),
        };
        let run = project::check_entry(&Entry { path, src: src.clone() }, &prj.opts);
        for f in &run.files {
            sources.entry(f.display.clone()).or_insert_with(|| f.src.clone());
        }
        names.push(run.files[0].display.clone());
        file_targets.push((
            run.files[0].display.clone(),
            a.target
                .as_deref()
                .and_then(oxiscript_abi::target::Target::parse)
                .or_else(|| prj.manifest.as_ref().and_then(|m| m.target))
                .unwrap_or_default(),
        ));
        let mut found = report::collect(&run);
        if let Some(m) = &prj.manifest {
            found.extend(needs::check(&run, m, &prj.opts.hosts));
        }
        for d in found {
            if seen.insert((d.file.clone(), d.range.start.byte, d.range.end.byte, d.code.clone())) {
                diags.push(d);
            }
        }
    }
    diags.sort_by(|x, y| (&x.file, x.range.start.byte).cmp(&(&y.file, y.range.start.byte)));

    let errors = diags.iter().filter(|d| d.severity == "error" || d.severity == "internal").count();
    if a.json {
        let fix = stats.as_ref().map(|s| {
            serde_json::json!({"applied": s.applied, "capped": s.capped, "reverted": s.reverted, "round_limit": s.round_limit})
        });
        // The checker is target-independent (the language is identical on every target); the field
        // reports the target the files would be built for: --target, else each file's project
        // `oxi.toml`. Files of projects with different targets: `"target": "mixed"` plus a
        // `file_targets` object (file -> target); never the first file's target for all.
        let first = file_targets.first().map_or_else(Default::default, |(_, t)| *t);
        let mixed = file_targets.iter().any(|(_, t)| *t != first);
        let (target, per_file) = if mixed {
            let m: serde_json::Map<String, serde_json::Value> =
                file_targets.iter().map(|(f, t)| (f.clone(), serde_json::json!(t.name()))).collect();
            ("mixed", Some(serde_json::Value::Object(m)))
        } else {
            (first.name(), None)
        };
        let mut doc = report::document(&names, &diags, None, &project::notes(), fix, Some(target));
        if let (Some(o), Some(m)) = (doc.as_object_mut(), per_file) {
            o.insert("file_targets".to_string(), m);
        }
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

/// Syntax errors of `own` (not the "too many errors" marker E0017), by line.
fn own_syntax_lines(diags: &[Diag], own: &str) -> Vec<u32> {
    diags
        .iter()
        .filter(|d| d.file == own && d.code != "OXS-E0017")
        .filter(|d| d.code.starts_with("OXS-E") && d.code[5..].parse::<u32>().is_ok_and(|n| n < 100))
        .map(|d| d.range.start.line)
        .collect()
}

/// Did a fix round make progress? Normally the syntax-error count must drop. When the diagnostics
/// before the round were capped (E0017 "too many errors": only the first N errors are listed, so the
/// totals before and after are both capped and equal), progress means fewer errors in the part of
/// the file the capped list covered (up to its last reported line; a `;` insertion adds no lines).
fn progressed(before: &[Diag], after: &[Diag], own: &str) -> bool {
    if before.iter().any(|d| d.code == "OXS-E0017") {
        let b = own_syntax_lines(before, own);
        let lim = b.iter().copied().max().unwrap_or(0);
        let a = own_syntax_lines(after, own).into_iter().filter(|l| *l <= lim).count();
        a < b.len()
    } else {
        syntax_errors(after) < syntax_errors(before)
    }
}

/// Apply machine-applicable fixes of `path` until none remain (syntax errors hide
/// type errors, so one run can uncover a second wave). All rounds run in memory; the result is
/// written once, atomically.
///
/// Convergence guard: after a round the number of syntax diagnostics (`OXS-E0001..E0099`)
/// must have strictly dropped (see [`progressed`]), otherwise the round is dropped and the loop stops.
/// (A fix such as the `;` for `"abc;` lands inside the still unterminated string and would repeat forever.)
fn fix_file(path: &Path, src: &mut String, cache: &mut HostCache, mode: FixMode, stats: &mut FixStats) -> Result<(), String> {
    let opts = &project_for(cache, path)?.opts;
    let cur = fix_source(path, src, opts, mode, stats);
    if cur != *src {
        project::write_atomic(path, &cur).map_err(|e| format!("{}: {e}", path.display()))?;
        *src = cur;
    }
    Ok(())
}

/// The fix rounds on a source in memory; returns the fixed source.
fn fix_source(path: &Path, src: &str, opts: &CheckOptions, mode: FixMode, stats: &mut FixStats) -> String {
    const ROUNDS: usize = 10;
    let shown = path.strip_prefix("./").unwrap_or(path).to_string_lossy().replace('\\', "/");
    let mut cur = src.to_string();
    let mut run = project::check_entry(&Entry { path, src: cur.clone() }, opts);
    let mut own = run.files[0].display.clone();
    let mut diags = report::collect(&run);
    let mut total = 0;
    let mut exhausted = true;
    for _ in 0..ROUNDS {
        if diags.iter().any(|d| d.code == "OXS-E0017") && !stats.capped.contains(&shown) {
            stats.capped.push(shown.clone());
        }
        let mut taken: Vec<(u32, u32)> = Vec::new();
        let mut edits: Vec<TextEdit> = Vec::new();
        let mut n = 0;
        for d in &diags {
            for f in &d.fixes {
                if f.applicability != MACHINE_APPLICABLE || f.edits.is_empty() || f.edits.iter().any(|e| e.file != own) {
                    continue;
                }
                if mode == FixMode::Semicolons
                    && !(d.code == "OXS-E0004"
                        && f.edits.len() == 1
                        && f.edits[0].new_text == ";"
                        && f.edits[0].range.start.byte == f.edits[0].range.end.byte)
                {
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
            exhausted = false;
            break;
        }
        let Some(new) = apply_edits(&cur, &edits) else {
            exhausted = false;
            break;
        };
        if new == cur {
            exhausted = false;
            break;
        }
        let nrun = project::check_entry(&Entry { path, src: new.clone() }, opts);
        let ndiags = report::collect(&nrun);
        if !progressed(&diags, &ndiags, &own) {
            stats.reverted.push(shown.clone());
            project::note(format!("{shown}: a fix round did not reduce the syntax errors and was undone; the remaining errors need a manual fix"));
            exhausted = false;
            break;
        }
        cur = new;
        total += n;
        run = nrun;
        diags = ndiags;
        own.clone_from(&run.files[0].display);
    }
    if exhausted && !diags.iter().any(|d| d.fixes.iter().any(|f| f.applicability == MACHINE_APPLICABLE && !f.edits.is_empty())) {
        exhausted = false;
    }
    if exhausted {
        stats.round_limit.push(shown.clone());
        project::note(format!("{shown}: stopped after {ROUNDS} fix rounds with fixes still pending; run `oxi check --fix` again"));
    }
    if stats.capped.contains(&shown) {
        project::note(format!("{shown}: more syntax errors than one check lists; fixed the first ones per round, run `oxi check --fix` again if errors remain"));
    }
    stats.applied += total;
    cur
}
