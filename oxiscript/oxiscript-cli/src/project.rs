//! Project discovery shared by `check`, `run` and `build`: the project root
//! (nearest ancestor holding `oxi.toml`), host interfaces (built-in IDLs plus
//! the project's `idl/*.idl`), a filesystem module loader and the source map
//! that turns `FileId`s back into paths.
//!
//! Host IDL rule: the built-in interfaces are `host:log` and `host:state`
//! (`oxiscript-abi/idl`). A project IDL that declares an interface with the same
//! `host:` module as a built-in one replaces it (so a project can pin or
//! extend it); two project IDL files declaring the same module is an error.

use oxiscript_abi::idl::{self, Interface};
use oxiscript_check::{check_program, CheckOptions, Checked, ModuleLoader};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;

pub const MANIFEST: &str = "oxi.toml";

/// Parsed `oxi.toml` (only what the tools need today).
#[derive(Debug, Clone, Default)]
#[allow(dead_code)] // `name` and `version` are read by `build`/`run` once 8.L lands
pub struct Manifest {
    pub name: String,
    pub version: String,
    /// `host:` interfaces `main` receives, in parameter order (`host:log@1`).
    pub needs: Vec<String>,
}

pub fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let t: toml::Table = text.parse().map_err(|e| format!("{e}"))?;
    let pkg = t
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or("missing [package] table")?;
    let get = |k: &str| -> Result<String, String> {
        pkg.get(k)
            .and_then(toml::Value::as_str)
            .map(str::to_string)
            .ok_or(format!("[package] needs a string `{k}`"))
    };
    let mut needs = Vec::new();
    if let Some(v) = pkg.get("needs") {
        let arr = v.as_array().ok_or("[package] `needs` must be an array of strings")?;
        for x in arr {
            needs.push(
                x.as_str()
                    .ok_or("`needs` must be an array of strings")?
                    .to_string(),
            );
        }
    }
    Ok(Manifest {
        name: get("name")?,
        version: get("version")?,
        needs,
    })
}

/// Source files larger than this are refused (a clean diagnostic, never an allocation failure).
pub const MAX_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
/// Host interface files (`idl/*.idl`) larger than this are refused.
pub const MAX_IDL_BYTES: u64 = 1024 * 1024;
/// `oxi.toml` larger than this is refused.
pub const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

static NOTES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Tell the user (stderr, once) and remember for the JSON `notes` field that something was skipped
/// or replaced. Notes never change the exit code.
pub fn note(msg: String) {
    let mut n = NOTES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if !n.contains(&msg) {
        eprintln!("note: {msg}");
        n.push(msg);
    }
}

