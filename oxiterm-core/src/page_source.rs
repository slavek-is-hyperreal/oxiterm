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
