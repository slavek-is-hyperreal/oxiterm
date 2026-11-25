//! Server side of the browser local mode (OxiUI Stage 7.1, `/?local=1`).
//!
//! **Off by default.** The routes below exist only when the operator sets `OXITERM_LOCAL_MODE=1`
//! (`oxiterm-cli serve --local-mode`); otherwise every `/oxiui/local/*` request is the same
//! generic 404 and the page falls back to the thin GUI client.
//!
//! The page runs `oxiterm-core` in the browser and needs three things from the server, all
//! same-origin and all behind the same guest/identity gate as `/ws`:
//!
//! * `GET /oxiui/local/pages.json`: the app's pages in one document
//!   (`{start, actions, truncated, dirs, pages}`) so the browser can navigate with no further
//!   requests (and keep working when the server is gone). The request carries no path: the server
//!   walks its own app base directory (bounded, cached for a few seconds). The response also
//!   carries a signed per-page-load session token in the `X-OxiTerm-Local-Session` header.
//! * `GET /oxiui/local/asset/<path>`: one media file (images, video) under the app base,
//!   streamed.
//! * `POST /oxiui/local/events`: the App Server proxy. The browser never talks to the App Server
//!   itself: the `Authorization: Bearer` token stays here, the identity fields and `session_id`
//!   are set by the server, and the answer is returned verbatim (the contract of
//!   `HttpActionHandler`). **Local-mode actions are untrusted client input**: only an `action`
//!   that some indexed page declares (`event-htmx` / `event-drag-end`, exact match, plus the
//!   core's own `app_token_sync`) is forwarded, and the state snapshot is cut down to the keys
//!   the pages bind (never `_`-prefixed keys; those the pages bind are set from the identity).
//!
//! Safety rules are those of `FsPageSource`/`pathsafe`: nothing from a request reaches the
//! filesystem unchecked, symlinks and multiply hard-linked files are never followed or served,
//! every file is canonicalised and must lie in the base, and every refusal is the same generic
//! `None` (the HTTP layer turns it into one 404).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::action::{ActionHandler, ActionReply, DispatchPayload};
use crate::identity::UserIdentity;

/// Most pages the index carries (deterministic: sorted path order).
pub const MAX_PAGES: usize = 512;
/// Largest single page the index carries.
pub const MAX_PAGE_BYTES: u64 = 512 * 1024;
/// Total THTML text the index carries.
pub const MAX_TOTAL_BYTES: u64 = 8 * 1024 * 1024;
/// Deepest directory the walk enters.
pub const MAX_DEPTH: usize = 8;
/// Directory entries the walk looks at in total (files, directories, anything). The walk stops
/// at the first cap it hits and reports `truncated`.
pub const MAX_ENTRIES: usize = 10_000;
/// How long a built index is reused before the directory is walked again.
pub const INDEX_TTL: Duration = Duration::from_secs(3);
/// Largest media file served.
pub const MAX_ASSET_BYTES: u64 = 16 * 1024 * 1024;
/// Largest `POST /oxiui/local/events` body (same cap as the patch endpoint).
pub const MAX_EVENT_BODY: usize = 64 * 1024;
/// Proxy requests in flight at once; more are answered 503.
pub const MAX_PROXY_IN_FLIGHT: usize = 16;
/// Request header carrying the session token minted with `pages.json`.
pub const SESSION_HEADER: &str = "x-oxiterm-local-session";

/// Local mode is switched on (`OXITERM_LOCAL_MODE=1` or `true`). Read per request.
pub fn enabled() -> bool {
    matches!(std::env::var("OXITERM_LOCAL_MODE").as_deref(), Ok("1") | Ok("true"))
}

/// How long the proxy waits for the App Server (`OXITERM_LOCAL_PROXY_TIMEOUT_MS`, default 15 s).
/// The HTTP client's own total timeout is this value, so no request thread outlives it.
pub fn proxy_timeout() -> Duration {
    std::env::var("OXITERM_LOCAL_PROXY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map_or(Duration::from_secs(15), |ms| Duration::from_millis(ms.clamp(50, 120_000)))
}

/// `OXITERM_ALLOWED_HOSTS`: comma-separated `host[:port]` values the `Host` header must match
/// (case-insensitive) on the local routes and on `/ws`. Unset or empty = no check. It is the
/// defence against DNS rebinding: a rebound attacker origin still carries its own `Host`.
pub fn allowed_hosts() -> Option<Vec<String>> {
    let v = std::env::var("OXITERM_ALLOWED_HOSTS").ok()?;
    let list: Vec<String> = v.split(',').map(|h| h.trim().to_ascii_lowercase()).filter(|h| !h.is_empty()).collect();
    if list.is_empty() {
        None
    } else {
        Some(list)
    }
}

/// The `Host` header is on the allowlist (or there is none).
pub fn host_allowed(headers: &hyper::HeaderMap) -> bool {
    match allowed_hosts() {
        None => true,
        Some(list) => {
            let host = headers.get("host").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
            list.iter().any(|h| *h == host)
        }
    }
}

// ---- session ids ---------------------------------------------------------------------------

/// Local clients get ids from `LOCAL_ID_BASE..LOCAL_ID_BASE + 2^27`. Registry ids count up from 0,
/// so the namespaces cannot meet before 268 million real sessions; the range still fits a
/// 32-bit `usize`.
pub const LOCAL_ID_BASE: usize = 1 << 28;
const LOCAL_ID_SPAN: usize = 1 << 27;

static SID_COUNTER: AtomicUsize = AtomicUsize::new(0);
static SID_KEY: OnceLock<std::collections::hash_map::RandomState> = OnceLock::new();

fn sid_mac(id: usize) -> u64 {
    // SipHash with per-process random keys: a keyed PRF the client cannot evaluate.
    let mut h = SID_KEY.get_or_init(std::collections::hash_map::RandomState::new).build_hasher();
    h.write_u64(id as u64);
    h.write(b"oxiterm-local-session");
    h.finish()
}

/// Is `id` in the local namespace (never a real WebSocket or SSH session)?
pub fn is_local_session_id(id: usize) -> bool {
    (LOCAL_ID_BASE..LOCAL_ID_BASE + LOCAL_ID_SPAN).contains(&id)
}

/// A new local session id and its signed token (`<id>.<mac>`), one per page load.
pub fn mint_session() -> (usize, String) {
    let id = LOCAL_ID_BASE + (SID_COUNTER.fetch_add(1, Ordering::Relaxed) & (LOCAL_ID_SPAN - 1));
    (id, format!("{id}.{:016x}", sid_mac(id)))
}

/// The session id of a token minted by [`mint_session`] in this process; `None` for anything
/// else (a forged id, a token of an earlier run). The tag is compared in constant time.
pub fn verify_session(token: &str) -> Option<usize> {
    use subtle::ConstantTimeEq;
    let (id, tag) = token.split_once('.')?;
    let id: usize = id.parse().ok()?;
    if !is_local_session_id(id) || tag.len() != 16 {
        return None;
    }
    let want = format!("{:016x}", sid_mac(id));
    bool::from(want.as_bytes().ct_eq(tag.as_bytes())).then_some(id)
}

// ---- the page index ------------------------------------------------------------------------

/// The pages of an app, keyed by path relative to the app base (`/` separators).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct PageIndex {
    pub pages: BTreeMap<String, String>,
    /// Every real, non-hidden directory the walk saw (so `..` resolves like the filesystem).
    pub dirs: BTreeSet<String>,
    /// A cap was hit: the index is incomplete (the page logs a warning).
    pub truncated: bool,
}

