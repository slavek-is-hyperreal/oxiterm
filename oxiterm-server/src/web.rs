//! WebSocket and HTTP Web interface implementation for OxiTerm.
//!
//! Provides WebSocket framing handlers (`WsFrameSink`), static assets routing,
//! input event translators (keyboard, mouse, resize), and path traversal checks.

#[cfg(feature = "web")]
pub mod web_impl {
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio_tungstenite::accept_async_with_config;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
    use tokio_tungstenite::tungstenite::Message;
    use futures::{StreamExt, SinkExt};
    use tracing::{info, warn, error};
    use crate::session::{SessionRegistry, PtyDimensions, EventLoop, ClientSession};
    use oxiterm_proto::input::{InputEvent, KeyEvent, KeyModifiers, KeyKind, MouseInput, MouseButton, MouseAction};
    use oxiterm_renderer::FrameSink;
    use oxiterm_renderer::CellBuffer;
    use oxiterm_renderer::DiffEngine;

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
    }

    impl WsFrameSink {
        /// Creates a new `WsFrameSink`.
        pub fn new(frame_tx: tokio::sync::mpsc::Sender<Vec<u8>>, session: Arc<ClientSession>, source_path: Option<std::path::PathBuf>) -> Self {
            Self {
                frame_tx,
                session,
                sent_assets: std::collections::HashSet::new(),
                sent_coordinates: std::collections::HashMap::new(),
                source_path,
            }
        }
    }

    impl FrameSink for WsFrameSink {
        fn send_frame(&mut self, front: &CellBuffer, back: &CellBuffer) -> anyhow::Result<bool> {
            let media_base_url = crate::config::OxiTermConfig::from_env().ok().and_then(|c| c.media_base_url).map(std::path::PathBuf::from);
            let base_dir = self.source_path.clone().and_then(|p| p.parent().map(|parent| parent.to_path_buf())).or(media_base_url);

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
                
                self.sent_coordinates.clear();

                for media in &active_media {
                    let h = hash_path(&media.path);
                    if !self.sent_assets.contains(&media.path) {
                        if let Some(ref base) = base_dir {
                            let full_path = base.join(&media.path);
                            let is_safe = (|| -> Option<bool> {
                                let canonical_base = base.canonicalize().ok()?;
                                let canonical_target = full_path.canonicalize().ok()?;
                                Some(canonical_target.starts_with(canonical_base))
                            })().unwrap_or(false);

                            if is_safe {
                                if let Ok(file_bytes) = std::fs::read(&full_path) {
                                    let mut msg = vec![0x20];
                                    msg.extend_from_slice(&h.to_le_bytes());
                                    msg.extend_from_slice(&file_bytes);
                                    let _ = self.frame_tx.try_send(msg);
                                    self.sent_assets.insert(media.path.clone());
                                }
                            } else {
                                warn!("Blocked media Path Traversal attempt: {:?}", full_path);
                            }
                        }
                    }

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

            info!("WsFrameSink::send_frame: front dims {}x{}, back dims {}x{}", front.width, front.height, back.width, back.height);
            let commands = DiffEngine::diff(front, back);
            info!("WsFrameSink::send_frame: diff generated {} commands", commands.len());
            
            if commands.is_empty() && !coords_changed {
                return Ok(false);
            }
            if !back.graphics.is_empty() {
                warn!("WsFrameSink::send_frame: graphics data ignored (not supported over WS)");
            }
            
            if !commands.is_empty() {
                let bytes = DiffEngine::encode_binary(&commands);
                info!("WsFrameSink::send_frame: sending {} bytes on WebSocket", bytes.len());
                match self.frame_tx.try_send(bytes) {
                    Ok(_) => {
                        info!("WsFrameSink::send_frame: send successful");
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
        tokio::spawn(async move {
            let addr = format!("{}:{}", host, port);
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
                    if let Err(e) = handle_connection(stream, registry, initial_doc, source_path).await {
                        warn!("Web connection handling error: {:?}", e);
                    }
                });
            }
        });
    }

    async fn handle_connection(
        stream: tokio::net::TcpStream,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut buf = [0u8; 8192];
        let n = stream.peek(&mut buf).await?;
        let header = String::from_utf8_lossy(&buf[..n]);

        if header.contains("Upgrade: websocket") || header.contains("upgrade: websocket") {
            let ws_config = WebSocketConfig {
                max_message_size: Some(128 * 1024),
                max_frame_size: Some(64 * 1024),
                ..Default::default()
            };
            let ws_stream = accept_async_with_config(stream, Some(ws_config)).await?;
            handle_websocket(ws_stream, registry, initial_doc, source_path).await?;
        } else {
            let mut stream = stream;
            let mut read_buf = vec![0u8; 8192];
            let mut read_bytes = 0;
            use tokio::io::AsyncReadExt;
            loop {
                let n = stream.read(&mut read_buf[read_bytes..]).await?;
                if n == 0 {
                    break;
                }
                read_bytes += n;
                let s = String::from_utf8_lossy(&read_buf[..read_bytes]);
                if s.contains("\r\n\r\n") || read_bytes >= 8192 {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&read_buf[..read_bytes]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");

            let (status, mime, body) = match path {
                "/" | "/index.html" => {
                    ("200 OK", "text/html", HTML_ASSET)
                }

                "/oxiterm_web.js" => {
                    ("200 OK", "application/javascript", JS_ASSET)
                }
                "/oxiterm_web_bg.wasm" => {
                    ("200 OK", "application/wasm", WASM_ASSET)
                }
                _ => {
                    ("404 NOT FOUND", "text/plain", b"Not Found" as &[u8])
                }
            };

            let accepts_gzip = request.to_lowercase().contains("accept-encoding: gzip")
                || request.to_lowercase().contains("accept-encoding: *")
                || request.contains("Accept-Encoding: gzip");

            let mut response_body = body.to_vec();
            let mut content_encoding = "";
            if accepts_gzip && status == "200 OK" {
                use flate2::write::GzEncoder;
                use flate2::Compression;
                use std::io::Write;
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                if encoder.write_all(body).is_ok() {
                    if let Ok(compressed) = encoder.finish() {
                        response_body = compressed;
                        content_encoding = "Content-Encoding: gzip\r\n";
                    }
                }
            }

            use tokio::io::AsyncWriteExt;
            let response = format!(
                "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",
                status, mime, response_body.len(), content_encoding
            );
            stream.write_all(response.as_bytes()).await?;
            stream.write_all(&response_body).await?;
            stream.flush().await?;
        }
        Ok(())
    }

    async fn handle_websocket(
        ws_stream: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        registry: Arc<SessionRegistry>,
        initial_doc: Option<oxiterm_renderer::THTMLDocument>,
        source_path: Option<std::path::PathBuf>,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let client_session = match registry.create_session() {
            Some(s) => s,
            None => {
                warn!("Rejecting WS connection: session registry full");
                return Ok(());
            }
        };

        info!("Created Web/WS session {}", client_session.id);
        let (ws_write, mut ws_read) = ws_stream.split();
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);

        let session_id = client_session.id;
        let registry_clone = registry.clone();
        let client_session_clone = client_session.clone();
        tokio::spawn(async move {
            let mut ws_write = ws_write;
            while let Some(bytes) = frame_rx.recv().await {
                if let Err(e) = ws_write.send(Message::Binary(bytes)).await {
                    warn!("WS send error for session {}: {:?}", session_id, e);
                    break;
                }
            }
            info!("WS writer task terminated, removing session {}", session_id);
            registry_clone.remove_session(session_id);
            client_session_clone.close();
        });

        let frame_sink = Box::new(WsFrameSink::new(frame_tx, client_session.clone(), source_path.clone()));
        let event_bus = Arc::new(crate::events::EventBus::new());
        
        let mut dims = *client_session.dims.read();

        // Wait for the first resize message (0x10) from the client to set initial dimensions.
        while let Some(msg_res) = ws_read.next().await {
            let msg = match msg_res {
                Ok(m) => m,
                Err(e) => {
                    warn!("WS read error during init for session {}: {:?}", client_session.id, e);
                    break;
                }
            };
            info!("Received message during init: {:?}", msg);
            if msg.is_close() {
                break;
            }
            if let Message::Binary(bytes) = msg {
                if !bytes.is_empty() {
                    info!("Received binary tag: 0x{:02X}, len: {}", bytes[0], bytes.len());
                }
                if !bytes.is_empty() && bytes[0] == 0x10 {
                    if bytes.len() >= 5 {
                        let cols = u16::from_le_bytes([bytes[1], bytes[2]]);
                        let rows = u16::from_le_bytes([bytes[3], bytes[4]]);
                        info!("Received initial resize from client: {}x{}", cols, rows);
                        *client_session.dims.write() = PtyDimensions { cols, rows };
                        dims = PtyDimensions { cols, rows };
                        break;
                    }
                }
            }
        }

        let doc = if let Some(ref initial) = initial_doc {
            initial.clone()
        } else {
            crate::placeholder::build_placeholder_doc(dims.cols, dims.rows)
        };

        let mut event_loop = EventLoop::new(client_session.clone(), event_bus, crate::backpressure::BoundedFrameChannel::new(1).0, doc, false);
        event_loop.frame_sink = frame_sink;
        event_loop.source_path = source_path;

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
                    _ => {}
                }
            }
        }

        client_session.close();
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
            assert_eq!(msg0[0], 0x21);

            let msg1 = rx.try_recv().unwrap();
            assert_eq!(msg1[0], 0x20);
            let expected_hash = hash_path("test_media.png");
            assert_eq!(&msg1[1..5], &expected_hash.to_le_bytes());
            assert_eq!(&msg1[5..], b"FAKE_PNG_BYTES");

            let msg2 = rx.try_recv().unwrap();
            assert_eq!(msg2[0], 0x21);
            assert_eq!(&msg2[1..5], &expected_hash.to_le_bytes());
            assert_eq!(u16::from_le_bytes([msg2[5], msg2[6]]), 10);
            assert_eq!(u16::from_le_bytes([msg2[7], msg2[8]]), 5);
            assert_eq!(u16::from_le_bytes([msg2[9], msg2[10]]), 20);
            assert_eq!(u16::from_le_bytes([msg2[11], msg2[12]]), 8);
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
            
            let msg1 = rx.try_recv().unwrap();
            assert_eq!(msg1[0], 0x21); 
            assert!(rx.try_recv().is_err());
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
