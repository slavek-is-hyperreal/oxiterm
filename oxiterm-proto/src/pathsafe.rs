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

/// Longest accepted media `src` (bytes). A real path is far shorter; the cap keeps every later
/// per-component step (and the filesystem probing in [`resolve_media`]) cheap (BAD ACTOR 7.6 F-05).
pub const MAX_SRC_BYTES: usize = 4096;

/// Purely lexical shape check of a media `src` (see the module notes above).
pub fn media_src_is_plain(src: &str) -> bool {
    if src.is_empty() || src.len() > MAX_SRC_BYTES || src.starts_with('/') || src.contains('\\') || src.contains('\0') {
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

/// The nearest existing ancestor (or the path itself), canonicalised. One pass down from the
/// root (one `symlink_metadata` per component, one `canonicalize` at the end) instead of one
/// `canonicalize` per shorter prefix. A dangling symlink as the last existing component fails
/// closed (`None`).
fn canonical_prefix(p: &Path) -> Option<PathBuf> {
    let mut cur = PathBuf::new();
    let mut last_existing: Option<PathBuf> = None;
    for c in p.components() {
        cur.push(c.as_os_str());
        match std::fs::symlink_metadata(&cur) {
            Ok(_) => last_existing = Some(cur.clone()),
            Err(_) => break,
        }
    }
    last_existing?.canonicalize().ok()
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
    if !canon_res.starts_with(&canon_base) {
        return None;
    }
    // A hard link inside the base to a file outside it is a path inside the base: refuse a
    // multiply linked regular file (the 7.1 rule; `OXITERM_ALLOW_HARDLINKS=1` accepts them). A
    // file that does not exist yet has nothing to check.
    if std::fs::symlink_metadata(&abs_res).is_ok_and(|m| meta_multi_linked(&m)) {
        return None;
    }
    Some(resolved)
}

/// A regular file with more than one hard link (unix; `OXITERM_ALLOW_HARDLINKS=1` turns the rule
/// off, for `cp -al` / `rsync --link-dest` deployments). Works on already opened metadata, so
/// there is no path race between check and use.
pub fn meta_multi_linked(m: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if std::env::var("OXITERM_ALLOW_HARDLINKS").is_ok_and(|v| v == "1") {
            return false;
        }
        m.is_file() && m.nlink() > 1
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        false
    }
}

/// Key of a media file on the wire (0x20 bytes, 0x21 position, 0xE0 request): FNV-1a, 64 bit, of
/// the file's path *relative to the app base* with `/` separators. Keyed by the resolved file,
/// not by the raw `src`, so the same `src` on pages in different directories is two keys; 64 bit
/// so that accidental collisions are out of reach (`Ez.png`/`FY.png` collided as 32-bit djb2).
pub fn media_key(app_rel_path: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in app_rel_path.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The path of `resolved` relative to `app_base` as a `/`-separated string; `None` when it is not
/// below it (lexically). An empty base (the virtual root) keeps the path as is.
pub fn app_rel(app_base: &Path, resolved: &Path) -> Option<String> {
    let rel = if app_base.as_os_str().is_empty() {
        resolved.to_path_buf()
    } else {
        absolute(resolved)?.strip_prefix(absolute(app_base)?).ok()?.to_path_buf()
    };
    let parts: Vec<&str> = rel.components().map(|c| c.as_os_str().to_str()).collect::<Option<_>>()?;
    Some(parts.join("/"))
}

/// Largest media file read whole into memory (the asset route's cap).
pub const MAX_MEDIA_BYTES: u64 = 16 * 1024 * 1024;

/// Opens `path` (a result of [`resolve_media`]) for reading and checks the *opened descriptor*,
/// closing the check-then-open race (BAD ACTOR 7.6 F-03/F-08/F-07/F-02):
///
/// 1. `symlink_metadata`: must be a regular file (a symlink, FIFO or device is refused here);
/// 2. open with `O_NOFOLLOW | O_NONBLOCK` (a swapped-in symlink fails; a FIFO swapped in
///    afterwards opens without blocking and fails step 3);
/// 3. on the descriptor: regular file, size <= `max`, not multiply hard-linked;
/// 4. where the descriptor really is: `/proc/self/fd/N` must lie inside the canonical base (this
///    catches a swapped *directory* in the middle of the path, which `O_NOFOLLOW` does not);
///    without `/proc`, the path is canonicalised again and must name the same device and inode.
#[cfg(unix)]
pub fn open_media_checked(app_base: &Path, path: &Path, max: u64) -> std::io::Result<std::fs::File> {
    use std::io::{Error, ErrorKind};
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let bad = |m: &str| Error::new(ErrorKind::InvalidInput, m.to_string());
    let canon_base = app_base.canonicalize()?;
    let abs = absolute(path).ok_or_else(|| bad("path"))?;
    if !abs.starts_with(absolute(app_base).ok_or_else(|| bad("base"))?) {
        return Err(bad("outside the app base"));
    }
    if !std::fs::symlink_metadata(&abs)?.is_file() {
        return Err(bad("not a regular file"));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(&abs)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(bad("not a regular file"));
    }
    if meta.len() > max {
        return Err(bad("file too large"));
    }
    if meta_multi_linked(&meta) {
        return Err(bad("hard-linked file"));
    }
    let located = if let Ok(real) = std::fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())) {
        real.starts_with(&canon_base)
    } else {
        let again = abs.canonicalize()?;
        let m2 = std::fs::metadata(&again)?;
        again.starts_with(&canon_base) && m2.dev() == meta.dev() && m2.ino() == meta.ino()
    };
    if !located {
        return Err(bad("opened file is outside the app base"));
    }
    Ok(file)
}

/// [`open_media_checked`] without the unix descriptor tricks: the same checks on the path.
#[cfg(not(unix))]
pub fn open_media_checked(app_base: &Path, path: &Path, max: u64) -> std::io::Result<std::fs::File> {
    use std::io::{Error, ErrorKind};
    let bad = |m: &str| Error::new(ErrorKind::InvalidInput, m.to_string());
    let canon = path.canonicalize()?;
    if !canon.starts_with(app_base.canonicalize()?) {
        return Err(bad("outside the app base"));
    }
    let file = std::fs::File::open(&canon)?;
    let meta = file.metadata()?;
    if !meta.is_file() || meta.len() > max {
        return Err(bad("not a regular file or too large"));
    }
    Ok(file)
}

/// Reads a media file through [`open_media_checked`], never more than `max` bytes.
pub fn read_media_checked(app_base: &Path, path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let file = open_media_checked(app_base, path, max)?;
    let mut out = Vec::new();
    file.take(max + 1).read_to_end(&mut out)?;
    if out.len() as u64 > max {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "file too large"));
    }
    Ok(out)
}

#[cfg(test)]
mod media_tests;