/// Notes collected so far (for the JSON report).
pub fn notes() -> Vec<String> {
    NOTES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// Why a file was not read.
pub enum ReadError {
    /// Not a regular file (FIFO, device, socket, directory): skipped with a note, no error.
    Skipped(String),
    /// An I/O error (permission denied, ...).
    Failed(String),
    /// A regular file above the size limit.
    TooLarge(String),
    /// A regular file that is not UTF-8.
    NotUtf8(String),
}

/// Read a regular file (symlinks are resolved, the target must be a regular file) of at most `cap`
/// bytes as UTF-8. Never opens a FIFO or device (the check comes before `open`) and never reads
/// more than `cap + 1` bytes.
pub fn read_regular(path: &Path, cap: u64) -> Result<String, ReadError> {
    let md = std::fs::metadata(path).map_err(|e| ReadError::Failed(e.to_string()))?;
    if !md.is_file() {
        return Err(ReadError::Skipped("not a regular file".to_string()));
    }
    let f = std::fs::File::open(path).map_err(|e| ReadError::Failed(e.to_string()))?;
    // the handle itself must also be a regular file (the path may have been swapped after the check)
    match f.metadata() {
        Ok(m) if m.is_file() => {
            if m.len() > cap {
                return Err(ReadError::TooLarge(format!("file is {} bytes, larger than the {cap}-byte limit", m.len())));
            }
        }
        Ok(_) => return Err(ReadError::Skipped("not a regular file".to_string())),
        Err(e) => return Err(ReadError::Failed(e.to_string())),
    }
    let mut buf = Vec::new();
    f.take(cap + 1).read_to_end(&mut buf).map_err(|e| ReadError::Failed(e.to_string()))?;
    if buf.len() as u64 > cap {
        return Err(ReadError::TooLarge(format!("file is larger than the {cap}-byte limit")));
    }
    String::from_utf8(buf).map_err(|e| ReadError::NotUtf8(format!("not valid UTF-8 (first bad byte at offset {})", e.utf8_error().valid_up_to())))
}

/// Replace the contents of the regular file `path` atomically: write a temp file in the same
/// directory, copy the permissions, rename over the original. Refuses symlinks and special files
/// (nothing is ever written through a link).
pub fn write_atomic(path: &Path, data: &str) -> Result<(), String> {
    static N: AtomicU32 = AtomicU32::new(0);
    let md = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if md.file_type().is_symlink() {
        return Err("refusing to write through a symlink".to_string());
    }
    if !md.is_file() {
        return Err("refusing to write: not a regular file".to_string());
    }
    let dir = match path.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let fname = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dir.join(format!(".{fname}.oxi-tmp-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let res = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data.as_bytes())?;
        f.flush()?;
        std::fs::set_permissions(&tmp, md.permissions())?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.to_string());
    }
    Ok(())
}

/// Project root of `file`: the nearest ancestor directory (the file's own directory included)
/// holding `oxi.toml`. The search never crosses a repository boundary (a directory containing
/// `.git`, a file or a directory) and never looks at `$HOME` or above, so a manifest or `idl/`
/// planted in `/tmp`, `$HOME` or a parent checkout cannot govern a project below it.
pub fn find_root(file: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(file)
        .or_else(|_| std::env::current_dir().map(|c| c.join(file)))
        .ok()?;
    let home = std::env::var_os("HOME").and_then(|h| std::fs::canonicalize(h).ok());
    let mut dir = abs.parent()?.to_path_buf();
    loop {
        if home.as_deref() == Some(dir.as_path()) {
            return None;
        }
        if dir.join(MANIFEST).is_file() {
            return Some(dir);
        }
        if dir.join(".git").exists() {
            return None;
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// Built-in host interfaces.
pub fn builtin_hosts() -> Vec<Interface> {
    let mut v = Vec::new();
    for src in [idl::EXAMPLE_LOG_IDL, idl::EXAMPLE_STATE_IDL] {
        v.extend(idl::parse(src).expect("built-in IDL is valid"));
    }
    v
}

/// Built-in hosts plus `<root>/idl/*.idl`. An IDL error is returned as
/// `path:line:col: message`. Linear in the number of interfaces. Only regular `.idl` files are
/// read (symlinks and special files are skipped with a note).
pub fn load_hosts(root: Option<&Path>) -> Result<Vec<Interface>, String> {
    let mut hosts = builtin_hosts();
    let Some(root) = root else { return Ok(hosts) };
    let dir = root.join("idl");
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Ok(hosts);
    }
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(hosts);
    };
    let mut files: Vec<PathBuf> = Vec::new();
    for e in rd.filter_map(Result::ok) {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "idl") {
            continue;
        }
        match e.file_type() {
            Ok(t) if t.is_file() => files.push(p),
            _ => note(format!("{}: skipped (IDL files must be regular files, not symlinks or special files)", p.display())),
        }
    }
    files.sort();
    let mut index: HashMap<String, usize> = hosts.iter().enumerate().map(|(i, h)| (h.module.clone(), i)).collect();
    let builtin_count = hosts.len();
    let mut project_modules: HashMap<String, PathBuf> = HashMap::new();
    for f in files {
        let text = match read_regular(&f, MAX_IDL_BYTES) {
            Ok(t) => t,
            Err(ReadError::Skipped(why)) => {
                note(format!("{}: skipped ({why})", f.display()));
                continue;
            }
            Err(ReadError::Failed(why) | ReadError::TooLarge(why) | ReadError::NotUtf8(why)) => return Err(format!("{}: {why}", f.display())),
        };
        let ifaces = idl::parse(&text)
            .map_err(|e| format!("{}:{}:{}: {}", f.display(), e.line, e.col, e.msg))?;
        for i in ifaces {
            if let Some(prev) = project_modules.insert(i.module.clone(), f.clone()) {
                return Err(format!(
                    "{}: interface module `{}` is also declared in {}",
                    f.display(),
                    i.module,
                    prev.display()
                ));
            }
            if let Some(&at) = index.get(&i.module) {
                if at < builtin_count {
                    note(format!(
                        "{} replaces the built-in `{}` interface (project IDL wins; `needs` entries for it are now satisfied by this declaration)",
                        f.display(),
                        i.module
                    ));
                }
                hosts[at] = i;
            } else {
                index.insert(i.module.clone(), hosts.len());
                hosts.push(i);
            }
        }
    }
    Ok(hosts)
}

/// One loaded source file.
pub struct SourceFile {
    /// Path shown to the user (relative to the current directory when possible, `/` separators).
    pub display: String,
    pub src: String,
    starts: oxiscript_syntax::LineIndex,
}

impl SourceFile {
    pub fn new(display: String, src: String) -> SourceFile {
        let starts = oxiscript_syntax::LineIndex::new(&src);
        SourceFile {
            display,
            src,
            starts,
        }
    }
    /// 1-based line, 1-based column in characters, 1-based column in UTF-16 units.
    pub fn pos(&self, byte: u32) -> (u32, u32, u32) {
        let (line, col) = self.starts.line_col(&self.src, byte);
        let b = (byte as usize).min(self.src.len());
        let line_start = self.src[..b].rfind(['\n', '\r']).map_or(0, |i| i + 1);
        let utf16: usize = self
            .src
            .get(line_start..b)
            .map_or(0, |s| s.encode_utf16().count());
        (line, col, u32::try_from(utf16 + 1).unwrap_or(u32::MAX))
    }
    /// Text of the 1-based line (without terminator).
    pub fn line_text(&self, line: u32) -> &str {
        self.src
            .lines()
            .nth(line.saturating_sub(1) as usize)
            .unwrap_or("")
    }
}

/// Resolves relative and package imports against the project root. Canonical
/// module names are root-relative paths without extension. An import can never
/// leave the root: absolute specifiers and any `..` that would pop past the root are
/// refused, and the resolved file (after symlinks) must still lie inside the root.
struct FsLoader {
    /// Canonical project root.
    root: PathBuf,
    /// First-seen order = the checker's `FileId` order (entry is 0).
    files: Vec<SourceFile>,
    names: Vec<String>,
    cwd: PathBuf,
    /// Imports that were refused: (specifier, why).
    rejected: Vec<(String, String)>,
}

impl FsLoader {
    fn display(&self, p: &Path) -> String {
        let rel = p.strip_prefix(&self.cwd).unwrap_or(p);
        rel.to_string_lossy().replace('\\', "/")
    }

    /// Root-relative module name of `spec` imported from `importer`.
    fn resolve(importer: &str, spec: &str) -> Result<String, String> {
        if spec.is_empty() || spec.starts_with('/') || spec.contains('\\') || spec.contains(':') {
            return Err("absolute or malformed import path; imports are relative to the project root".to_string());
        }
        let mut parts: Vec<&str> = if spec.starts_with("./") || spec.starts_with("../") {
            let mut p: Vec<&str> = importer.split('/').collect();
            p.pop();
            p
        } else {
            Vec::new()
        };
        for seg in spec.split('/') {
            match seg {
                "." | "" => {}
                ".." => {
                    if parts.pop().is_none() {
                        return Err("the import path leaves the project root".to_string());
                    }
                }
                s => parts.push(s),
            }
        }
        if parts.is_empty() {
            return Err("empty import path".to_string());
        }
        Ok(parts.join("/"))
    }
}

impl ModuleLoader for FsLoader {
    fn load(&mut self, importer: &str, spec: &str) -> Option<(String, String)> {
        let name = match Self::resolve(importer, spec) {
            Ok(n) => n,
            Err(why) => {
                self.rejected.push((spec.to_string(), why));
                return None;
            }
        };
        let path = self.root.join(format!("{name}.oxs"));
        let canon = std::fs::canonicalize(&path).ok()?;
        if !canon.starts_with(&self.root) {
            self.rejected.push((spec.to_string(), "the file resolves (through a symlink) to a place outside the project root".to_string()));
            return None;
        }
        let src = match read_regular(&canon, MAX_SOURCE_BYTES) {
            Ok(s) => s,
            Err(ReadError::Skipped(why) | ReadError::Failed(why) | ReadError::TooLarge(why) | ReadError::NotUtf8(why)) => {
                self.rejected.push((spec.to_string(), format!("cannot read the module: {why}")));
                return None;
            }
        };
        if !self.names.contains(&name) {
            let display = self.display(&path);
            self.names.push(name.clone());
            self.files.push(SourceFile::new(display, src.clone()));
        }
        Some((name, src))
    }
}

/// The result of checking one entry file.
pub struct Run {
    /// Root-relative canonical name of the entry module (`main` for `<root>/main.oxs`).
    pub entry_name: String,
    pub checked: Checked,
    /// Indexed by `FileId`; the implicit prelude (last module) has no entry.
    pub files: Vec<SourceFile>,
}

/// Where the entry source comes from.
pub struct Entry<'a> {
    /// Path of the entry file (real or `--stdin-path`).
    pub path: &'a Path,
    pub src: String,
}