fn within(base: &Path, p: &Path) -> bool {
    crate::pathsafe::is_within_base(base, p)
}

struct Walk<'a> {
    base: &'a Path,
    entries: usize,
    total: u64,
    out: PageIndex,
    stop: bool,
}

impl Walk<'_> {
    fn cap(&mut self) {
        self.out.truncated = true;
        self.stop = true;
    }

    fn walk(&mut self, dir: &Path, rel: &str, depth: usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        // At most the remaining entry budget (+1 to notice overflow) is ever read from one
        // directory, so a directory of millions of files costs the same as one of 10 000.
        let budget = MAX_ENTRIES - self.entries;
        let mut items: Vec<(std::ffi::OsString, bool, bool)> = Vec::new();
        for e in rd.take(budget + 1) {
            let Ok(e) = e else { continue };
            // `file_type()` comes from the directory entry (d_type): no stat, never follows a link.
            let Ok(ft) = e.file_type() else { continue };
            items.push((e.file_name(), ft.is_dir(), ft.is_file()));
        }
        if items.len() > budget {
            items.truncate(budget);
            self.cap();
        }
        items.sort();
        for (name, is_dir, is_file) in items {
            if self.stop {
                return;
            }
            self.entries += 1;
            let Some(name_s) = name.to_str() else { continue };
            // Hidden files (and `.git`, `.env`) are never part of the app.
            if name_s.starts_with('.') {
                continue;
            }
            let rel_name = if rel.is_empty() { name_s.to_string() } else { format!("{rel}/{name_s}") };
            let path = dir.join(&name);
            if is_dir {
                self.out.dirs.insert(rel_name.clone());
                if depth < MAX_DEPTH {
                    self.walk(&path, &rel_name, depth + 1);
                }
            } else if is_file && name_s.ends_with(".thtml") {
                // Only now a stat: the size cap and the hard-link check need it.
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                if meta.len() > MAX_PAGE_BYTES {
                    self.out.truncated = true;
                    continue;
                }
                if self.out.pages.len() >= MAX_PAGES || self.total + meta.len() > MAX_TOTAL_BYTES {
                    self.cap();
                    return;
                }
                if crate::pathsafe::meta_multi_linked(&meta) || !within(self.base, &path) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else { continue };
                self.total += text.len() as u64;
                self.out.pages.insert(rel_name, text);
            }
        }
    }
}

/// Reads every `.thtml` page under `base` (symlinks, hidden entries, multiply hard-linked files and
/// anything that fails the containment check are skipped). The walk is bounded: it stops at the
/// first cap it hits (`MAX_PAGES`, `MAX_TOTAL_BYTES`, `MAX_ENTRIES`) and reports `truncated`.
/// `None` when `base` is not a directory.
pub fn build_page_index(base: &Path) -> Option<PageIndex> {
    let meta = std::fs::symlink_metadata(base).ok()?;
    // The base itself may be a symlink to the app directory (deployments do that); its content may
    // not be.
    let base = base.canonicalize().ok()?;
    if !meta.is_dir() && !base.is_dir() {
        return None;
    }
    let mut w = Walk { base: &base, entries: 0, total: 0, out: PageIndex::default(), stop: false };
    w.walk(&base, "", 0);
    Some(w.out)
}

fn scan_page(text: &str, actions: &mut HashSet<String>, bound: &mut HashSet<String>) {
    let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        oxiterm_renderer::parser::THTMLParser::parse_with_mode(text, oxiterm_proto::style::RenderMode::Gui)
    }));
    let Ok(Ok(doc)) = parsed else { return };
    for (_, n) in doc.arena.iter() {
        let a = &n.attrs;
        for act in [&a.event_htmx, &a.event_drag_end].into_iter().flatten() {
            actions.insert(act.clone());
        }
        for key in [&a.bind_state, &a.bind_value, &a.drag_state_x, &a.drag_state_y].into_iter().flatten() {
            bound.insert(key.clone());
        }
        if let Some(show) = &a.bind_show {
            bound.insert(show.split('=').next().unwrap_or(show).to_string());
        }
    }
}

