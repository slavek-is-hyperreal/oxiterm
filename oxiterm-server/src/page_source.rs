//! Page source abstraction: where the event loop gets THTML pages from.
//!
//! The loop asks a [`PageSource`] to *resolve* a navigation target (containment, `_mobile`
//! variant selection) and to *load* the resolved page. [`FsPageSource`] is the server
//! implementation (filesystem, canonicalize-based containment); [`MemPageSource`] serves pages
//! from a map (tests, and later wasm / local mode).
//!
//! This module is written to move to `oxiterm-core`: the trait signature uses only
//! `std::path` types (no `std::fs`); `FsPageSource` is the only part that touches the disk.
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
use tracing::warn;

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

fn strip_mobile(target: &str) -> String {
    if target.contains("_mobile.thtml") {
        target.replace("_mobile.thtml", ".thtml")
    } else {
        target.to_string()
    }
}

/// How `FsPageSource` finds its root.
enum Root {
    Fixed(PathBuf),
    /// Read on every call (the session's `app_base_dir`, which may be set after the loop is
    /// built); `None` falls back to the process working directory.
    Dynamic(Box<dyn Fn() -> Option<PathBuf> + Send + Sync>),
}

/// Filesystem page source: pages live under `root`; containment uses canonicalized paths, so
/// `..` traversal, absolute paths and symlinks pointing outside the root are refused.
pub struct FsPageSource {
    root: Root,
}

impl FsPageSource {
    pub fn new(root: PathBuf) -> Self {
        Self { root: Root::Fixed(root) }
    }

    /// Root read lazily from `f` on each call (cwd when it returns `None`).
    pub fn with_dynamic_root(f: impl Fn() -> Option<PathBuf> + Send + Sync + 'static) -> Self {
        Self { root: Root::Dynamic(Box::new(f)) }
    }

    fn root(&self) -> PathBuf {
        match &self.root {
            Root::Fixed(p) => p.clone(),
            Root::Dynamic(f) => f().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))),
        }
    }

    fn page_id(root: &Path, base: &Path) -> String {
        if let Ok(rel) = base.strip_prefix(root) {
            rel.to_string_lossy().into_owned()
        } else {
            base.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
        }
    }
}

impl PageSource for FsPageSource {
    fn resolve(&self, base: Option<&PageRef>, target: &str, mobile: bool) -> Result<PageRef, PageError> {
        let root = self.root();
        let dir = match base {
            Some(b) => b.base.parent().map(|d| d.to_path_buf()).unwrap_or_else(|| root.clone()),
            None => root.clone(),
        };
        let non_mobile = dir.join(strip_mobile(target));
        let actual = crate::pathsafe::resolve_variant(&non_mobile, mobile);

        // Containment gate: reject path traversals and non-existent targets with one generic
        // error; do not distinguish traversal from a missing file.
        if !crate::pathsafe::is_within_base(&root, &actual) {
            warn!("resolve_and_load_page: blocked path traversal or missing file for {:?}", actual);
            return Err(PageError::Unavailable);
        }
        Ok(PageRef { page: Self::page_id(&root, &non_mobile), base: non_mobile, actual })
    }

    fn load(&self, page: &PageRef, mode: RenderMode) -> Result<THTMLDocument, PageError> {
        crate::loader::load_thtml_file_mode(&page.actual, mode).map_err(|e| PageError::Load(e.to_string()))
    }

    fn page_at(&self, location: &Path) -> PageRef {
        PageRef {
            page: Self::page_id(&self.root(), location),
            base: location.to_path_buf(),
            actual: location.to_path_buf(),
        }
    }
}

/// In-memory page source: `path -> THTML text`. Paths are lexically normalised (`.`/`..`
/// collapsed); a `..` that would climb above the root, or an absolute path, is refused with the
/// same generic error as a missing page.
#[derive(Default, Clone)]
pub struct MemPageSource {
    pages: Arc<HashMap<String, String>>,
}

