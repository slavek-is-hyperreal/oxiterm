//! Client session representation and event loop processing.
//!
//! Exposes registries tracking interactive users, input event dispatchers, PTY configurations,
//! screen double-buffering, and rendering coordinates updates.

use std::collections::HashMap;
use std::sync::Arc;
use crate::ssh::reactor::ReactorMessage;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::{info, warn, debug};

pub use oxiterm_renderer::document::THTMLDocument;
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::render::buffer::DoubleBuffer;
use oxiterm_renderer::render::renderer::Renderer;

/// Unique identifier type for a client session.
pub type SessionId = usize;

/// Registry holding active client session objects.
pub struct SessionRegistry {
    /// Active sessions map.
    pub sessions: RwLock<HashMap<SessionId, Arc<ClientSession>>>,
    /// Active tokens map.
    pub tokens: RwLock<HashMap<String, SessionId>>,
    next_id: RwLock<SessionId>,
    prometheus_registry: Arc<prometheus::Registry>,
    max_sessions: usize,
}

/// Pty dimensions configuration.
#[derive(Debug, Clone, Copy)]
pub struct PtyDimensions {
    /// Number of terminal columns.
    pub cols: u16,
    /// Number of terminal rows.
    pub rows: u16,
}

use std::sync::mpsc;
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::input::InputEvent;

/// Predictive echo buffer for client side latency hiding.
///
/// Anchored by spec [S5-21].
#[derive(Debug, Default)]
pub struct PredictiveEcho {
    /// Characters typed by the user but not yet confirmed by the server.
    pub buffer: String,
    /// The input node currently targeted by the echo.
    pub active_node: Option<NodeId>,
}

impl PredictiveEcho {
    /// Pushes a new character to the predictive buffer.
    pub fn push(&mut self, ch: char) {
        self.buffer.push(ch);
    }

    /// Confirms a character from the predictive buffer.
    ///
    /// Removes character from the buffer in FIFO order if it matches the confirmed character.
    pub fn confirm(&mut self, ch: char) {
        let mut chars: Vec<char> = self.buffer.chars().collect();
        if let Some(&first) = chars.first() {
            if first == ch {
                chars.remove(0);
                self.buffer = chars.into_iter().collect();
            } else {
                warn!("PredictiveEcho: expected '{}', got '{}'", first, ch);
            }
        }
    }

    /// Drains and returns all pending buffered text.
    pub fn flush_to_server(&mut self) -> String {
        self.buffer.drain(..).collect()
    }
}

/// Debouncer to prevent rapid, redundant terminal layout recomputations on resizing.
#[derive(Debug)]
pub struct ResizeDebouncer {
    pub pending: Option<PtyDimensions>,
    pub pushed_at: std::time::Instant,
    pub last_update: std::time::Instant,
}

impl ResizeDebouncer {
    /// Creates a new debouncer.
    pub fn new() -> Self {
        Self {
            pending: None,
            pushed_at: std::time::Instant::now(),
            last_update: std::time::Instant::now(),
        }
    }

    /// Queues a new dimensions update.
    pub fn push(&mut self, dims: PtyDimensions) {
        self.pending = Some(dims);
        self.pushed_at = std::time::Instant::now();
    }

    /// Polls for a debounced layout change.
    ///
    /// Returns the dimensions if the debouncing window has elapsed since the last push.
    pub fn poll(&mut self) -> Option<PtyDimensions> {
        if let Some(dims) = self.pending {
            if self.pushed_at.elapsed() > Duration::from_millis(100) {
                self.pending = None;
                self.last_update = std::time::Instant::now();
                return Some(dims);
            }
        }
        None
    }
}

/// Info details for active media elements.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaRenderInfo {
    /// File path of the media resource.
    pub path: String,
    /// X coordinate of layout position.
    pub x: u16,
    /// Y coordinate of layout position.
    pub y: u16,
    /// Width bounding box.
    pub width: u16,
    /// Height bounding box.
    pub height: u16,
}

/// Represents an active interactive SSH client session.
pub struct ClientSession {
    /// Unique identifier.
    pub id: SessionId,
    /// Shared terminal grid dimensions.
    pub dims: RwLock<PtyDimensions>,
    /// Prometheus metrics tracker.
    pub metrics: Arc<crate::metrics::SessionMetrics>,
    /// Channel to send raw bytes or commands to the `ReactorThread`.
    pub raw_input_tx: mpsc::Sender<ReactorMessage>,
    /// Channel to receive processed `InputEvents` from the `ReactorThread`.
    pub event_rx: Arc<parking_lot::Mutex<crate::backpressure::Receiver<InputEvent>>>,
    /// Input events sender.
    pub event_tx: crate::backpressure::BoundedFrameChannel<InputEvent>,
    /// Predictive input echo engine.
    pub predictive_echo: RwLock<PredictiveEcho>,
    /// Debouncer for terminal window changes.
    pub resize_debouncer: RwLock<ResizeDebouncer>,
    /// Active terminal emulator profile capability features.
    pub terminal_profile: RwLock<crate::ssh::negotiator::TerminalProfile>,
    /// Instant of the last received input action.
    pub last_activity: RwLock<std::time::Instant>,
    /// Session-wide local DOM state database.
    pub state: RwLock<crate::state::StateManager>,
    /// Active media placement layout tracking list.
    pub active_media: RwLock<Vec<MediaRenderInfo>>,
    /// Indicates if the session is a mobile web session.
    pub is_mobile: std::sync::atomic::AtomicBool,
    /// Indicates if the session is a web (WebSocket) session with DOM media overlay.
    pub is_web_client: std::sync::atomic::AtomicBool,
    /// The current relative page path for WebSocket announce.
    pub current_page: RwLock<Option<String>>,
    /// The application base directory.
    pub app_base_dir: RwLock<Option<std::path::PathBuf>>,
    /// Number of active WebSocket connections.
    pub active_connections: std::sync::atomic::AtomicUsize,
    /// Unique connection token for session reattachment.
    pub token: String,
    /// Authenticated user identity for this session.
    /// Set exactly once at session start by identity.rs; never overwritten on reattach.
    pub identity: parking_lot::RwLock<Option<crate::identity::UserIdentity>>,
}

impl ClientSession {
    /// Attempts to attach an identity to the session.
    ///
    /// Injects reserved state keys and sets the identity field if identity is None (C2/M2).
    /// Returns true if the identity was successfully attached.
    pub fn attach_identity(&self, id: crate::identity::UserIdentity) -> bool {
        let mut stored = self.identity.write();
        if stored.is_none() {
            let mut state = self.state.write();
            id.inject_reserved_keys(&mut *state);
            *stored = Some(id);
            true
        } else {
            false
        }
    }

    /// Closes input receiver streams, initiating shutdown.
    pub fn close(&self) {
        self.event_rx.lock().close();
    }

    /// Reopens the input receiver stream for reconnects.
    pub fn reopen(&self) {
        self.event_rx.lock().reopen();
    }

    /// Evaluates dynamic client state change patches.
    ///
    /// Enforces limits on key lengths, array items, and payload sizes to mitigate DoS.
    pub fn apply_state_patch(&self, patch: serde_json::Value) {
        if let serde_json::Value::Object(obj) = patch {
            // Anchored by spec [SEC-11a]: limit 100 keys per patch
            if obj.len() > 100 {
                warn!("Ignoring state patch with too many keys: {}", obj.len());
                return;
            }
            let mut state = self.state.write();
            for (key, val) in obj {
                // Reserved keys (prefixed `_`) are written only by identity.rs at session start.
                if key.starts_with('_') {
                    warn!("apply_state_patch: skipping reserved key '{}'", key);
                    continue;
                }
                // Anchored by spec [SEC-11b]: limit 256 characters per key
                if key.len() > 256 {
                    warn!("Skipping key in state patch: key length exceeds 256 characters");
                    continue;
                }
                let state_val = match val {
                    serde_json::Value::Number(num) => {
                        if let Some(i) = num.as_i64() {
                            crate::state::StateValue::Int(i)
                        } else {
                            let s = num.to_string();
                            if s.len() > 65536 {
                                warn!("Skipping key {}: number string length exceeds 64KiB", key);
                                continue;
                            }
                            crate::state::StateValue::Str(s)
                        }
                    }
                    serde_json::Value::String(s) => {
                        // Anchored by spec [SEC-11c]: limit 64KiB per string
                        if s.len() > 65536 {
                            warn!("Skipping key {}: string length exceeds 64KiB", key);
                            continue;
                        }
                        crate::state::StateValue::Str(s)
                    }
                    serde_json::Value::Bool(b) => crate::state::StateValue::Bool(b),
                    serde_json::Value::Array(arr) => {
                        // Anchored by spec [SEC-11c]: limit 1000 items in list
                        if arr.len() > 1000 {
                            warn!("Skipping key {}: array size exceeds 1000 items", key);
                            continue;
                        }
                        let mut list = Vec::new();
                        let mut string_too_long = false;
                        for v in arr {
                            let s = match v {
                                serde_json::Value::String(st) => st,
                                other => other.to_string(),
                            };
                            if s.len() > 65536 {
                                string_too_long = true;
                                break;
                            }
                            list.push(s);
                        }
                        if string_too_long {
                            warn!("Skipping key {}: list item string length exceeds 64KiB", key);
                            continue;
                        }
                        crate::state::StateValue::List(list)
                    }
                    serde_json::Value::Null => {
                        crate::state::StateValue::Str(String::new())
                    }
                    other => {
                        let s = other.to_string();
                        if s.len() > 65536 {
                            warn!("Skipping key {}: serialized value length exceeds 64KiB", key);
                            continue;
                        }
                        crate::state::StateValue::Str(s)
                    }
                };
                state.set(key, state_val);
            }
        }
    }
}