/// A built index plus what the proxy checks requests against.
#[derive(Debug)]
pub struct LocalIndex {
    /// `pages` as serialised JSON (so a request does not re-serialise megabytes).
    pages_json: Vec<u8>,
    page_names: BTreeSet<String>,
    dirs: Vec<String>,
    truncated: bool,
    /// Every action some page declares (`event-htmx`, `event-drag-end`) and the core's
    /// `app_token_sync`.
    pub actions: Arc<HashSet<String>>,
    /// Every state key some page binds.
    pub bound_keys: Arc<HashSet<String>>,
}

impl LocalIndex {
    /// Builds the index of `base`; `None` when it is not a directory.
    pub fn build(base: &Path) -> Option<Self> {
        let idx = build_page_index(base)?;
        let mut actions = HashSet::new();
        let mut bound = HashSet::new();
        actions.insert("app_token_sync".to_string());
        for text in idx.pages.values() {
            scan_page(text, &mut actions, &mut bound);
        }
        Some(Self {
            pages_json: serde_json::to_vec(&idx.pages).ok()?,
            page_names: idx.pages.keys().cloned().collect(),
            dirs: idx.dirs.into_iter().collect(),
            truncated: idx.truncated,
            actions: Arc::new(actions),
            bound_keys: Arc::new(bound),
        })
    }

    /// The `pages.json` document. `None` when `start` is not an indexed page.
    pub fn json(&self, start: &str, actions: bool) -> Option<Vec<u8>> {
        if !self.page_names.contains(start) {
            return None;
        }
        let head = serde_json::json!({ "start": start, "actions": actions, "truncated": self.truncated, "dirs": self.dirs });
        let mut out = serde_json::to_vec(&head).ok()?;
        out.pop(); // the closing brace
        out.extend_from_slice(b",\"pages\":");
        out.extend_from_slice(&self.pages_json);
        out.push(b'}');
        Some(out)
    }
}

type IndexCache = Mutex<HashMap<PathBuf, (Instant, Arc<LocalIndex>)>>;
static INDEX_CACHE: OnceLock<IndexCache> = OnceLock::new();

/// [`LocalIndex::build`] cached per base directory for [`INDEX_TTL`]: guest requests cannot make
/// the server walk the tree more than once per window.
pub fn local_index(base: &Path) -> Option<Arc<LocalIndex>> {
    let key = base.canonicalize().ok()?;
    let cache = INDEX_CACHE.get_or_init(Default::default);
    if let Some((at, idx)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if at.elapsed() < INDEX_TTL {
            return Some(idx.clone());
        }
    }
    let idx = Arc::new(LocalIndex::build(&key)?);
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (Instant::now(), idx.clone()));
    Some(idx)
}

/// The `pages.json` document, uncached. `start` is the page the server was started with
/// (relative to `base`), `actions` whether `POST /oxiui/local/events` has an App Server behind it.
pub fn index_json(base: &Path, start: &str, actions: bool) -> Option<Vec<u8>> {
    LocalIndex::build(base)?.json(start, actions)
}

// ---- assets --------------------------------------------------------------------------------

/// Media types served by the asset route. `.json` (Lottie) and `.riv` are deliberately absent:
/// the app directory may hold other JSON, and local mode serves only what a page can display
/// as a plain image or video (see the 7.1 notes).
fn asset_mime(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => return None,
    })
}

/// Percent-decoding of a URL path (`%XX` only; `+` stays). `None` for malformed escapes and
/// non-UTF-8 results.
pub fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// A validated media file: where it is, how big, its content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFile {
    pub mime: &'static str,
    pub path: PathBuf,
    pub len: u64,
}

/// Validates `rel_url` (the part of the URL after `/oxiui/local/asset/`, still percent-encoded)
/// as a media file inside `base`, without reading it. Every refusal is `None`. The caller opens
/// the path and must re-check the opened file with [`check_opened_asset`].
pub fn resolve_asset(base: &Path, rel_url: &str) -> Option<AssetFile> {
    let rel = percent_decode(rel_url)?;
    if rel.is_empty() || rel.starts_with('/') || rel.contains('\\') || rel.contains('\0') {
        return None;
    }
    let mut full = base.to_path_buf();
    for seg in rel.split('/') {
        // `..` and `.` never reach the filesystem; hidden names are never served.
        if seg.is_empty() || seg == "." || seg == ".." || seg.starts_with('.') {
            return None;
        }
        full.push(seg);
    }
    let mime = asset_mime(&rel)?;
    // No symlinks anywhere on the path below the base, then canonical containment.
    let mut probe = base.to_path_buf();
    for seg in rel.split('/') {
        probe.push(seg);
        let m = std::fs::symlink_metadata(&probe).ok()?;
        if m.file_type().is_symlink() {
            return None;
        }
    }
    let meta = std::fs::metadata(&full).ok()?;
    if !meta.is_file() || meta.len() > MAX_ASSET_BYTES || crate::pathsafe::meta_multi_linked(&meta) || !within(&base.canonicalize().ok()?, &full) {
        return None;
    }
    Some(AssetFile { mime, path: full, len: meta.len() })
}

/// The checks that must hold for the file actually opened (the path may have changed since
/// [`resolve_asset`]): a regular, singly linked file of the validated size cap.
pub fn check_opened_asset(meta: &std::fs::Metadata) -> bool {
    meta.is_file() && meta.len() <= MAX_ASSET_BYTES && !crate::pathsafe::meta_multi_linked(meta)
}

/// [`resolve_asset`] plus a whole read (tests, small tools); the HTTP route streams instead.
pub fn read_asset(base: &Path, rel_url: &str) -> Option<(&'static str, Vec<u8>)> {
    let a = resolve_asset(base, rel_url)?;
    Some((a.mime, std::fs::read(&a.path).ok()?))
}

// ---- the events proxy ----------------------------------------------------------------------

