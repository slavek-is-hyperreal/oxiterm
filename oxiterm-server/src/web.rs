//! WebSocket and HTTP Web interface implementation for OxiTerm.
//!
//! Provides WebSocket framing handlers (`WsFrameSink`), static assets routing,
//! input event translators (keyboard, mouse, resize), and path traversal checks.

/// WebSocket wire encoding of a [`oxiterm_renderer::ClientNotice`]: the ONLY place the notice
/// tags and texts are defined. `0x33` + URL, `0x34` + fixed generic text, `0x35` + token.
/// (Ungated: the single place for the notice wire bytes.)
pub fn encode_notice(notice: &oxiterm_renderer::ClientNotice) -> Vec<u8> {
    use oxiterm_renderer::ClientNotice as N;
    let (tag, text): (u8, &str) = match notice {
        N::NavError => (0x34, "Nie znaleziono strony"),
        N::NavThrottle => (0x34, "Zwolnij ;)"),
        N::OpenUrl(url) => (0x33, url),
        N::AppToken(token) => (0x35, token),
    };
    let mut frame = Vec::with_capacity(1 + text.len());
    frame.push(tag);
    frame.extend_from_slice(text.as_bytes());
    frame
}

#[cfg(feature = "web")]
pub mod web_impl {
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    use tokio_tungstenite::tungstenite::Message;
    use futures::{StreamExt, SinkExt};
    use tracing::{info, warn, error};
    use crate::session::{SessionRegistry, PtyDimensions, EventLoop, ClientSession};
    #[cfg(test)]
    #[allow(unused_imports)]
    use oxiterm_proto::input::{MouseAction, MouseButton};
    use oxiterm_proto::input::{InputEvent, PointerInput};
    use oxiterm_renderer::FrameSink;
    use oxiterm_renderer::CellBuffer;
    use oxiterm_renderer::DiffEngine;
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Request, Response, StatusCode};
    use hyper::body::{Bytes, Incoming};
    use http_body_util::Full;

    // Load assets at compile time.
    const HTML_ASSET: &[u8] = include_bytes!("../assets/index.html");
    const JS_ASSET: &[u8] = include_bytes!("../assets/pkg/oxiterm_web.js");
    const WASM_ASSET: &[u8] = include_bytes!("../assets/pkg/oxiterm_web_bg.wasm");
    // The local-mode bundle (the same crate built with `--features local`, `--out-name
    // oxiterm_local`): served only when local mode is on, loaded by the page only for `?local=1`.
    const LOCAL_JS_ASSET: &[u8] = include_bytes!("../assets/pkg/oxiterm_local.js");
    const LOCAL_WASM_ASSET: &[u8] = include_bytes!("../assets/pkg/oxiterm_local_bg.wasm");
    /// Client-side coordinate round-trip tests. Compiled into the binary but injected into
    /// the served page ONLY in web-test mode (`OXITERM_WEB_TEST`), so real users never load it.
    const WEB_TEST_JS: &str = include_str!("../assets/web_coord_tests.js");
    /// Marker inside `index.html` replaced with [`WEB_TEST_JS`] in web-test mode.
    const WEB_TEST_MARKER: &str = "/*__OXITERM_WEB_TEST_HOOK__*/";

    /// Whether the web client should ship its in-browser test suite (dev/CI only).
    fn web_test_enabled() -> bool {
        std::env::var("OXITERM_WEB_TEST")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    /// Returns the HTML to serve at `/`, injecting the coordinate test suite at the marker
    /// when in web-test mode; otherwise the marker stays an inert comment.
    fn index_html() -> std::borrow::Cow<'static, [u8]> {
        if web_test_enabled() {
            let injected = String::from_utf8_lossy(HTML_ASSET).replace(WEB_TEST_MARKER, WEB_TEST_JS);
            std::borrow::Cow::Owned(injected.into_bytes())
        } else {
            std::borrow::Cow::Borrowed(HTML_ASSET)
        }
    }



    /// Simple hash path utility.
    /// Decodes the fractional pointer message `0x03` (OxiUI Phase 6.5):
    /// `[0x03, x:f32le, y:f32le, button:u8, action:u8, flags:u8]` (12 bytes). `x`/`y` are
    /// 0-based canvas cells (`px / on-screen cell size`), fractional. Button, action and flag
    /// bytes mean what they mean in `0x02`. Anything shorter than 12 bytes or with a
    /// non-finite coordinate is dropped (`None`); trailing bytes are ignored.
    pub fn decode_pointer(bytes: &[u8]) -> Option<PointerInput> {
        oxiterm_proto::input::wire::decode_pointer(bytes)
    }

    pub fn hash_path(path: &str) -> u32 {
        let mut hash: u32 = 5381;
        for c in path.chars() {
            hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
        }
        hash
    }

    /// Core writer loop: forwards binary frames from `frame_rx` to `sink`, sending
    /// a WebSocket Ping every `ping_interval` when no frames are queued.
    ///
    /// Generic over any `S: Sink<Message, Error=E> + Unpin` so that tests can
    /// substitute a channel-backed sink without a real WebSocket connection.
    /// The production task passes the real `ws_write` half.
    ///
    /// Returns when the channel closes or a send error occurs.
    pub(crate) async fn ws_writer_loop<S, E>(
        mut sink: S,
        mut frame_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        ping_interval: std::time::Duration,
        label: &str,
    ) where
        S: futures::Sink<Message, Error = E> + Unpin,
        E: std::fmt::Debug,
    {
        loop {
            match tokio::time::timeout(ping_interval, frame_rx.recv()).await {
                Ok(Some(bytes)) => {
                    if let Err(e) = sink.send(Message::Binary(bytes)).await {
                        warn!("WS send error {}: {:?}", label, e);
                        break;
                    }
                }
                Ok(None) => break, // channel closed
                Err(_elapsed) => {
                    // No frame in ping_interval: send Ping to keep the connection alive
                    // through Cloudflare and other proxies that kill idle WS at ~100 s.
                    if let Err(e) = sink.send(Message::Ping(vec![])).await {
                        warn!("WS ping error {}: {:?}", label, e);
                        break;
                    }
                }
            }
        }
    }

    /// Server -> client tag of a display-list frame: `0x36` + zlib(wire::encode(list)).
    /// (0x30 is taken by the page announce, 0x31..0x35 by other notices.)
    pub const WS_TAG_DISPLAY_LIST: u8 = 0x36;
    /// Server -> client tag of an ARIA-mirror frame: `0x37` + UTF-8 JSON (OxiUI phase 3c).
    /// Sent only to display-list sessions, right after the display list of the same frame.
    pub const WS_TAG_A11Y: u8 = 0x37;

    /// WebSocket Frame Sink for delivering binary cell buffers to browser clients.
    pub struct WsFrameSink {
        session: Arc<ClientSession>,
        sent_assets: std::collections::HashSet<String>,
        sent_coordinates: std::collections::HashMap<u32, crate::session::MediaRenderInfo>,
        source_path: Option<std::path::PathBuf>,
        media_base_url: Option<std::path::PathBuf>,
        sent_page: Option<String>,
        last_seen_epoch: usize,
        last_warned_epoch: usize,
        /// Wire bytes (uncompressed) of the last display list sent; cleared on epoch change.
        last_list: Option<Vec<u8>>,
        /// Last ARIA-mirror JSON sent (0x37); reset together with `last_list`.
        last_a11y: Option<Vec<u8>>,
    }

    impl WsFrameSink {
        /// Creates a new `WsFrameSink`.
        pub fn new(session: Arc<ClientSession>, source_path: Option<std::path::PathBuf>) -> Self {
            let media_base_url = crate::config::OxiTermConfig::from_env().ok().and_then(|c| c.media_base_url).map(std::path::PathBuf::from);
            Self {
                session,
                sent_assets: std::collections::HashSet::new(),
                sent_coordinates: std::collections::HashMap::new(),
                source_path,
                media_base_url,
                sent_page: None,
                last_seen_epoch: 0,
                last_warned_epoch: 0,
                last_list: None,
                last_a11y: None,
            }
        }

        fn send_bytes(&mut self, bytes: Vec<u8>) -> Result<(), ()> {
            if let Some((_, ref tx)) = *self.session.web_frame_tx.read() {
                match tx.try_send(bytes) {
                    Ok(_) => Ok(()),
                    Err(_) => Err(()),
                }
            } else {
                Err(())
            }
        }

        /// Per-frame bookkeeping shared by cell-diff and display-list frames: epoch reset,
        /// page announce (0x30), media assets (0x20) and media coordinates (0x21).
        /// Returns `(epoch_changed, coords_changed)`.
        fn sync_preamble(&mut self) -> (bool, bool) {
            let current_epoch = self.session.connection_epoch.load(std::sync::atomic::Ordering::SeqCst);
            let epoch_changed = current_epoch != self.last_seen_epoch;
            if epoch_changed {
                self.sent_assets.clear();
                self.sent_coordinates.clear();
                self.sent_page = None;
                self.last_list = None;
                self.last_a11y = None;
                self.last_seen_epoch = current_epoch;
            }

            let media_base_url = self.media_base_url.clone();
            // R5: resolve base_dir from the current navigated page's directory, not the
            // initial source_path, so media assets in page subdirs are reachable.
            let base_dir = {
                let cp = self.session.current_page.read().clone();
                let app_base = self.session.app_base_dir.read().clone();
                cp.and_then(|p| app_base.map(|base| base.join(&p)))
                    .and_then(|pp| pp.parent().map(|d| d.to_path_buf()))
                    .or_else(|| self.source_path.clone().and_then(|p| p.parent().map(|d| d.to_path_buf())))
                    .or(media_base_url)
            };

            let current_page = self.session.current_page.read().clone();
            if current_page != self.sent_page {
                if let Some(ref page) = current_page {
                    let mut msg = vec![0x30];
                    msg.extend_from_slice(page.as_bytes());
                    if self.send_bytes(msg).is_ok() {
                        self.sent_page = current_page;
                    }
                } else {
                    self.sent_page = None;
                }
            }

            let active_media = self.session.active_media.read().clone();
            
            let mut coords_changed = false;
            if active_media.len() != self.sent_coordinates.len() {
                coords_changed = true;
            } else {
                for media in &active_media {
                    let h = hash_path(&media.path);
                    match self.sent_coordinates.get(&h) {
                        Some(prev) => {
                            if prev != media {
                                coords_changed = true;
                                break;
                            }
                        }
                        None => {
                            coords_changed = true;
                            break;
                        }
                    }
                }
            }

            if coords_changed {
                let _ = self.send_bytes(vec![0x21]);
            }

            let app_base_dir = self.session.app_base_dir.read().clone();
            for media in &active_media {
                let is_safe = if let (Some(ref base), Some(ref app_base)) = (&base_dir, &app_base_dir) {
                    let full_path = base.join(&media.path);
                    crate::pathsafe::is_within_base(app_base, &full_path)
                } else {
                    false
                };

                if !is_safe {
                    // Distinguish traversal attempt from a simply missing asset.
                    if let Some(ref base) = base_dir {
                        let full_path = base.join(&media.path);
                        if full_path.exists() {
                            warn!("media outside app_base_dir (path traversal attempt): {:?}", full_path);
                        } else {
                            tracing::debug!("media not found (missing asset): {:?}", full_path);
                        }
                    } else {
                        tracing::debug!("media not found (no base dir): {:?}", media.path);
                    }
                    continue;
                }

                if !self.sent_assets.contains(&media.path) {
                    if let Some(ref base) = base_dir {
                        let full_path = base.join(&media.path);
                        if let Ok(file_bytes) = std::fs::read(&full_path) {
                            let mut msg = vec![0x20];
                            msg.extend_from_slice(&hash_path(&media.path).to_le_bytes());
                            msg.extend_from_slice(&file_bytes);
                            if self.send_bytes(msg).is_ok() {
                                self.sent_assets.insert(media.path.clone());
                            }
                        }
                    }
                }
            }

            if coords_changed {
                self.sent_coordinates.clear();

                for media in &active_media {
                    let is_safe = if let Some(ref base) = base_dir {
                        let full_path = base.join(&media.path);
                        crate::pathsafe::is_within_base(base, &full_path)
                    } else {
                        false
                    };

                    if !is_safe {
                        continue;
                    }

                    let h = hash_path(&media.path);
                    let mut msg = vec![0x21];
                    msg.extend_from_slice(&h.to_le_bytes());
                    msg.extend_from_slice(&media.x.to_le_bytes());
                    msg.extend_from_slice(&media.y.to_le_bytes());
                    msg.extend_from_slice(&media.width.to_le_bytes());
                    msg.extend_from_slice(&media.height.to_le_bytes());
                    let _ = self.send_bytes(msg);
                    
                    self.sent_coordinates.insert(h, media.clone());
                }
            }
            (epoch_changed, coords_changed)
        }
    }

    impl FrameSink for WsFrameSink {
        fn send_notice(&mut self, notice: oxiterm_renderer::ClientNotice) -> anyhow::Result<bool> {
            // Notices are web-only. No sync_preamble: a notice must not emit 0x30/0x20/0x21.
            // A full or missing channel is ignored, exactly as the loop did before.
            if !self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(false);
            }
            Ok(self.send_bytes(super::encode_notice(&notice)).is_ok())
        }

        fn update_document(&mut self, _doc: &oxiterm_renderer::document::THTMLDocument) -> anyhow::Result<()> {
            // R3: doc swap -> next frame is full, 0x21 emitted
            let _ = self.send_bytes(vec![0x21]);
            self.sent_coordinates.clear();
            Ok(())
        }

        fn send_display_list(&mut self, list: &oxiterm_renderer::render::display_list::DisplayList) -> anyhow::Result<bool> {
            // Capability is per connection: if it was reset (new connection/takeover) while a GUI
            // frame was in flight, drop it. Must precede sync_preamble so the epoch change is
            // still seen by the next send_frame (forced full cell frame).
            if !self.session.display_list_client.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(false);
            }
            let (_epoch_changed, coords_changed) = self.sync_preamble();
            // Wire v3 only for a client that announced it (caps bit 1) and only when the list has
            // a shaped run; otherwise the bytes are exactly the v2 ones.
            let max_version = if self.session.display_list_v3.load(std::sync::atomic::Ordering::SeqCst) {
                oxiterm_renderer::render::wire::WIRE_VERSION_3
            } else {
                oxiterm_renderer::render::wire::WIRE_VERSION
            };
            let encoded = oxiterm_renderer::render::wire::encode_for(list, max_version);
            // Unchanged frame (also resets on epoch change via sync_preamble): nothing to send.
            if self.last_list.as_deref() == Some(encoded.as_slice()) {
                return Ok(coords_changed);
            }
            let mut enc = flate2::write::ZlibEncoder::new(vec![WS_TAG_DISPLAY_LIST], flate2::Compression::fast());
            std::io::Write::write_all(&mut enc, &encoded)?;
            let msg = enc.finish()?;
            if self.send_bytes(msg).is_err() {
                let current_epoch = self.session.connection_epoch.load(std::sync::atomic::Ordering::SeqCst);
                if self.last_warned_epoch < current_epoch {
                    warn!("WsFrameSink::send_display_list: send failed/no channel for epoch {}", current_epoch);
                    self.last_warned_epoch = current_epoch;
                }
                return Ok(false);
            }
            self.last_list = Some(encoded);
            Ok(true)
        }

        fn send_a11y(&mut self, json: &[u8]) -> anyhow::Result<bool> {
            if !self.session.display_list_client.load(std::sync::atomic::Ordering::SeqCst) {
                return Ok(false);
            }
            // Epoch reset (reattach) is handled by send_display_list's preamble, which runs
            // first for the same frame; a stale epoch here only costs a redundant resend.
            if self.last_a11y.as_deref() == Some(json) {
                return Ok(false);
            }
            let mut msg = Vec::with_capacity(json.len() + 1);
            msg.push(WS_TAG_A11Y);
            msg.extend_from_slice(json);
            if self.send_bytes(msg).is_err() {
                return Ok(false);
            }
            self.last_a11y = Some(json.to_vec());
            Ok(true)
        }

        fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
            let (epoch_changed, coords_changed) = self.sync_preamble();
            // Switching back from display lists: the next list (if toggled again) must be sent.
            self.last_list = None;
            self.last_a11y = None;

            tracing::trace!("WsFrameSink::send_frame: front dims {}x{}, back dims {}x{}", front.width, front.height, back.width, back.height);
            
            // K4: full frame covers cols×rows cells
            let commands = if epoch_changed {
                let mut dummy_prev = CellBuffer::new(back.width, back.height);
                dummy_prev.force_dirty();
                DiffEngine::diff(&dummy_prev, back)
            } else {
                DiffEngine::diff(front, back)
            };
            
            tracing::trace!("WsFrameSink::send_frame: diff generated {} commands", commands.len());
            
            if commands.is_empty() && !coords_changed {
                return Ok(false);
            }
            if !back.graphics.is_empty() {
                warn!("WsFrameSink::send_frame: graphics data ignored (not supported over WS)");
            }
            
            if !commands.is_empty() {
                let bytes = DiffEngine::encode_binary(&commands);
                tracing::trace!("WsFrameSink::send_frame: sending {} bytes on WebSocket", bytes.len());
                if let Err(_) = self.send_bytes(bytes) {
                    let current_epoch = self.session.connection_epoch.load(std::sync::atomic::Ordering::SeqCst);
                    if self.last_warned_epoch < current_epoch {
                        warn!("WsFrameSink::send_frame: send failed/no channel for epoch {}", current_epoch);
                        self.last_warned_epoch = current_epoch;
                    }
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }

    impl Drop for WsFrameSink {
        fn drop(&mut self) {
            let reason = self.session.death_reason.load(std::sync::atomic::Ordering::SeqCst);
            let _ = self.send_bytes(vec![0xFF, reason]);
        }
    }

    /// Starts the HTTP / WS serving thread loop.
    pub fn start_web_server(
        host: String,
        port: u16,
        registry: Arc<SessionRegistry>,
        rate_limiter: Arc<crate::ratelimit::RateLimiter>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
    ) {
        let reaper_registry = registry.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                let mut to_remove = Vec::new();
                {
                    let sessions = reaper_registry.sessions.read();
                    for (&id, session) in sessions.iter() {
                        if session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
                            let active_conns = session.active_connections.load(std::sync::atomic::Ordering::SeqCst);
                            let idle_time = session.last_activity.read().elapsed();
                            if active_conns == 0 && idle_time > std::time::Duration::from_secs(600) {
                                info!("Reaping disconnected web session {} (idle for {}s)", id, idle_time.as_secs());
                                // Belt-and-suspenders: send idle-death frame if any live sender is present.
                                // In the common case (conn==0) web_frame_tx is None; this is a no-op.
                                if let Some((_, ref tx)) = *session.web_frame_tx.read() {
                                    let _ = tx.try_send(vec![0xFF, 1u8]);
                                }
                                to_remove.push(id);
                            }
                        }
                    }
                }
                for id in to_remove {
                    reaper_registry.remove_session(id);
                }
            }
        });

        tokio::spawn(async move {
            let addr_str = format!("{}:{}", host, port);
            let addr = match addr_str.parse::<std::net::SocketAddr>() {
                Ok(a) => a,
                Err(e) => {
                    error!("Failed to parse server address {}: {:?}", addr_str, e);
                    return;
                }
            };
            let listener = match TcpListener::bind(&addr).await {
                Ok(l) => {
                    info!("Web/WS server listening on http://{}", addr);
                    l
                }
                Err(e) => {
                    error!("Failed to bind web server to {}: {:?}", addr, e);
                    return;
                }
            };

            loop {
                let (stream, peer_addr) = match listener.accept().await {
                    Ok(res) => res,
                    Err(e) => {
                        warn!("Web accept error: {:?}", e);
                        continue;
                    }
                };

                let limit_res = rate_limiter.check_and_record(peer_addr.ip());
                match limit_res {
                    crate::ratelimit::RateResult::Deny => {
                        warn!("Web rate limit DENY for client: {}", peer_addr);
                        continue;
                    }
                    crate::ratelimit::RateResult::Throttle(delay) => {
                        warn!("Web rate limit THROTTLE ({:?}) for client: {}", delay, peer_addr);
                        tokio::time::sleep(delay).await;
                    }
                    crate::ratelimit::RateResult::Allow => {}
                }

                let registry = registry.clone();
                let initial_doc = initial_doc.clone();
                let source_path = source_path.clone();

                tokio::spawn(async move {
                    let io = hyper_util::rt::TokioIo::new(stream);
                    let service = service_fn(move |req| {
                        let registry = registry.clone();
                        let initial_doc = initial_doc.clone();
                        let source_path = source_path.clone();
                        async move {
                            serve_request(req, registry, initial_doc, source_path, peer_addr).await
                        }
                    });

                    if let Err(err) = http1::Builder::new()
                        .serve_connection(io, service)
                        .with_upgrades()
                        .await
                    {
                        warn!("Error serving connection: {:?}", err);
                    }
                });
            }
        });
    }

    /// `<id>.ttf` or `LICENSE-<id>.txt` of a bundled face -> (content type, embedded bytes).
    fn font_route(name: &str) -> Option<(&'static str, &'static [u8])> {
        use oxiterm_renderer::text::bundle::FontBundle;
        if let Some(id) = name.strip_suffix(".ttf") {
            FontBundle::font_bytes(id).map(|b| ("font/ttf", b))
        } else if let Some(id) = name.strip_prefix("LICENSE-").and_then(|n| n.strip_suffix(".txt")) {
            FontBundle::license_bytes(id).map(|b| ("text/plain; charset=utf-8", b))
        } else {
            None
        }
    }

    /// Who a web request is: a trusted-header user, or a guest (when guests are allowed).
    /// `None` = refuse with 401. Shared by `/ws` and the local-mode routes.
    fn resolve_identity(headers: &hyper::HeaderMap, peer_addr: std::net::SocketAddr) -> Option<crate::identity::UserIdentity> {
        let forwarded_user = headers.get("X-Forwarded-User").and_then(|h| h.to_str().ok()).map(|s| s.to_string());
        let trusted_proxy = std::env::var("OXITERM_TRUSTED_PROXY").ok();
        let allow_guest = match std::env::var("OXITERM_ALLOW_GUEST") {
            Ok(v) => v != "false" && v != "0",
            Err(_) => true, // default = true (unset) (F1)
        };
        match (forwarded_user, trusted_proxy) {
            (Some(user), Some(proxy)) => match crate::identity::UserIdentity::from_trusted_header(&user, peer_addr, &proxy) {
                Some(id) => Some(id),
                None => allow_guest.then(crate::identity::UserIdentity::guest),
            },
            (None, _) => allow_guest.then(crate::identity::UserIdentity::guest),
            (Some(user), None) => {
                warn!("X-Forwarded-User '{}' received but OXITERM_TRUSTED_PROXY not set; ignoring", user);
                allow_guest.then(crate::identity::UserIdentity::guest)
            }
        }
    }

    fn http_401() -> Response<Full<Bytes>> {
        Response::builder()
            .status(StatusCode::UNAUTHORIZED)
            .body(Full::new(Bytes::from("Unauthorized")))
            .unwrap()
    }

    fn payload_too_large() -> Response<Full<Bytes>> {
        Response::builder()
            .status(StatusCode::PAYLOAD_TOO_LARGE)
            .body(Full::new(Bytes::from("Payload Too Large")))
            .unwrap()
    }

    /// Plain response with optional gzip (same policy as the static routes) and extra headers.
    fn local_response(accepts_gzip: bool, status: StatusCode, mime: &str, body: Vec<u8>, extra: &[(&'static str, &'static str)]) -> Response<Full<Bytes>> {
        let mut out = body;
        let mut gz = false;
        if accepts_gzip && status == StatusCode::OK {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;
            let mut enc = GzEncoder::new(Vec::new(), Compression::default());
            if enc.write_all(&out).is_ok() {
                if let Ok(c) = enc.finish() {
                    out = c;
                    gz = true;
                }
            }
        }
        let mut res = Response::builder()
            .status(status)
            .header("Content-Type", mime)
            .header("Content-Length", out.len())
            .body(Full::new(Bytes::from(out)))
            .unwrap();
        if gz {
            res.headers_mut().insert("Content-Encoding", hyper::header::HeaderValue::from_static("gzip"));
        }
        for (k, v) in extra {
            res.headers_mut().insert(*k, hyper::header::HeaderValue::from_static(v));
        }
        res
    }

    /// Generic 403/404/... answer of the local routes.
    fn local_status(accepts_gzip: bool, status: StatusCode) -> Response<Full<Bytes>> {
        local_response(accepts_gzip, status, "text/plain", status.canonical_reason().unwrap_or("Error").as_bytes().to_vec(), &[])
    }

    /// The common front door of every `/oxiui/local/*` request: local mode on, `Host` allowed,
    /// identity (the `/ws` rules). `Err` is the response to send. Off means one uniform 404.
    fn local_gate(req_headers: &hyper::HeaderMap, accepts_gzip: bool, peer_addr: std::net::SocketAddr) -> Result<crate::identity::UserIdentity, Response<Full<Bytes>>> {
        if !crate::local_mode::enabled() {
            return Err(local_status(accepts_gzip, StatusCode::NOT_FOUND));
        }
        if !crate::local_mode::host_allowed(req_headers) {
            return Err(local_status(accepts_gzip, StatusCode::FORBIDDEN));
        }
        resolve_identity(req_headers, peer_addr).ok_or_else(http_401)
    }

    /// `/oxiui/local/*` except assets: `GET pages.json`, `POST events`. Off by default; with no
    /// start page (the server was not started with one) or on any refusal the answer is one
    /// generic 404, so nothing about the filesystem can be probed.
    async fn local_route(
        req: Request<Incoming>,
        source_path: Option<std::path::PathBuf>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<Response<Full<Bytes>>, hyper::Error> {
        use crate::local_mode as lm;
        let accepts_gzip = req.headers().get("Accept-Encoding").and_then(|v| v.to_str().ok()).map_or(false, |v| v.contains("gzip"));
        let identity = match local_gate(req.headers(), accepts_gzip, peer_addr) {
            Ok(i) => i,
            Err(r) => return Ok(r),
        };
        let not_found = || local_status(accepts_gzip, StatusCode::NOT_FOUND);
        let path = req.uri().path().to_string();
        let method = req.method().clone();
        let Some(source) = source_path else { return Ok(not_found()) };
        let base = lm::app_base(&source);

        if method == hyper::Method::POST && path == "/oxiui/local/events" {
            use http_body_util::BodyExt;
            if lm::app_server_url().is_none() {
                return Ok(not_found());
            }
            if let Err(code) = lm::check_same_origin_json(req.headers()) {
                return Ok(local_status(accepts_gzip, StatusCode::from_u16(code).unwrap_or(StatusCode::FORBIDDEN)));
            }
            // The session token was minted with pages.json for this page load; a missing or
            // forged one is refused (the custom header also forces a CORS preflight).
            let Some(session_id) = req.headers().get(lm::SESSION_HEADER).and_then(|v| v.to_str().ok()).and_then(lm::verify_session) else {
                return Ok(local_status(accepts_gzip, StatusCode::FORBIDDEN));
            };
            let limited = http_body_util::Limited::new(req.into_body(), lm::MAX_EVENT_BODY);
            let bytes = match limited.collect().await {
                Ok(c) => c.to_bytes(),
                Err(_) => return Ok(payload_too_large()),
            };
            let (status, body) = tokio::task::spawn_blocking(move || match lm::local_index(&base) {
                Some(idx) => {
                    let ctx = lm::ProxyCtx { session_id, actions: idx.actions.clone(), bound_keys: idx.bound_keys.clone() };
                    lm::proxy_event_ctx(&bytes, &identity, &ctx)
                }
                None => (404, b"Not Found".to_vec()),
            })
            .await
            .unwrap_or((502, b"Bad Gateway".to_vec()));
            let status = StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_GATEWAY);
            let mime = if status == StatusCode::OK { "application/json" } else { "text/plain" };
            return Ok(local_response(accepts_gzip, status, mime, body, &[("Cache-Control", "no-store")]));
        }
        if method == hyper::Method::GET && path == "/oxiui/local/pages.json" {
            let Some(start) = lm::start_page(&source) else { return Ok(not_found()) };
            let actions = lm::app_server_url().is_some();
            let json = tokio::task::spawn_blocking(move || lm::local_index(&base).and_then(|i| i.json(&start, actions))).await.ok().flatten();
            return Ok(match json {
                Some(j) => {
                    let (_, token) = lm::mint_session();
                    let mut r = local_response(accepts_gzip, StatusCode::OK, "application/json", j, &[("Cache-Control", "no-store")]);
                    if let Ok(v) = hyper::header::HeaderValue::from_str(&token) {
                        r.headers_mut().insert("X-OxiTerm-Local-Session", v);
                    }
                    r
                }
                None => not_found(),
            });
        }
        Ok(not_found())
    }

    /// Body type of the server's responses: everything but a local asset is a `Full` body.
    type RespBody = http_body_util::combinators::BoxBody<Bytes, std::io::Error>;

    /// `GET /oxiui/local/asset/<path>`: a media file under the app base, streamed from disk in
    /// 64 KiB frames (a 16 MiB video no longer sits in memory per request). sandbox + nosniff:
    /// an SVG opened directly cannot run script in the page's origin.
    async fn local_asset(
        req: Request<Incoming>,
        source_path: Option<std::path::PathBuf>,
        peer_addr: std::net::SocketAddr,
    ) -> Response<RespBody> {
        use crate::local_mode as lm;
        use http_body_util::BodyExt;
        use tokio::io::AsyncReadExt;
        let boxed = |r: Response<Full<Bytes>>| r.map(|b| b.map_err(|e| match e {}).boxed());
        let accepts_gzip = false;
        if let Err(r) = local_gate(req.headers(), accepts_gzip, peer_addr) {
            return boxed(r);
        }
        let not_found = || boxed(local_status(accepts_gzip, StatusCode::NOT_FOUND));
        if req.method() != hyper::Method::GET {
            return not_found();
        }
        let Some(source) = source_path else { return not_found() };
        let base = lm::app_base(&source);
        let rest = req.uri().path().strip_prefix("/oxiui/local/asset/").unwrap_or("").to_string();
        let Some(asset) = tokio::task::spawn_blocking(move || lm::resolve_asset(&base, &rest)).await.ok().flatten() else { return not_found() };
        let Ok(file) = tokio::fs::File::open(&asset.path).await else { return not_found() };
        // Re-check the file actually opened (the path may have been swapped since resolve).
        let Ok(meta) = file.metadata().await else { return not_found() };
        if !lm::check_opened_asset(&meta) {
            return not_found();
        }
        let len = meta.len();
        let stream = futures::stream::unfold((file, len), |(mut f, rem)| async move {
            if rem == 0 {
                return None;
            }
            let mut buf = vec![0u8; rem.min(64 * 1024) as usize];
            match f.read(&mut buf).await {
                Ok(0) => None,
                Ok(n) => {
                    buf.truncate(n);
                    Some((Ok(hyper::body::Frame::data(Bytes::from(buf))), (f, rem - n as u64)))
                }
                Err(e) => Some((Err(e), (f, 0))),
            }
        });
        Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", asset.mime)
            .header("Content-Length", len)
            .header("Cache-Control", "no-cache")
            .header("X-Content-Type-Options", "nosniff")
            .header("Content-Security-Policy", "sandbox")
            .body(BodyExt::boxed(http_body_util::StreamBody::new(stream)))
            .unwrap()
    }

    /// The service entry: assets stream, everything else is `handle_request`'s `Full` body.
    async fn serve_request(
        req: Request<Incoming>,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<Response<RespBody>, hyper::Error> {
        use http_body_util::BodyExt;
        if req.uri().path().starts_with("/oxiui/local/asset/") {
            return Ok(local_asset(req, source_path, peer_addr).await);
        }
        let r = handle_request(req, registry, initial_doc, source_path, peer_addr).await?;
        Ok(r.map(|b| b.map_err(|e| match e {}).boxed()))
    }

    async fn handle_request(
        mut req: Request<Incoming>,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<Response<Full<Bytes>>, hyper::Error> {
        // Browser local mode (OxiUI 7.1): page index, media assets, App Server proxy.
        if req.uri().path().starts_with("/oxiui/local/") {
            return local_route(req, source_path, peer_addr).await;
        }
        let path = req.uri().path();

        // Handle POST /sessions/{id}/patch (Phase 5)
        if path.starts_with("/sessions/") && path.ends_with("/patch") && req.method() == hyper::Method::POST {
            let app_token = std::env::var("OXITERM_APP_TOKEN").ok();
            let expected_str = match app_token {
                Some(ref s) if !s.is_empty() => s,
                _ => {
                    // Fail-closed: If token is unset or empty, patch endpoint is disabled (404)
                    return Ok(Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(Full::new(Bytes::from("Not Found")))
                        .unwrap());
                }
            };

            // Check Bearer authorization token
            let auth_header = req.headers().get("Authorization").and_then(|h| h.to_str().ok()).unwrap_or_default();
            let token_prefix = "Bearer ";
            if !auth_header.starts_with(token_prefix) {
                return Ok(http_401());
            }
            let token = &auth_header[token_prefix.len()..];
            use subtle::ConstantTimeEq;
            let token_bytes = token.as_bytes();
            let expected_bytes = expected_str.as_bytes();
            let len_match = token_bytes.len() == expected_bytes.len();
            let (to_compare_a, to_compare_b) = if len_match {
                (token_bytes, expected_bytes)
            } else {
                (token_bytes, token_bytes)
            };
            let is_match = bool::from(to_compare_a.ct_eq(to_compare_b)) && len_match;
            if !is_match {
                return Ok(http_401());
            }

            // Extract session id
            let path_parts: Vec<&str> = path.split('/').collect();
            if path_parts.len() < 4 {
                return Ok(Response::builder().status(StatusCode::BAD_REQUEST).body(Full::new(Bytes::from("Bad Request"))).unwrap());
            }
            let session_id_str = path_parts[2];
            let session_id = match session_id_str.parse::<usize>() {
                Ok(id) => id,
                Err(_) => return Ok(Response::builder().status(StatusCode::BAD_REQUEST).body(Full::new(Bytes::from("Bad Request"))).unwrap()),
            };

            // Local-mode page loads have ids in their own namespace; a push cannot address one (and
            // never reaches a thin client, whose ids are small registry counters).
            if crate::local_mode::is_local_session_id(session_id) {
                return Ok(Response::builder().status(StatusCode::NOT_FOUND).body(Full::new(Bytes::from("Not Found"))).unwrap());
            }
            let session = {
                let sessions = registry.sessions.read();
                sessions.get(&session_id).cloned()
            };
            let session = match session {
                Some(s) => s,
                None => return Ok(Response::builder().status(StatusCode::NOT_FOUND).body(Full::new(Bytes::from("Not Found"))).unwrap()),
            };

            // Enforce body cap during read (C3/M3)
            use http_body_util::BodyExt;
            let body = req.into_body();
            let limited_body = http_body_util::Limited::new(body, 65536);
            let body_bytes = match limited_body.collect().await {
                Ok(collected) => collected.to_bytes(),
                Err(_) => return Ok(payload_too_large()),
            };

            // Parse state patch
            let patch: serde_json::Value = match serde_json::from_slice(&body_bytes) {
                Ok(v) => v,
                Err(e) => {
                    warn!("POST patch: invalid JSON: {}", e);
                    return Ok(Response::builder().status(StatusCode::BAD_REQUEST).body(Full::new(Bytes::from("Bad Request"))).unwrap());
                }
            };

            // One patch path: the event loop applies it (a full queue drops the OLDEST event, not
            // this one). While the client is disconnected the session closes its event channel
            // until it reattaches; then the patch cannot be queued, so its state keys are stored
            // directly (as before the ActionHandler refactor) and the reattached client sees them.
            // Client-side effects (navigate, open_url) are meaningless without a client and dropped.
            if let crate::backpressure::SendResult::Closed =
                session.event_tx.try_send(oxiterm_proto::input::InputEvent::PushPatch(oxiterm_proto::input::AppPatch(patch.clone())))
            {
                if let serde_json::Value::Object(mut obj) = patch {
                    obj.remove("navigate");
                    obj.remove("open_url");
                    if let Some(tok) = obj.remove("set_app_token") {
                        let tok = match tok {
                            serde_json::Value::String(s) => s,
                            serde_json::Value::Null => String::new(),
                            other => other.to_string(),
                        };
                        let mut state = session.state.write();
                        if tok.is_empty() {
                            state.remove("_app_token");
                        } else {
                            state.set("_app_token".to_string(), crate::state::StateValue::Str(tok));
                        }
                    }
                    session.apply_patch_keys(obj);
                }
            }

            let page_val = session.current_page.read().clone();
            {
                static LAST_LOGGED: std::sync::LazyLock<parking_lot::Mutex<std::collections::HashMap<usize, Option<String>>>> =
                    std::sync::LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));
                let mut guard = LAST_LOGGED.lock();
                let prev = guard.get(&session_id);
                if prev != Some(&page_val) {
                    info!("Session {} current_page: {:?}", session_id, page_val);
                    guard.insert(session_id, page_val.clone());
                }
            }

            let body_json = serde_json::json!({
                "page": page_val
            });
            let resp_bytes = Bytes::from(body_json.to_string());

            return Ok(Response::builder()
                .status(StatusCode::OK)
                .header(hyper::header::CONTENT_TYPE, "application/json")
                .body(Full::new(resp_bytes))
                .unwrap());
        }
        
        let upgrade_header = req.headers().get("Upgrade").and_then(|h| h.to_str().ok()).unwrap_or_default().to_lowercase();
        let connection_header = req.headers().get("Connection").and_then(|h| h.to_str().ok()).unwrap_or_default().to_lowercase();
        if path == "/ws" && upgrade_header == "websocket" && connection_header.contains("upgrade") {
            // Determine user identity for this Web session (Phase 4 / F1)
            if !crate::local_mode::host_allowed(req.headers()) {
                return Ok(Response::builder().status(StatusCode::FORBIDDEN).body(Full::new(Bytes::from("Forbidden"))).unwrap());
            }
            let identity: Option<crate::identity::UserIdentity> = match resolve_identity(req.headers(), peer_addr) {
                Some(id) => Some(id),
                None => return Ok(http_401()),
            };

            let sec_ws_key = req.headers().get("Sec-WebSocket-Key")
                .and_then(|val| val.to_str().ok())
                .unwrap_or_default();
            let accept_val = tokio_tungstenite::tungstenite::handshake::derive_accept_key(sec_ws_key.as_bytes());

            let query = req.uri().query().unwrap_or_default();
            let mut page_param = None;
            let mut session_param = None;
            for pair in query.split('&') {
                let mut parts = pair.splitn(2, '=');
                if let (Some(k), Some(v)) = (parts.next(), parts.next()) {
                    if k == "page" {
                        page_param = Some(v.to_string());
                    } else if k == "session" {
                        session_param = Some(v.to_string());
                    }
                }
            }

            let upgrade_fut = hyper::upgrade::on(&mut req);
            let registry_ws = registry.clone();
            let initial_doc_ws = initial_doc.clone();
            let source_path_ws = source_path.clone();

            tokio::spawn(async move {
                match upgrade_fut.await {
                    Ok(upgraded) => {
                        let io = hyper_util::rt::TokioIo::new(upgraded);
                        let ws_config = WebSocketConfig {
                            max_message_size: Some(128 * 1024),
                            max_frame_size: Some(64 * 1024),
                            ..Default::default()
                        };
                        let ws_stream = tokio_tungstenite::WebSocketStream::from_raw_socket(
                            io,
                            tokio_tungstenite::tungstenite::protocol::Role::Server,
                            Some(ws_config),
                        ).await;
                        if let Err(e) = handle_websocket(ws_stream, registry_ws, initial_doc_ws, source_path_ws, page_param, session_param, identity).await {
                            warn!("WebSocket session error: {:?}", e);
                        }
                    }
                    Err(e) => {
                        warn!("WebSocket upgrade error: {:?}", e);
                    }
                }
            });

            let mut res = Response::builder()
                .status(StatusCode::SWITCHING_PROTOCOLS)
                .body(Full::new(Bytes::new()))
                .unwrap();
            res.headers_mut().insert("Connection", hyper::header::HeaderValue::from_static("Upgrade"));
            res.headers_mut().insert("Upgrade", hyper::header::HeaderValue::from_static("websocket"));
            res.headers_mut().insert("Sec-WebSocket-Accept", hyper::header::HeaderValue::try_from(accept_val).unwrap());
            return Ok(res);
        }

        let (status, mime, body): (StatusCode, &str, std::borrow::Cow<'static, [u8]>) = match path {
            "/" | "/index.html" => {
                (StatusCode::OK, "text/html", index_html())
            }
            "/oxiterm_web.js" => {
                (StatusCode::OK, "application/javascript", std::borrow::Cow::Borrowed(JS_ASSET))
            }
            "/oxiterm_web_bg.wasm" => {
                (StatusCode::OK, "application/wasm", std::borrow::Cow::Borrowed(WASM_ASSET))
            }
            "/oxiterm_local.js" if crate::local_mode::enabled() => {
                (StatusCode::OK, "application/javascript", std::borrow::Cow::Borrowed(LOCAL_JS_ASSET))
            }
            "/oxiterm_local_bg.wasm" if crate::local_mode::enabled() => {
                (StatusCode::OK, "application/wasm", std::borrow::Cow::Borrowed(LOCAL_WASM_ASSET))
            }
            "/oxiui/fonts.json" => {
                let json = serde_json::to_vec(oxiterm_renderer::text::bundle::FontBundle::metrics()).unwrap_or_default();
                (StatusCode::OK, "application/json", std::borrow::Cow::Owned(json))
            }
            // Fixed lookup of embedded bytes by exact id: nothing from the request reaches the
            // filesystem.
            p if p.starts_with("/oxiui/fonts/") => match font_route(&p["/oxiui/fonts/".len()..]) {
                Some((mime, bytes)) => (StatusCode::OK, mime, std::borrow::Cow::Borrowed(bytes)),
                None => (StatusCode::NOT_FOUND, "text/plain", std::borrow::Cow::Borrowed(b"Not Found" as &[u8])),
            },
            _ => {
                (StatusCode::NOT_FOUND, "text/plain", std::borrow::Cow::Borrowed(b"Not Found" as &[u8]))
            }
        };

        let accepts_gzip = req.headers().get("Accept-Encoding")
            .and_then(|val| val.to_str().ok())
            .map(|val| val.contains("gzip"))
            .unwrap_or(false);

        let mut response_body = body.to_vec();
        let mut content_encoding = false;

        if accepts_gzip && status == StatusCode::OK {
            use flate2::write::GzEncoder;
            use flate2::Compression;
            use std::io::Write;
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            if encoder.write_all(&body).is_ok() {
                if let Ok(compressed) = encoder.finish() {
                    response_body = compressed;
                    content_encoding = true;
                }
            }
        }

        let mut res = Response::builder()
            .status(status)
            .header("Content-Type", mime)
            .header("Content-Length", response_body.len())
            .body(Full::new(Bytes::from(response_body)))
            .unwrap();

        if content_encoding {
            res.headers_mut().insert("Content-Encoding", hyper::header::HeaderValue::from_static("gzip"));
        }
        if status == StatusCode::OK && path.starts_with("/oxiui/fonts/") {
            // The embedded bytes only change with a new server build; a year, immutable.
            res.headers_mut().insert("Cache-Control", hyper::header::HeaderValue::from_static("public, max-age=31536000, immutable"));
        }

        Ok(res)
    }

    async fn handle_websocket(
        ws_stream: tokio_tungstenite::WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
        page_param: Option<String>,
        session_param: Option<String>,
        identity: Option<crate::identity::UserIdentity>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let app_base_dir = source_path.clone()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

        let client_session = if let Some(ref tok) = session_param {
            if let Some(s) = registry.reattach(tok) {
                info!("Reattaching to existing session {} with token {}", s.id, tok);
                s.reopen();
                s.active_connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                s
            } else {
                let s = match registry.create_session() {
                    Some(s) => s,
                    None => {
                        warn!("Rejecting WS connection: session registry full");
                        return Ok(());
                    }
                };
                *s.app_base_dir.write() = Some(app_base_dir.clone());
                s.active_connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                s
            }
        } else {
            let s = match registry.create_session() {
                Some(s) => s,
                None => {
                    warn!("Rejecting WS connection: session registry full");
                    return Ok(());
                }
            };
            *s.app_base_dir.write() = Some(app_base_dir.clone());
            s.active_connections.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            s
        };

        client_session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

        // Capabilities are per connection: a new/reattached/taken-over client starts with cell
        // diffs and must send 0x12 again. Reset BEFORE the new sender is installed so no
        // display-list frame can reach it; the event is sent only if the flag was set, so the
        // doc is re-parsed in TUI mode.
        client_session.display_list_v3.store(false, std::sync::atomic::Ordering::SeqCst);
        if client_session.display_list_client.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let _ = client_session.event_tx.try_send(InputEvent::SetDisplayList(false));
        }

        let user_identity = identity.unwrap_or_else(crate::identity::UserIdentity::guest);
        client_session.attach_identity(user_identity);

        let new_epoch = client_session.connection_epoch.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;

        let (ws_write, mut ws_read) = ws_stream.split();
        let (frame_tx, frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

        // Install sender and handle takeover
        {
            let mut w = client_session.web_frame_tx.write();
            if let Some((_, old_tx)) = w.take() {
                info!("WS takeover for session {}: closing old connection with 0xFF", client_session.id);
                // Takeover reason is hardcoded 0 — does not use death_reason.
                match old_tx.try_send(vec![0xFF, 0u8]) {
                    Ok(_) => {}
                    Err(_) => {
                        tracing::trace!("WS takeover: old sender closed/full (expected on dead connections)");
                    }
                }
            }
            *w = Some((new_epoch, frame_tx.clone()));
        }

        let mut token_frame = vec![0x32];
        token_frame.extend_from_slice(client_session.token.as_bytes());
        let _ = frame_tx.try_send(token_frame);

        let session_id = client_session.id;
        let client_session_clone = client_session.clone();
        let my_epoch = new_epoch;
        tokio::spawn(async move {
            let ws_write = ws_write;
            let label = format!("session {} epoch {}", session_id, my_epoch);
            // F3: production calls the same ws_writer_loop as the unit test.
            ws_writer_loop(ws_write, frame_rx, std::time::Duration::from_secs(30), &label).await;
            info!("WS writer task terminated for session {} (epoch {})", session_id, my_epoch);
            {
                let mut w = client_session_clone.web_frame_tx.write();
                if let Some((epoch, _)) = *w {
                    if epoch == my_epoch {
                        *w = None;
                    }
                }
            }
            let active = client_session_clone.active_connections.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if active <= 1 {
                client_session_clone.close();
            }
        });

        // Initial handshake loop to retrieve the first resize frame
        let mut dims = *client_session.dims.read();
        while let Some(msg_res) = ws_read.next().await {
            let msg = match msg_res {
                Ok(m) => m,
                Err(e) => {
                    warn!("WS read error during init for session {}: {:?}", client_session.id, e);
                    break;
                }
            };
            if msg.is_close() {
                if let Message::Close(Some(cf)) = msg {
                    info!("WS read close during init for session {}: code={}, reason={}", client_session.id, cf.code, cf.reason);
                } else {
                    info!("WS read close during init for session {} (no close frame)", client_session.id);
                }
                break;
            }
            if let Message::Binary(bytes) = msg {
                // Capabilities may arrive before the first resize. Honour them here so the
                // very first frame (and the first parse of the page) already uses the
                // requested mode instead of sending a cell frame and switching later.
                if bytes.len() >= 2 && bytes[0] == 0x12 {
                    let on = bytes[1] & 1 != 0;
                    client_session.display_list_v3.store(on && bytes[1] & 2 != 0, std::sync::atomic::Ordering::SeqCst);
                    let prev = client_session.display_list_client.swap(on, std::sync::atomic::Ordering::SeqCst);
                    // A running event loop (reattach) must re-parse; a loop that is not
                    // running yet reads the flag when it loads the page.
                    if prev != on && client_session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst) {
                        let _ = client_session.event_tx.try_send(InputEvent::SetDisplayList(on));
                    }
                    continue;
                }
                if !bytes.is_empty() && bytes[0] == 0x10 {
                    if let Some((cols, rows)) = oxiterm_proto::input::wire::decode_resize(&bytes) {
                        *client_session.dims.write() = PtyDimensions { cols, rows };
                        dims = PtyDimensions { cols, rows };
                        break;
                    }
                }
            }
        }

        // K1: trigger render on attach
        // Reset last_activity (F2: also reset death_reason so a reattached session
        // does not inherit a stale reason from a prior idle cycle).
        client_session.touch_activity();
        client_session.death_reason.store(2, std::sync::atomic::Ordering::SeqCst);
        let _ = client_session.event_tx.try_send(InputEvent::Refresh);

        // K3: reattach honors page param
        if let Some(ref page) = page_param {
            let target_path = app_base_dir.join(page);
            if crate::pathsafe::is_within_base(&app_base_dir, &target_path) {
                let current_p = client_session.current_page.read().clone();
                if Some(page.clone()) != current_p {
                    info!("Honoring new page param on attach: {} (current: {:?})", page, current_p);
                    let _ = client_session.event_tx.try_send(InputEvent::NavigateTo(page.clone()));
                }
            } else {
                warn!("Attach page parameter blocked (path traversal): {:?}", target_path);
            }
        }

        // Spawn EventLoop if it's not already running
        let is_running = client_session.event_loop_running.swap(true, std::sync::atomic::Ordering::SeqCst);
        if !is_running {
            let mut resolved_source_path = source_path.clone();
            let mut base_source_path = source_path.clone();

            if let Some(ref page) = page_param {
                let target_path = app_base_dir.join(page);
                if crate::pathsafe::is_within_base(&app_base_dir, &target_path) {
                    let actual = crate::pathsafe::resolve_variant(&target_path, client_session.is_mobile.load(std::sync::atomic::Ordering::SeqCst));
                    resolved_source_path = Some(actual);
                    base_source_path = Some(target_path);
                    // 2.5/R4: Set current_page before spawning so the K3 guard in the
                    // already-running path does not send a redundant NavigateTo for the
                    // same page, preventing a double load_thtml_file call on first attach.
                    *client_session.current_page.write() = Some(page.clone());
                } else {
                    warn!("Upgrade page parameter blocked (path traversal): {:?}", target_path);
                }
            } else if let Some(ref bp) = source_path {
                let actual = crate::pathsafe::resolve_variant(bp, client_session.is_mobile.load(std::sync::atomic::Ordering::SeqCst));
                resolved_source_path = Some(actual);
                base_source_path = Some(bp.clone());
            }

            let frame_sink = Box::new(WsFrameSink::new(client_session.clone(), resolved_source_path.clone()));
            let event_bus = Arc::new(crate::events::EventBus::new());

            let doc = if let Some(ref path) = resolved_source_path {
                match crate::loader::load_thtml_file_mode(path, if client_session.display_list_client.load(std::sync::atomic::Ordering::SeqCst) { oxiterm_proto::style::RenderMode::Gui } else { oxiterm_proto::style::RenderMode::Tui }) {
                    Ok(mut loaded) => {
                        let state = client_session.state.read();
                        EventLoop::inject_initial_state(&mut loaded, &*state);
                        loaded
                    }
                    Err(e) => {
                        warn!("Failed to load document from source path {:?}: {}, falling back to initial_doc", path, e);
                        if let Some(ref initial) = initial_doc {
                            let mut loaded = initial.clone();
                            let state = client_session.state.read();
                            EventLoop::inject_initial_state(&mut loaded, &*state);
                            loaded
                        } else {
                            crate::placeholder::build_placeholder_doc(dims.cols, dims.rows)
                        }
                    }
                }
            } else if let Some(ref initial) = initial_doc {
                let mut loaded = initial.clone();
                let state = client_session.state.read();
                EventLoop::inject_initial_state(&mut loaded, &*state);
                loaded
            } else {
                crate::placeholder::build_placeholder_doc(dims.cols, dims.rows)
            };

            let mut event_loop = client_session.new_event_loop(event_bus, crate::backpressure::BoundedFrameChannel::new(1).0, doc, false);
            event_loop.frame_sink = frame_sink;
            event_loop.source_path = resolved_source_path;
            event_loop.base_source_path = base_source_path;
            event_loop.page_source = Arc::new(crate::fs_page_source::FsPageSource::new(app_base_dir.clone()));
            if let Ok(url) = std::env::var("OXITERM_APP_SERVER") {
                info!("Web EventLoop initializing HttpActionHandler targeting {}", url);
                event_loop.action_handler = Some(Arc::new(crate::dispatcher::HttpActionHandler::new(url)));
            } else {
                warn!("Web EventLoop: OXITERM_APP_SERVER env var not set");
            }

            std::thread::spawn(move || {
                event_loop.run();
            });
        }

        // Reader loop
        while let Some(msg_res) = ws_read.next().await {
            let msg = match msg_res {
                Ok(m) => m,
                Err(e) => {
                    warn!("WS read error for session {}: {:?}", client_session.id, e);
                    break;
                }
            };

            if msg.is_close() {
                if let Message::Close(Some(cf)) = msg {
                    info!("WS read close frame for session {}: code={}, reason={}", client_session.id, cf.code, cf.reason);
                } else {
                    info!("WS read close for session {} (no close frame)", client_session.id);
                }
                break;
            }

            if let Message::Binary(bytes) = msg {
                if bytes.is_empty() {
                    continue;
                }

                let tag = bytes[0];
                match tag {
                    0x01 => {
                        if let Some(key_event) = oxiterm_proto::input::wire::decode_key(&bytes) {
                            let _ = client_session.event_tx.try_send(InputEvent::KeyPress(key_event));
                        }
                    }
                    0x02 => {
                        if let Some(mouse_event) = oxiterm_proto::input::wire::decode_mouse(&bytes) {
                            let _ = client_session.event_tx.try_send(InputEvent::MouseEvent(mouse_event));
                        }
                    }
                    0x03 => {
                        // Fractional pointer (Phase 6.5). Only display-list sessions lay out in
                        // fractional cells; any other session gets the containing cell as an
                        // ordinary 0x02 event, so nothing is lost if a client sends it anyway.
                        if let Some(p) = decode_pointer(&bytes) {
                            let ev = if client_session.display_list_client.load(std::sync::atomic::Ordering::SeqCst) {
                                InputEvent::PointerEvent(p)
                            } else {
                                InputEvent::MouseEvent(p.to_cell_mouse())
                            };
                            let _ = client_session.event_tx.try_send(ev);
                        }
                    }
                    0x10 => {
                        // Clamped by the decoder: an unclamped 65535x65535 resize aborts the process.
                        if let Some((cols, rows)) = oxiterm_proto::input::wire::decode_resize(&bytes) {
                            { let now = client_session.clock.now(); client_session.resize_debouncer.write().push(PtyDimensions { cols, rows }, now); }
                            let _ = client_session.event_tx.try_send(InputEvent::Resize { cols, rows });
                        }
                    }
                    0x11 => {
                        if let Some(is_mobile) = oxiterm_proto::input::wire::decode_viewport(&bytes) {
                            let _ = client_session.event_tx.try_send(InputEvent::SwitchViewport(is_mobile));
                        }
                    }
                    0x12 => {
                        // Capabilities: bit 0 = display-list frames, bit 1 = wire v3 (proportional glyph runs).
                        // Ignore caps from a superseded connection (stale 0x12 after takeover).
                        if bytes.len() >= 2 && client_session.connection_epoch.load(std::sync::atomic::Ordering::SeqCst) == new_epoch {
                            let on = bytes[1] & 1 != 0;
                            client_session.display_list_v3.store(on && bytes[1] & 2 != 0, std::sync::atomic::Ordering::SeqCst);
                            // Only act on a change: clients resend caps on every reconnect and a
                            // reload would reset the scroll position.
                            let prev = client_session.display_list_client.swap(on, std::sync::atomic::Ordering::SeqCst);
                            if prev != on {
                                let _ = client_session.event_tx.try_send(InputEvent::SetDisplayList(on));
                            }
                        }
                    }
                    0x31 => {
                        if let Some(rel_path) = oxiterm_proto::input::wire::decode_navigate(&bytes) {
                            let _ = client_session.event_tx.try_send(InputEvent::NavigateTo(rel_path));
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn pointer_msg(x: f32, y: f32, button: u8, action: u8, flags: u8) -> Vec<u8> {
            let mut v = vec![0x03];
            v.extend_from_slice(&x.to_le_bytes());
            v.extend_from_slice(&y.to_le_bytes());
            v.extend_from_slice(&[button, action, flags]);
            v
        }

        #[test]
        fn test_decode_pointer_0x03() {
            let p = decode_pointer(&pointer_msg(12.25, 3.5, 0, 0, 0b101)).unwrap();
            assert_eq!((p.x, p.y), (12.25, 3.5));
            assert_eq!(p.button, MouseButton::Left);
            assert_eq!(p.action, MouseAction::Press);
            assert!(p.modifiers.shift && !p.modifiers.ctrl && p.modifiers.alt);
            let p = decode_pointer(&pointer_msg(-0.5, 1e6, 4, 1, 2)).unwrap();
            assert_eq!((p.x, p.y), (-0.5, 1e6), "off-canvas coordinates are legal");
            assert_eq!(p.button, MouseButton::WheelDown);
            assert_eq!(p.action, MouseAction::Release);
            assert!(p.modifiers.ctrl);
            assert_eq!(decode_pointer(&pointer_msg(1.0, 1.0, 9, 7, 0)).unwrap().button, MouseButton::None);
            assert_eq!(decode_pointer(&pointer_msg(1.0, 1.0, 0, 7, 0)).unwrap().action, MouseAction::Move);
            // Trailing bytes are ignored; short or malformed messages are dropped.
            let mut long = pointer_msg(1.0, 2.0, 0, 0, 0);
            long.extend_from_slice(&[9, 9]);
            assert!(decode_pointer(&long).is_some());
            let full = pointer_msg(1.0, 2.0, 0, 0, 0);
            for n in 0..full.len() {
                assert!(decode_pointer(&full[..n]).is_none(), "{n} bytes");
            }
            for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(decode_pointer(&pointer_msg(bad, 1.0, 0, 0, 0)).is_none());
                assert!(decode_pointer(&pointer_msg(1.0, bad, 0, 0, 0)).is_none());
            }
            assert!(decode_pointer(&[0x02; 12]).is_none(), "wrong tag");
        }

        #[test]
        fn test_pointer_falls_back_to_the_one_based_cell_event() {
            // What a non-display-list session gets for a stray 0x03: the cell containing the
            // point, in 0x02's 1-based convention.
            let p = decode_pointer(&pointer_msg(5.99, 0.0, 0, 0, 0)).unwrap();
            let m = p.to_cell_mouse();
            assert_eq!((m.col, m.row), (6, 1));
            let p = decode_pointer(&pointer_msg(-3.0, -0.2, 2, 1, 0)).unwrap();
            let m = p.to_cell_mouse();
            assert_eq!((m.col, m.row), (1, 1), "negative points clamp to the first cell");
            assert_eq!((m.button, m.action), (MouseButton::Right, MouseAction::Release));
        }

        #[test]
        fn test_hash_path() {
            assert_ne!(hash_path("a.png"), hash_path("b.png"));
            assert_eq!(hash_path("logo.svg"), hash_path("logo.svg"));
        }

        #[test]
        fn test_web_test_suite_is_injected_only_at_the_marker() {
            let html = String::from_utf8_lossy(HTML_ASSET);
            // The shipped page carries the injection marker but NOT the test suite itself.
            assert!(html.contains(WEB_TEST_MARKER), "index.html is missing the test-injection marker");
            assert!(!html.contains("__OXITERM_WEB_TEST_RESULT__"),
                "the web test suite must not be present in the normally-served page");
            // In web-test mode the marker is replaced by the full suite.
            let injected = html.replace(WEB_TEST_MARKER, WEB_TEST_JS);
            assert!(injected.contains("__OXITERM_WEB_TEST_RESULT__"),
                "injected HTML must contain the coordinate test suite");
            assert!(injected.contains("clientXToCol"), "test suite must exercise the real mapping helper");
            assert!(!injected.contains(WEB_TEST_MARKER), "marker must be consumed by injection");
        }

        /// F3 / 2.5/t1 — ws_writer_loop: ≥1 Ping within 35 s idle AND binary frames pass through.
        ///
        /// Uses tokio::time::pause() so the test runs instantly without real wall-clock delay.
        /// The production task calls this same function (no separate inline loop).
        #[tokio::test]
        async fn test_t_writer_loop_ping_and_binary() {
            tokio::time::pause();

            // Channel-backed sink: accumulates outgoing Message values.
            let (sink_tx, mut sink_rx) = tokio::sync::mpsc::unbounded_channel::<Message>();

            // Wrap in a futures::Sink adapter
            struct ChanSink(tokio::sync::mpsc::UnboundedSender<Message>);
            impl futures::Sink<Message> for ChanSink {
                type Error = tokio::sync::mpsc::error::SendError<Message>;
                fn poll_ready(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
                    -> std::task::Poll<Result<(), Self::Error>> { std::task::Poll::Ready(Ok(())) }
                fn start_send(self: std::pin::Pin<&mut Self>, item: Message)
                    -> Result<(), Self::Error> { self.0.send(item) }
                fn poll_flush(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
                    -> std::task::Poll<Result<(), Self::Error>> { std::task::Poll::Ready(Ok(())) }
                fn poll_close(self: std::pin::Pin<&mut Self>, _: &mut std::task::Context<'_>)
                    -> std::task::Poll<Result<(), Self::Error>> { std::task::Poll::Ready(Ok(())) }
            }

            let (frame_tx, frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(8);

            // Send a binary frame before the ping window
            frame_tx.send(b"hello".to_vec()).await.unwrap();

            let handle = tokio::spawn(ws_writer_loop(
                ChanSink(sink_tx),
                frame_rx,
                std::time::Duration::from_secs(30),
                "test",
            ));

            // Let the binary frame flush
            tokio::task::yield_now().await;

            // Advance time past the ping interval (35 s > 30 s threshold)
            tokio::time::advance(std::time::Duration::from_secs(35)).await;
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;

            // Drop the sender to terminate the loop
            drop(frame_tx);
            let _ = tokio::time::timeout(std::time::Duration::from_secs(1), handle).await;

            // Collect all messages
            let mut msgs = Vec::new();
            while let Ok(m) = sink_rx.try_recv() { msgs.push(m); }

            let binary_count = msgs.iter().filter(|m| matches!(m, Message::Binary(_))).count();
            let ping_count   = msgs.iter().filter(|m| matches!(m, Message::Ping(_))).count();

            assert!(binary_count >= 1, "binary frame must pass through; got msgs: {:?}", msgs);
            assert!(ping_count   >= 1, "≥1 Ping must be sent within 35 s idle; got msgs: {:?}", msgs);
        }

        #[tokio::test]
        async fn test_ws_frame_sink_media_buffering() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            use crate::session::MediaRenderInfo;
            session.active_media.write().push(MediaRenderInfo {
                path: "test_media.png".to_string(),
                x: 10,
                y: 5,
                width: 20,
                height: 8,
            });

            let temp_dir = std::env::temp_dir();
            let test_file = temp_dir.join("test_media.png");
            std::fs::write(&test_file, b"FAKE_PNG_BYTES").unwrap();

            let session_path = temp_dir.join("session.thtml");
            *session.app_base_dir.write() = Some(temp_dir.clone());

            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            let mut sink = WsFrameSink::new(session.clone(), Some(session_path));

            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            let _ = sink.send_frame(&front, &back);

            let _ = std::fs::remove_file(test_file);

            let msg0 = rx.try_recv().unwrap();
            assert_eq!(msg0.len(), 1);
            let msg1 = rx.try_recv().unwrap();
            assert_eq!(msg1[0], 0x20);
            let msg2 = rx.try_recv().unwrap();
            assert_eq!(msg2[0], 0x21);

            // Drain the full repaint rendering commands frame
            let _ = rx.try_recv().unwrap();

            let res = sink.send_frame(&front, &back).unwrap();
            assert!(!res);
            assert!(rx.try_recv().is_err());
        }

        #[tokio::test]
        async fn test_ws_frame_sink_coordinate_caching() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            use crate::session::MediaRenderInfo;
            session.active_media.write().push(MediaRenderInfo {
                path: "cached_media.png".to_string(),
                x: 10,
                y: 5,
                width: 20,
                height: 8,
            });

            let temp_dir = std::env::temp_dir();
            let test_file = temp_dir.join("cached_media.png");
            std::fs::write(&test_file, b"CACHED_BYTES").unwrap();

            let session_path = temp_dir.join("session.thtml");
            *session.app_base_dir.write() = Some(temp_dir.clone());

            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            let mut sink = WsFrameSink::new(session.clone(), Some(session_path));

            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            let _ = sink.send_frame(&front, &back);
            let _ = std::fs::remove_file(test_file);

            let msg0 = rx.try_recv().unwrap();
            assert_eq!(msg0.len(), 1); // clear
            let msg1 = rx.try_recv().unwrap();
            assert_eq!(msg1[0], 0x20); // payload
            let msg2 = rx.try_recv().unwrap();
            assert_eq!(msg2[0], 0x21); // coordinates

            // Drain the full repaint rendering commands frame
            let _ = rx.try_recv().unwrap();

            let res = sink.send_frame(&front, &back).unwrap();
            assert!(!res);
            assert!(rx.try_recv().is_err());
        }

        #[test]
        fn test_sec_path_traversal_media_blocked() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            use crate::session::MediaRenderInfo;
            session.active_media.write().push(MediaRenderInfo {
                path: "../../../escaped_media.png".to_string(),
                x: 10,
                y: 5,
                width: 20,
                height: 8,
            });

            let temp_dir = std::env::temp_dir();
            let session_path = temp_dir.join("subdir").join("session.thtml");
            std::fs::create_dir_all(session_path.parent().unwrap()).unwrap();

            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            let mut sink = WsFrameSink::new(session.clone(), Some(session_path));

            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            let _ = sink.send_frame(&front, &back);

            let msg0 = rx.try_recv().unwrap();
            assert_eq!(msg0[0], 0x21); // clear
            
            // Drain the full repaint rendering commands frame
            let _ = rx.try_recv().unwrap();

            assert!(rx.try_recv().is_err());
        }

        #[tokio::test]
        async fn test_20_media_send_full_channel() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            use crate::session::MediaRenderInfo;
            session.active_media.write().push(MediaRenderInfo {
                path: "test_media_20.png".to_string(),
                x: 0,
                y: 0,
                width: 5,
                height: 5,
            });

            let temp_dir = std::env::temp_dir();
            let test_file = temp_dir.join("test_media_20.png");
            std::fs::write(&test_file, b"MEDIA_20_BYTES").unwrap();

            let (tx, mut rx) = tokio::sync::mpsc::channel(1);
            *session.web_frame_tx.write() = Some((1, tx));
            *session.app_base_dir.write() = Some(temp_dir.clone());
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            let mut sink = WsFrameSink::new(session.clone(), Some(temp_dir.join("session.thtml")));

            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            let _ = sink.send_frame(&front, &back);
            assert!(!sink.sent_assets.contains("test_media_20.png"));

            while rx.try_recv().is_ok() {}

            let _ = sink.send_frame(&front, &back);
            assert!(sink.sent_assets.contains("test_media_20.png"));

            let _ = std::fs::remove_file(test_file);
        }

        #[tokio::test]
        async fn test_21_current_page_announce() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            *session.current_page.write() = Some("page1.thtml".to_string());

            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            let mut sink = WsFrameSink::new(session.clone(), None);

            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            let _ = sink.send_frame(&front, &back);
            
            let mut found_30 = false;
            while let Ok(msg) = rx.try_recv() {
                if !msg.is_empty() && msg[0] == 0x30 {
                    found_30 = true;
                    assert_eq!(&msg[1..], b"page1.thtml");
                }
            }
            assert!(found_30);

            let _ = sink.send_frame(&front, &back);
            while let Ok(msg) = rx.try_recv() {
                assert!(msg.is_empty() || msg[0] != 0x30);
            }
        }

        // Use the single shared ENV_LOCK from crate::test_env to avoid races with dispatcher.rs tests.
        use crate::test_env::EnvGuard;

        async fn boot_test_server(reg: Arc<SessionRegistry>) -> std::net::SocketAddr {
            boot_test_server_with_source(reg, None).await
        }

        /// Boots a test web server whose start page (and therefore app base dir) is
        /// `source_path`, so tests can serve pages from an isolated directory instead
        /// of writing into the shared working directory.
        async fn boot_test_server_with_source(
            reg: Arc<SessionRegistry>,
            source_path: Option<std::path::PathBuf>,
        ) -> std::net::SocketAddr {
            let rate_limiter = Arc::new(crate::ratelimit::RateLimiter::new(60));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            drop(listener);

            let reg_clone = reg.clone();
            let rate_limiter_clone = rate_limiter.clone();
            start_web_server("127.0.0.1".to_string(), local_addr.port(), reg_clone, rate_limiter_clone, None, source_path);

            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            local_addr
        }

        /// Polls `cond` until it holds or `timeout` elapses; returns whether it held.
        /// Replaces fixed sleeps, which flake when the event-loop thread starts late
        /// under CPU load.
        async fn wait_until(timeout: std::time::Duration, mut cond: impl FnMut() -> bool) -> bool {
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if cond() {
                    return true;
                }
                if std::time::Instant::now() >= deadline {
                    return false;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }

        #[tokio::test]
        async fn test_18_ws_upgrade_no_identity_401() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_TRUSTED_PROXY", Some("127.0.0.1")),
                ("OXITERM_ALLOW_GUEST", Some("false")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = "GET /ws HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\n";
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let response = String::from_utf8_lossy(&buf[..n]);
            assert!(response.contains("401 Unauthorized") || response.contains("401 UNAUTHORIZED"));
        }

        #[tokio::test]
        async fn test_4_ws_page_traversal_blocked() {
            let _env = EnvGuard::lock_and_set(&[]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let rate_limiter = Arc::new(crate::ratelimit::RateLimiter::new(60));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            drop(listener);

            let reg_clone = reg.clone();
            let rate_limiter_clone = rate_limiter.clone();
            let temp = std::env::temp_dir();
            let base_dir = temp.join("oxiterm_base_test_4");
            let _ = std::fs::remove_dir_all(&base_dir);
            std::fs::create_dir_all(&base_dir).unwrap();
            let index_path = base_dir.join("index.thtml");
            std::fs::write(&index_path, b"<screen><text>hello</text></screen>").unwrap();

            let index_path_clone = index_path.clone();
            tokio::spawn(async move {
                start_web_server(
                    "127.0.0.1".to_string(),
                    local_addr.port(),
                    reg_clone,
                    rate_limiter_clone,
                    None,
                    Some(index_path_clone),
                );
            });
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = "GET /ws?page=../escape HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: Upgrade\r\nUpgrade: websocket\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n";
            stream.write_all(request.as_bytes()).await.unwrap();

            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await.unwrap();
            let response = String::from_utf8_lossy(&buf[..n]);
            assert!(response.contains("101 Switching Protocols"));

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            let sessions = reg.sessions.read();
            assert!(!sessions.is_empty());

            let _ = std::fs::remove_file(index_path);
            let _ = std::fs::remove_dir_all(base_dir);
        }

        #[tokio::test]
        async fn test_patch_endpoint_stores_state_while_client_disconnected() {
            // A disconnected client's session closes its event channel until it reattaches; a push
            // in that window must still land in the session state (and drop client-only effects).
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            session.close();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let body = r#"{"progress": 42, "navigate": "x.thtml", "open_url": "https://example.com", "set_app_token": "tok-off"}"#;
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sid, body.len(), body
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("200 OK"), "{response}");

            let state = session.state.read();
            assert_eq!(state.get("progress"), Some(&crate::state::StateValue::Int(42)));
            assert_eq!(state.get("_app_token"), Some(&crate::state::StateValue::Str("tok-off".to_string())));
            assert!(state.get("navigate").is_none());
            assert!(state.get("open_url").is_none());
        }

        #[tokio::test]
        async fn test_19_patch_endpoint_200() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let body = r#"{"valid_key": "applied"}"#;
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sid, body.len(), body
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("200 OK"));
            // No event loop runs in this test: the endpoint only queues the patch for the loop.
            let ev = session.event_rx.lock().try_recv().expect("PushPatch queued");
            assert_eq!(ev, oxiterm_proto::input::InputEvent::PushPatch(oxiterm_proto::input::AppPatch(serde_json::json!({"valid_key": "applied"}))));
        }

        #[tokio::test]
        async fn test_38_patch_endpoint_returns_json_with_current_page() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            *session.current_page.write() = Some("spotify/panel.thtml".to_string());
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let body = r#"{"valid_key": "applied"}"#;
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sid, body.len(), body
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("200 OK"));
            assert!(response.contains(r#"{"page":"spotify/panel.thtml"}"#));
        }

        #[tokio::test]
        async fn test_39_patch_endpoint_returns_json_with_null_page() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            *session.current_page.write() = None;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let body = r#"{"valid_key": "applied"}"#;
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sid, body.len(), body
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("200 OK"));
            assert!(response.contains(r#"{"page":null}"#));
        }

        #[tokio::test]
        async fn test_20_patch_endpoint_wrong_token_and_unknown_session() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            
            // Wrong token -> 401
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer bad_token\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                sid
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("401 Unauthorized") || response.contains("401 UNAUTHORIZED"));

            // Unknown session -> 404
            let mut stream2 = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request2 = format!(
                "POST /sessions/99999/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            );
            stream2.write_all(request2.as_bytes()).await.unwrap();
            let mut response2 = String::new();
            stream2.read_to_string(&mut response2).await.unwrap();
            assert!(response2.contains("404 Not Found") || response2.contains("404 NOT FOUND"));
        }

        #[tokio::test]
        async fn test_21_patch_endpoint_no_token_env_404() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", None),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                sid
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("404 Not Found") || response.contains("404 NOT FOUND"));
        }

        #[tokio::test]
        async fn test_21b_patch_endpoint_empty_token_env_404() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                sid
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("404 Not Found") || response.contains("404 NOT FOUND"));
        }

        #[tokio::test]
        async fn test_21c_patch_endpoint_missing_header_401() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                sid
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("401 Unauthorized") || response.contains("401 UNAUTHORIZED"));
        }

        #[tokio::test]
        async fn test_21d_patch_endpoint_bad_token_401() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer wrong_token_999\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                sid
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("401 Unauthorized") || response.contains("401 UNAUTHORIZED"));
        }

        #[tokio::test]
        async fn test_21e_patch_endpoint_valid_token_200() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}",
                sid
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("200 OK"));
        }

        #[tokio::test]
        async fn test_22_patch_endpoint_exceeds_state_limits() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let long_key = "a".repeat(257);
            let body = format!(r#"{{"valid_key_22": "applied", "{}": "skipped"}}"#, long_key);
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sid, body.len(), body
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("200 OK"));

            // The endpoint queues the raw patch; the loop enforces the limits (session.rs tests
            // `test_sec_state_patch_limits`).
            let ev = session.event_rx.lock().try_recv().expect("PushPatch queued");
            match ev {
                oxiterm_proto::input::InputEvent::PushPatch(p) => {
                    assert_eq!(p.0["valid_key_22"], "applied");
                    assert!(p.0.get(&long_key).is_some());
                }
                other => panic!("unexpected event {:?}", other),
            }
            assert!(session.state.read().get("valid_key_22").is_none());
        }

        #[tokio::test]
        async fn test_37_patch_endpoint_body_too_large_413() {
            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_TOKEN", Some("valid_token_123")),
            ]);

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let session = reg.create_session().unwrap();
            let sid = session.id;
            let local_addr = boot_test_server(reg).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};
            let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
            let payload = "x".repeat(65538);
            let body = format!(r#"{{"large_key": "{}"}}"#, payload);
            let request = format!(
                "POST /sessions/{}/patch HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer valid_token_123\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                sid, body.len(), body
            );
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            assert!(response.contains("413 Payload Too Large") || response.contains("413 PAYLOAD TOO LARGE"));

            assert!(session.state.read().get("large_key").is_none());
        }

        /// Two bundles from one crate: the default (thin + cell) one carries no core, the local one
        /// does and loads its own wasm. A CI that forgets `--no-default-features` for the default
        /// bundle fails here.
        #[test]
        fn test_the_two_bundles_are_split() {
            let thin_js = String::from_utf8_lossy(JS_ASSET);
            let local_js = String::from_utf8_lossy(LOCAL_JS_ASSET);
            assert!(!thin_js.contains("LocalApp") && !thin_js.contains("local_font_bytes"), "the default bundle must be built with --no-default-features");
            assert!(local_js.contains("LocalApp") && local_js.contains("local_font_bytes"), "the local bundle must be built with --features local");
            assert!(local_js.contains("oxiterm_local_bg.wasm"), "the local JS loads its own wasm");
            assert!(thin_js.contains("oxiterm_web_bg.wasm"));
            assert!(WASM_ASSET.len() + 1_000_000 < LOCAL_WASM_ASSET.len(), "thin {} vs local {}", WASM_ASSET.len(), LOCAL_WASM_ASSET.len());
        }

        /// OxiUI 7.1 local-mode routes over a real socket: page index, media assets, App Server
        /// proxy; the identity gate; and the generic 404 for everything else.
        #[tokio::test]
        async fn test_local_mode_routes() {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let app = std::env::temp_dir().join(format!("oxi_local_routes_{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&app);
            std::fs::create_dir_all(app.join("sub")).unwrap();
            std::fs::write(app.join("index.thtml"), r#"<screen><text bind-state="c">hi</text><button event-htmx="inc:c">go</button></screen>"#).unwrap();
            std::fs::write(app.join("sub/b.thtml"), "<screen><text>b</text></screen>").unwrap();
            std::fs::write(app.join("pic.png"), b"\x89PNG").unwrap();
            std::fs::write(app.join("pic.svg"), b"<svg/>").unwrap();
            std::fs::write(app.join("secret.json"), b"{}").unwrap();
            std::fs::write(app.parent().unwrap().join(format!("oxi_outside_{}.png", std::process::id())), b"OUT").unwrap();

            // A mock App Server: answers one POST with a patch and keeps the request text.
            let mock = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let mock_url = format!("http://127.0.0.1:{}/events", mock.local_addr().unwrap().port());
            let mock_req = std::thread::spawn(move || {
                use std::io::{Read, Write};
                let (mut s, _) = mock.accept().unwrap();
                let mut buf = vec![0u8; 8192];
                let mut n = 0;
                loop {
                    n += s.read(&mut buf[n..]).unwrap();
                    let t = String::from_utf8_lossy(&buf[..n]).into_owned();
                    if let Some(sp) = t.find("\r\n\r\n") {
                        let len: usize = t.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                        if n >= sp + 4 + len { break; }
                    }
                }
                let body = r#"{"c":"7"}"#;
                s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", body.len(), body).as_bytes()).unwrap();
                String::from_utf8_lossy(&buf[..n]).into_owned()
            });

            let _env = EnvGuard::lock_and_set(&[
                ("OXITERM_APP_SERVER", Some(&mock_url)),
                ("OXITERM_APP_TOKEN", Some("tok_local_route")),
                ("OXITERM_ALLOW_GUEST", None),
                ("OXITERM_TRUSTED_PROXY", None),
                ("OXITERM_LOCAL_MODE", Some("1")),
                ("OXITERM_ALLOWED_HOSTS", None),
            ]);

            async fn start(source: Option<std::path::PathBuf>) -> std::net::SocketAddr {
                let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
                let rl = Arc::new(crate::ratelimit::RateLimiter::new(600));
                let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let addr = l.local_addr().unwrap();
                drop(l);
                tokio::spawn(async move { start_web_server("127.0.0.1".to_string(), addr.port(), reg, rl, None, source); });
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                addr
            }
            // (lowercased head, body)
            async fn req(addr: std::net::SocketAddr, method: &str, path: &str, body: &[u8]) -> (String, Vec<u8>) {
                req_h(addr, method, path, "Content-Type: application/json\r\n", body).await
            }
            async fn req_h(addr: std::net::SocketAddr, method: &str, path: &str, extra: &str, body: &[u8]) -> (String, Vec<u8>) {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                let head = format!("{method} {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n{extra}Content-Length: {}\r\n\r\n", body.len());
                s.write_all(head.as_bytes()).await.unwrap();
                s.write_all(body).await.unwrap();
                let mut raw = Vec::new();
                s.read_to_end(&mut raw).await.unwrap();
                let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                (String::from_utf8_lossy(&raw[..split]).to_lowercase(), raw[split..].to_vec())
            }

            let addr = start(Some(app.join("index.thtml"))).await;

            // page index: the start page, every page, the actions flag, no cache
            let (head, body) = req(addr, "GET", "/oxiui/local/pages.json", b"").await;
            assert!(head.contains("200 ok") && head.contains("application/json") && head.contains("no-store"), "{head}");
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["start"], "index.thtml");
            assert_eq!(v["actions"], true);
            assert_eq!(v["pages"]["sub/b.thtml"], "<screen><text>b</text></screen>");
            assert!(v["pages"].as_object().unwrap().len() == 2, "{v}");
            assert_eq!(v["dirs"], serde_json::json!(["sub"]));
            // a signed session token per page load, in a header (never in the cached body)
            let token = head.lines().find_map(|l| l.strip_prefix("x-oxiterm-local-session: ")).expect("session header").trim().to_string();
            let (head2, _) = req(addr, "GET", "/oxiui/local/pages.json", b"").await;
            let token2 = head2.lines().find_map(|l| l.strip_prefix("x-oxiterm-local-session: ")).unwrap().trim().to_string();
            assert_ne!(token, token2, "one session per page load");
            let sess = format!("X-OxiTerm-Local-Session: {token}\r\n");

            // assets: media only, sandboxed, nothing outside the base
            let (head, body) = req(addr, "GET", "/oxiui/local/asset/pic.png", b"").await;
            assert!(head.contains("200 ok") && head.contains("image/png") && head.contains("nosniff") && head.contains("sandbox"), "{head}");
            assert_eq!(body, b"\x89PNG");
            let (head, _) = req(addr, "GET", "/oxiui/local/asset/pic.svg", b"").await;
            assert!(head.contains("200 ok") && head.contains("image/svg+xml") && head.contains("content-security-policy: sandbox"), "{head}");
            for bad in [
                "/oxiui/local/asset/secret.json",
                "/oxiui/local/asset/index.thtml",
                "/oxiui/local/asset/../pic.png",
                "/oxiui/local/asset/%2e%2e/pic.png",
                &format!("/oxiui/local/asset/..%2Foxi_outside_{}.png", std::process::id()),
                "/oxiui/local/asset/",
                "/oxiui/local/asset//etc/passwd",
                "/oxiui/local/nothing",
                "/oxiui/local/pages.json/x",
            ] {
                let (head, body) = req(addr, "GET", bad, b"").await;
                assert!(head.contains("404"), "{bad}: {head}");
                assert_eq!(body, b"Not Found", "{bad}: one generic body");
            }
            let (head, _) = req(addr, "POST", "/oxiui/local/pages.json", b"{}").await;
            assert!(head.contains("404"), "{head}");

            // events proxy: the server sets identity and token, the answer comes back verbatim
            let json_sess = format!("Content-Type: application/json\r\n{sess}");
            let (head, _) = req_h(addr, "POST", "/oxiui/local/events", &json_sess, b"not json").await;
            assert!(head.contains("400"), "{head}");
            let big = vec![b' '; crate::local_mode::MAX_EVENT_BODY + 10];
            let (head, _) = req_h(addr, "POST", "/oxiui/local/events", &json_sess, &big).await;
            assert!(head.contains("413"), "{head}");
            // no session token, a forged one, one of another process: refused before anything else
            let id: usize = token.split('.').next().unwrap().parse().unwrap();
            for t in [String::new(), format!("{}.{}", id + 1, token.split('.').nth(1).unwrap()), "0.0000000000000000".to_string(), "garbage".to_string()] {
                let extra = if t.is_empty() { "Content-Type: application/json\r\n".to_string() } else { format!("Content-Type: application/json\r\nX-OxiTerm-Local-Session: {t}\r\n") };
                let (head, _) = req_h(addr, "POST", "/oxiui/local/events", &extra, br#"{"action":"inc:c","state":{},"session_id":1}"#).await;
                assert!(head.contains("403"), "{t:?}: {head}");
            }
            // an action no page declares is refused (the mock below still has its one connection)
            let (head, _) = req_h(addr, "POST", "/oxiui/local/events", &json_sess, br#"{"action":"admin:delete_all","state":{},"session_id":1}"#).await;
            assert!(head.contains("403"), "{head}");
            let forged = br#"{"action":"inc:c","state":{"c":"6"},"session_id":9,"username":"root","auth_method":"SshKey"}"#;
            // CSRF: a cross-site page can send a text/plain POST or carry a foreign Origin; neither
            // may reach the App Server (the mock accepts exactly one connection, used below)
            for (extra, code) in [
                ("Content-Type: text/plain\r\n".to_string(), "415"),
                ("".to_string(), "415"),
                ("Content-Type: application/json\r\nOrigin: http://evil.example\r\n".to_string(), "403"),
                (format!("Content-Type: application/json\r\nOrigin: http://{addr}.evil.example\r\n"), "403"),
                ("Content-Type: application/json\r\nSec-Fetch-Site: cross-site\r\n".to_string(), "403"),
            ] {
                let (head, _) = req_h(addr, "POST", "/oxiui/local/events", &extra, forged).await;
                assert!(head.contains(code), "{extra:?}: {head}");
            }
            let same = format!("Content-Type: application/json; charset=utf-8\r\nOrigin: http://{addr}\r\nSec-Fetch-Site: same-origin\r\n{sess}");
            let (head, body) = req_h(addr, "POST", "/oxiui/local/events", &same, forged).await;
            assert!(head.contains("200 ok") && head.contains("application/json"), "{head}");
            assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap(), serde_json::json!({"c":"7"}));
            let seen = tokio::task::spawn_blocking(move || mock_req.join().unwrap()).await.unwrap();
            assert!(seen.contains("Bearer tok_local_route"), "{seen}");
            assert!(seen.contains("\"username\":\"guest\"") && seen.contains(&format!("\"session_id\":{id}")), "{seen}");
            assert!(!seen.contains("\"session_id\":0") && !seen.contains("root"), "{seen}");

            // a push to a local id never reaches a session (and an ordinary id is just unknown)
            let patch_hdr = "Authorization: Bearer tok_local_route\r\nContent-Type: application/json\r\n";
            let (head, _) = req_h(addr, "POST", &format!("/sessions/{id}/patch"), patch_hdr, b"{}").await;
            assert!(head.contains("404"), "{head}");

            // the local bundle is served only with local mode on
            let (head, body) = req(addr, "GET", "/oxiterm_local.js", b"").await;
            assert!(head.contains("200 ok") && head.contains("javascript") && !body.is_empty(), "{head}");
            let (head, _) = req(addr, "GET", "/oxiterm_local_bg.wasm", b"").await;
            assert!(head.contains("200 ok") && head.contains("application/wasm"), "{head}");

            // /ws and the local routes honour OXITERM_ALLOWED_HOSTS (DNS rebinding)
            let ws_raw = |host: &str| format!("GET /ws HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n");
            async fn raw_head(addr: std::net::SocketAddr, raw: String) -> String {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                s.write_all(raw.as_bytes()).await.unwrap();
                let mut buf = vec![0u8; 512];
                let n = s.read(&mut buf).await.unwrap();
                String::from_utf8_lossy(&buf[..n]).to_lowercase()
            }
            assert!(raw_head(addr, ws_raw("evil.test")).await.contains("101"), "no allowlist: the upgrade works");
            // (the guard above owns the lock and restores the variable)
            std::env::set_var("OXITERM_ALLOWED_HOSTS", "good.test");
            assert!(raw_head(addr, ws_raw("evil.test")).await.contains("403"));
            assert!(raw_head(addr, ws_raw("good.test")).await.contains("101"));
            let (head, _) = req_h(addr, "GET", "/oxiui/local/pages.json", "", b"").await;
            assert!(head.contains("403"), "{head}");
            std::env::remove_var("OXITERM_ALLOWED_HOSTS");

            // without a start page (server not started on a page) every local route is a 404
            let addr_none = start(None).await;
            for (m, p) in [("GET", "/oxiui/local/pages.json"), ("GET", "/oxiui/local/asset/pic.png")] {
                let (head, _) = req(addr_none, m, p, b"").await;
                assert!(head.contains("404"), "{p}: {head}");
            }

            // guests refused: every local route is 401, like /ws
            drop(_env);
            let _env2 = EnvGuard::lock_and_set(&[("OXITERM_LOCAL_MODE", Some("1")), ("OXITERM_ALLOW_GUEST", Some("false")), ("OXITERM_APP_SERVER", Some(&mock_url)), ("OXITERM_TRUSTED_PROXY", None)]);
            for (m, p) in [("GET", "/oxiui/local/pages.json"), ("GET", "/oxiui/local/asset/pic.png"), ("POST", "/oxiui/local/events")] {
                let (head, _) = req(addr, m, p, b"{}").await;
                assert!(head.contains("401"), "{p}: {head}");
            }

            // no App Server: the proxy does not exist and the index says so
            drop(_env2);
            let _env3 = EnvGuard::lock_and_set(&[("OXITERM_LOCAL_MODE", Some("1")), ("OXITERM_APP_SERVER", None), ("OXITERM_ALLOW_GUEST", None), ("OXITERM_TRUSTED_PROXY", None)]);
            let (head, _) = req(addr, "POST", "/oxiui/local/events", forged).await;
            assert!(head.contains("404"), "{head}");
            let (_, body) = req(addr, "GET", "/oxiui/local/pages.json", b"").await;
            assert_eq!(serde_json::from_slice::<serde_json::Value>(&body).unwrap()["actions"], false);

            // local mode OFF (the default): every local route and the local bundle are one 404
            drop(_env3);
            let _off = EnvGuard::lock_and_set(&[("OXITERM_LOCAL_MODE", None), ("OXITERM_APP_SERVER", Some(&mock_url)), ("OXITERM_ALLOW_GUEST", None)]);
            for (m, p) in [("GET", "/oxiui/local/pages.json"), ("GET", "/oxiui/local/asset/pic.png"), ("POST", "/oxiui/local/events"), ("GET", "/oxiterm_local.js"), ("GET", "/oxiterm_local_bg.wasm")] {
                let (head, body) = req_h(addr, m, p, &json_sess, b"{}").await;
                assert!(head.contains("404"), "{p}: {head}");
                assert_eq!(body, b"Not Found", "{p}");
            }
            let (head, _) = req(addr, "GET", "/oxiterm_web.js", b"").await;
            assert!(head.contains("200 ok"), "the thin bundle is always served");

            let _ = std::fs::remove_dir_all(&app);
            let _ = std::fs::remove_file(app.parent().unwrap().join(format!("oxi_outside_{}.png", std::process::id())));
        }

        #[tokio::test]
        async fn test_oxiui_font_routes() {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            use oxiterm_renderer::text::bundle::{FontBundle, MONO_ID, SANS_ID};
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let rate_limiter = Arc::new(crate::ratelimit::RateLimiter::new(60));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            drop(listener);
            tokio::spawn(async move {
                start_web_server("127.0.0.1".to_string(), addr.port(), reg, rate_limiter, None, None);
            });
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            // (lowercased head, body). No Accept-Encoding, so the body is the raw bytes.
            async fn get(addr: std::net::SocketAddr, path: &str) -> (String, Vec<u8>) {
                let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
                s.write_all(format!("GET {path} HTTP/1.1\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
                let mut raw = Vec::new();
                s.read_to_end(&mut raw).await.unwrap();
                let split = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                (String::from_utf8_lossy(&raw[..split]).to_lowercase(), raw[split..].to_vec())
            }

            let (head, body) = get(addr, "/oxiui/fonts.json").await;
            assert!(head.contains("200 ok") && head.contains("application/json"), "{head}");
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(v["font_size_px"], 16.0);
            assert_eq!(v["cell_h_px"], 20.0);
            assert!((v["cell_w_px"].as_f64().unwrap() - 9.6).abs() < 1e-4);
            assert_eq!(v["faces"][0]["id"], MONO_ID);
            assert_eq!(v["faces"][1]["id"], SANS_ID);
            for f in v["faces"].as_array().unwrap() {
                for key in ["url_path", "license_path"] {
                    let (h, _) = get(addr, f[key].as_str().unwrap()).await;
                    assert!(h.contains("200 ok"), "{key}: {h}");
                }
            }
            for id in [MONO_ID, SANS_ID] {
                let (head, body) = get(addr, &format!("/oxiui/fonts/{id}.ttf")).await;
                assert!(head.contains("200 ok") && head.contains("font/ttf"), "{head}");
                assert!(head.contains("max-age=31536000"), "{head}");
                assert_eq!(body, FontBundle::font_bytes(id).unwrap());
                let (head, body) = get(addr, &format!("/oxiui/fonts/LICENSE-{id}.txt")).await;
                assert!(head.contains("200 ok") && head.contains("text/plain"), "{head}");
                assert_eq!(body, FontBundle::license_bytes(id).unwrap());
            }
            for bad in ["/oxiui/fonts/nope.ttf", "/oxiui/fonts/../index.html", "/oxiui/fonts/LICENSE-nope.txt",
                        "/oxiui/fonts/oxi-mono", "/oxiui/fonts/", "/oxiui/fonts/oxi-mono.ttf/x"] {
                let (head, _) = get(addr, bad).await;
                assert!(head.contains("404"), "{bad}: {head}");
            }
        }

        #[tokio::test]
        async fn test_playground_routes() {
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let rate_limiter = Arc::new(crate::ratelimit::RateLimiter::new(60));

            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            drop(listener);
            
            let reg_clone = reg.clone();
            let rate_limiter_clone = rate_limiter.clone();
            tokio::spawn(async move {
                start_web_server("127.0.0.1".to_string(), local_addr.port(), reg_clone, rate_limiter_clone, None, None);
            });

            tokio::time::sleep(std::time::Duration::from_millis(100)).await;

            use tokio::io::{AsyncWriteExt, AsyncReadExt};

            // Test 7: GET / -> serves index.html (200 OK)
            {
                let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
                let request = "GET / HTTP/1.1\r\nConnection: close\r\n\r\n";
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                assert!(response.contains("200 OK"));
                assert!(response.contains("text/html"));
            }

            // Test 8: GET /index.html -> serves index.html (200 OK)
            {
                let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
                let request = "GET /index.html HTTP/1.1\r\nConnection: close\r\n\r\n";
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                assert!(response.contains("200 OK"));
                assert!(response.contains("text/html"));
            }

            // Test 9: GET /mobile -> 404 NOT FOUND
            {
                let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
                let request = "GET /mobile HTTP/1.1\r\nConnection: close\r\n\r\n";
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                assert!(response.contains("404 Not Found") || response.contains("404 NOT FOUND"));
            }

            // Test 10: GET /oxiterm_web.js -> 200 OK
            {
                let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
                let request = "GET /oxiterm_web.js HTTP/1.1\r\nConnection: close\r\n\r\n";
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                assert!(response.contains("200 OK"));
                assert!(response.contains("application/javascript"));
            }

            // Test 11: GET /oxiterm_web_bg.wasm -> 200 OK
            {
                let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
                let request = "GET /oxiterm_web_bg.wasm HTTP/1.1\r\nConnection: close\r\n\r\n";
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response_bytes = Vec::new();
                stream.read_to_end(&mut response_bytes).await.unwrap();
                let response = String::from_utf8_lossy(&response_bytes);
                assert!(response.contains("200 OK"));
                assert!(response.contains("application/wasm"));
            }

            // Test 12: GET /nonexistent -> 404 NOT FOUND
            {
                let mut stream = tokio::net::TcpStream::connect(local_addr).await.unwrap();
                let request = "GET /nonexistent HTTP/1.1\r\nConnection: close\r\n\r\n";
                stream.write_all(request.as_bytes()).await.unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).await.unwrap();
                assert!(response.contains("404 Not Found") || response.contains("404 NOT FOUND"));
            }
        }

        #[tokio::test]
        async fn test_t1_reattach_installs_new_sender() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            let (tx1, mut rx1) = tokio::sync::mpsc::channel(10);
            let (tx2, mut rx2) = tokio::sync::mpsc::channel(10);

            // Set up initial sender
            *session.web_frame_tx.write() = Some((1, tx1));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);

            let mut sink = WsFrameSink::new(session.clone(), None);
            
            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            // Render 1: goes to tx1/rx1
            let res = sink.send_frame(&front, &back).unwrap();
            assert!(res);
            assert!(rx1.try_recv().is_ok());
            assert!(rx2.try_recv().is_err());

            // Reattach: update web_frame_tx and connection_epoch
            *session.web_frame_tx.write() = Some((2, tx2));
            session.connection_epoch.store(2, std::sync::atomic::Ordering::SeqCst);

            // Render 2: goes to tx2/rx2, nothing to rx1
            let res2 = sink.send_frame(&front, &back).unwrap();
            assert!(res2);
            
            // rx2 receives the frame
            assert!(rx2.try_recv().is_ok());
            // rx1 does NOT receive the frame
            assert!(rx1.try_recv().is_err());
        }

        #[tokio::test]
        async fn test_t2_reattach_forces_full_frame() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            let (tx1, mut rx1) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx1));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);

            let mut sink = WsFrameSink::new(session.clone(), None);
            
            let front = CellBuffer::new(80, 24);
            let mut back = CellBuffer::new(80, 24);
            for cell in &mut back.cells {
                cell.ch = 'A';
            }

            // Render 1: first render of epoch 1 is full frame
            let _ = sink.send_frame(&front, &back).unwrap();
            let msg1 = rx1.try_recv().unwrap();
            
            // Decode and count WriteChar commands (K4: covers cols * rows cells)
            use oxiterm_renderer::render::diff::AnsiCommand;
            let commands1 = DiffEngine::decode_binary(&msg1).unwrap();
            let count1 = commands1.iter().filter(|c| matches!(c, AnsiCommand::WriteChar(_))).count();
            assert_eq!(count1, 80 * 24);

            // Reattach to same session, increment connection_epoch
            let (tx2, mut rx2) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((2, tx2));
            session.connection_epoch.store(2, std::sync::atomic::Ordering::SeqCst);

            // Render 2: should be full frame on the new connection even though front & back are unchanged!
            let _ = sink.send_frame(&front, &back).unwrap();
            let msg2 = rx2.try_recv().unwrap();
            let commands2 = DiffEngine::decode_binary(&msg2).unwrap();
            
            let count2 = commands2.iter().filter(|c| matches!(c, AnsiCommand::WriteChar(_))).count();
            assert_eq!(count2, 80 * 24);
        }

        #[tokio::test]
        async fn test_t3_exactly_one_event_loop_spawned() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;

            let url = format!("ws://{}/ws", local_addr);
            
            // Connect 1: starts session and spawns EventLoop
            let (ws_stream1, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut ws_write1, mut ws_read1) = ws_stream1.split();
            
            // Send initial resize frame for Connect 1
            ws_write1.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // Read 0x32 (token) to get the token
            let msg = ws_read1.next().await.unwrap().unwrap();
            let bytes = msg.into_data();
            assert_eq!(bytes[0], 0x32);
            let token = String::from_utf8(bytes[1..].to_vec()).unwrap();

            // Check session exists and event_loop_running is true (sleep to ensure spawned)
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let session = reg.reattach(&token).unwrap();
            assert!(session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst));

            // Connect 2 (reattach): should not spawn another event loop
            let url2 = format!("ws://{}/ws?session={}", local_addr, token);
            let (ws_stream2, _) = tokio_tungstenite::connect_async(&url2).await.unwrap();
            let (mut ws_write2, _) = ws_stream2.split();
            
            // Send initial resize frame for Connect 2
            ws_write2.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // Wait a bit
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            
            // Verify event_loop_running is still true
            assert!(session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst));
        }

        #[tokio::test]
        async fn test_t4_document_swap_clears_cache_and_sends_0x21() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);

            let mut sink = WsFrameSink::new(session.clone(), None);

            // Add dummy media rendering info
            use crate::session::MediaRenderInfo;
            sink.sent_coordinates.insert(123, MediaRenderInfo {
                path: "test.png".to_string(),
                x: 0,
                y: 0,
                width: 10,
                height: 10,
            });

            let doc = oxiterm_renderer::document::THTMLDocument {
                arena: oxiterm_renderer::arena::NodeArena::new(),
                root: oxiterm_proto::dom::NodeId(0),
                dirty_nodes: Vec::new(),
            };

            sink.update_document(&doc).unwrap();

            // Assert coordinates cleared
            assert!(sink.sent_coordinates.is_empty());
            
            // Assert 0x21 sent
            let msg = rx.try_recv().unwrap();
            assert_eq!(msg, vec![0x21]);
        }

        #[tokio::test]
        async fn test_t5_handle_open_url_routes_to_new_sender() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            let (tx1, mut rx1) = tokio::sync::mpsc::channel(10);
            let (tx2, mut rx2) = tokio::sync::mpsc::channel(10);

            // Initial connection
            *session.web_frame_tx.write() = Some((1, tx1));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

            let doc = oxiterm_renderer::document::THTMLDocument {
                arena: oxiterm_renderer::arena::NodeArena::new(),
                root: oxiterm_proto::dom::NodeId(0),
                dirty_nodes: Vec::new(),
            };
            let mut event_loop = session.new_event_loop(Arc::new(crate::events::EventBus::new()), crate::backpressure::BoundedFrameChannel::new(1).0, doc, false);
            event_loop.frame_sink = Box::new(WsFrameSink::new(session.clone(), None));

            event_loop.handle_open_url("https://example.com".to_string(), oxiterm_proto::dom::NodeId(0));
            let msg1 = rx1.try_recv().unwrap();
            assert_eq!(msg1[0], 0x33);

            // Reattach
            *session.web_frame_tx.write() = Some((2, tx2));
            session.connection_epoch.store(2, std::sync::atomic::Ordering::SeqCst);

            event_loop.handle_open_url("https://example2.com".to_string(), oxiterm_proto::dom::NodeId(0));
            assert!(rx1.try_recv().is_err());
            let msg2 = rx2.try_recv().unwrap();
            assert_eq!(msg2[0], 0x33);
            assert_eq!(&msg2[1..], b"https://example2.com");
        }

        #[tokio::test]
        async fn test_t6_writer_exit_clears_web_frame_tx_if_epoch_matches() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            let (tx, _rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((5, tx));

            // Simulated exit of writer for epoch 4 (different)
            {
                let mut w = session.web_frame_tx.write();
                if let Some((epoch, _)) = *w {
                    if epoch == 4 {
                        *w = None;
                    }
                }
            }
            assert!(session.web_frame_tx.read().is_some(), "Should NOT clear if epoch differs");

            // Simulated exit of writer for epoch 5 (matching)
            {
                let mut w = session.web_frame_tx.write();
                if let Some((epoch, _)) = *w {
                    if epoch == 5 {
                        *w = None;
                    }
                }
            }
            assert!(session.web_frame_tx.read().is_none(), "Should clear if epoch matches");

            // Subsequent send degrades to no-op
            let mut sink = WsFrameSink::new(session.clone(), None);
            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);
            let res = sink.send_frame(&front, &back).unwrap();
            assert!(!res); // Should return false (no-op)
        }

        #[tokio::test]
        async fn test_t7_connect_without_token_creates_new_session() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;

            let url = format!("ws://{}/ws", local_addr);
            let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut ws_write, mut ws_read) = ws_stream.split();

            // Send initial resize frame
            ws_write.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // Check we received 0x32 with token
            let msg = ws_read.next().await.unwrap().unwrap();
            let bytes = msg.into_data();
            assert_eq!(bytes[0], 0x32);
            assert!(bytes.len() > 1);
        }

        fn inflate_display_list(msg: &[u8]) -> oxiterm_renderer::render::display_list::DisplayList {
            assert_eq!(msg[0], WS_TAG_DISPLAY_LIST);
            let mut raw = Vec::new();
            std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&msg[1..]), &mut raw).unwrap();
            assert_eq!(raw[0], oxiterm_renderer::render::wire::WIRE_VERSION, "wire version");
            oxiterm_renderer::render::wire::decode(&raw).unwrap()
        }

        #[tokio::test]
        async fn test_display_list_negotiation_sends_0x36_and_no_cell_diffs() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;

            let url = format!("ws://{}/ws", local_addr);
            let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut ws_write, mut ws_read) = ws_stream.split();
            ws_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            ws_write.send(Message::Binary(vec![0x12, 0x01])).await.unwrap();

            // Read until the first display-list frame (cell diffs may precede the switch).
            let mut list = None;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while tokio::time::Instant::now() < deadline {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(500), ws_read.next()).await else { continue };
                let data = m.into_data();
                if !data.is_empty() && data[0] == WS_TAG_DISPLAY_LIST {
                    list = Some(inflate_display_list(&data));
                    break;
                }
            }
            let list = list.expect("no display-list frame received");
            assert!(matches!(list.items.first(), Some(oxiterm_renderer::render::display_list::DisplayItem::Fill { .. })));

            // Force a new frame: from now on only display lists (or 0x2x/0x3x notices), never cell diffs.
            ws_write.send(Message::Binary(vec![0x10, 100, 0, 30, 0])).await.unwrap();
            let mut got_second = false;
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
            while tokio::time::Instant::now() < end {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(300), ws_read.next()).await else { continue };
                let data = m.into_data();
                if data.is_empty() { continue; }
                assert!(data[0] >= 0x20, "cell-diff frame (tag {:#x}) sent to display-list client", data[0]);
                if data[0] == WS_TAG_DISPLAY_LIST {
                    got_second = true;
                    inflate_display_list(&data);
                }
            }
            assert!(got_second, "resize did not produce a new display list");
        }

        #[tokio::test]
        async fn test_caps_before_resize_first_frame_is_display_list() {
            // A GUI client sends 0x12 before its first 0x10. The handshake must honour it so
            // the first frame is already a display list (no cell frame painted first and no
            // delayed switch) and the page is parsed in GUI mode from the start.
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;
            let url = format!("ws://{}/ws", local_addr);
            let (ws_stream, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut ws_write, mut ws_read) = ws_stream.split();
            ws_write.send(Message::Binary(vec![0x12, 0x01])).await.unwrap();
            ws_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            let mut got_list = false;
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while tokio::time::Instant::now() < end && !got_list {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(300), ws_read.next()).await else { continue };
                let data = m.into_data();
                if data.is_empty() { continue; }
                assert!(data[0] >= 0x20, "cell-diff frame (tag {:#x}) sent before the first display list", data[0]);
                if data[0] == WS_TAG_DISPLAY_LIST {
                    inflate_display_list(&data);
                    got_list = true;
                }
            }
            assert!(got_list, "no display-list frame received");
            let session = reg.sessions.read().values().next().cloned().expect("session");
            assert!(session.display_list_client.load(std::sync::atomic::Ordering::SeqCst));
        }

        /// A client-chosen grid size is untrusted: 65535x65535 cells used to abort the whole server
        /// process (a ~gigabyte allocation per buffer). Both the handshake `0x10` (before the loop
        /// exists) and a later one are clamped; the process survives and another session is served.
        #[tokio::test]
        async fn test_huge_resize_is_clamped_and_other_sessions_keep_working() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;
            let url = format!("ws://{}/ws", local_addr);

            // Session A: the very first message is the hostile resize (handshake path).
            let (ws_a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut a_write, mut a_read) = ws_a.split();
            a_write.send(Message::Binary(vec![0x12, 0x03])).await.unwrap();
            a_write.send(Message::Binary(vec![0x10, 0xff, 0xff, 0xff, 0xff])).await.unwrap();
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut a_list = false;
            while tokio::time::Instant::now() < end && !a_list {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(300), a_read.next()).await else { continue };
                let data = m.into_data();
                a_list = data.first() == Some(&WS_TAG_DISPLAY_LIST);
            }
            assert!(a_list, "the session with a hostile first resize still gets frames");
            let max = (oxiterm_proto::input::wire::MAX_COLS, oxiterm_proto::input::wire::MAX_ROWS);
            {
                let sessions = reg.sessions.read();
                let d = *sessions.values().next().unwrap().dims.read();
                assert_eq!((d.cols, d.rows), max, "handshake resize clamped");
            }

            // ... and again later, through the loop path.
            a_write.send(Message::Binary(vec![0x10, 0xff, 0xff, 0xff, 0xff])).await.unwrap();
            a_write.send(Message::Binary(vec![0x10, 0xff, 0xff, 0x00, 0x00])).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            // Session B, afterwards: a normal client is served.
            let (ws_b, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut b_write, mut b_read) = ws_b.split();
            b_write.send(Message::Binary(vec![0x12, 0x03])).await.unwrap();
            b_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut b_list = false;
            while tokio::time::Instant::now() < end && !b_list {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(300), b_read.next()).await else { continue };
                b_list = m.into_data().first() == Some(&WS_TAG_DISPLAY_LIST);
            }
            assert!(b_list, "another session keeps working after a hostile resize");
            // the hostile session's dims never exceeded the cap (and a zero is at least 1)
            let sessions = reg.sessions.read();
            for s in sessions.values() {
                let d = *s.dims.read();
                assert!(d.cols <= max.0 && d.rows <= max.1 && d.cols >= 1 && d.rows >= 1, "{d:?}");
            }
        }

        #[tokio::test]
        async fn test_display_list_flag_resets_on_takeover() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;

            let url = format!("ws://{}/ws", local_addr);
            let (ws_a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut a_write, mut a_read) = ws_a.split();
            a_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            let token_msg = a_read.next().await.unwrap().unwrap().into_data();
            assert_eq!(token_msg[0], 0x32);
            let token = String::from_utf8(token_msg[1..].to_vec()).unwrap();
            a_write.send(Message::Binary(vec![0x12, 0x01])).await.unwrap();
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            let mut a_got_list = false;
            while !a_got_list && tokio::time::Instant::now() < deadline {
                if let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(500), a_read.next()).await {
                    a_got_list = m.into_data().first() == Some(&WS_TAG_DISPLAY_LIST);
                }
            }
            assert!(a_got_list, "client A never got a display list");

            // B takes over without sending 0x12.
            let (ws_b, _) = tokio_tungstenite::connect_async(&format!("ws://{}/ws?session={}", local_addr, token)).await.unwrap();
            let (mut b_write, mut b_read) = ws_b.split();
            b_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            let mut got_diff = false;
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
            while tokio::time::Instant::now() < end {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(300), b_read.next()).await else { continue };
                let data = m.into_data();
                if data.is_empty() { continue; }
                assert_ne!(data[0], WS_TAG_DISPLAY_LIST, "display list sent to a client that did not opt in");
                if data[0] < 0x20 { got_diff = true; }
            }
            assert!(got_diff, "taken-over client received no cell-diff frame");
        }

        #[tokio::test]
        async fn test_caps_bit1_negotiates_wire_v3_and_resets_per_connection() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;
            let v3 = |reg: &Arc<SessionRegistry>| {
                reg.sessions.read().values().next().cloned().expect("session").display_list_v3.load(std::sync::atomic::Ordering::SeqCst)
            };

            let url = format!("ws://{}/ws", local_addr);
            let (ws_a, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut a_write, mut a_read) = ws_a.split();
            // Caps before the first 0x10: handled by the handshake loop.
            a_write.send(Message::Binary(vec![0x12, 0x03])).await.unwrap();
            a_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            let mut token = None;
            let mut got_list = false;
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            while !(got_list && token.is_some()) && tokio::time::Instant::now() < deadline {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(500), a_read.next()).await else { continue };
                let d = m.into_data();
                if d.first() == Some(&0x32) {
                    token = Some(String::from_utf8(d[1..].to_vec()).unwrap());
                }
                got_list |= d.first() == Some(&WS_TAG_DISPLAY_LIST);
            }
            assert!(got_list && token.is_some(), "no display list / token");
            assert!(v3(&reg), "0x12 bit 1 in the handshake loop sets wire v3");

            // Main-loop handler: caps without bit 1 clear it again (display list stays on).
            a_write.send(Message::Binary(vec![0x12, 0x01])).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert!(!v3(&reg), "0x12 without bit 1 clears wire v3");
            a_write.send(Message::Binary(vec![0x12, 0x03])).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert!(v3(&reg), "0x12 bit 1 in the main loop sets wire v3");
            // Bit 1 without bit 0 is meaningless.
            a_write.send(Message::Binary(vec![0x12, 0x02])).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert!(!v3(&reg));
            a_write.send(Message::Binary(vec![0x12, 0x03])).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            assert!(v3(&reg));

            // A takeover without caps starts from the baseline again.
            let (ws_b, _) = tokio_tungstenite::connect_async(&format!("ws://{}/ws?session={}", local_addr, token.unwrap())).await.unwrap();
            let (mut b_write, _b_read) = ws_b.split();
            b_write.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            assert!(!v3(&reg), "wire v3 is per connection");
        }

        /// Collects the ARIA-mirror (0x37) JSON documents and whether any display list / cell
        /// diff arrived for `window` after the client sent its handshake.
        async fn collect_a11y(gui: bool) -> (Vec<serde_json::Value>, bool) {
            let _env = EnvGuard::lock_and_set(&[]);
            let dir = std::env::temp_dir().join(format!(
                "oxiterm_a11y_{}_{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("a.thtml"),
                b"<screen style=\"width: 40; height: 8;\"><box><text>Hi</text><button>Save</button></box></screen>",
            )
            .unwrap();
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server_with_source(reg.clone(), Some(dir.join("a.thtml"))).await;
            let (ws, _) = tokio_tungstenite::connect_async(&format!("ws://{}/ws", local_addr)).await.unwrap();
            let (mut w, mut r) = ws.split();
            if gui {
                w.send(Message::Binary(vec![0x12, 0x01])).await.unwrap();
            }
            w.send(Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            let mut docs = Vec::new();
            let mut saw_frame = false;
            let end = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
            while tokio::time::Instant::now() < end {
                let Ok(Some(Ok(m))) = tokio::time::timeout(std::time::Duration::from_millis(300), r.next()).await else { continue };
                let data = m.into_data();
                if data.is_empty() { continue; }
                if data[0] == WS_TAG_A11Y {
                    docs.push(serde_json::from_slice(&data[1..]).expect("0x37 payload is JSON"));
                } else if data[0] == WS_TAG_DISPLAY_LIST || data[0] < 0x20 {
                    saw_frame = true;
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
            (docs, saw_frame)
        }

        fn has_role(nodes: &serde_json::Value, role: &str) -> bool {
            nodes.as_array().map_or(false, |a| {
                a.iter().any(|n| n["role"] == role || has_role(&n["children"], role))
            })
        }

        #[tokio::test]
        async fn test_display_list_client_receives_0x37_with_button() {
            let (docs, saw_frame) = collect_a11y(true).await;
            assert!(saw_frame, "no frame received");
            let doc = docs.first().expect("no 0x37 frame received");
            assert_eq!(doc["v"], 1);
            assert!(has_role(&doc["nodes"], "button"), "mirror lacks a button node: {}", doc);
            // Unchanged frames are not resent: a static page yields a single document.
            assert_eq!(docs.len(), 1, "identical 0x37 documents were resent");
        }

        #[tokio::test]
        async fn test_cell_client_never_receives_0x37() {
            let (docs, saw_frame) = collect_a11y(false).await;
            assert!(saw_frame, "no cell frame received");
            assert!(docs.is_empty(), "cell client received 0x37");
        }

        #[tokio::test]
        async fn test_display_list_sink_skips_identical_and_resends_on_epoch() {
            use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList, RectF};
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            session.display_list_client.store(true, std::sync::atomic::Ordering::SeqCst);
            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((1, tx));
            session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
            let mut sink = WsFrameSink::new(session.clone(), None);

            let mut list = DisplayList::new();
            list.push(DisplayItem::Legacy { node: oxiterm_proto::dom::NodeId(1), content: RectF::new(0.0, 0.0, 1.0, 1.0) });
            assert!(sink.send_display_list(&list).unwrap());
            assert_eq!(rx.try_recv().unwrap()[0], WS_TAG_DISPLAY_LIST);
            // Identical frame: nothing sent.
            assert!(!sink.send_display_list(&list).unwrap());
            assert!(rx.try_recv().is_err());
            // Changed frame: sent.
            list.push(DisplayItem::PopClip);
            assert!(sink.send_display_list(&list).unwrap());
            assert_eq!(rx.try_recv().unwrap()[0], WS_TAG_DISPLAY_LIST);
            // Reattach (epoch change): identical frame is sent again.
            let (tx2, mut rx2) = tokio::sync::mpsc::channel(10);
            *session.web_frame_tx.write() = Some((2, tx2));
            session.connection_epoch.store(2, std::sync::atomic::Ordering::SeqCst);
            assert!(sink.send_display_list(&list).unwrap());
            let mut saw = false;
            while let Ok(m) = rx2.try_recv() {
                saw |= m[0] == WS_TAG_DISPLAY_LIST;
            }
            assert!(saw);
        }

        #[tokio::test]
        async fn test_display_list_sink_picks_the_wire_version_from_the_caps() {
            use oxiterm_proto::style::AnsiColor;
            use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList, Glyph, Modifiers, PointF, RectF, ShapedRun};
            let shaped = DisplayList {
                items: vec![DisplayItem::Text {
                    node: oxiterm_proto::dom::NodeId(1),
                    origin: PointF { x: 0.0, y: 0.0 },
                    clip: RectF::new(0.0, 0.0, 10.0, 1.0),
                    glyphs: vec![Glyph { x: 0.0, y: 0.0, ch: 'f', advance: 1.2 }],
                    fg: AnsiColor::Reset,
                    bg: AnsiColor::Reset,
                    mods: Modifiers::default(),
                    shaped: Some(Box::new(ShapedRun { face: 1, size_px: 16.0, line_height: 1.0, baseline: 0.8, glyph_faces: vec![], clusters: vec![(0, "fi".into())] })),
                }],
            };
            let plain = DisplayList { items: vec![DisplayItem::PopClip] };
            let version_of = |msg: &[u8]| {
                let mut raw = Vec::new();
                std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&msg[1..]), &mut raw).unwrap();
                raw[0]
            };
            for (v3, want_shaped, want_plain) in [(false, 2u8, 2u8), (true, 3, 2)] {
                let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
                let session = reg.create_session().unwrap();
                session.display_list_client.store(true, std::sync::atomic::Ordering::SeqCst);
                session.display_list_v3.store(v3, std::sync::atomic::Ordering::SeqCst);
                let (tx, mut rx) = tokio::sync::mpsc::channel(10);
                *session.web_frame_tx.write() = Some((1, tx));
                session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
                let mut sink = WsFrameSink::new(session.clone(), None);
                assert!(sink.send_display_list(&shaped).unwrap());
                assert_eq!(version_of(&rx.try_recv().unwrap()), want_shaped, "v3 caps = {v3}");
                assert!(sink.send_display_list(&plain).unwrap());
                assert_eq!(version_of(&rx.try_recv().unwrap()), want_plain, "a list without shaped runs stays v2");
            }
        }

        #[tokio::test]
        async fn test_t8_takeover_sends_0xff_and_closes_old() {
            let _env = EnvGuard::lock_and_set(&[]);
            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;

            let url = format!("ws://{}/ws", local_addr);
            
            // Connect 1
            let (ws_stream1, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut ws_write1, mut ws_read1) = ws_stream1.split();
            
            // Send resize frame for Connect 1
            ws_write1.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // Get token
            let msg = ws_read1.next().await.unwrap().unwrap();
            let bytes = msg.into_data();
            assert_eq!(bytes[0], 0x32);
            let token = String::from_utf8(bytes[1..].to_vec()).unwrap();

            // Connect 2 with token (takeover)
            let url2 = format!("ws://{}/ws?session={}", local_addr, token);
            let (ws_stream2, _) = tokio_tungstenite::connect_async(&url2).await.unwrap();
            let (mut ws_write2, _ws_read2) = ws_stream2.split();

            // Send resize frame for Connect 2
            ws_write2.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // The first connection should receive 0xFF takeover frame [0xFF, 0]
            let mut found_ff = false;
            while let Some(msg) = ws_read1.next().await {
                match msg {
                    Ok(m) => {
                        let data = m.into_data();
                        // Takeover sends [0xFF, 0] — reason byte 0 is hardcoded for takeover.
                        if data.len() >= 1 && data[0] == 0xFF {
                            assert_eq!(data.get(1).copied(), Some(0u8),
                                "takeover must send reason byte 0, got: {:?}", data);
                            found_ff = true;
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            assert!(found_ff, "First connection must receive [0xFF, 0]");
        }

        #[tokio::test]
        async fn test_t11_reattach_honors_different_page() {
            let _env = EnvGuard::lock_and_set(&[]);

            // Serve pages from an isolated directory (not the shared cwd), with
            // hello.thtml as the start page so the event loop itself sets
            // current_page; writing current_page from the test races with the
            // event-loop thread, which initialises it when it starts.
            let dir = std::env::temp_dir().join(format!(
                "oxiterm_t11_{}_{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("hello.thtml"), b"<screen>Hello</screen>").unwrap();
            std::fs::write(dir.join("other.thtml"), b"<screen>Other</screen>").unwrap();
            struct DirCleanup(std::path::PathBuf);
            impl Drop for DirCleanup {
                fn drop(&mut self) {
                    let _ = std::fs::remove_dir_all(&self.0);
                }
            }
            let _cleanup = DirCleanup(dir.clone());

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server_with_source(reg.clone(), Some(dir.join("hello.thtml"))).await;
            let timeout = std::time::Duration::from_secs(10);

            let url = format!("ws://{}/ws", local_addr);

            // Connect 1
            let (ws_stream1, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
            let (mut ws_write1, mut ws_read1) = ws_stream1.split();

            // Send initial resize frame for Connect 1
            ws_write1.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // Get token
            let msg = ws_read1.next().await.unwrap().unwrap();
            let bytes = msg.into_data();
            assert_eq!(bytes[0], 0x32);
            let token = String::from_utf8(bytes[1..].to_vec()).unwrap();

            let session = reg.reattach(&token).unwrap();
            assert!(
                wait_until(timeout, || session.current_page.read().as_deref() == Some("hello.thtml")).await,
                "start page never became current: {:?}",
                session.current_page.read()
            );

            // Connect 2 with token and a different page param
            let url2 = format!("ws://{}/ws?session={}&page=other.thtml", local_addr, token);
            let (ws_stream2, _) = tokio_tungstenite::connect_async(&url2).await.unwrap();
            let (mut ws_write2, _) = ws_stream2.split();

            // Send initial resize frame [0x10, cols_low, cols_high, rows_low, rows_high]
            ws_write2.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();

            // The reattach must navigate to the requested page.
            assert!(
                wait_until(timeout, || session.current_page.read().as_deref() == Some("other.thtml")).await,
                "reattach did not honor page param, current_page = {:?}",
                session.current_page.read()
            );
        }

        #[test]
        fn test_40_web_guest_connection_has_is_web_true() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();

            session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
            let user_identity = crate::identity::UserIdentity::guest();
            session.attach_identity(user_identity);

            let state = session.state.read();
            assert_eq!(state.get("_is_web"), Some(&crate::state::StateValue::Str("true".to_string())));
            assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("guest".to_string())));
        }

        #[test]
        fn test_ws_sink_send_notice_writes_wire_bytes_for_web_only() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            let (tx, mut rx) = tokio::sync::mpsc::channel(8);
            *session.web_frame_tx.write() = Some((1, tx));
            let mut sink = WsFrameSink::new(session.clone(), None);

            // SSH-like session: dropped.
            assert!(!sink.send_notice(oxiterm_renderer::ClientNotice::NavError).unwrap());
            assert!(rx.try_recv().is_err());

            session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
            assert!(sink.send_notice(oxiterm_renderer::ClientNotice::NavThrottle).unwrap());
            assert_eq!(rx.try_recv().unwrap(), [&[0x34u8][..], b"Zwolnij ;)"].concat());
            // No preamble frames (0x30/0x20/0x21) are interleaved.
            assert!(rx.try_recv().is_err());
        }
    }
}

#[cfg(test)]
mod notice_encoding_tests {
    use super::encode_notice;
    use oxiterm_renderer::ClientNotice as N;

    #[test]
    fn notice_wire_bytes_are_unchanged() {
        assert_eq!(encode_notice(&N::NavError), [&[0x34u8][..], b"Nie znaleziono strony"].concat());
        assert_eq!(encode_notice(&N::NavThrottle), [&[0x34u8][..], b"Zwolnij ;)"].concat());
        assert_eq!(encode_notice(&N::OpenUrl("https://example.com/a?b=1".into())), [&[0x33u8][..], b"https://example.com/a?b=1"].concat());
        assert_eq!(encode_notice(&N::AppToken("tok-\u{142}".into())), [&[0x35u8][..], "tok-\u{142}".as_bytes()].concat());
        // Empty token (cleared) is a bare tag.
        assert_eq!(encode_notice(&N::AppToken(String::new())), vec![0x35]);
    }
}

#[cfg(not(feature = "web"))]
pub mod web_impl {
    use std::sync::Arc;
    use crate::session::SessionRegistry;

    /// No-op fallback when web compile feature is disabled.
    pub fn start_web_server(
        _host: String,
        _port: u16,
        _registry: Arc<SessionRegistry>,
        _rate_limiter: Arc<crate::ratelimit::RateLimiter>,
        _initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        _source_path: Option<std::path::PathBuf>,
    ) {
    }
}