/// Check one entry file with its imports. `hosts` come from [`load_hosts`].
pub fn check_entry(entry: &Entry<'_>, opts: &CheckOptions) -> Run {
    let cwd = std::env::current_dir().unwrap_or_default();
    let abs = if entry.path.is_absolute() {
        entry.path.to_path_buf()
    } else {
        cwd.join(entry.path)
    };
    let base = find_root(entry.path)
        .unwrap_or_else(|| abs.parent().map(Path::to_path_buf).unwrap_or_default());
    let abs = std::fs::canonicalize(&abs).unwrap_or(abs);
    let base = std::fs::canonicalize(&base).unwrap_or(base);
    let name = abs
        .strip_prefix(&base)
        .unwrap_or(&abs)
        .with_extension("")
        .to_string_lossy()
        .replace('\\', "/");
    let display = entry
        .path
        .strip_prefix("./")
        .unwrap_or(entry.path)
        .to_string_lossy()
        .replace('\\', "/");
    let mut loader = FsLoader {
        root: base,
        files: vec![SourceFile::new(display, entry.src.clone())],
        names: vec![name.clone()],
        cwd,
        rejected: Vec::new(),
    };
    let mut checked = check_program(&name, &entry.src, &mut loader, opts);
    // say why an import was refused instead of a bare "module not found"
    for (spec, why) in &loader.rejected {
        let from = format!("`{spec}`: module not found");
        for d in &mut checked.diagnostics {
            if d.code == "OXS-E0302" && d.message.ends_with(&from) {
                d.message = format!("cannot resolve import `{spec}`: {why}");
            }
        }
    }
    Run {
        entry_name: name,
        checked,
        files: loader.files,
    }
}