/// An App Server is configured (`OXITERM_APP_SERVER`), i.e. the events proxy has a target.
pub fn app_server_url() -> Option<String> {
    std::env::var("OXITERM_APP_SERVER").ok().filter(|u| !u.is_empty())
}

/// CSRF and rebinding gate of the events proxy (and the host check of every local route). The
/// proxy attaches the server's App Server token and, behind a trusted-header proxy, acts as the
/// logged-in user, so a page of another site must not be able to trigger it. Requires
/// `Content-Type: application/json` (a cross-site form or `text/plain` POST is a "simple" request
/// that needs no preflight; JSON forces one, which is refused because no CORS headers are ever
/// sent), refuses `Sec-Fetch-Site: cross-site|same-site`, requires `Host` to be on
/// `OXITERM_ALLOWED_HOSTS` when that is set, and when `Origin` is present it must name this host.
/// `Err(status)` on refusal.
pub fn check_same_origin_json(headers: &hyper::HeaderMap) -> Result<(), u16> {
    let get = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    let ct = get("content-type").unwrap_or("").to_ascii_lowercase();
    let media = ct.split(';').next().unwrap_or("").trim();
    if media != "application/json" {
        return Err(415);
    }
    if !host_allowed(headers) {
        return Err(403);
    }
    if let Some(site) = get("sec-fetch-site") {
        if site != "same-origin" && site != "none" {
            return Err(403);
        }
    }
    if let Some(origin) = get("origin") {
        let host = get("host").unwrap_or("");
        let authority = origin.split_once("://").map_or(origin, |(_, a)| a);
        if host.is_empty() || !authority.eq_ignore_ascii_case(host) {
            return Err(403);
        }
    }
    Ok(())
}

/// Result of the events proxy: `(HTTP status, body)`.
pub type ProxyReply = (u16, Vec<u8>);

/// What one proxied request is checked against: the page-load session it belongs to and what the
/// indexed pages declare.
#[derive(Debug, Clone)]
pub struct ProxyCtx {
    pub session_id: usize,
    pub actions: Arc<HashSet<String>>,
    pub bound_keys: Arc<HashSet<String>>,
}

impl ProxyCtx {
    pub fn new(session_id: usize, actions: impl IntoIterator<Item = String>, bound_keys: impl IntoIterator<Item = String>) -> Self {
        Self { session_id, actions: Arc::new(actions.into_iter().collect()), bound_keys: Arc::new(bound_keys.into_iter().collect()) }
    }
}

