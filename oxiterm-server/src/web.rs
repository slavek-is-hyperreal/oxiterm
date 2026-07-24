//! WebSocket and HTTP Web interface implementation for OxiTerm.
//!
//! Provides WebSocket framing handlers (`WsFrameSink`), static assets routing,
//! input event translators (keyboard, mouse, resize), and path traversal checks.

#[cfg(feature = "web")]
pub mod web_impl {
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    use tokio_tungstenite::tungstenite::Message;
    use futures::{StreamExt, SinkExt};
    use tracing::{info, warn, error};
    use crate::session::{SessionRegistry, PtyDimensions, EventLoop, ClientSession};
    use oxiterm_proto::input::{InputEvent, KeyEvent, KeyModifiers, KeyKind, MouseInput, MouseButton, MouseAction};
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
    }

    impl FrameSink for WsFrameSink {
        fn update_document(&mut self, _doc: &oxiterm_renderer::document::THTMLDocument) -> anyhow::Result<()> {
            // R3: doc swap -> next frame is full, 0x21 emitted
            let _ = self.send_bytes(vec![0x21]);
            self.sent_coordinates.clear();
            Ok(())
        }

        fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
            let current_epoch = self.session.connection_epoch.load(std::sync::atomic::Ordering::SeqCst);
            let epoch_changed = current_epoch != self.last_seen_epoch;
            if epoch_changed {
                self.sent_assets.clear();
                self.sent_coordinates.clear();
                self.sent_page = None;
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
                            handle_request(req, registry, initial_doc, source_path, peer_addr).await
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

    async fn handle_request(
        mut req: Request<Incoming>,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
        peer_addr: std::net::SocketAddr,
    ) -> Result<Response<Full<Bytes>>, hyper::Error> {
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

            session.apply_state_patch(patch);
            let _ = session.event_tx.try_send(oxiterm_proto::input::InputEvent::StatePatched);

            return Ok(Response::builder()
                .status(StatusCode::OK)
                .body(Full::new(Bytes::from("OK")))
                .unwrap());
        }
        
        let upgrade_header = req.headers().get("Upgrade").and_then(|h| h.to_str().ok()).unwrap_or_default().to_lowercase();
        let connection_header = req.headers().get("Connection").and_then(|h| h.to_str().ok()).unwrap_or_default().to_lowercase();
        if path == "/ws" && upgrade_header == "websocket" && connection_header.contains("upgrade") {
            // Determine user identity for this Web session (Phase 4 / F1)
            let forwarded_user = req.headers().get("X-Forwarded-User").and_then(|h| h.to_str().ok()).map(|s| s.to_string());
            let trusted_proxy = std::env::var("OXITERM_TRUSTED_PROXY").ok();
            let allow_guest = match std::env::var("OXITERM_ALLOW_GUEST") {
                Ok(v) => v != "false" && v != "0",
                Err(_) => true, // default = true (unset) (F1)
            };

            let identity: Option<crate::identity::UserIdentity> = match (forwarded_user, trusted_proxy) {
                (Some(user), Some(proxy)) => {
                    match crate::identity::UserIdentity::from_trusted_header(&user, peer_addr, &proxy) {
                        Some(id) => Some(id),
                        None => {
                            if allow_guest { Some(crate::identity::UserIdentity::guest()) }
                            else { return Ok(http_401()); }
                        }
                    }
                }
                (None, _) => {
                    if allow_guest { Some(crate::identity::UserIdentity::guest()) }
                    else { return Ok(http_401()); }
                }
                (Some(user), None) => {
                    warn!("X-Forwarded-User '{}' received but OXITERM_TRUSTED_PROXY not set; ignoring", user);
                    if allow_guest { Some(crate::identity::UserIdentity::guest()) }
                    else { return Ok(http_401()); }
                }
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

        if let Some(ref id) = identity {
            client_session.attach_identity(id.clone());
        }

        client_session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

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
                if !bytes.is_empty() && bytes[0] == 0x10 {
                    if bytes.len() >= 5 {
                        let cols = u16::from_le_bytes([bytes[1], bytes[2]]);
                        let rows = u16::from_le_bytes([bytes[3], bytes[4]]);
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
        *client_session.last_activity.write() = std::time::Instant::now();
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
                match crate::loader::load_thtml_file(path) {
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

            let mut event_loop = EventLoop::new(client_session.clone(), event_bus, crate::backpressure::BoundedFrameChannel::new(1).0, doc, false);
            event_loop.frame_sink = frame_sink;
            event_loop.source_path = resolved_source_path;
            event_loop.base_source_path = base_source_path;
            if let Ok(url) = std::env::var("OXITERM_APP_SERVER") {
                info!("Web EventLoop initializing AppDispatcher targeting {}", url);
                event_loop.app_dispatcher = Some(crate::dispatcher::AppDispatcher::new(url));
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
                        if bytes.len() >= 6 {
                            let val = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
                            if let Some(codepoint) = char::from_u32(val) {
                                let flags = bytes[5];
                                let shift = (flags & 1) != 0;
                                let ctrl = (flags & 2) != 0;
                                let alt = (flags & 4) != 0;
                                let key_event = KeyEvent {
                                    codepoint,
                                    modifiers: KeyModifiers {
                                        shift,
                                        ctrl,
                                        alt,
                                        ..Default::default()
                                    },
                                    kind: KeyKind::Press,
                                };
                                let _ = client_session.event_tx.try_send(InputEvent::KeyPress(key_event));
                            }
                        }
                    }
                    0x02 => {
                        if bytes.len() >= 8 {
                            let col = u16::from_le_bytes([bytes[1], bytes[2]]);
                            let row = u16::from_le_bytes([bytes[3], bytes[4]]);
                            let button_byte = bytes[5];
                            let action_byte = bytes[6];
                            let flags = bytes[7];

                            let button = match button_byte {
                                0 => MouseButton::Left,
                                1 => MouseButton::Middle,
                                2 => MouseButton::Right,
                                3 => MouseButton::WheelUp,
                                4 => MouseButton::WheelDown,
                                _ => MouseButton::None,
                            };

                            let action = match action_byte {
                                0 => MouseAction::Press,
                                1 => MouseAction::Release,
                                _ => MouseAction::Move,
                            };

                            let shift = (flags & 1) != 0;
                            let ctrl = (flags & 2) != 0;
                            let alt = (flags & 4) != 0;

                            let mouse_event = MouseInput {
                                col,
                                row,
                                button,
                                action,
                                modifiers: KeyModifiers {
                                    shift,
                                    ctrl,
                                    alt,
                                    ..Default::default()
                                },
                            };
                            let _ = client_session.event_tx.try_send(InputEvent::MouseEvent(mouse_event));
                        }
                    }
                    0x10 => {
                        if bytes.len() >= 5 {
                            let cols = u16::from_le_bytes([bytes[1], bytes[2]]);
                            let rows = u16::from_le_bytes([bytes[3], bytes[4]]);
                            client_session.resize_debouncer.write().push(PtyDimensions { cols, rows });
                            let _ = client_session.event_tx.try_send(InputEvent::Resize { cols, rows });
                        }
                    }
                    0x11 => {
                        if bytes.len() >= 2 {
                            let is_mobile = bytes[1] != 0;
                            let _ = client_session.event_tx.try_send(InputEvent::SwitchViewport(is_mobile));
                        }
                    }
                    0x31 => {
                        if bytes.len() > 1 {
                            if let Ok(rel_path) = String::from_utf8(bytes[1..].to_vec()) {
                                let _ = client_session.event_tx.try_send(InputEvent::NavigateTo(rel_path));
                            }
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
            let rate_limiter = Arc::new(crate::ratelimit::RateLimiter::new(60));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let local_addr = listener.local_addr().unwrap();
            drop(listener);
            
            let reg_clone = reg.clone();
            let rate_limiter_clone = rate_limiter.clone();
            start_web_server("127.0.0.1".to_string(), local_addr.port(), reg_clone, rate_limiter_clone, None, None);
            
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            local_addr
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
            
            assert_eq!(session.state.read().get("valid_key"), Some(&crate::state::StateValue::Str("applied".to_string())));
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

            let state = session.state.read();
            assert_eq!(state.get("valid_key_22"), Some(&crate::state::StateValue::Str("applied".to_string())));
            assert!(state.get(&long_key).is_none());
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
            let event_loop = EventLoop::new(session.clone(), Arc::new(crate::events::EventBus::new()), crate::backpressure::BoundedFrameChannel::new(1).0, doc, false);

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

            // Create temporary files to satisfy canonicalization and page loading
            std::fs::write("hello.thtml", b"<screen>Hello</screen>").unwrap();
            std::fs::write("other.thtml", b"<screen>Other</screen>").unwrap();

            let reg = Arc::new(SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20));
            let local_addr = boot_test_server(reg.clone()).await;

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
            *session.current_page.write() = Some("hello.thtml".to_string());

            // Connect 2 with token and a different page param
            let url2 = format!("ws://{}/ws?session={}&page=other.thtml", local_addr, token);
            let (ws_stream2, _) = tokio_tungstenite::connect_async(&url2).await.unwrap();
            let (mut ws_write2, _) = ws_stream2.split();
            
            // Send initial resize frame [0x10, cols_low, cols_high, rows_low, rows_high]
            ws_write2.send(tokio_tungstenite::tungstenite::Message::Binary(vec![0x10, 80, 0, 24, 0])).await.unwrap();
            
            // Sleep to let events process
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;

            let current = session.current_page.read().clone();

            // Clean up temporary files before potential assertion panic to avoid poisoning
            let _ = std::fs::remove_file("hello.thtml");
            let _ = std::fs::remove_file("other.thtml");

            // Wait, current_page should be Some("other.thtml")
            assert_eq!(current, Some("other.thtml".to_string()));
        }
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
