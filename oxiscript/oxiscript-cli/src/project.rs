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
use std::path::{Path, PathBuf};

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

/// Nearest ancestor directory of `file` (inclusive of its directory) holding `oxi.toml`.
pub fn find_root(file: &Path) -> Option<PathBuf> {
    let abs = std::fs::canonicalize(file)
        .or_else(|_| std::env::current_dir().map(|c| c.join(file)))
        .ok()?;
    let mut dir = abs.parent()?.to_path_buf();
    loop {
        if dir.join(MANIFEST).is_file() {
            return Some(dir);
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
/// `path:line:col: message`.
pub fn load_hosts(root: Option<&Path>) -> Result<Vec<Interface>, String> {
    let mut hosts = builtin_hosts();
    let Some(root) = root else { return Ok(hosts) };
    let dir = root.join("idl");
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Ok(hosts);
    };
    let mut files: Vec<PathBuf> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "idl"))
        .collect();
    files.sort();
    let mut project_modules: HashMap<String, PathBuf> = HashMap::new();
    for f in files {
        let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
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
            hosts.retain(|h| h.module != i.module);
            hosts.push(i);
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
/// module names are root-relative paths without extension, so an import can
/// never leave the root.
struct FsLoader {
    root: PathBuf,
    /// First-seen order = the checker's `FileId` order (entry is 0).
    files: Vec<SourceFile>,
    names: Vec<String>,
    cwd: PathBuf,
}

impl FsLoader {
    fn display(&self, p: &Path) -> String {
        let rel = p.strip_prefix(&self.cwd).unwrap_or(p);
        rel.to_string_lossy().replace('\\', "/")
    }
}

impl ModuleLoader for FsLoader {
    fn load(&mut self, importer: &str, spec: &str) -> Option<(String, String)> {
        let name = if spec.starts_with("./") || spec.starts_with("../") {
            let mut parts: Vec<&str> = importer.split('/').collect();
            parts.pop();
            for seg in spec.split('/') {
                match seg {
                    "." | "" => {}
                    ".." => {
                        parts.pop();
                    }
                    s => parts.push(s),
                }
            }
            parts.join("/")
        } else {
            spec.to_string()
        };
        if name.is_empty() || name.contains('\\') || name.contains(':') {
            return None;
        }
        let path = self.root.join(format!("{name}.oxs"));
        let src = std::fs::read_to_string(&path).ok()?;
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
pub fn check_entry(entry: &Entry<'_>, hosts: &[Interface]) -> Run {
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
    };
    let checked = check_program(
        &name,
        &entry.src,
        &mut loader,
        &CheckOptions {
            hosts: hosts.to_vec(),
        },
    );
    Run {
        checked,
        files: loader.files,
    }
}

/// Every `.oxs` file under `path` (a file is returned as is). Skips hidden
/// directories, `target` and `node_modules`.
pub fn collect_oxs(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    let rd = std::fs::read_dir(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut entries: Vec<PathBuf> = rd.filter_map(Result::ok).map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        let fname = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if p.is_dir() {
            if fname.starts_with('.') || fname == "target" || fname == "node_modules" {
                continue;
            }
            collect_oxs(&p, out)?;
        } else if p.extension().is_some_and(|x| x == "oxs") {
            out.push(p);
        }
    }
    Ok(())
}
