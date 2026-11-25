//! Page source abstraction: where the event loop gets THTML pages from.
//!
//! The loop asks a [`PageSource`] to *resolve* a navigation target (containment, `_mobile`
//! variant selection) and to *load* the resolved page. `FsPageSource` (in `fs_page_source.rs`) is
//! the server implementation (filesystem, canonicalize-based containment); [`MemPageSource`]
//! serves pages from a map (tests, and later wasm / local mode).
//!
//! This module is written to move to `oxiterm-core`: the trait signature uses only
//! `std::path` types and nothing here touches the disk. It still parses through
//! `crate::loader::load_thtml_str_mode` (pure).
//!
//! Errors are deliberately generic: a missing page, a traversal attempt, an absolute path and a
//! symlink escape are all the same [`PageError::Unavailable`], so a client cannot probe the
//! filesystem layout through navigation.

use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::document::THTMLDocument;

/// A resolved page.
///
/// - `page`: the logical page id announced to clients as `current_page` (the non-mobile target
///   relative to the source root, *not* lexically normalised, e.g. `demos/../features.thtml`).
/// - `base`: the non-mobile target path; relative navigation is resolved against its parent.
/// - `actual`: the variant actually loaded (`foo_mobile.thtml` or `foo.thtml`).
///
/// `base`/`actual` are plain `std::path` values; for `FsPageSource` they are filesystem paths
/// (the loop also hands them to the renderer as the media base until media loading moves behind
/// a trait), for `MemPageSource` they are logical paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    pub page: String,
    pub base: PathBuf,
    pub actual: PathBuf,
}

/// Page resolution/loading failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageError {
    /// The target is missing, outside the source root, or otherwise not servable. Intentionally
    /// does not say which.
    Unavailable,
    /// The page was resolved but could not be read or parsed.
    Load(String),
}

impl fmt::Display for PageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PageError::Unavailable => write!(f, "blocked"),
            PageError::Load(m) => write!(f, "{}", m),
        }
    }
}

impl std::error::Error for PageError {}

/// Source of THTML pages for an event loop.
pub trait PageSource: Send + Sync {
    /// Resolves `target` to a page. With `base = Some(current)` the target is relative to the
    /// current page's directory; with `None` it is relative to the source root. Any
    /// `_mobile.thtml` in `target` is replaced by `.thtml`, then (when `mobile`) the
    /// `_mobile` variant is selected if it exists. Enforces containment in the source root.
    /// The caller appends the `.thtml` extension where that is wanted (relative navigation).
    fn resolve(&self, base: Option<&PageRef>, target: &str, mobile: bool) -> Result<PageRef, PageError>;

    /// Loads (reads and parses) the `actual` variant of `page`. Does not re-check containment:
    /// `page` must come from `resolve` or `page_at`.
    fn load(&self, page: &PageRef, mode: RenderMode) -> Result<THTMLDocument, PageError>;

    /// Adopts an externally known location (the loop's initial `source_path` or a test-provided
    /// one) as a `PageRef` with `base == actual == location`. No containment check and no
    /// variant selection; pure path arithmetic.
    fn page_at(&self, location: &Path) -> PageRef;
}

/// Appends the `.thtml` extension if missing (relative navigation targets).
pub fn with_thtml_ext(target: &str) -> String {
    let mut t = target.to_string();
    if !t.ends_with(".thtml") {
        t.push_str(".thtml");
    }
    t
}

pub fn strip_mobile(target: &str) -> String {
    if target.contains("_mobile.thtml") {
        target.replace("_mobile.thtml", ".thtml")
    } else {
        target.to_string()
    }
}

/// A navigation target both page sources accept for resolution at all: non-empty, no NUL, no
/// backslash and no trailing `/` (`sub/` names a directory, `a.thtml/` is not a file). One rule
/// for `FsPageSource` and `MemPageSource`, so they cannot drift.
pub fn target_is_well_formed(target: &str) -> bool {
    !target.is_empty() && !target.contains('\0') && !target.contains('\\') && !target.ends_with('/')
}

