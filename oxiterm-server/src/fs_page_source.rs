//! Filesystem page source (server side): pages live under a root directory, containment uses
//! canonicalized paths. Split from `page_source.rs` so that module stays core-bound.

use std::path::{Path, PathBuf};
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::document::THTMLDocument;
use tracing::warn;

use crate::page_source::{strip_mobile, PageError, PageRef, PageSource};

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    const PAGE: &str = "<screen><text>hi</text></screen>";

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
}