/// Every `.oxs` file under `path` (a file is returned as is). Skips hidden
/// directories, `target` and `node_modules`. Symlinks are never followed (no escapes out of the
/// tree, no cycles, nothing is ever written through a link) and FIFOs, devices and sockets are
/// skipped; each skipped `.oxs` name gets a note.
pub fn collect_oxs(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let md = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !md.is_dir() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    walk(path, out, true)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, top: bool) -> Result<(), String> {
    let rd = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if top => return Err(format!("{}: {e}", dir.display())),
        Err(e) => {
            note(format!("{}: cannot read directory: {e}", dir.display()));
            return Ok(());
        }
    };
    let mut entries: Vec<(PathBuf, std::fs::FileType)> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_type().ok().map(|t| (e.path(), t)))
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (p, t) in entries {
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let is_oxs = p.extension().is_some_and(|x| x == "oxs");
        if t.is_symlink() {
            if is_oxs {
                note(format!("{}: skipped (oxi does not follow symlinks)", p.display()));
            }
        } else if t.is_dir() {
            if fname.starts_with('.') || fname == "target" || fname == "node_modules" {
                continue;
            }
            walk(&p, out, false)?;
        } else if is_oxs {
            if t.is_file() {
                out.push(p);
            } else {
                note(format!("{}: skipped (not a regular file)", p.display()));
            }
        }
    }
    Ok(())
}