impl SessionRegistry {
    /// Creates a new SessionRegistry with configured limits.
    pub fn new(prometheus_registry: Arc<prometheus::Registry>, max_sessions: usize) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            tokens: RwLock::new(HashMap::new()),
            next_id: RwLock::new(0),
            prometheus_registry,
            max_sessions,
        }
    }

    fn generate_token() -> String {
        use std::io::Read;
        let mut bytes = [0u8; 16];
        if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
            let _ = f.read_exact(&mut bytes);
        } else {
            let duration = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
            let mut seed = duration.as_nanos();
            for byte in bytes.iter_mut() {
                *byte = (seed & 0xFF) as u8;
                seed >>= 8;
                if seed == 0 {
                    seed = duration.as_nanos();
                }
            }
        }
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Returns the live session if the token is valid, else None.
    pub fn reattach(&self, token: &str) -> Option<Arc<ClientSession>> {
        let tokens_lock = self.tokens.read();
        let sid = tokens_lock.get(token)?;
        let sessions_lock = self.sessions.read();
        sessions_lock.get(sid).cloned()
    }

    /// Allocates and spawns a new ClientSession, respecting capacity thresholds.
    pub fn create_session(&self) -> Option<Arc<ClientSession>> {
        {
            let count = self.sessions.read().len();
            if count >= self.max_sessions {
                warn!("Session limit reached ({}/{})", count, self.max_sessions);
                return None;
            }
        }

        let mut id_lock = self.next_id.write();
        let id = *id_lock;
        *id_lock += 1;

        let (raw_tx, raw_rx) = mpsc::channel();
        let (event_tx, event_rx) = crate::backpressure::BoundedFrameChannel::<InputEvent>::new(128);

        // Spawn the RRT (Resilient Reactor Thread)
        crate::ssh::reactor::ReactorThread::spawn(raw_rx, event_tx.clone());

        let token = Self::generate_token();

        let session = Arc::new(ClientSession { 
            id,
            dims: RwLock::new(PtyDimensions { cols: 80, rows: 24 }),
            metrics: crate::metrics::SessionMetrics::new(&id.to_string(), &self.prometheus_registry),
            raw_input_tx: raw_tx,
            event_rx: Arc::new(parking_lot::Mutex::new(event_rx)),
            event_tx: event_tx.clone(),
            predictive_echo: RwLock::new(PredictiveEcho::default()),
            resize_debouncer: RwLock::new(ResizeDebouncer::new()),
            terminal_profile: RwLock::new(crate::ssh::negotiator::TerminalProfile::default()),
            last_activity: RwLock::new(std::time::Instant::now()),
            state: RwLock::new(crate::state::StateManager::new()),
            active_media: RwLock::new(Vec::new()),
            is_mobile: std::sync::atomic::AtomicBool::new(false),
            is_web_client: std::sync::atomic::AtomicBool::new(false),
            current_page: RwLock::new(None),
            app_base_dir: RwLock::new(None),
            active_connections: std::sync::atomic::AtomicUsize::new(0),
            token: token.clone(),
            identity: parking_lot::RwLock::new(None),
        });
        self.sessions.write().insert(id, session.clone());
        self.tokens.write().insert(token, id);
        Some(session)
    }

    /// Removes session from the registry and frees allocated resources.
    pub fn remove_session(&self, id: SessionId) {
        let count_before = self.sessions.read().len();
        if let Some(session) = self.sessions.write().remove(&id) {
            self.tokens.write().retain(|_, &mut v| v != id);
            session.close();
        }
        let count_after = self.sessions.read().len();
        if count_after == 0 && count_before > 0 {
            info!("No active sessions remaining. Cleaning up video players.");
            oxiterm_renderer::render::cache::VideoPlayerRegistry::get().cleanup();
        }
    }

    /// Broadcasts an input event to all active sessions.
    pub fn broadcast_input_event(&self, event: InputEvent) {
        let sessions = self.sessions.read();
        for session in sessions.values() {
            let _ = session.event_tx.try_send(event.clone());
        }
    }

    /// Gracefully waits for sessions to close during process drains.
    pub async fn drain_sessions(&self, timeout: Duration) {
        info!("Draining sessions with timeout: {}s", timeout.as_secs());
        
        let start = std::time::Instant::now();
        while start.elapsed() < timeout {
            let count = self.sessions.read().len();
            if count == 0 {
                info!("All sessions drained successfully.");
                return;
            }
            info!("Waiting for {} sessions to close...", count);
            tokio::task::yield_now().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        warn!("Drain timeout reached. {} sessions still active.", self.sessions.read().len());
    }
}

/// ANSI compatible frame writer.
pub struct AnsiFrameSink {
    output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
}

impl AnsiFrameSink {
    /// Creates a new `AnsiFrameSink`.
    pub fn new(output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>) -> Self {
        Self { output_tx }
    }
}

impl oxiterm_renderer::FrameSink for AnsiFrameSink {
    fn send_frame(&mut self, front: &oxiterm_renderer::CellBuffer, back: &oxiterm_renderer::CellBuffer) -> anyhow::Result<bool> {
        let commands = oxiterm_renderer::DiffEngine::diff(front, back);
        let transitioning_graphics_cleanup = !front.graphics.is_empty() && back.graphics.is_empty();
        
        debug!("send_frame: commands count = {}, graphics count = {}, transitioning_graphics_cleanup = {}", commands.len(), back.graphics.len(), transitioning_graphics_cleanup);
        
        if commands.is_empty() && back.graphics.is_empty() && !transitioning_graphics_cleanup {
            return Ok(false);
        }

        let mut out = Vec::new();
        // BSU: CSI ? 2026 h
        out.extend_from_slice(b"\x1b[?2026h");

        // When transitioning from graphics to no graphics, delete active placement handles.
        if !front.graphics.is_empty() && back.graphics.is_empty() {
            out.extend_from_slice(&oxiterm_renderer::render::kitty::KittyImageManager::delete_all_placements());
        }
        
        let bytes = oxiterm_renderer::DiffEngine::encode_ansi(&commands);
        debug!("send_frame: encoded ANSI size = {} bytes", bytes.len());
        out.extend_from_slice(&bytes);
        
        for g in &back.graphics {
            out.extend_from_slice(g);
        }
        
        // ESU: CSI ? 2026 l
        out.extend_from_slice(b"\x1b[?2026l");
        
        let sent_len = out.len();
        match self.output_tx.try_send(out) {
            crate::backpressure::SendResult::Sent => {
                debug!("send_frame: successfully sent frame ({} bytes) to output_tx", sent_len);
                Ok(true)
            }
            crate::backpressure::SendResult::Dropped => {
                warn!("send_frame: frame dropped due to backpressure");
                Ok(false)
            }
            crate::backpressure::SendResult::Closed => {
                warn!("send_frame: frame closed");
                Ok(false)
            }
        }
    }