/// The non-mobile page path of `target` relative to `dir`, as it is announced as the page id:
/// `.` and empty segments are dropped (`./a`, `.//a`, `sub//b` -> `a`, `sub/b`), `..` is kept
/// (the id of `demos/../features.thtml` is not collapsed). Both page sources build the id with
/// this function.
pub fn page_path(dir: &Path, target: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for c in dir.join(target).components() {
        match c {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// In-memory page source: `path -> THTML text`. Paths are lexically normalised (`.`/`..`
/// collapsed); a `..` that would climb above the root, or an absolute path, is refused with the
/// same generic error as a missing page.
#[derive(Default, Clone)]
pub struct MemPageSource {
    pages: Arc<HashMap<String, String>>,
    /// Directories that exist (the parents of every page, plus any given explicitly): a `..`
    /// may only follow a segment that is one of them, exactly as on a filesystem
    /// (`nodir/../a.thtml` and `a.thtml/../b.thtml` are refused).
    dirs: Arc<std::collections::HashSet<String>>,
}

impl MemPageSource {
    pub fn new<I, K, V>(pages: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        Self::with_dirs(pages, std::iter::empty::<String>())
    }

    /// Like [`MemPageSource::new`], with extra existing directories (empty ones have no page to
    /// derive them from).
    pub fn with_dirs<I, K, V, D, S>(pages: I, extra_dirs: D) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
        D: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut map = HashMap::new();
        let mut dirs = std::collections::HashSet::new();
        for (k, v) in pages {
            // Page keys are plain relative paths; anything with `..` or an absolute start is dropped.
            if let Some(n) = Self::normalise_in(Path::new(k.as_ref()), None) {
                let mut prefix = String::new();
                for seg in n.split('/').take(n.matches('/').count()) {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(seg);
                    dirs.insert(prefix.clone());
                }
                map.insert(n, v.into());
            }
        }
        for d in extra_dirs {
            if let Some(n) = Self::normalise_in(Path::new(d.as_ref()), None) {
                let mut prefix = String::new();
                for seg in n.split('/').filter(|s| !s.is_empty()) {
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    prefix.push_str(seg);
                    dirs.insert(prefix.clone());
                }
            }
        }
        Self { pages: Arc::new(map), dirs: Arc::new(dirs) }
    }

    /// Lexical normalisation. `None` for absolute paths and for `..` escaping the root. With
    /// `dirs`, a `..` must also follow a directory that exists (filesystem semantics).
    fn normalise_in(p: &Path, dirs: Option<&std::collections::HashSet<String>>) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir => {
                    if let Some(dirs) = dirs {
                        if !dirs.contains(&parts.join("/")) {
                            return None;
                        }
                    }
                    parts.pop()?;
                }
                Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(parts.join("/"))
    }

    fn normalise(&self, p: &Path) -> Option<String> {
        Self::normalise_in(p, Some(&self.dirs))
    }

    fn exists(&self, p: &Path) -> bool {
        self.normalise(p).map_or(false, |n| self.pages.contains_key(&n))
    }
}

impl PageSource for MemPageSource {
    fn resolve(&self, base: Option<&PageRef>, target: &str, mobile: bool) -> Result<PageRef, PageError> {
        if !target_is_well_formed(target) {
            return Err(PageError::Unavailable);
        }
        let dir = base.and_then(|b| b.base.parent()).map(|d| d.to_path_buf()).unwrap_or_default();
        let non_mobile = page_path(&dir, &strip_mobile(target));
        let mut actual = non_mobile.clone();
        if mobile {
            if let (Some(stem), Some(ext)) = (non_mobile.file_stem().and_then(|s| s.to_str()), non_mobile.extension().and_then(|e| e.to_str())) {
                let variant = non_mobile.with_file_name(format!("{}_mobile.{}", stem, ext));
                if self.exists(&variant) {
                    actual = variant;
                }
            }
        }
        if !self.exists(&actual) {
            return Err(PageError::Unavailable);
        }
        Ok(PageRef { page: non_mobile.to_string_lossy().into_owned(), base: non_mobile, actual })
    }

    fn load(&self, page: &PageRef, mode: RenderMode) -> Result<THTMLDocument, PageError> {
        let key = self.normalise(&page.actual).ok_or(PageError::Unavailable)?;
        let text = self.pages.get(&key).ok_or(PageError::Unavailable)?;
        crate::loader::load_thtml_str_mode(text, mode, &key).map_err(|e| PageError::Load(e.to_string()))
    }

    fn page_at(&self, location: &Path) -> PageRef {
        PageRef {
            page: location.to_string_lossy().into_owned(),
            base: location.to_path_buf(),
            actual: location.to_path_buf(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<screen><text>hi</text></screen>";

    fn mem() -> MemPageSource {
        MemPageSource::new([
            ("index.thtml", PAGE),
            ("demos/a.thtml", PAGE),
            ("features.thtml", PAGE),
            ("features_mobile.thtml", "<screen><text>mobile</text></screen>"),
        ])
    }

    #[test]
    fn mem_resolves_loads_and_keeps_unnormalised_page_id() {
        let m = mem();
        let a = m.resolve(None, "demos/a.thtml", false).unwrap();
        assert_eq!(a.page, "demos/a.thtml");
        assert!(m.load(&a, RenderMode::Tui).is_ok());
        let f = m.resolve(Some(&a), "../features.thtml", false).unwrap();
        assert_eq!(f.page, "demos/../features.thtml");
        assert!(m.load(&f, RenderMode::Tui).is_ok());
        let fm = m.resolve(None, "features.thtml", true).unwrap();
        assert_eq!(fm.actual, PathBuf::from("features_mobile.thtml"));
        assert_eq!(fm.page, "features.thtml");
    }

    #[test]
    fn mem_normalises_targets_like_a_filesystem() {
        let m = MemPageSource::with_dirs(
            [("index.thtml", PAGE), ("a.thtml", PAGE), ("a_mobile.thtml", PAGE), ("sub/b.thtml", PAGE)],
            ["emptydir"],
        );
        let id = |t: &str| m.resolve(None, t, false).map(|p| p.page);
        // `.` and empty segments never reach the page id; `..` stays
        assert_eq!(id("./a.thtml").unwrap(), "a.thtml");
        assert_eq!(id(".//a.thtml").unwrap(), "a.thtml");
        assert_eq!(id("sub//b.thtml").unwrap(), "sub/b.thtml");
        assert_eq!(id("sub/./b.thtml").unwrap(), "sub/b.thtml");
        assert_eq!(id("sub/../a.thtml").unwrap(), "sub/../a.thtml");
        assert_eq!(id("emptydir/../a.thtml").unwrap(), "emptydir/../a.thtml");
        // a `..` needs an existing directory before it; a file or a missing name is no directory
        for bad in ["nodir/../a.thtml", "a.thtml/../index.thtml", "sub/b.thtml/../a.thtml", "../a.thtml", "sub/../../a.thtml"] {
            assert_eq!(id(bad), Err(PageError::Unavailable), "{bad}");
        }
        // directories, trailing slashes, NUL and backslashes are not pages (either variant)
        for bad in ["sub", "sub/", "a.thtml/", "", "a.thtml\0", "sub\\b.thtml", "/a.thtml", "emptydir"] {
            for mobile in [false, true] {
                assert_eq!(m.resolve(None, bad, mobile), Err(PageError::Unavailable), "{bad:?} mobile={mobile}");
            }
        }
        assert!(target_is_well_formed("a.thtml") && !target_is_well_formed("a/") && !target_is_well_formed("a\\b"));
        assert_eq!(page_path(Path::new("demos"), "../x.thtml"), PathBuf::from("demos/../x.thtml"));
        assert_eq!(page_path(Path::new(""), "./x.thtml"), PathBuf::from("x.thtml"));
    }

    #[test]
    fn mem_refuses_escape_absolute_and_missing_identically() {
        let m = mem();
        let a = m.resolve(None, "demos/a.thtml", false).unwrap();
        let errs = [
            m.resolve(None, "../index.thtml", false),
            m.resolve(Some(&a), "../../index.thtml", false),
            m.resolve(None, "/index.thtml", false),
            m.resolve(None, "nope.thtml", false),
        ];
        for e in errs {
            assert_eq!(e, Err(PageError::Unavailable));
        }
    }
}
