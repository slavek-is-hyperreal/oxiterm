use std::path::{Component, Path, PathBuf};

/// Checks if `target` path is strictly within the `base` directory, preventing path traversal attacks.
///
/// Returns false if canonicalization fails (e.g. file does not exist) or if target is outside base.
/// Strictly fail-closed.
pub fn is_within_base(base: &Path, target: &Path) -> bool {
    let canonical_base = match base.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let canonical_target = match target.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };
    canonical_target.starts_with(canonical_base)
}

// ---- media `src` resolution (Stage 7.6) ----------------------------------------------------
//
// One rule for every `src` of `img`, `video`, `diagram` (and the Rive click handler, the web
// transports and the local-mode client): a `src` is **relative to the directory of the page that
// contains it**, like in HTML. `../assets/a.png` from `demos/media.thtml` is `assets/a.png`.
// Absolute paths, URLs/schemes, backslashes, NUL and percent-encoded dot/slash/backslash are
// refused outright (nothing in the stack percent-decodes a `src`, but the local-mode asset route
// decodes URLs, so an encoded traversal is treated as an attack everywhere). The result must stay
// inside the app base: lexically always, and on the filesystem (symlinks) when there is one.

/// Collapses `.` and `..` without touching the filesystem. `None` when a `..` would climb above
/// the root of an absolute path. A relative path may keep leading `..` components (the
/// containment check refuses those).
pub fn normalize_lexical(p: &Path) -> Option<PathBuf> {
    let mut out: Vec<Component> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => return None,
                _ => out.push(c),
            },
            other => out.push(other),
        }
    }
    Some(out.iter().collect())
}

/// Purely lexical shape check of a media `src` (see the module notes above).
pub fn media_src_is_plain(src: &str) -> bool {
    if src.is_empty() || src.starts_with('/') || src.contains('\\') || src.contains('\0') {
        return false;
    }
    // `scheme:`/`C:`: a colon in the first segment. URLs (`http://x`) are one case of it.
    if src.split('/').next().is_some_and(|first| first.contains(':')) {
        return false;
    }
    let lower = src.to_ascii_lowercase();
    if lower.contains("%2e") || lower.contains("%2f") || lower.contains("%5c") {
        return false;
    }
    // A file name is required: `.`, `..` and `dir/` name directories.
    !matches!(src.rsplit('/').next(), None | Some("" | "." | ".."))
}

/// `dir(page)/src` as a relative path with `.`/`..` collapsed, inside the app root; `None` when
/// the `src` is not plain or would leave the root. This is the key local mode uses for assets
/// (`/oxiui/local/asset/<rel>`) and for its in-memory file system.
pub fn media_rel_path(current_page: &str, src: &str) -> Option<String> {
    if !media_src_is_plain(src) {
        return None;
    }
    let dir = current_page.rsplit_once('/').map_or("", |(d, _)| d);
    let mut parts: Vec<&str> = Vec::new();
    for seg in dir.split('/').chain(src.split('/')) {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn absolute(p: &Path) -> Option<PathBuf> {
    if p.is_absolute() {
        normalize_lexical(p)
    } else {
        normalize_lexical(&std::env::current_dir().ok()?.join(p))
    }
}

/// The nearest existing ancestor (or the path itself), canonicalised.
fn canonical_prefix(p: &Path) -> Option<PathBuf> {
    let mut cur = p;
    loop {
        if let Ok(c) = cur.canonicalize() {
            return Some(c);
        }
        cur = cur.parent()?;
    }
}

/// Resolves a media `src` of the page whose directory is `base_dir` and checks it against the app
/// base. Returns the normalised path (`base_dir/src`, `..` collapsed) or `None` when it is not
/// allowed. The file need not exist (the renderer draws a placeholder for a missing one), so a
/// missing file is not a traversal.
///
/// * `app_base = None`: fail closed (`None`).
/// * `app_base = Some("")` (empty path): a virtual app root with no filesystem (the browser's
///   local mode): the lexical check only.
/// * otherwise: the lexical check on absolute forms, then the nearest existing ancestor of the
///   result must canonicalise to somewhere inside the canonical base (so a symlink pointing out
///   of the base is refused, also for a file that does not exist yet below it).
pub fn resolve_media(base_dir: Option<&Path>, app_base: Option<&Path>, src: &str) -> Option<PathBuf> {
    if !media_src_is_plain(src) {
        return None;
    }
    let app_base = app_base?;
    let joined = match base_dir {
        Some(b) => b.join(src),
        None => PathBuf::from(src),
    };
    let resolved = normalize_lexical(&joined)?;
    if app_base.as_os_str().is_empty() {
        // Virtual root: relative, never starting with `..`.
        let ok = !resolved.is_absolute() && !resolved.components().any(|c| !matches!(c, Component::Normal(_)));
        return ok.then_some(resolved);
    }
    let abs_base = absolute(app_base)?;
    let abs_res = absolute(&resolved)?;
    if !abs_res.starts_with(&abs_base) {
        return None;
    }
    let canon_base = abs_base.canonicalize().ok()?;
    let canon_res = canonical_prefix(&abs_res)?;
    canon_res.starts_with(&canon_base).then_some(resolved)
}

#[cfg(test)]
mod media_tests;