    fn setup(&mut self) -> anyhow::Result<()> {
        debug!("AnsiFrameSink::setup called");
        let mut initial_clear = Vec::new();
        // Enable Alt Buffer, Clear screen (with scrollback history), Hide cursor.
        initial_clear.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[3J\x1b[H\x1b[?25l");
        let _ = self.output_tx.try_send(initial_clear);
        Ok(())
    }

    fn clear_screen(&mut self) -> anyhow::Result<()> {
        let mut clear_seq = Vec::new();
        clear_seq.extend_from_slice(&oxiterm_renderer::render::kitty::KittyImageManager::delete_all_placements());
        clear_seq.extend_from_slice(b"\x1b[2J\x1b[H");
        let _ = self.output_tx.try_send(clear_seq);
        Ok(())
    }
}

impl Drop for AnsiFrameSink {
    fn drop(&mut self) {
        let mut cleanup = Vec::new();
        cleanup.extend_from_slice(b"\x1b[?2026l"); // ESU
        cleanup.extend_from_slice(b"\x1b[?25h");   // Show Cursor
        cleanup.extend_from_slice(b"\x1b[?1000l"); // Disable Standard Mouse Tracking
        cleanup.extend_from_slice(b"\x1b[?1003l"); // Disable Any-event Mouse Tracking
        cleanup.extend_from_slice(b"\x1b[?1006l"); // Disable SGR Mouse Mode
        cleanup.extend_from_slice(b"\x1b[=0u");     // Disable Kitty Keyboard Protocol
        cleanup.extend_from_slice(b"\x1b[?1049l"); // Disable Alt Buffer
        let _ = self.output_tx.try_send(cleanup);
    }
}

struct ChannelWriter(crate::backpressure::BoundedFrameChannel<Vec<u8>>);
impl std::io::Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.try_send(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Standard processing loop coordination.
pub struct EventLoop {
    /// Session model reference.
    pub session: Arc<ClientSession>,
    /// Global notification dispatcher.
    pub event_bus: Arc<crate::events::EventBus>,
    /// Current loaded document tree.
    pub doc: THTMLDocument,
    /// Rendering layout compiler.
    pub layout_engine: LayoutEngine,
    /// Local frame buffer cache.
    pub buffer: DoubleBuffer,
    /// Flag indicating if data output is paused.
    pub output_paused: bool,
    /// Bounded channel delivering final frame sequences to transport endpoints.
    pub output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
    /// Active frame receiver/renderer sink.
    pub frame_sink: Box<dyn oxiterm_renderer::FrameSink>,
    /// Framerate throttling manager.
    pub frame_limiter: crate::ratelimit::FrameRateLimiter,
    /// Pending mouse events map.
    pub pending_mouse: Option<oxiterm_proto::input::MouseInput>,
    /// Template file path supporting hot reloads.
    pub source_path: Option<std::path::PathBuf>,
    /// The non-mobile variant of the template path.
    pub base_source_path: Option<std::path::PathBuf>,
    /// dbus a11y coordination bridge.
    pub dbus_bridge: Option<oxiterm_a11y::DBusBridge>,
    /// Correlation table maps child nodes to parent IDs.
    pub parent_map: HashMap<NodeId, NodeId>,
    /// Active interactive nodes array.
    pub focusable_nodes: Vec<NodeId>,
    /// Focused input node identifier.
    pub focused_node: Option<NodeId>,
    /// Vertical scrolling offset.
    pub scroll_offset: u16,
    /// Computed document height.
    pub total_height: u16,
    /// Optional dispatcher for external app server notifications.
    pub app_dispatcher: Option<crate::dispatcher::AppDispatcher>,
    /// Template cache for For-node expansion. Maps For node id → original template child ids.
    /// MUST be cleared in update_doc, resolve_and_load_page, and Reload handlers because
    /// NodeIds are arena-local and become invalid after a document swap.
    pub for_template_cache: HashMap<NodeId, Vec<NodeId>>,
    /// Sender for 0x33 (open-URL) frames on WebSocket sessions.
    /// None for SSH sessions. Wired from handle_websocket via frame_tx.clone().
    pub open_url_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
}

impl EventLoop {
    fn resolve_non_mobile_path(&self, filename: &str) -> std::path::PathBuf {
        let base = self.base_source_path.as_ref().or(self.source_path.as_ref()).and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .or_else(|| self.session.app_base_dir.read().clone())
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
        let mut target_name = filename.to_string();
        if !target_name.ends_with(".thtml") {
            target_name.push_str(".thtml");
        }
        if target_name.contains("_mobile.thtml") {
            target_name = target_name.replace("_mobile.thtml", ".thtml");
        }
        base.join(target_name)
    }

    fn resolve_and_load_page(&mut self, target_filename: &str) -> anyhow::Result<()> {
        let non_mobile_target = self.resolve_non_mobile_path(target_filename);
        let is_mobile = self.session.is_mobile.load(std::sync::atomic::Ordering::SeqCst);
        let actual_path = crate::pathsafe::resolve_variant(&non_mobile_target, is_mobile);

        // Containment gate: reject path traversals and non-existent targets.
        // Q2: single generic error message — do not distinguish traversal from missing file.
        let app_base_dir = self.session.app_base_dir.read().clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
        if !crate::pathsafe::is_within_base(&app_base_dir, &actual_path) {
            warn!("resolve_and_load_page: blocked path traversal or missing file for {:?}", actual_path);
            anyhow::bail!("blocked");
        }

        let mut new_doc = crate::loader::load_thtml_file(&actual_path)?;
        let mut state = self.session.state.write();
        Self::setup_state_subscriptions(&new_doc, &mut *state);
        Self::inject_initial_state(&mut new_doc, &*state);
        drop(state);

        // update_doc clears for_template_cache (NodeIds are arena-local)
        self.update_doc(new_doc);
        self.reset_layout_and_scroll();

        self.source_path = Some(actual_path);
        self.base_source_path = Some(non_mobile_target.clone());

        let rel_path = if let Ok(rel) = non_mobile_target.strip_prefix(&app_base_dir) {
            rel.to_string_lossy().into_owned()
        } else {
            non_mobile_target.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
        };
        *self.session.current_page.write() = Some(rel_path);
        Ok(())
    }

    /// Creates a new `EventLoop` with clean session parameters.
    pub fn new(
        session: Arc<ClientSession>, 
        event_bus: Arc<crate::events::EventBus>, 
        output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
        doc: THTMLDocument,
        a11y_mode: bool,
    ) -> Self {
        let dims = *session.dims.read();
        
        let mut dbus_bridge = None;
        let frame_sink: Box<dyn oxiterm_renderer::FrameSink> = if a11y_mode {
            let writer = ChannelWriter(output_tx.clone());
            let mut bridge = oxiterm_a11y::DBusBridge::new();
            if let Ok(addr) = oxiterm_a11y::DBusBridge::read_dbus_address() {
                let _ = bridge.open_tunnel(std::path::Path::new("/tmp/dbus-local"), std::path::Path::new(&addr));
            }
            dbus_bridge = Some(bridge);
            Box::new(oxiterm_a11y::LinearFrameSink::new(writer))
        } else {
            Box::new(AnsiFrameSink::new(output_tx.clone()))
        };
        
        let mut event_loop = Self { 
            session, 
            event_bus,
            doc,
            layout_engine: LayoutEngine::new(),
            buffer: DoubleBuffer::new(dims.cols, dims.rows),
            output_paused: false,
            output_tx,
            frame_sink,
            frame_limiter: crate::ratelimit::FrameRateLimiter::new(60),
            pending_mouse: None,
            source_path: None,
            base_source_path: None,
            dbus_bridge,
            parent_map: HashMap::new(),
            focusable_nodes: Vec::new(),
            focused_node: None,
            scroll_offset: 0,
            total_height: 0,
            app_dispatcher: None,
            for_template_cache: HashMap::new(),
            open_url_tx: None,
        };
        event_loop.rebuild_parent_map();
        event_loop.rebuild_focusable_nodes();
        event_loop
    }

    /// Resets all compiled nodes and resets scroll margins.
    pub fn reset_layout_and_scroll(&mut self) {
        self.layout_engine.reset_nodes();
        self.scroll_offset = 0;
    }

    /// Re-evaluates parent associations for all nodes.
    pub fn rebuild_parent_map(&mut self) {
        let mut parent_map = HashMap::new();
        let mut stack = vec![self.doc.root];
        while let Some(parent_id) = stack.pop() {
            if let Some(node) = self.doc.arena.get(parent_id) {
                for &child_id in &node.children {
                    parent_map.insert(child_id, parent_id);
                    stack.push(child_id);
                }
            }
        }
        self.parent_map = parent_map;
    }

    /// Updates active document node tree and updates mappings.
    ///
    /// Clears `for_template_cache` because NodeIds are arena-local and become invalid
    /// after a document swap.
    pub fn update_doc(&mut self, new_doc: THTMLDocument) {
        self.doc = new_doc;
        self.for_template_cache.clear();
        self.rebuild_parent_map();
        self.rebuild_focusable_nodes();
    }

    /// Resolves interactive elements respecting bind conditional rules.
    pub fn rebuild_focusable_nodes(&mut self) {
        use oxiterm_proto::dom::StateEvaluator;
        let state_guard = self.session.state.read();
        let mut nodes = Vec::new();
        let mut stack = vec![self.doc.root];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.doc.arena.get(id) {
                if let Some(ref cond) = node.attrs.bind_show {
                    if !state_guard.evaluate_bind_show(cond) {
                        continue;
                    }
                }

                let is_interactive = node.attrs.event_htmx.is_some()
                    || (node.tag == oxiterm_proto::dom::NodeTag::Input
                        && node.attrs.bind_value.is_some());
                if is_interactive {
                    nodes.push(id);
                }
                for &child in node.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        if let Some(focused) = self.focused_node {
            if !nodes.contains(&focused) {
                self.focused_node = if nodes.is_empty() { None } else { Some(nodes[0]) };
            }
        } else if !nodes.is_empty() {
            self.focused_node = Some(nodes[0]);
        }
        self.focusable_nodes = nodes;
    }

    fn setup_state_subscriptions(doc: &THTMLDocument, state: &mut crate::state::StateManager) {
        state.clear_subscriptions();
        for (id, node) in doc.arena.iter() {
            if let Some(key) = &node.attrs.bind_state {
                state.subscribe(key.clone(), id);
            }
            if let Some(key) = &node.attrs.bind_value {
                state.subscribe(key.clone(), id);
            }
        }
    }

    fn sync_dirty_state(doc: &mut THTMLDocument, state: &mut crate::state::StateManager) {
        let dirty_nodes = state.get_dirty_nodes();
        for node_id in dirty_nodes {
            let mut changed = false;
            if let Some(node) = doc.arena.get_mut(node_id) {
                if let Some(key) = &node.attrs.bind_state {
                    if let Some(val) = state.get(key) {
                        node.text = Some(val.to_string());
                        changed = true;
                    }
                }
                if let Some(key) = &node.attrs.bind_value {
                    if let Some(val) = state.get(key) {
                        node.text = Some(val.to_string());
                        changed = true;
                    }
                }
            }
            if changed {
                doc.mark_dirty(node_id);
            }
        }
    }

    pub fn inject_initial_state(doc: &mut THTMLDocument, state: &crate::state::StateManager) {
        let mut dirty = Vec::new();
        for (id, node) in doc.arena.iter_mut() {
            if let Some(key) = &node.attrs.bind_state {
                if let Some(val) = state.get(key) {
                    node.text = Some(val.to_string());
                    dirty.push(id);
                }
            }
            if let Some(key) = &node.attrs.bind_value {
                if let Some(val) = state.get(key) {
                    node.text = Some(val.to_string());
                    dirty.push(id);
                }
            }
        }
        for id in dirty {
            doc.mark_dirty(id);
        }
    }

    fn get_htmx_node_and_target(&self, mut node_id: NodeId) -> Option<(NodeId, String)> {
        loop {
            if let Some(node) = self.doc.get_node(node_id) {
                if let Some(ref htmx) = node.attrs.event_htmx {
                    return Some((node_id, htmx.clone()));
                }
            }
            if node_id == self.doc.root {
                break;
            }
            if let Some(&parent_id) = self.parent_map.get(&node_id) {
                node_id = parent_id;
            } else {
                break;
            }
        }
        None
    }

    /// Handles a target HTMX action.
    ///
    /// Web-only open: targets strip 'open:' and emit 0x33 frames.
    /// Navigation targets (.thtml) resolve and load the page.
    /// Action targets set state keys and dispatch notifications.
    pub(crate) fn handle_htmx_target(&mut self, node_id: NodeId, target: &str) {
        if let Some(url) = target.strip_prefix("open:") {
            self.handle_open_url(url.to_string(), node_id);
        } else if target.ends_with(".thtml") {
            if let Err(e) = self.resolve_and_load_page(target) {
                warn!("handle_htmx_target: failed to load page {}: {}", target, e);
            }
        } else {
            self.session.state.write().apply_action(target);
            self.try_dispatch(target);
        }
    }

    /// Preprocesses a raw InputEvent before dispatch.
    ///
    /// - Remaps Page-Up / `b` → `ScrollUp` when input is not focused.
    /// - Remaps Page-Down / Space → `ScrollDown` when input is not focused.
    /// - Returns `None` to signal a quit request when `q`/`Q` is pressed outside an input.
    /// - All other events pass through unchanged.
    pub(crate) fn preprocess_key(
        event: InputEvent,
        is_input_focused: bool,
    ) -> Option<InputEvent> {
        match event {
            InputEvent::KeyPress(ref key) if !is_input_focused
                && (key.codepoint == '\u{F72E}' || key.codepoint == 'b') => {
                Some(InputEvent::ScrollUp)
            }
            InputEvent::KeyPress(ref key) if !is_input_focused
                && (key.codepoint == '\u{F72D}' || key.codepoint == ' ') => {
                Some(InputEvent::ScrollDown)
            }
            InputEvent::KeyPress(ref key) if !is_input_focused
                && (key.codepoint == 'q' || key.codepoint == 'Q') => {
                None // quit signal
            }
            e => Some(e),
        }
    }

    /// Handles an `open:` htmx target.
    ///
    /// Web sessions: emits a 0x33 frame containing the URL bytes via `open_url_tx`.
    /// SSH sessions: logs a warning and does nothing (`open:` is web-only in plan 2).
    ///
    /// Only `http` and `https` schemes are permitted; others are rejected with a warning.
    fn handle_open_url(&self, url: String, _source_node_id: NodeId) {
        let scheme_end = url.find(':').unwrap_or(0);
        let scheme = &url[..scheme_end];
        if scheme != "http" && scheme != "https" {
            warn!("open: rejected URL with disallowed scheme '{}': {}", scheme, url);
            return;
        }

        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some(ref tx) = self.open_url_tx {
                let mut frame = vec![0x33];
                frame.extend_from_slice(url.as_bytes());
                let _ = tx.try_send(frame);
            }
        } else {
            // SSH sessions: open: is WEB-ONLY in plan 2.
            // Plan 3 will add OSC 8 cell-level hyperlinks.
            warn!("open: action '{}' on SSH session — not implemented (web-only in plan 2)", url);
        }
    }

    /// Expands all For nodes in the current document against their List state values.
    ///
    /// Called at session start and after every state mutation that may affect a For node.
    pub(crate) fn expand_all_for_nodes(&mut self) {
        let for_ids: Vec<NodeId> = self.doc.arena.iter()
            .filter(|(_, n)| n.tag == oxiterm_proto::dom::NodeTag::For)
            .map(|(id, _)| id)
            .collect();

        for for_id in for_ids {
            let each_key = self.doc.arena.get(for_id)
                .and_then(|n| n.attrs.each.as_deref());
            if let Some(key) = each_key {
                let list = match self.session.state.read().get(&key) {
                    Some(crate::state::StateValue::List(l)) => l.clone(),
                    _ => Vec::new(),
                };
                if let Err(e) = crate::expand::expand_for_node(
                    &mut self.doc, for_id, &list, &mut self.for_template_cache
                ) {
                    tracing::warn!("expand_all_for_nodes: expand_for_node failed for {:?}: {}", for_id, e);
                }
            }
        }
    }

    fn try_dispatch(&self, action: &str) {
        if let Some(ref dispatcher) = self.app_dispatcher {
            let state_guard = self.session.state.read();
            let mut state_snapshot = std::collections::HashMap::new();
            for (_id, node) in self.doc.arena.iter() {
                if let Some(ref key) = node.attrs.bind_state {
                    if let Some(val) = state_guard.get(key) {
                        state_snapshot.insert(key.clone(), val.to_string());
                    }
                }
                if let Some(ref key) = node.attrs.bind_value {
                    if let Some(val) = state_guard.get(key) {
                        state_snapshot.insert(key.clone(), val.to_string());
                    }
                }
                if let Some(ref key) = node.attrs.bind_show {
                    let key_part = key.split('=').next().unwrap_or(key);
                    if let Some(val) = state_guard.get(key_part) {
                        state_snapshot.insert(key_part.to_string(), val.to_string());
                    }
                }
            }
            drop(state_guard);
            let (username, auth_method) = {
                let id = self.session.identity.read();
                match id.as_ref() {
                    Some(i) => (Some(i.username.clone()), Some(format!("{:?}", i.auth_method))),
                    None => (None, None),
                }
            };
            let payload = crate::dispatcher::DispatchPayload {
                action: action.to_string(),
                state: state_snapshot,
                session_id: self.session.id,
                username,
                auth_method,
            };
            dispatcher.dispatch(payload, self.session.clone());
        }
    }

    /// Starts the main event loop thread driving keyboard and mouse coordinate reactions.
    pub fn run(&mut self) {
        {
            let mut state = self.session.state.write();
            Self::setup_state_subscriptions(&self.doc, &mut *state);
            Self::inject_initial_state(&mut self.doc, &*state);
        }
        // Expand For-template nodes after initial state injection.
        self.expand_all_for_nodes();

        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            let app_base_dir = self.session.app_base_dir.read().clone().unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")));
            let rel_path = self.base_source_path.as_ref().map(|bp| {
                if let Ok(rel) = bp.strip_prefix(&app_base_dir) {
                    rel.to_string_lossy().into_owned()
                } else {
                    bp.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
                }
            });
            *self.session.current_page.write() = rel_path;
        }

        let _ = self.frame_sink.setup();
        
        let mut first_frame = true;
        let mut pending_render = false;
        loop {
            // Anchored by spec [SA-10]. Perform idle timeout check (10 minutes) and terminate the session if exceeded.
            let elapsed = self.session.last_activity.read().elapsed();
            if elapsed > std::time::Duration::from_secs(600) {
                info!("Session {} timed out ({}s idle)", self.session.id, elapsed.as_secs());
                return;
            }

            let mut needs_render = first_frame || pending_render;
            first_frame = false;
            pending_render = false;

            let has_animations = self.has_active_animations();
            let sleep_dur = if has_animations {
                std::time::Duration::from_millis(66)
            } else {
                std::time::Duration::from_millis(5)
            };

            let mut disconnected = false;
            {
                let session = self.session.clone();
                let mut rx_lock = session.event_rx.lock();
                let mut processed_count = 0;
                loop {
                    if processed_count >= 100 {
                        pending_render = true;
                        break;
                    }

                    let event_opt = if processed_count == 0 {
                        match rx_lock.recv_timeout(sleep_dur) {
                            Ok(e) => Some(e),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                if has_animations {
                                    needs_render = true;
                                }
                                break;
                            }
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                                info!("Event loop receiver disconnected. Exiting run loop.");
                                disconnected = true;
                                break;
                            }
                        }
                    } else {
                        rx_lock.try_recv()
                    };

                    let event = match event_opt {
                        Some(e) => e,
                        None => break,
                    };

                    processed_count += 1;

                    *self.session.last_activity.write() = std::time::Instant::now();
                    let is_input_focused = self.focused_node
                        .and_then(|id| self.doc.get_node(id))
                        .map(|node| node.tag == oxiterm_proto::dom::NodeTag::Input)
                        .unwrap_or(false);
                    // Preprocess: remap scroll keys and detect quit. Returns None on quit.
                    let event = match Self::preprocess_key(event, is_input_focused) {
                        Some(e) => e,
                        None => {
                            info!("Quit requested via keyboard");
                            disconnected = true;
                            break;
                        }
                    };
                    match event {
                                InputEvent::StatePatched => {
                                    self.expand_all_for_nodes();
                                    needs_render = true;
                                }
                                InputEvent::Resize { cols, rows } => {
                                    info!("Reactor notified resize to {}x{}", cols, rows);
                                    needs_render = true;
                                }
                                InputEvent::SwitchViewport(is_mobile) => {
                                    self.session.is_mobile.store(is_mobile, std::sync::atomic::Ordering::SeqCst);
                                    if let Some(bp) = self.base_source_path.clone() {
                                        let file_name = bp.file_name().and_then(|f| f.to_str()).unwrap_or("index.thtml").to_string();
                                        if let Err(e) = self.resolve_and_load_page(&file_name) {
                                            warn!("SwitchViewport: failed to load page: {}", e);
                                        }
                                        needs_render = true;
                                    }
                                }
                                InputEvent::NavigateTo(rel_path) => {
                                    // Containment check is inside resolve_and_load_page.
                                    if let Err(e) = self.resolve_and_load_page(&rel_path) {
                                        warn!("NavigateTo: failed to load page: {}", e);
                                    }
                                    needs_render = true;
                                }
                                InputEvent::Reload => {
                                    if let Some(ref path) = self.source_path {
                                        match crate::loader::load_thtml_file(path) {
                                            Ok(mut new_doc) => {
                                                info!("Hot Reload successful for session {}", self.session.id);
                                                let mut state = self.session.state.write();
                                                Self::setup_state_subscriptions(&new_doc, &mut *state);
                                                Self::inject_initial_state(&mut new_doc, &*state);
                                                drop(state);
                                                self.update_doc(new_doc);
                                                self.reset_layout_and_scroll();
                                                needs_render = true;
                                            }
                                            Err(e) => {
                                                warn!("Hot Reload failed for session {}: {}", self.session.id, e);
                                            }
                                        }
                                    }
                                }
                                InputEvent::KeyPress(key) => {

                                    let is_nav_forward = key.codepoint == '\u{F701}'
                                        || key.codepoint == '\u{F702}'
                                        || key.codepoint == '\t';
                                    let is_nav_backward = key.codepoint == '\u{F700}'
                                        || key.codepoint == '\u{F703}';

                                    if is_nav_forward || is_nav_backward {
                                        if !self.focusable_nodes.is_empty() {
                                            let n = self.focusable_nodes.len();
                                            let cur = self.focused_node
                                                .and_then(|f| self.focusable_nodes.iter().position(|&x| x == f))
                                                .unwrap_or(0);
                                            let next = if is_nav_forward {
                                                (cur + 1) % n
                                            } else {
                                                (cur + n - 1) % n
                                            };
                                            self.focused_node = Some(self.focusable_nodes[next]);
                                            info!("Focus moved to node {:?}", self.focused_node);
                                            needs_render = true;
                                        }
                                    } else if key.codepoint == '\r' || key.codepoint as u32 == 13 {
                                        if let Some(focused) = self.focused_node {
                                            if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(focused) {
                                                info!("Enter activated HTMX (node {:?}): {}", target_node_id, htmx_target);
                                                self.handle_htmx_target(target_node_id, &htmx_target);
                                                needs_render = true;
                                            }
                                        }
                                    } else {
                                        let handled_by_input = if let Some(focused_id) = self.focused_node {
                                            if let Some(node) = self.doc.get_node(focused_id) {
                                                if node.tag == oxiterm_proto::dom::NodeTag::Input {
                                                    if let Some(ref state_key) = node.attrs.bind_value.clone() {
                                                        let cp = key.codepoint;
                                                        if cp as u32 == 127 || cp as u32 == 8 {
                                                            let current = match self.session.state.read().get(state_key) {
                                                                Some(crate::state::StateValue::Str(s)) => s.clone(),
                                                                _ => String::new(),
                                                            };
                                                            let mut new_val = current;
                                                            new_val.pop();
                                                            self.session.state.write().set(
                                                                state_key.clone(),
                                                                crate::state::StateValue::Str(new_val),
                                                            );
                                                            needs_render = true;
                                                            true
                                                        } else if cp == '\r' || cp as u32 == 13 {
                                                            false
                                                        } else if !cp.is_control() {
                                                            let current = match self.session.state.read().get(state_key) {
                                                                Some(crate::state::StateValue::Str(s)) => s.clone(),
                                                                _ => String::new(),
                                                            };
                                                            let mut new_val = current;
                                                            new_val.push(cp);
                                                            self.session.state.write().set(
                                                                state_key.clone(),
                                                                crate::state::StateValue::Str(new_val),
                                                            );
                                                            needs_render = true;
                                                            true
                                                        } else {
                                                            false
                                                        }
                                                    } else { false }
                                                } else { false }
                                            } else { false }
                                        } else { false };

                                        if !handled_by_input {
                                            // D7: suppress predictive echo for password inputs
                                            let is_password_focused = self.focused_node
                                                .and_then(|id| self.doc.get_node(id))
                                                .and_then(|n| n.attrs.input_type.as_deref().map(|t| t == "password"))
                                                .unwrap_or(false);
                                            if !is_password_focused {
                                                let mut echo = self.session.predictive_echo.write();
                                                echo.buffer.push(key.codepoint);
                                            }
                                            needs_render = true;
                                        }
                                    }
                                }
                                InputEvent::MouseEvent(mut mouse) => {
                                    let dims = *self.session.dims.read();
                                    tracing::trace!("Received MouseEvent: col={}, row={}, action={:?}", mouse.col, mouse.row, mouse.action);
                                    if let Some(layout) = &self.layout_engine.last_layout {
                                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
                                        tracing::trace!("Document centering offset: offset_x={}, offset_y={}", offset_x, offset_y);
                                        mouse.col = mouse.col.saturating_sub(offset_x).saturating_sub(1);
                                        mouse.row = mouse.row.saturating_sub(offset_y).saturating_sub(1).saturating_add(self.scroll_offset);
                                        tracing::trace!("Adjusted MouseEvent: col={}, row={}", mouse.col, mouse.row);
                                    } else {
                                        warn!("No last layout found!");
                                    }
                                    
                                    self.update_interactive_animations(&mouse);

                                    self.pending_mouse = Some(mouse.clone());
                                    
                                    // Anchored by spec [SC-05]. Handle interactive navigation or state updates based on htmx event targets.
                                    if mouse.action == oxiterm_proto::input::MouseAction::Press {
                                        if let Some(node_id) = self.layout_engine.hit_test(mouse.col, mouse.row) {
                                            tracing::trace!("Hit test found node: {:?}", node_id);
                                            if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(node_id) {
                                                tracing::trace!("Found HTMX target (node {:?}): {}", target_node_id, htmx_target);
                                                self.handle_htmx_target(target_node_id, &htmx_target);
                                            } else {
                                                tracing::trace!("No HTMX target found for node {:?}", node_id);
                                            }
                                        } else {
                                            info!("Hit test returned None");
                                        }
                                    }

                                    needs_render = true;
                                }
                                InputEvent::CapabilityResponse(raw) => {
                                    info!("Received DA1 response: {}", String::from_utf8_lossy(&raw));
                                    self.session.terminal_profile.write().parse_da1_response(&raw);
                                }
                                InputEvent::Xoff => { self.output_paused = true; }
                                InputEvent::Xon => { self.output_paused = false; needs_render = true; }
                                InputEvent::Refresh => { needs_render = true; }
                                InputEvent::ScrollUp => {
                                    let dims = *self.session.dims.read();
                                    let has_scroll = self.total_height > dims.rows;
                                    let viewport_h = if has_scroll {
                                        dims.rows.saturating_sub(1)
                                    } else {
                                        dims.rows
                                    };
                                    self.scroll_offset = self.scroll_offset.saturating_sub(viewport_h);
                                    needs_render = true;
                                }
                                InputEvent::ScrollDown => {
                                    let dims = *self.session.dims.read();
                                    let has_scroll = self.total_height > dims.rows;
                                    let viewport_h = if has_scroll {
                                        dims.rows.saturating_sub(1)
                                    } else {
                                        dims.rows
                                    };
                                    let max_scroll = self.total_height.saturating_sub(viewport_h);
                                    self.scroll_offset = (self.scroll_offset + viewport_h).min(max_scroll);
                                    needs_render = true;
                                }
                                InputEvent::TextInput(text) => {
                                    info!("TextInput received (len={}): {:?}", text.len(), text);
                                }
                                _ => {}
                            }
                }
            }

            if disconnected {
                return;
            }

            let dims_opt = self.session.resize_debouncer.write().poll();
            if let Some(dims) = dims_opt {
                *self.session.dims.write() = dims;
                self.buffer = DoubleBuffer::new(dims.cols, dims.rows);
                info!("Resized session {} to {:?}", self.session.id, dims);

                let _ = self.frame_sink.clear_screen();

                needs_render = true;
            }

            if needs_render && !self.output_paused {
                if self.frame_limiter.should_render() {
                    Self::sync_dirty_state(&mut self.doc, &mut *self.session.state.write());
                    self.rebuild_focusable_nodes();

                    let dims = *self.session.dims.read();
                    let state_guard = self.session.state.read();
                    if let Ok(layout) = self.layout_engine.compute(&mut self.doc, dims.cols, self.scroll_offset, Some(&*state_guard)) {
                    self.total_height = layout.total_height;

                    let has_scroll = self.total_height > dims.rows;
                    let viewport_h = if has_scroll {
                        dims.rows.saturating_sub(1)
                    } else {
                        dims.rows
                    };

                    // Auto-scroll the viewport to keep the focused element fully visible inside layout boundaries.
                    if let Some(focused_id) = self.focused_node {
                        if let Some(rect) = layout.nodes.get(&focused_id) {
                            if rect.y < self.scroll_offset {
                                self.scroll_offset = rect.y;
                            } else if rect.y + rect.height > self.scroll_offset + viewport_h {
                                self.scroll_offset = (rect.y + rect.height).saturating_sub(viewport_h);
                            }
                        }
                    }

                    let max_scroll = self.total_height.saturating_sub(viewport_h);
                    self.scroll_offset = self.scroll_offset.min(max_scroll);

                    {
                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
                        let start_x = offset_x as i32;
                        let start_y = (offset_y as i32) - (self.scroll_offset as i32);

                        let mut active = Vec::new();
                        let mut stack = vec![(self.doc.root, start_x, start_y)];
                        while let Some((node_id, parent_x, parent_y)) = stack.pop() {
                            if let Some(node) = self.doc.arena.get(node_id) {
                                let rect = layout.nodes.get(&node_id).copied().unwrap_or_default();
                                let abs_x = parent_x + rect.x as i32;
                                let abs_y = parent_y + rect.y as i32;
                                
                                let has_border = node.style.border.is_some();
                                let content_x = if has_border { abs_x + 1 } else { abs_x };
                                let content_y = if has_border { abs_y + 1 } else { abs_y };
                                let content_w = if has_border { rect.width.saturating_sub(2) } else { rect.width };
                                let content_h = if has_border { rect.height.saturating_sub(2) } else { rect.height };

                                if node.tag == oxiterm_proto::dom::NodeTag::Img || node.tag == oxiterm_proto::dom::NodeTag::Video {
                                    if let Some(ref src) = node.attrs.src {
                                        active.push(MediaRenderInfo {
                                            path: src.clone(),
                                            x: content_x.max(0) as u16,
                                            y: content_y.max(0) as u16,
                                            width: content_w,
                                            height: content_h,
                                        });
                                    }
                                }
                                
                                for &child in &node.children {
                                    stack.push((child, parent_x, parent_y));
                                }
                            }
                        }
                        *self.session.active_media.write() = active;
                    }

                    if let Some(ref mut bridge) = self.dbus_bridge {
                        let tree = oxiterm_a11y::build_a11y_tree(&self.doc);
                        let _ = bridge.register_at_spi(&tree);
                        if let Some(active_id) = self.session.predictive_echo.read().active_node {
                            let _ = bridge.update_focus(active_id, &tree);
                        }
                    }

                    // Anchored by spec [QUAL-01]. Dispatch deferred mouse inputs once a valid layout is available.
                    if let Some(mouse) = self.pending_mouse.take() {
                        let _ = self.event_bus.dispatch_mouse(mouse, &mut self.doc, &layout);
                    }

                    let mut profile = self.session.terminal_profile.read().clone();
                    profile.is_web = self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst);
                    let base_dir = self.source_path.as_ref().and_then(|p| p.parent());
                    self.buffer.back.clear();
                    Renderer::render_node(&self.doc, &layout, &mut self.buffer.back, &profile, base_dir, self.scroll_offset);

                    // Focus ring: draw markers on left/right edge of focused node
                    if let Some(focused_id) = self.focused_node {
                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
                        if let Some(rect) = layout.nodes.get(&focused_id) {
                            let mid_row_offset = (rect.y + rect.height / 2 + offset_y) as i32 - self.scroll_offset as i32;
                            if mid_row_offset >= 0 && mid_row_offset < viewport_h as i32 {
                                let mid_row = mid_row_offset as u16;
                                let focus_fg = oxiterm_proto::style::AnsiColor::Color256(51); // bright cyan
                                let focus_bg = oxiterm_proto::style::AnsiColor::Reset;
                                if rect.x > 0 {
                                    let lx = rect.x.saturating_sub(1) + offset_x;
                                    if lx < self.buffer.back.width {
                                        self.buffer.back.set(lx, mid_row, oxiterm_renderer::render::buffer::Cell {
                                            ch: '▶',
                                            fg: focus_fg,
                                            bg: focus_bg,
                                            bold: true,
                                            ..Default::default()
                                        });
                                    }
                                }
                                let right = rect.x + rect.width + offset_x;
                                if right < self.buffer.back.width {
                                    self.buffer.back.set(right, mid_row, oxiterm_renderer::render::buffer::Cell {
                                        ch: '◀',
                                        fg: focus_fg,
                                        bg: focus_bg,
                                        bold: true,
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }

                    // S5-21: PredictiveEcho overlay
                    // D7: suppress overlay for focused password inputs
                    let echo_suppressed = self.focused_node
                        .and_then(|id| self.doc.get_node(id))
                        .and_then(|n| n.attrs.input_type.as_deref().map(|t| t == "password"))
                        .unwrap_or(false);
                    let echo = self.session.predictive_echo.read();
                    if !echo.buffer.is_empty() && !echo_suppressed {
                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);

                        let (mut cursor_x, mut cursor_y) = (offset_x, offset_y.saturating_sub(self.scroll_offset));
                        if let Some(node_id) = echo.active_node {
                            if let Some(rect) = layout.nodes.get(&node_id) {
                                cursor_x = rect.x + offset_x;
                                cursor_y = (rect.y + offset_y).saturating_sub(self.scroll_offset);
                            }
                        }

                        if cursor_y < viewport_h {
                            for ch in echo.buffer.chars() {
                                if cursor_x < self.buffer.back.width {
                                    self.buffer.back.set(cursor_x, cursor_y, oxiterm_renderer::render::buffer::Cell {
                                        ch,
                                        fg: oxiterm_proto::style::AnsiColor::Color256(2),
                                        ..Default::default()
                                    });
                                    cursor_x += 1;
                                }
                            }
                        }
                    }

                    // Renders a visual scroll progress indicator at the bottom of the viewport if content exceeds terminal rows.
                    if self.total_height > dims.rows {
                        let last_row = dims.rows.saturating_sub(1);
                        let mut status_parts = Vec::new();
                        
                        if self.scroll_offset > 0 {
                            status_parts.push("▲ PgUp".to_string());
                        }
                        if self.scroll_offset < self.total_height.saturating_sub(viewport_h) {
                            status_parts.push("▼ PgDn".to_string());
                        }
                        
                        let current_row = self.scroll_offset + 1;
                        status_parts.push(format!("line {}/{}", current_row, self.total_height));
                        
                        let max_scroll = self.total_height.saturating_sub(viewport_h);
                        let pct = if max_scroll > 0 {
                            (self.scroll_offset as f32 / max_scroll as f32 * 100.0).round() as u8
                        } else {
                            0
                        };
                        
                        let bar_width = 8;
                        let filled_width = if max_scroll > 0 {
                            (self.scroll_offset as usize * bar_width) / max_scroll as usize
                        } else {
                            0
                        };
                        let filled_width = filled_width.min(bar_width);
                        let bar: String = std::iter::repeat('█').take(filled_width)
                            .chain(std::iter::repeat('░').take(bar_width - filled_width))
                            .collect();
                            
                        status_parts.push(format!("{} {}%", bar, pct));
                        
                        let status_str = status_parts.join("  ");
                        let bar_bg = oxiterm_proto::style::AnsiColor::Color256(236); // dark grey
                        let bar_fg = oxiterm_proto::style::AnsiColor::Color256(255); // white
                        
                        for x in 0..dims.cols {
                            self.buffer.back.set(x, last_row, oxiterm_renderer::render::buffer::Cell {
                                ch: ' ',
                                fg: bar_fg,
                                bg: bar_bg,
                                ..Default::default()
                            });
                        }
                        
                        let start_x = if dims.cols > status_str.chars().count() as u16 {
                            (dims.cols - status_str.chars().count() as u16) / 2
                        } else {
                            0
                        };
                        
                        for (i, ch) in status_str.chars().enumerate() {
                            let x = start_x + i as u16;
                            if x < dims.cols {
                                self.buffer.back.set(x, last_row, oxiterm_renderer::render::buffer::Cell {
                                    ch,
                                    fg: bar_fg,
                                    bg: bar_bg,
                                    bold: true,
                                    ..Default::default()
                                });
                            }
                        }
                    }
                    
                    let _ = self.frame_sink.update_document(&self.doc);
                    if let Ok(true) = self.frame_sink.send_frame(&self.buffer.front, &self.buffer.back) {
                        self.buffer.swap();
                        self.frame_limiter.record_frame();
                    }
                }
                } else {
                    pending_render = true;
                }
            }
        }
    }

    fn has_active_animations(&self) -> bool {
        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        if let Some(ref layout) = self.layout_engine.last_layout {
            for node_id in layout.nodes.keys() {
                if let Some(node) = self.doc.get_node(*node_id) {
                    if node.tag == oxiterm_proto::dom::NodeTag::Img {
                        if let Some(ref src) = node.attrs.src {
                            if src.ends_with(".json") || src.ends_with(".riv") {
                                return true;
                            }
                        }
                    } else if node.tag == oxiterm_proto::dom::NodeTag::Video {
                        if node.attrs.src.is_some() {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    fn update_interactive_animations(&self, mouse: &oxiterm_proto::input::MouseInput) {
        if let Some(ref layout) = self.layout_engine.last_layout {
            for (node_id, rect) in &layout.nodes {
                if let Some(node) = self.doc.get_node(*node_id) {
                    if node.tag == oxiterm_proto::dom::NodeTag::Img {
                        if let Some(ref src) = node.attrs.src {
                            if src.ends_with(".riv") {
                                let is_inside = mouse.col >= rect.x
                                    && mouse.col < rect.x + rect.width
                                    && mouse.row >= rect.y
                                    && mouse.row < rect.y + rect.height;
                                
                                let resolved_path = if let Some(ref base) = self.source_path.as_ref().and_then(|p| p.parent()) {
                                    base.join(src)
                                } else {
                                    std::path::PathBuf::from(src)
                                };
                                
                                let registry = oxiterm_renderer::render::cache::PlaybackRegistry::get();
                                registry.set_hover(&resolved_path, is_inside);
                                if is_inside && mouse.action == oxiterm_proto::input::MouseAction::Press {
                                    registry.set_click(&resolved_path, true, Some((mouse.col - rect.x, mouse.row - rect.y)));
                                } else if mouse.action == oxiterm_proto::input::MouseAction::Release {
                                    registry.set_click(&resolved_path, false, None);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_renderer::render::buffer::CellBuffer;
    use oxiterm_renderer::FrameSink;

    #[test]
    fn test_predictive_echo() {
        let mut echo = PredictiveEcho::default();
        echo.push('a');
        echo.push('b');
        assert_eq!(echo.buffer, "ab");
        echo.buffer.clear();
        assert_eq!(echo.buffer, "");
    }

    #[test]
    fn test_ansi_frame_sink_kitty_ghost_fix() {
        let (tx, mut rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let mut sink = AnsiFrameSink::new(tx);

        let mut front = CellBuffer::new(10, 10);
        let mut back = CellBuffer::new(10, 10);

        let res = sink.send_frame(&front, &back).unwrap();
        assert!(!res);

        front.graphics.push(b"some_img".to_vec());
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(res);
        let frame = rx.try_recv().expect("Should receive frame");
        let delete_seq = b"\x1b_Ga=d,d=A\x1b\\";
        let seq_pos = frame.windows(delete_seq.len()).position(|w| w == delete_seq);
        assert!(seq_pos.is_some(), "delete_all_placements sequence not found in frame");

        front.graphics.clear();
        back.graphics.push(b"new_img_data".to_vec());
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(res);
        let frame2 = rx.try_recv().expect("Should receive frame 2");
        let seq_pos2 = frame2.windows(delete_seq.len()).position(|w| w == delete_seq);
        assert!(seq_pos2.is_none(), "delete_all_placements should not be sent on transition to graphics");
        let img_pos = frame2.windows(12).position(|w| w == b"new_img_data");
        assert!(img_pos.is_some(), "new_img_data not found in frame");

        sink.clear_screen().unwrap();
        let frame3 = rx.try_recv().expect("Should receive clear screen");
        let seq_pos3 = frame3.windows(delete_seq.len()).position(|w| w == delete_seq);
        let clear_pos = frame3.windows(4).position(|w| w == b"\x1b[2J");
        assert!(seq_pos3.is_some(), "delete_all_placements not found in clear_screen");
        assert!(clear_pos.is_some(), "ESC[2J not found in clear_screen");
        assert!(seq_pos3.unwrap() < clear_pos.unwrap(), "delete_all_placements must be before ESC[2J");
    }

    #[test]
    fn test_rebuild_focusable_nodes_respects_bind_show() {
        use oxiterm_proto::dom::{Node, NodeTag};
        
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        
        let mut node2 = Node::new(NodeTag::Box);
        node2.attrs.event_htmx = Some("click_action".to_string());
        node2.attrs.bind_show = Some("show_node2".to_string());
        let id2 = arena.alloc(node2);
        
        let mut node3 = Node::new(NodeTag::Box);
        node3.attrs.event_htmx = Some("click_action2".to_string());
        node3.attrs.bind_show = Some("show_node3".to_string());
        let id3 = arena.alloc(node3);
        
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![id2, id3];
        let root_id = arena.alloc(root);
        
        let doc = THTMLDocument {
            arena,
            root: root_id,
            dirty_nodes: Vec::new(),
        };
        
        let mut event_loop = EventLoop::new(
            client_session,
            event_bus,
            output_tx,
            doc,
            false,
        );
        
        event_loop.rebuild_focusable_nodes();
        assert!(!event_loop.focusable_nodes.contains(&id2));
        assert!(!event_loop.focusable_nodes.contains(&id3));

        {
            let mut state = event_loop.session.state.write();
            state.set("show_node2".to_string(), crate::state::StateValue::Bool(true));
            state.set("show_node3".to_string(), crate::state::StateValue::Bool(false));
        }
        
        event_loop.rebuild_focusable_nodes();
        assert!(event_loop.focusable_nodes.contains(&id2));
        assert!(!event_loop.focusable_nodes.contains(&id3));
        
        {
            let mut state = event_loop.session.state.write();
            state.set("show_node2".to_string(), crate::state::StateValue::Bool(false));
            state.set("show_node3".to_string(), crate::state::StateValue::Bool(true));
        }
        
        event_loop.rebuild_focusable_nodes();
        assert!(!event_loop.focusable_nodes.contains(&id2));
        assert!(event_loop.focusable_nodes.contains(&id3));
    }

    #[test]
    fn test_apply_state_patch() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let patch = serde_json::json!({
            "count": 42,
            "name": "OxiTerm",
            "active": true,
            "list": ["a", "b", "c"],
            "empty": null
        });
        
        client_session.apply_state_patch(patch);
        
        let state = client_session.state.read();
        assert_eq!(state.get("count"), Some(&crate::state::StateValue::Int(42)));
        assert_eq!(state.get("name"), Some(&crate::state::StateValue::Str("OxiTerm".to_string())));
        assert_eq!(state.get("active"), Some(&crate::state::StateValue::Bool(true)));
        assert_eq!(state.get("list"), Some(&crate::state::StateValue::List(vec!["a".to_string(), "b".to_string(), "c".to_string()])));
        assert_eq!(state.get("empty"), Some(&crate::state::StateValue::Str(String::new())));
    }

    #[test]
    fn test_sec_state_patch_limits() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let mut huge_patch = serde_json::Map::new();
        for i in 0..105 {
            huge_patch.insert(format!("key{}", i), serde_json::json!(i));
        }
        client_session.apply_state_patch(serde_json::Value::Object(huge_patch));
        assert!(client_session.state.read().get("key0").is_none());

        let long_key = "a".repeat(257);
        let patch_key_too_long = serde_json::json!({
            long_key.clone(): 42
        });
        client_session.apply_state_patch(patch_key_too_long);
        assert!(client_session.state.read().get(&long_key).is_none());

        let long_val = "x".repeat(65537);
        let patch_val_too_long = serde_json::json!({
            "valid_key": long_val
        });
        client_session.apply_state_patch(patch_val_too_long);
        assert!(client_session.state.read().get("valid_key").is_none());

        let huge_arr: Vec<i32> = (0..1005).collect();
        let patch_arr_too_long = serde_json::json!({
            "arr_key": huge_arr
        });
        client_session.apply_state_patch(patch_arr_too_long);
        assert!(client_session.state.read().get("arr_key").is_none());
    }


    #[test]
    fn test_14_switch_viewport_event_loop() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let temp_dir = std::env::temp_dir();
        let app_file = temp_dir.join("index_t14.thtml");
        let mobile_file = temp_dir.join("index_t14_mobile.thtml");
        std::fs::write(&app_file, b"<screen>Desktop</screen>").unwrap();
        std::fs::write(&mobile_file, b"<screen>Mobile</screen>").unwrap();

        let doc = crate::loader::load_thtml_file(&app_file).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut event_loop = EventLoop::new(client_session.clone(), event_bus, output_tx, doc, false);
        event_loop.source_path = Some(app_file.clone());
        event_loop.base_source_path = Some(app_file.clone());
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        event_loop.session.is_mobile.store(true, std::sync::atomic::Ordering::SeqCst);
        event_loop.resolve_and_load_page("index_t14.thtml").unwrap();

        assert_eq!(event_loop.source_path, Some(mobile_file.clone()));
        assert_eq!(event_loop.base_source_path, Some(app_file.clone()));

        let _ = std::fs::remove_file(app_file);
        let _ = std::fs::remove_file(mobile_file);
    }

    #[test]
    fn test_15_navigate_to_event_loop() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let temp_dir = std::env::temp_dir();
        let app_file = temp_dir.join("index_t15.thtml");
        let target_file = temp_dir.join("about_t15.thtml");
        std::fs::write(&app_file, b"<screen>Home</screen>").unwrap();
        std::fs::write(&target_file, b"<screen>About</screen>").unwrap();

        let doc = crate::loader::load_thtml_file(&app_file).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut event_loop = EventLoop::new(client_session.clone(), event_bus, output_tx, doc, false);
        event_loop.source_path = Some(app_file.clone());
        event_loop.base_source_path = Some(app_file.clone());
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        event_loop.resolve_and_load_page("about_t15.thtml").unwrap();

        assert_eq!(event_loop.source_path, Some(target_file.clone()));
        assert_eq!(*client_session.current_page.read(), Some("about_t15.thtml".to_string()));

        let _ = std::fs::remove_file(app_file);
        let _ = std::fs::remove_file(target_file);
    }

    #[test]
    fn test_16_base_source_path_resolving() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        
        let temp_dir = std::env::temp_dir();
        let app_file = temp_dir.join("subdir").join("index_mobile.thtml");
        std::fs::create_dir_all(app_file.parent().unwrap()).unwrap();
        std::fs::write(&app_file, b"<screen></screen>").unwrap();

        let doc = crate::loader::load_thtml_file(&app_file).unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut event_loop = EventLoop::new(client_session.clone(), event_bus, output_tx, doc, false);
        event_loop.source_path = Some(app_file.clone());
        event_loop.base_source_path = Some(temp_dir.join("subdir").join("index.thtml"));
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        let resolved = event_loop.resolve_non_mobile_path("about");
        assert_eq!(resolved, temp_dir.join("subdir").join("about.thtml"));

        let _ = std::fs::remove_file(app_file);
    }

    #[test]
    fn test_17_session_registry_reattach() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let token = session.token.clone();

        let reattached = reg.reattach(&token).unwrap();
        assert_eq!(reattached.id, session.id);
    }

    #[test]
    fn test_18_reopen_channel() {
        let (tx, mut rx) = crate::backpressure::BoundedFrameChannel::<i32>::new(5);
        assert_eq!(tx.try_send(10), crate::backpressure::SendResult::Sent);
        assert_eq!(tx.try_send(20), crate::backpressure::SendResult::Sent);
        
        rx.close();
        assert_eq!(tx.try_send(30), crate::backpressure::SendResult::Closed);
 
        rx.reopen();
        assert_eq!(tx.try_send(40), crate::backpressure::SendResult::Sent);
        assert_eq!(rx.blocking_recv().unwrap(), 40);
    }

    #[test]
    fn test_19_active_connections_reaper() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        session.active_connections.store(0, std::sync::atomic::Ordering::SeqCst);
        
        let ten_mins_ago = std::time::Instant::now() - std::time::Duration::from_secs(900);
        *session.last_activity.write() = ten_mins_ago;

        let sessions = reg.sessions.read();
        let mut to_remove = Vec::new();
        for (&id, s) in sessions.iter() {
            if s.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
                let active_conns = s.active_connections.load(std::sync::atomic::Ordering::SeqCst);
                let idle_time = s.last_activity.read().elapsed();
                if active_conns == 0 && idle_time > std::time::Duration::from_secs(600) {
                    to_remove.push(id);
                }
            }
        }
        assert_eq!(to_remove, vec![session.id]);
    }
    // --- Plan 2 tests ---

    #[test]
    fn test_1_resolve_load_blocks_traversal() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        let temp_dir = std::env::temp_dir();
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        let doc_file = temp_dir.join("dummy_t1.thtml");
        std::fs::write(&doc_file, b"<screen></screen>").unwrap();
        let doc = crate::loader::load_thtml_file(&doc_file).unwrap();

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut event_loop = EventLoop::new(client_session, event_bus, output_tx, doc, false);
        event_loop.base_source_path = Some(doc_file.clone());

        // Attempt path traversal: "../../etc/passwd" — must be blocked
        let result = event_loop.resolve_and_load_page("../../etc/passwd");
        assert!(result.is_err(), "Traversal to ../../etc/passwd must be blocked");

        let _ = std::fs::remove_file(doc_file);
    }

    #[test]
    fn test_2_resolve_load_valid_subdir() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();
        let temp_dir = std::env::temp_dir();
        let sub_dir = temp_dir.join("t2sub");
        std::fs::create_dir_all(&sub_dir).unwrap();
        let page = sub_dir.join("page_t2.thtml");
        std::fs::write(&page, b"<screen></screen>").unwrap();
        *client_session.app_base_dir.write() = Some(temp_dir.clone());

        let index = temp_dir.join("index_t2.thtml");
        std::fs::write(&index, b"<screen></screen>").unwrap();
        let doc = crate::loader::load_thtml_file(&index).unwrap();

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut event_loop = EventLoop::new(client_session, event_bus, output_tx, doc, false);
        event_loop.base_source_path = Some(index.clone());

        // Valid path inside base: must succeed
        let result = event_loop.resolve_and_load_page("t2sub/page_t2.thtml");
        assert!(result.is_ok(), "Valid subdir path must succeed: {:?}", result);

        let _ = std::fs::remove_file(page);
        let _ = std::fs::remove_file(index);
        let _ = std::fs::remove_dir(sub_dir);
    }

    #[test]
    fn test_3_htmx_no_thtml_calls_apply_action() {
        use oxiterm_proto::dom::{Node, NodeTag};
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let client_session = reg.create_session().unwrap();

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut btn = Node::new(NodeTag::Button);
        btn.attrs.event_htmx = Some("set:tab=home".to_string());
        let btn_id = arena.alloc(btn);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![btn_id];
        let root_id = arena.alloc(root);

        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut event_loop = EventLoop::new(client_session.clone(), event_bus, output_tx, doc, false);
        event_loop.focused_node = Some(btn_id);

        // Drive handle_htmx_target with action target
        event_loop.handle_htmx_target(btn_id, "set:tab=home");
        assert_eq!(client_session.state.read().get("tab"), Some(&crate::state::StateValue::Str("home".to_string())));
        assert!(event_loop.source_path.is_none());

        // Drive handle_htmx_target with navigation target -> attempts navigation
        // Since nonexistent.thtml doesn't exist, it fails resolution and logs warn without panic.
        event_loop.handle_htmx_target(btn_id, "nonexistent.thtml");
        assert!(event_loop.source_path.is_none()); // failed, so remains None
    }

    #[test]
    fn test_5_preprocess_space_with_input_focused() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: ' ', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        let result = EventLoop::preprocess_key(key, true);
        // input focused: Space is NOT remapped
        match result {
            Some(InputEvent::KeyPress(k)) => assert_eq!(k.codepoint, ' '),
            _ => panic!("Expected KeyPress(' ')"),
        }
    }

    #[test]
    fn test_6_preprocess_q_with_input_focused() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        let result = EventLoop::preprocess_key(key, true);
        // input focused: 'q' is NOT a quit signal
        assert!(result.is_some(), "q with focused input must pass through");
    }

    #[test]
    fn test_7_preprocess_q_without_input_focused() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        let result = EventLoop::preprocess_key(key, false);
        // not focused: 'q' IS a quit signal (returns None)
        assert!(result.is_none(), "q without focused input must return None");
    }

    #[test]
    fn test_13_identity_injects_reserved_keys() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let id = crate::identity::UserIdentity::ssh_key("alice");
        
        let attached = session.attach_identity(id);
        assert!(attached);

        let state = session.state.read();
        assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("alice".to_string())));
        assert_eq!(state.get("_auth_method"), Some(&crate::state::StateValue::Str("SshKey".to_string())));
    }

    #[test]
    fn test_15_patch_skips_reserved_applies_others() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        // Pre-set a reserved key via identity (bypasses _-prefix guard)
        session.state.write().set("_username".to_string(), crate::state::StateValue::Str("real".to_string()));
        // Patch: contains _username (reserved) and a normal key
        let patch = serde_json::json!({"_username": "hacked", "tab": "home"});
        session.apply_state_patch(patch);
        let state = session.state.read();
        assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("real".to_string())));
        assert_eq!(state.get("tab"), Some(&crate::state::StateValue::Str("home".to_string())));
    }