static PROXY_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct InFlight;
impl InFlight {
    fn acquire() -> Option<Self> {
        if PROXY_IN_FLIGHT.fetch_add(1, Ordering::SeqCst) >= MAX_PROXY_IN_FLIGHT {
            PROXY_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
            return None;
        }
        Some(Self)
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        PROXY_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Forwards one action of a local-mode page to the App Server. Blocking (run it on a blocking
/// thread). The body must be a `DispatchPayload`. Everything in it is client input and is
/// treated as such:
///
/// * `action` must be declared by an indexed page (`ctx.actions`, exact match); otherwise 403
///   and the App Server is never contacted;
/// * `state` keeps only keys some page binds (`ctx.bound_keys`) and never a `_`-prefixed one;
///   the server-owned `_username` / `_auth_method` / `_is_web` are set from `identity` when a
///   page binds them;
/// * `username`, `auth_method` and `session_id` are overwritten (`ctx.session_id` is a signed,
///   server-minted id in the local namespace, never 0 and never client-chosen).
///
/// The App Server's 200 JSON answer is returned verbatim; any other outcome is a generic 502.
/// At most [`MAX_PROXY_IN_FLIGHT`] requests run at once (503 beyond).
pub fn proxy_event_ctx(body: &[u8], identity: &UserIdentity, ctx: &ProxyCtx) -> ProxyReply {
    let Some(url) = app_server_url() else { return (404, b"Not Found".to_vec()) };
    if body.len() > MAX_EVENT_BODY {
        return (413, b"Payload Too Large".to_vec());
    }
    let mut payload: DispatchPayload = match serde_json::from_slice(body) {
        Ok(p) => p,
        Err(_) => return (400, b"Bad Request".to_vec()),
    };
    if payload.action.is_empty() || payload.action.len() > 2048 {
        return (400, b"Bad Request".to_vec());
    }
    if !ctx.actions.contains(&payload.action) {
        return (403, b"Forbidden".to_vec());
    }
    payload.state.retain(|k, _| !k.starts_with('_') && ctx.bound_keys.contains(k));
    for (key, value) in [
        ("_username", identity.username.clone()),
        ("_auth_method", format!("{:?}", identity.auth_method)),
        ("_is_web", "true".to_string()),
    ] {
        if ctx.bound_keys.contains(key) {
            payload.state.insert(key.to_string(), value);
        }
    }
    payload.username = Some(identity.username.clone());
    payload.auth_method = Some(format!("{:?}", identity.auth_method));
    payload.session_id = ctx.session_id;

    let Some(_slot) = InFlight::acquire() else { return (503, b"Service Unavailable".to_vec()) };
    let timeout = proxy_timeout();
    let (tx, rx) = std::sync::mpsc::channel();
    crate::dispatcher::HttpActionHandler::with_timeout(url, timeout).dispatch(
        payload,
        ActionReply::new(move |patch| {
            let _ = tx.send(patch);
        }),
    );
    // The HTTP client gives up at `timeout` and drops the reply (the channel disconnects); the
    // extra margin only covers a stuck worker.
    match rx.recv_timeout(timeout + Duration::from_millis(500)) {
        Ok(patch) => (200, patch.to_string().into_bytes()),
        // Dropped reply (unreachable, non-200, bad JSON) or timeout.
        Err(_) => (502, b"Bad Gateway".to_vec()),
    }
}

/// Start page of the app relative to `base`: the file name of the page the server was started
/// with (`serve examples/index.thtml` -> `index.thtml`).
pub fn start_page(source_path: &Path) -> Option<String> {
    source_path.file_name()?.to_str().map(str::to_string)
}

/// App base directory: the directory of the start page (`.` for a bare file name).
pub fn app_base(source_path: &Path) -> PathBuf {
    match source_path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs_page_source::FsPageSource;
    use crate::page_source::{MemPageSource, PageError, PageSource};
    use crate::test_env::EnvGuard;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    const PAGE: &str = "<screen><text>hi</text></screen>";

    struct Tmp(PathBuf);
    impl Tmp {
        fn new(name: &str) -> Self {
            let d = std::env::temp_dir().join(format!("oxi_local_{}_{}", name, std::process::id()));
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

    /// root/{secret.thtml, app/{index,a,a_mobile,sub/b,sub/deep/c}.thtml + assets}.
    fn fixture(name: &str) -> (Tmp, PathBuf) {
        let t = Tmp::new(name);
        let app = t.0.join("app");
        std::fs::create_dir_all(app.join("sub/deep")).unwrap();
        for p in ["index.thtml", "a.thtml", "a_mobile.thtml", "sub/b.thtml", "sub/deep/c.thtml"] {
            std::fs::write(app.join(p), PAGE).unwrap();
        }
        std::fs::write(t.0.join("secret.thtml"), "SECRET").unwrap();
        std::fs::write(t.0.join("secret.png"), b"SECRETPNG").unwrap();
        std::fs::write(app.join("pic.png"), b"\x89PNG-bytes").unwrap();
        std::fs::write(app.join("sub/clip.MP4"), b"mp4-bytes").unwrap();
        std::fs::write(app.join("config.json"), b"{\"key\":1}").unwrap();
        std::fs::write(app.join("notes.txt"), b"x").unwrap();
        std::fs::write(app.join(".env.thtml"), "HIDDEN").unwrap();
        (t, app)
    }

    #[test]
    fn index_has_every_page_and_nothing_else() {
        let (_t, app) = fixture("idx");
        let idx = build_page_index(&app).unwrap();
        let keys: Vec<&str> = idx.pages.keys().map(String::as_str).collect();
        assert_eq!(keys, ["a.thtml", "a_mobile.thtml", "index.thtml", "sub/b.thtml", "sub/deep/c.thtml"]);
        assert!(!idx.truncated);
        assert!(idx.pages.values().all(|v| v == PAGE));
        // not a directory
        assert!(build_page_index(&app.join("index.thtml")).is_none());
        assert!(build_page_index(&app.join("missing")).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn index_never_follows_symlinks() {
        let (t, app) = fixture("sym");
        std::os::unix::fs::symlink(t.0.join("secret.thtml"), app.join("link.thtml")).unwrap();
        std::os::unix::fs::symlink(&t.0, app.join("linkdir")).unwrap();
        std::os::unix::fs::symlink(app.join("a.thtml"), app.join("inner_link.thtml")).unwrap();
        let idx = build_page_index(&app).unwrap();
        assert!(!idx.pages.values().any(|v| v.contains("SECRET")));
        assert!(!idx.pages.keys().any(|k| k.contains("link")), "{:?}", idx.pages.keys().collect::<Vec<_>>());
        // the base itself may be a symlink to the app dir (its content is still read)
        let via = t.0.join("via");
        std::os::unix::fs::symlink(&app, &via).unwrap();
        assert_eq!(build_page_index(&via).unwrap().pages.len(), 5);
    }

    #[test]
    fn index_caps_are_deterministic_and_reported() {
        let t = Tmp::new("caps");
        for i in 0..(MAX_PAGES + 5) {
            std::fs::write(t.0.join(format!("p{i:04}.thtml")), "x").unwrap();
        }
        std::fs::write(t.0.join("big.thtml"), vec![b'x'; MAX_PAGE_BYTES as usize + 1]).unwrap();
        let idx = build_page_index(&t.0).unwrap();
        assert_eq!(idx.pages.len(), MAX_PAGES);
        assert!(idx.truncated);
        assert!(!idx.pages.contains_key("big.thtml"));
        // deepest directories beyond MAX_DEPTH are not entered
        let mut deep = t.0.join("d");
        for i in 0..(MAX_DEPTH + 2) {
            deep = deep.join(format!("l{i}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("z.thtml"), "x").unwrap();
        assert!(!build_page_index(&t.0).unwrap().pages.keys().any(|k| k.ends_with("z.thtml")));
    }

    #[test]
    fn index_json_needs_the_start_page_and_reports_actions() {
        let (_t, app) = fixture("json");
        let v: serde_json::Value = serde_json::from_slice(&index_json(&app, "index.thtml", true).unwrap()).unwrap();
        assert_eq!(v["start"], "index.thtml");
        assert_eq!(v["actions"], true);
        assert_eq!(v["truncated"], false);
        assert_eq!(v["pages"]["sub/b.thtml"], PAGE);
        assert!(index_json(&app, "nope.thtml", false).is_none());
    }

    /// The page source the browser builds from the index and the server's own `FsPageSource`
    /// resolve the same targets to the same page ids and refuse the same escapes with the same
    /// generic error.
    #[test]
    fn mem_page_source_from_index_matches_fs_page_source() {
        let (t, app) = fixture("parity");
        let idx = build_page_index(&app).unwrap();
        let mem = MemPageSource::with_dirs(idx.pages.clone(), idx.dirs.iter().cloned());
        let fs = FsPageSource::new(app.canonicalize().unwrap());

        // (current page or None for the root, target, mobile)
        let cases: &[(Option<&str>, &str, bool)] = &[
            (None, "index.thtml", false),
            (None, "a.thtml", false),
            (None, "a.thtml", true),
            (None, "a_mobile.thtml", false),
            (None, "index.thtml", true),
            (None, "sub/b.thtml", false),
            (None, "sub/deep/c.thtml", false),
            (None, "sub/../a.thtml", false),
            (None, "sub/deep/../../index.thtml", false),
            (Some("sub/b.thtml"), "deep/c.thtml", false),
            (Some("sub/b.thtml"), "../a.thtml", false),
            (Some("sub/deep/c.thtml"), "../../a.thtml", true),
            // refusals
            (None, "../secret.thtml", false),
            (None, "sub/../../secret.thtml", false),
            (Some("sub/b.thtml"), "../../secret.thtml", false),
            (None, "/etc/passwd", false),
            (None, "missing.thtml", false),
        ];
        let abs_secret = t.0.join("secret.thtml").to_string_lossy().into_owned();
        let mut all: Vec<(Option<&str>, String, bool)> = cases.iter().map(|(b, t, m)| (*b, t.to_string(), *m)).collect();
        all.push((None, abs_secret, false));
        for (base, target, mobile) in all {
            let mb = base.map(|b| mem.resolve(None, b, false).unwrap());
            let fb = base.map(|b| fs.resolve(None, b, false).unwrap());
            let m = mem.resolve(mb.as_ref(), &target, mobile);
            let f = fs.resolve(fb.as_ref(), &target, mobile);
            match (&m, &f) {
                (Ok(m), Ok(f)) => {
                    assert_eq!(m.page, f.page, "page id of {target:?} from {base:?}");
                    // the loaded documents are the same text
                    let md = mem.load(m, oxiterm_proto::style::RenderMode::Gui).is_ok();
                    let fd = fs.load(f, oxiterm_proto::style::RenderMode::Gui).is_ok();
                    assert!(md && fd, "{target:?}");
                    assert_eq!(m.actual.file_name(), f.actual.file_name(), "variant of {target:?}");
                }
                (Err(me), Err(fe)) => {
                    assert_eq!(me, &PageError::Unavailable, "{target:?}");
                    assert_eq!(me, fe, "{target:?}");
                    assert_eq!(me.to_string(), fe.to_string());
                }
                _ => panic!("sources disagree on {target:?} from {base:?}: mem {m:?} / fs {f:?}"),
            }
        }
    }

    /// The one deliberate difference: hidden pages are not in the index, so the browser cannot
    /// navigate to them (the server's own page source still can).
    #[test]
    fn hidden_pages_are_not_navigable_locally() {
        let (_t, app) = fixture("hidden");
        let i = build_page_index(&app).unwrap();
        let mem = MemPageSource::with_dirs(i.pages, i.dirs);
        assert_eq!(mem.resolve(None, ".env.thtml", false), Err(PageError::Unavailable));
    }

    #[test]
    fn percent_decode_is_strict() {
        assert_eq!(percent_decode("a%20b%2Fc").as_deref(), Some("a b/c"));
        assert_eq!(percent_decode("a+b").as_deref(), Some("a+b"));
        assert_eq!(percent_decode("%zz"), None);
        assert_eq!(percent_decode("%4"), None);
        assert_eq!(percent_decode("%ff"), None, "not UTF-8");
    }

    #[test]
    fn assets_only_media_inside_the_base() {
        let (_t, app) = fixture("asset");
        let (mime, bytes) = read_asset(&app, "pic.png").unwrap();
        assert_eq!((mime, bytes.as_slice()), ("image/png", &b"\x89PNG-bytes"[..]));
        assert_eq!(read_asset(&app, "sub/clip.MP4").unwrap().0, "video/mp4");
        assert_eq!(read_asset(&app, "sub%2Fclip.MP4").unwrap().1, b"mp4-bytes");
        for bad in [
            "config.json",        // not a media type
            "notes.txt",
            "index.thtml",
            "../secret.png",
            "sub/../pic.png",     // `..` never reaches the filesystem
            "%2e%2e/secret.png",
            "sub/%2E%2E/pic.png",
            "/pic.png",
            "%2Fpic.png",
            "sub//clip.MP4",
            "pic.png/",
            "./pic.png",
            "",
            "missing.png",
            "sub",
            "pic.png%00.txt",
            "..%5Csecret.png",
            ".hidden.png",
        ] {
            assert!(read_asset(&app, bad).is_none(), "{bad:?} must be refused");
        }
    }

    #[cfg(unix)]
    #[test]
    fn assets_refuse_symlinks_and_oversize() {
        let (t, app) = fixture("assetsym");
        std::os::unix::fs::symlink(t.0.join("secret.png"), app.join("link.png")).unwrap();
        std::os::unix::fs::symlink(&t.0, app.join("linkdir")).unwrap();
        assert!(read_asset(&app, "link.png").is_none());
        assert!(read_asset(&app, "linkdir/secret.png").is_none());
        let f = std::fs::File::create(app.join("huge.png")).unwrap();
        f.set_len(MAX_ASSET_BYTES + 1).unwrap();
        assert!(read_asset(&app, "huge.png").is_none());
    }

    fn guest() -> UserIdentity {
        UserIdentity::guest()
    }

    fn ctx(actions: &[&str], bound: &[&str]) -> ProxyCtx {
        ProxyCtx::new(mint_session().0, actions.iter().map(|s| s.to_string()), bound.iter().map(|s| s.to_string()))
    }

    /// Accepts one connection, answers `{"c":"7"}`, returns the raw request.
    fn mock_app_server() -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/events", listener.local_addr().unwrap().port());
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let mut n = 0;
            loop {
                n += s.read(&mut buf[n..]).unwrap();
                let text = String::from_utf8_lossy(&buf[..n]).into_owned();
                if let Some(split) = text.find("\r\n\r\n") {
                    let len: usize = text.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                    if n >= split + 4 + len {
                        break;
                    }
                }
            }
            let body = r#"{"c":"Clicked 1 times","open_url":"https://example.com/x"}"#;
            s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes()).unwrap();
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (url, h)
    }

    #[test]
    fn proxy_is_404_without_an_app_server_and_validates_the_body() {
        let c = ctx(&["x"], &[]);
        {
            let _env = EnvGuard::lock_and_set(&[("OXITERM_APP_SERVER", None)]);
            assert_eq!(proxy_event_ctx(br#"{"action":"x","state":{},"session_id":1}"#, &guest(), &c).0, 404);
        }
        let _env = EnvGuard::lock_and_set(&[("OXITERM_APP_SERVER", Some("http://127.0.0.1:1/events")), ("OXITERM_LOCAL_PROXY_TIMEOUT_MS", Some("2000"))]);
        assert_eq!(proxy_event_ctx(b"not json", &guest(), &c).0, 400);
        assert_eq!(proxy_event_ctx(br#"{"state":{}}"#, &guest(), &c).0, 400);
        assert_eq!(proxy_event_ctx(br#"{"action":"","state":{},"session_id":1}"#, &guest(), &c).0, 400);
        assert_eq!(proxy_event_ctx(&vec![b' '; MAX_EVENT_BODY + 1], &guest(), &c).0, 413);
        // an action no page declares is refused before anything is sent
        assert_eq!(proxy_event_ctx(br#"{"action":"admin:x","state":{},"session_id":1}"#, &guest(), &c).0, 403);
        // an unreachable App Server is a generic 502, not a hang
        assert_eq!(proxy_event_ctx(br#"{"action":"x","state":{},"session_id":1}"#, &guest(), &c), (502, b"Bad Gateway".to_vec()));
    }

    #[test]
    fn proxy_forwards_with_the_bearer_token_server_set_identity_and_filtered_state() {
        let (url, server) = mock_app_server();
        let _env = EnvGuard::lock_and_set(&[("OXITERM_APP_SERVER", Some(&url)), ("OXITERM_APP_TOKEN", Some("sekret_token"))]);
        let c = ctx(&["inc:c"], &["c", "_username"]);
        // The page lies about who it is, about its session and about server-held state.
        let forged = br#"{"action":"inc:c","state":{"c":"3","role":"admin","is_authenticated":"true","_username":"root","_auth_method":"SshKey","_app_token":"x"},"session_id":777,"username":"admin","auth_method":"SshKey","app_token":"perm"}"#;
        let (status, body) = proxy_event_ctx(forged, &guest(), &c);
        assert_eq!(status, 200);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), serde_json::json!({"c":"Clicked 1 times","open_url":"https://example.com/x"}));
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /events"), "{req}");
        assert!(req.contains("Bearer sekret_token"), "the token is added by the server: {req}");
        let sent: serde_json::Value = serde_json::from_str(&req[req.find("\r\n\r\n").unwrap() + 4..]).unwrap();
        assert_eq!(sent["action"], "inc:c");
        assert_eq!(sent["state"], serde_json::json!({"c": "3", "_username": "guest"}), "unbound and `_` keys are dropped, bound reserved ones come from the identity");
        assert_eq!(sent["username"], "guest");
        assert_eq!(sent["auth_method"], "Guest");
        assert_eq!(sent["session_id"], c.session_id as u64);
        assert!(is_local_session_id(c.session_id) && c.session_id != 0);
        assert_eq!(sent["app_token"], "perm", "the permanent app token is the page's own and passes through");
    }

    #[test]
    fn proxy_refuses_beyond_the_in_flight_limit() {
        // Take the env lock FIRST: the other proxy tests hold it while they use slots, so grabbing
        // the slots before waiting for it would starve them (and flake).
        let _env = EnvGuard::lock_and_set(&[("OXITERM_APP_SERVER", Some("http://127.0.0.1:1/events"))]);
        // Hold every slot, as 16 slow requests would.
        let slots: Vec<_> = (0..MAX_PROXY_IN_FLIGHT).map(|_| InFlight::acquire().expect("free slot")).collect();
        assert_eq!(proxy_event_ctx(br#"{"action":"x","state":{},"session_id":1}"#, &guest(), &ctx(&["x"], &[])).0, 503);
        drop(slots);
        assert!(InFlight::acquire().is_some());
    }

    #[test]
    fn session_tokens_are_signed_and_local() {
        let (id, tok) = mint_session();
        assert!(is_local_session_id(id) && id >= LOCAL_ID_BASE);
        assert_eq!(verify_session(&tok), Some(id));
        let (id2, tok2) = mint_session();
        assert_ne!(id, id2);
        // forged ids, swapped tags, garbage, the real session namespace
        let tag = tok.split_once('.').unwrap().1;
        for bad in [format!("{id2}.{tag}"), format!("0.{tag}"), format!("5.{tag}"), format!("{id}."), format!("{id}.{}", "0".repeat(16)), "x".to_string(), String::new(), format!("{id}.{tag}0"), format!("-1.{tag}")] {
            assert_eq!(verify_session(&bad), None, "{bad:?}");
        }
        assert_eq!(verify_session(&tok2), Some(id2));
        assert!(!is_local_session_id(0) && !is_local_session_id(LOCAL_ID_BASE - 1));
    }

    #[test]
    fn local_index_collects_actions_and_bound_keys_and_is_cached() {
        let t = Tmp::new("actions");
        std::fs::write(
            t.0.join("index.thtml"),
            r#"<screen><text bind-state="count">0</text><input bind-value="name"/><box bind-show="open=yes"><button event-htmx="inc:count">+</button><button event-htmx="login">go</button></box><box draggable="true" drag-state-x="px" event-drag-end="dropped"/></screen>"#,
        )
        .unwrap();
        let i = LocalIndex::build(&t.0).unwrap();
        for a in ["inc:count", "login", "dropped", "app_token_sync"] {
            assert!(i.actions.contains(a), "{a} in {:?}", i.actions);
        }
        assert!(!i.actions.contains("admin:delete") && !i.actions.contains("inc:other"));
        for k in ["count", "name", "open", "px"] {
            assert!(i.bound_keys.contains(k), "{k} in {:?}", i.bound_keys);
        }
        assert!(!i.bound_keys.contains("open=yes"));
        // cached: a page added now is not seen until the TTL ends
        let a = local_index(&t.0).unwrap();
        std::fs::write(t.0.join("new.thtml"), "<screen/>").unwrap();
        let b = local_index(&t.0).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(b.json("new.thtml", false).is_none());
    }

    #[test]
    fn index_json_lists_directories_for_dotdot_parity() {
        let (_t, app) = fixture("dirs");
        std::fs::create_dir_all(app.join("emptydir")).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&index_json(&app, "index.thtml", false).unwrap()).unwrap();
        let dirs: Vec<&str> = v["dirs"].as_array().unwrap().iter().map(|d| d.as_str().unwrap()).collect();
        assert_eq!(dirs, ["emptydir", "sub", "sub/deep"]);
    }

    #[cfg(unix)]
    #[test]
    fn hard_linked_files_are_not_indexed_or_served() {
        let (t, app) = fixture("hardlink");
        std::fs::hard_link(t.0.join("secret.thtml"), app.join("h.thtml")).unwrap();
        std::fs::hard_link(t.0.join("secret.png"), app.join("h.png")).unwrap();
        assert!(!build_page_index(&app).unwrap().pages.contains_key("h.thtml"));
        assert!(read_asset(&app, "h.png").is_none());
        assert!(resolve_asset(&app, "h.png").is_none());
        // the server's own page source agrees
        let fs = FsPageSource::new(app.canonicalize().unwrap());
        assert_eq!(fs.resolve(None, "h.thtml", false), Err(PageError::Unavailable));
        assert!(fs.resolve(None, "index.thtml", false).is_ok());
        // explicit opt-out for `cp -al` deployments
        let _env = EnvGuard::lock_and_set(&[("OXITERM_ALLOW_HARDLINKS", Some("1"))]);
        assert!(build_page_index(&app).unwrap().pages.contains_key("h.thtml"));
    }

    #[test]
    fn walk_stops_at_the_entry_cap() {
        let t = Tmp::new("entries");
        std::fs::write(t.0.join("a.thtml"), "x").unwrap();
        for d in 0..4 {
            let dd = t.0.join(format!("d{d}"));
            std::fs::create_dir_all(&dd).unwrap();
            for f in 0..(MAX_ENTRIES / 2) {
                std::fs::write(dd.join(format!("f{f}.dat")), "").unwrap();
            }
        }
        let started = Instant::now();
        let idx = build_page_index(&t.0).unwrap();
        assert!(idx.truncated, "20 000 entries exceed the 10 000 cap");
        assert!(idx.pages.contains_key("a.thtml"));
        assert!(started.elapsed() < Duration::from_secs(5), "{:?}", started.elapsed());
    }

    #[test]
    fn host_allowlist_is_optional_and_exact() {
        let mut m = hyper::HeaderMap::new();
        m.insert("host", "Good.Test:8080".parse().unwrap());
        {
            let _env = EnvGuard::lock_and_set(&[("OXITERM_ALLOWED_HOSTS", None)]);
            assert!(host_allowed(&m));
        }
        let _env = EnvGuard::lock_and_set(&[("OXITERM_ALLOWED_HOSTS", Some("good.test:8080, other.test"))]);
        assert!(host_allowed(&m));
        m.insert("host", "evil.test:8080".parse().unwrap());
        assert!(!host_allowed(&m));
        m.insert("host", "good.test:8080.evil".parse().unwrap());
        assert!(!host_allowed(&m));
        m.remove("host");
        assert!(!host_allowed(&m));
    }

    #[test]
    fn csrf_gate_needs_json_and_same_origin() {
        let _env = EnvGuard::lock_and_set(&[("OXITERM_ALLOWED_HOSTS", None)]);
        fn h(pairs: &[(&str, &str)]) -> hyper::HeaderMap {
            let mut m = hyper::HeaderMap::new();
            for (k, v) in pairs {
                m.insert(hyper::header::HeaderName::from_bytes(k.as_bytes()).unwrap(), v.parse().unwrap());
            }
            m
        }
        let json = ("content-type", "application/json");
        assert_eq!(check_same_origin_json(&h(&[json, ("host", "a.test:80")])), Ok(()));
        assert_eq!(check_same_origin_json(&h(&[("content-type", "Application/JSON; charset=utf-8")])), Ok(()));
        assert_eq!(check_same_origin_json(&h(&[json, ("host", "a.test:80"), ("origin", "http://a.test:80"), ("sec-fetch-site", "same-origin")])), Ok(()));
        assert_eq!(check_same_origin_json(&h(&[json, ("sec-fetch-site", "none")])), Ok(()));
        for (headers, code) in [
            (h(&[]), 415),
            (h(&[("content-type", "text/plain")]), 415),
            (h(&[("content-type", "application/x-www-form-urlencoded")]), 415),
            (h(&[("content-type", "application/jsonx")]), 415),
            (h(&[json, ("sec-fetch-site", "cross-site")]), 403),
            (h(&[json, ("sec-fetch-site", "same-site")]), 403),
            (h(&[json, ("host", "a.test"), ("origin", "http://b.test")]), 403),
            (h(&[json, ("host", "a.test"), ("origin", "http://a.test.evil")]), 403),
            (h(&[json, ("origin", "http://a.test")]), 403),
        ] {
            assert_eq!(check_same_origin_json(&headers), Err(code), "{headers:?}");
        }
    }

    #[test]
    fn start_page_and_base() {
        assert_eq!(start_page(Path::new("examples/index.thtml")).as_deref(), Some("index.thtml"));
        assert_eq!(app_base(Path::new("examples/index.thtml")), PathBuf::from("examples"));
        assert_eq!(app_base(Path::new("index.thtml")), PathBuf::from("."));
        assert_eq!(start_page(Path::new("/")), None);
    }
}
