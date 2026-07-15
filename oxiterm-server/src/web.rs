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



    /// Simple hash path utility.
    pub fn hash_path(path: &str) -> u32 {
        let mut hash: u32 = 5381;
        for c in path.chars() {
            hash = ((hash << 5).wrapping_add(hash)).wrapping_add(c as u32);
        }
        hash
    }

    /// WebSocket Frame Sink for delivering binary cell buffers to browser clients.
    pub struct WsFrameSink {
        frame_tx: tokio::sync::mpsc::Sender<Vec<u8>>,
        session: Arc<ClientSession>,
        sent_assets: std::collections::HashSet<String>,
        sent_coordinates: std::collections::HashMap<u32, crate::session::MediaRenderInfo>,
        source_path: Option<std::path::PathBuf>,
        media_base_url: Option<std::path::PathBuf>,
        sent_page: Option<String>,
    }

    impl WsFrameSink {
        /// Creates a new `WsFrameSink`.
        pub fn new(frame_tx: tokio::sync::mpsc::Sender<Vec<u8>>, session: Arc<ClientSession>, source_path: Option<std::path::PathBuf>) -> Self {
            let media_base_url = crate::config::OxiTermConfig::from_env().ok().and_then(|c| c.media_base_url).map(std::path::PathBuf::from);
            Self {
                frame_tx,
                session,
                sent_assets: std::collections::HashSet::new(),
                sent_coordinates: std::collections::HashMap::new(),
                source_path,
                media_base_url,
                sent_page: None,
            }
        }
    }

    impl FrameSink for WsFrameSink {
        fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
            let media_base_url = self.media_base_url.clone();
            let base_dir = self.source_path.clone().and_then(|p| p.parent().map(|parent| parent.to_path_buf())).or(media_base_url);

            let current_page = self.session.current_page.read().clone();
            if current_page != self.sent_page {
                if let Some(ref page) = current_page {
                    let mut msg = vec![0x30];
                    msg.extend_from_slice(page.as_bytes());
                    if self.frame_tx.try_send(msg).is_ok() {
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
                let _ = self.frame_tx.try_send(vec![0x21]);
            }

            for media in &active_media {
                let is_safe = if let Some(ref base) = base_dir {
                    let full_path = base.join(&media.path);
                    crate::pathsafe::is_within_base(base, &full_path)
                } else {
                    false
                };

                if !is_safe {
                    warn!("Blocked media Path Traversal attempt: {:?}", media.path);
                    continue;
                }

                if !self.sent_assets.contains(&media.path) {
                    if let Some(ref base) = base_dir {
                        let full_path = base.join(&media.path);
                        if let Ok(file_bytes) = std::fs::read(&full_path) {
                            let mut msg = vec![0x20];
                            msg.extend_from_slice(&hash_path(&media.path).to_le_bytes());
                            msg.extend_from_slice(&file_bytes);
                            if self.frame_tx.try_send(msg).is_ok() {
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
                    let _ = self.frame_tx.try_send(msg);
                    
                    self.sent_coordinates.insert(h, media.clone());
                }
            }

            tracing::trace!("WsFrameSink::send_frame: front dims {}x{}, back dims {}x{}", front.width, front.height, back.width, back.height);
            let commands = DiffEngine::diff(front, back);
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
                match self.frame_tx.try_send(bytes) {
                    Ok(_) => {
                        tracing::trace!("WsFrameSink::send_frame: send successful");
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        warn!("WsFrameSink::send_frame: channel full (backpressure)");
                        return Ok(false);
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        warn!("WsFrameSink::send_frame: channel closed");
                        anyhow::bail!("WebSocket send channel closed")
                    }
                }
            }
            Ok(true)
        }
    }

    impl Drop for WsFrameSink {
        fn drop(&mut self) {
            let _ = self.frame_tx.try_send(vec![0xFF]);
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
                            handle_request(req, registry, initial_doc, source_path).await
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

    async fn handle_request(
        mut req: Request<Incoming>,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
    ) -> Result<Response<Full<Bytes>>, hyper::Error> {
        let path = req.uri().path();
        
        let upgrade_header = req.headers().get("Upgrade").and_then(|h| h.to_str().ok()).unwrap_or_default().to_lowercase();
        let connection_header = req.headers().get("Connection").and_then(|h| h.to_str().ok()).unwrap_or_default().to_lowercase();
        if path == "/ws" && upgrade_header == "websocket" && connection_header.contains("upgrade") {
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
                        if let Err(e) = handle_websocket(ws_stream, registry_ws, initial_doc_ws, source_path_ws, page_param, session_param).await {
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

        let (status, mime, body) = match path {
            "/" | "/index.html" => {
                (StatusCode::OK, "text/html", HTML_ASSET)
            }
            "/oxiterm_web.js" => {
                (StatusCode::OK, "application/javascript", JS_ASSET)
            }
            "/oxiterm_web_bg.wasm" => {
                (StatusCode::OK, "application/wasm", WASM_ASSET)
            }
            _ => {
                (StatusCode::NOT_FOUND, "text/plain", b"Not Found" as &[u8])
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
            if encoder.write_all(body).is_ok() {
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

        let (ws_write, mut ws_read) = ws_stream.split();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

        let mut token_frame = vec![0x32];
        token_frame.extend_from_slice(client_session.token.as_bytes());
        let _ = frame_tx.try_send(token_frame);

        let session_id = client_session.id;
        let client_session_clone = client_session.clone();
        tokio::spawn(async move {
            let mut ws_write = ws_write;
            while let Some(bytes) = frame_rx.recv().await {
                if let Err(e) = ws_write.send(Message::Binary(bytes)).await {
                    warn!("WS send error for session {}: {:?}", session_id, e);
                    break;
                }
            }
            info!("WS writer task terminated for session {}", session_id);
            let active = client_session_clone.active_connections.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if active <= 1 {
                client_session_clone.close();
            }
        });

        let mut resolved_source_path = source_path.clone();
        let mut base_source_path = source_path.clone();

        if let Some(ref page) = page_param {
            let target_path = app_base_dir.join(page);
            if crate::pathsafe::is_within_base(&app_base_dir, &target_path) {
                let actual = crate::pathsafe::resolve_variant(&target_path, client_session.is_mobile.load(std::sync::atomic::Ordering::SeqCst));
                resolved_source_path = Some(actual);
                base_source_path = Some(target_path);
            } else {
                warn!("Upgrade page parameter blocked (path traversal): {:?}", target_path);
            }
        } else if let Some(ref bp) = source_path {
            let actual = crate::pathsafe::resolve_variant(bp, client_session.is_mobile.load(std::sync::atomic::Ordering::SeqCst));
            resolved_source_path = Some(actual);
            base_source_path = Some(bp.clone());
        }

        let frame_sink = Box::new(WsFrameSink::new(frame_tx, client_session.clone(), resolved_source_path.clone()));
        let event_bus = Arc::new(crate::events::EventBus::new());
        
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

        std::thread::spawn(move || {
            event_loop.run();
        });

        while let Some(msg_res) = ws_read.next().await {
            let msg = match msg_res {
                Ok(m) => m,
                Err(e) => {
                    warn!("WS read error for session {}: {:?}", client_session.id, e);
                    break;
                }
            };

            if msg.is_close() {
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

            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            let mut sink = WsFrameSink::new(tx, session, Some(session_path));

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

            let (tx, mut rx) = tokio::sync::mpsc::channel(10);
            let mut sink = WsFrameSink::new(tx, session.clone(), Some(session_path));

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
            let mut sink = WsFrameSink::new(tx, session.clone(), Some(session_path));

            let front = CellBuffer::new(80, 24);
            let back = CellBuffer::new(80, 24);

            let _ = sink.send_frame(&front, &back);

            let msg0 = rx.try_recv().unwrap();
            assert_eq!(msg0[0], 0x21); // clear
            
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
            let mut sink = WsFrameSink::new(tx, session.clone(), Some(temp_dir.join("session.thtml")));

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
            let mut sink = WsFrameSink::new(tx, session.clone(), None);

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

        #[tokio::test]
        async fn test_22_media_base_env_cached() {
            let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
            let session = reg.create_session().unwrap();
            
            std::env::set_var("OXITERM_MEDIA_BASE_URL", "/cached/media/base");
            let (tx, _rx) = tokio::sync::mpsc::channel(10);
            let sink = WsFrameSink::new(tx, session.clone(), None);
            std::env::remove_var("OXITERM_MEDIA_BASE_URL");

            assert_eq!(sink.media_base_url, Some(std::path::PathBuf::from("/cached/media/base")));
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