    #[test]
    fn test_17_reattach_preserves_identity() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let id = crate::identity::UserIdentity::ssh_key("bob");
        let attached1 = session.attach_identity(id);
        assert!(attached1);

        // Reattach and attempt downgrade/overwrite with Guest identity (C2/M2)
        let guest_id = crate::identity::UserIdentity::guest();
        let attached2 = session.attach_identity(guest_id);
        assert!(!attached2, "Reattach must not overwrite existing identity");

        // Assert original identity is preserved
        let ident = session.identity.read();
        assert!(ident.is_some());
        assert_eq!(ident.as_ref().unwrap().username, "bob");
        
        // Assert state keys are preserved (not overwritten by guest)
        let state = session.state.read();
        assert_eq!(state.get("_username"), Some(&crate::state::StateValue::Str("bob".to_string())));
    }

    #[test]
    fn test_32_open_https_emits_0x33() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut event_loop = EventLoop::new(session, event_bus, output_tx, doc, false);

        let (open_tx, mut open_rx) = tokio::sync::mpsc::channel(4);
        event_loop.open_url_tx = Some(open_tx);

        use oxiterm_proto::dom::NodeId;
        event_loop.handle_open_url("https://example.com".to_string(), NodeId(0));

        let frame = open_rx.try_recv().expect("Should have received 0x33 frame");
        assert_eq!(frame[0], 0x33);
        assert_eq!(&frame[1..], b"https://example.com");
    }

    #[test]
    fn test_33_open_javascript_rejected() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut event_loop = EventLoop::new(session, event_bus, output_tx, doc, false);

        let (open_tx, mut open_rx) = tokio::sync::mpsc::channel(4);
        event_loop.open_url_tx = Some(open_tx);

        use oxiterm_proto::dom::NodeId;
        event_loop.handle_open_url("javascript:alert(1)".to_string(), NodeId(0));

        assert!(open_rx.try_recv().is_err(), "javascript: URL must not emit 0x33");
    }

    #[test]
    fn test_34_open_ssh_session_no_output_warn_only() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        // SSH session: is_web_client = false (default)
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let event_loop = EventLoop::new(session, event_bus, output_tx, doc, false);
        // open_url_tx intentionally left as None (SSH path)

        use oxiterm_proto::dom::NodeId;
        // Must not panic, must not write to any channel
        event_loop.handle_open_url("https://example.com".to_string(), NodeId(0));
        // Test passes if handle_open_url returns without panic
    }

    #[test]
    fn test_26_for_expansion_on_append_action() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.state.write().set("items".to_string(), crate::state::StateValue::List(vec!["A".to_string()]));

        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut for_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::For);
        for_node.attrs.each = Some("items".to_string());
        
        let mut tmpl_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Text);
        tmpl_node.text = Some("{item}".to_string());
        let tmpl_id = arena.alloc(tmpl_node);
        for_node.children.push(tmpl_id);

        let for_id = arena.alloc(for_node);
        let doc = THTMLDocument { arena, root: for_id, dirty_nodes: Vec::new() };

        let mut event_loop = EventLoop::new(session.clone(), event_bus, output_tx, doc, false);
        
        event_loop.expand_all_for_nodes();
        {
            let node = event_loop.doc.arena.get(for_id).unwrap();
            assert_eq!(node.children.len(), 2);
            assert_eq!(event_loop.doc.arena.get(node.children[1]).unwrap().text.as_deref(), Some("A"));
        }

        event_loop.handle_htmx_target(for_id, "append:items=B");
        event_loop.expand_all_for_nodes();

        {
            let node = event_loop.doc.arena.get(for_id).unwrap();
            assert_eq!(node.children.len(), 3);
            assert_eq!(event_loop.doc.arena.get(node.children[1]).unwrap().text.as_deref(), Some("A"));
            assert_eq!(event_loop.doc.arena.get(node.children[2]).unwrap().text.as_deref(), Some("B"));
        }
    }

    #[test]
    fn test_35_stale_cache_cleared_on_page_change() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let (output_tx, _output_rx) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut event_loop = EventLoop::new(session, event_bus, output_tx, doc, false);
        
        use oxiterm_proto::dom::NodeId;
        event_loop.for_template_cache.insert(NodeId(42), vec![NodeId(43)]);
        assert!(!event_loop.for_template_cache.is_empty());

        let temp = std::env::temp_dir();
        let base_dir = temp.join("test_35_dir");
        std::fs::create_dir_all(&base_dir).unwrap();
        let page_path = base_dir.join("next.thtml");
        std::fs::write(&page_path, b"<screen><text>page content</text></screen>").unwrap();

        *event_loop.session.app_base_dir.write() = Some(base_dir.clone());
        event_loop.base_source_path = Some(page_path.clone());
        event_loop.source_path = Some(page_path.clone());

        event_loop.resolve_and_load_page("next.thtml").unwrap();

        assert!(event_loop.for_template_cache.is_empty());

        let _ = std::fs::remove_file(page_path);
        let _ = std::fs::remove_dir(base_dir);
    }
}