impl MemPageSource {
    pub fn new<I, K, V>(pages: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<str>,
        V: Into<String>,
    {
        let mut map = HashMap::new();
        for (k, v) in pages {
            if let Some(n) = Self::normalise(Path::new(k.as_ref())) {
                map.insert(n, v.into());
            }
        }
        Self { pages: Arc::new(map) }
    }

    /// Lexical normalisation. `None` for absolute paths and for `..` escaping the root.
    fn normalise(p: &Path) -> Option<String> {
        let mut parts: Vec<String> = Vec::new();
        for c in p.components() {
            match c {
                Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
                Component::CurDir => {}
                Component::ParentDir => {
                    parts.pop()?;
                }
                Component::RootDir | Component::Prefix(_) => return None,
            }
        }
        Some(parts.join("/"))
    }

    fn exists(&self, p: &Path) -> bool {
        Self::normalise(p).map_or(false, |n| self.pages.contains_key(&n))
    }
}

impl PageSource for MemPageSource {
    fn resolve(&self, base: Option<&PageRef>, target: &str, mobile: bool) -> Result<PageRef, PageError> {
        let dir = base.and_then(|b| b.base.parent()).map(|d| d.to_path_buf()).unwrap_or_default();
        let non_mobile = dir.join(strip_mobile(target));
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
        let key = Self::normalise(&page.actual).ok_or(PageError::Unavailable)?;
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

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("oxi_ps_{}_{}", name, std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            Tmp(d)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const PAGE: &str = "<screen><text>hi</text></screen>";

    /// root/app/{index,a_mobile?,sub/b}.thtml plus root/secret.thtml outside the app dir.
    fn fs_fixture(name: &str) -> (Tmp, PathBuf, FsPageSource) {
        let t = Tmp::new(name);
        let app = t.0.join("app");
        std::fs::create_dir_all(app.join("sub")).unwrap();
        std::fs::write(app.join("index.thtml"), PAGE).unwrap();
        std::fs::write(app.join("sub/b.thtml"), PAGE).unwrap();
        std::fs::write(app.join("a.thtml"), PAGE).unwrap();
        std::fs::write(app.join("a_mobile.thtml"), PAGE).unwrap();
        std::fs::write(t.0.join("secret.thtml"), PAGE).unwrap();
        let src = FsPageSource::new(app.clone());
        (t, app, src)
    }

    #[test]
    fn fs_resolves_and_loads_inside_root() {
        let (_t, app, src) = fs_fixture("ok");
        let p = src.resolve(None, "index.thtml", false).unwrap();
        assert_eq!(p.page, "index.thtml");
        assert_eq!(p.actual, app.join("index.thtml"));
        assert!(src.load(&p, RenderMode::Tui).is_ok());
        let rel = src.resolve(Some(&p), "sub/b.thtml", false).unwrap();
        assert_eq!(rel.page, "sub/b.thtml");
        // relative to the current page's directory, and the page id is not normalised
        let back = src.resolve(Some(&rel), "../index.thtml", false).unwrap();
        assert_eq!(back.page, "sub/../index.thtml");
    }

    #[test]
    fn fs_mobile_variant() {
        let (_t, app, src) = fs_fixture("mobile");
        let p = src.resolve(None, "a.thtml", true).unwrap();
        assert_eq!(p.actual, app.join("a_mobile.thtml"));
        assert_eq!(p.base, app.join("a.thtml"));
        let p = src.resolve(None, "a_mobile.thtml", false).unwrap();
        assert_eq!(p.actual, app.join("a.thtml"));
        // no variant on disk: falls back to the base page
        let p = src.resolve(None, "index.thtml", true).unwrap();
        assert_eq!(p.actual, app.join("index.thtml"));
    }

    #[test]
    fn fs_refuses_traversal_absolute_missing_and_symlinks_identically() {
        let (t, app, src) = fs_fixture("sec");
        let outside_file = t.0.join("secret.thtml");
        let mut cases: Vec<String> = vec![
            "../secret.thtml".into(),
            "sub/../../secret.thtml".into(),
            outside_file.to_string_lossy().into_owned(),
            "/etc/passwd".into(),
            "missing.thtml".into(),
        ];
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside_file, app.join("link.thtml")).unwrap();
            std::os::unix::fs::symlink(&t.0, app.join("linkdir")).unwrap();
            cases.push("link.thtml".into());
            cases.push("linkdir/secret.thtml".into());
        }
        let mut errs = Vec::new();
        for c in &cases {
            let e = src.resolve(None, c, false).expect_err(c);
            errs.push((c.clone(), e));
        }
        for (c, e) in &errs {
            assert_eq!(*e, PageError::Unavailable, "{}", c);
            assert_eq!(e.to_string(), errs[0].1.to_string());
        }
        // relative from a page inside the root is equally contained
        let cur = src.resolve(None, "sub/b.thtml", false).unwrap();
        assert_eq!(src.resolve(Some(&cur), "../../secret.thtml", false), Err(PageError::Unavailable));
        // a mobile variant that is a symlink outside the root is refused too
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside_file, app.join("index_mobile.thtml")).unwrap();
            assert_eq!(src.resolve(None, "index.thtml", true), Err(PageError::Unavailable));
            assert!(src.resolve(None, "index.thtml", false).is_ok());
        }
    }

    #[test]
    fn fs_dynamic_root_is_read_per_call() {
        let (_t, app, _) = fs_fixture("dyn");
        let cell = Arc::new(parking_lot::RwLock::new(None::<PathBuf>));
        let c2 = cell.clone();
        let src = FsPageSource::with_dynamic_root(move || c2.read().clone());
        *cell.write() = Some(app.clone());
        assert!(src.resolve(None, "index.thtml", false).is_ok());
        *cell.write() = Some(app.join("sub"));
        assert!(src.resolve(None, "index.thtml", false).is_err());
    }

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
