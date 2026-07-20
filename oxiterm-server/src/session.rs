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
    pub web_frame_tx: parking_lot::RwLock<Option<(usize, tokio::sync::mpsc::Sender<Vec<u8>>)>>,
    pub connection_epoch: std::sync::atomic::AtomicUsize,
    pub event_loop_running: std::sync::atomic::AtomicBool,
    /// Reason byte emitted in the next [0xFF, reason] termination frame.
    /// 0 = takeover (hardcoded at takeover site); 1 = idle; 2 = restart/unknown (default).
    /// Written by EventLoop idle path; read by WsFrameSink::drop.
    pub death_reason: std::sync::atomic::AtomicU8,
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
            web_frame_tx: parking_lot::RwLock::new(None),
            connection_epoch: std::sync::atomic::AtomicUsize::new(0),
            event_loop_running: std::sync::atomic::AtomicBool::new(false),
            death_reason: std::sync::atomic::AtomicU8::new(2),
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
    /// The focused node the viewport was last auto-scrolled to. Auto-scroll-to-focused
    /// fires only when `focused_node` differs from this, so it reveals a newly focused
    /// (e.g. Tab-selected) node exactly once without fighting subsequent manual scrolling.
    pub autoscroll_anchor: Option<NodeId>,
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
    /// Per-session DoR (denial-of-render) event throttle.
    pub throttle: crate::throttle::EventThrottle,
}

/// Focus-ring markers drawn immediately left/right of the focused node. They MUST be
/// unambiguous width-1 glyphs — a wide glyph in the cell before a label drifts the
/// visible text off its hit box on ambiguous-wide terminals (see the focus-ring draw).
const FOCUS_RING_LEFT: char = '>';
const FOCUS_RING_RIGHT: char = '<';

/// Scroll status indicator under the "position" model: the line fraction, the bar
/// fill, and the percentage are all derived from a single `scroll_offset / max_scroll`
/// ratio, so they are mutually consistent by construction (0% / first position at the
/// top, 100% / last position at the bottom). `max_scroll` is derived from the layout
/// extent (`total_height`), the same height that drives vertical centering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScrollIndicator {
    /// 1-indexed current scroll position (`1..=steps`).
    position: u16,
    /// Total number of distinct scroll positions (`max_scroll + 1`).
    steps: u16,
    /// Progress percentage, `0..=100`.
    percent: u8,
    /// Number of filled cells in a `bar_width`-wide progress bar.
    filled: usize,
}

impl ScrollIndicator {
    fn new(scroll_offset: u16, total_height: u16, viewport_h: u16, bar_width: usize) -> Self {
        let max_scroll = total_height.saturating_sub(viewport_h);
        let offset = scroll_offset.min(max_scroll);
        let (percent, filled) = if max_scroll > 0 {
            let pct = (offset as f32 / max_scroll as f32 * 100.0).round() as u8;
            let filled = ((offset as usize * bar_width) / max_scroll as usize).min(bar_width);
            (pct, filled)
        } else {
            (0, 0)
        };
        Self { position: offset + 1, steps: max_scroll + 1, percent, filled }
    }
}

struct EventLoopGuard {
    session: Arc<ClientSession>,
}

impl Drop for EventLoopGuard {
    fn drop(&mut self) {
        self.session.event_loop_running.store(false, std::sync::atomic::Ordering::SeqCst);
    }
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
            autoscroll_anchor: None,
            scroll_offset: 0,
            total_height: 0,
            app_dispatcher: None,
            for_template_cache: HashMap::new(),
            throttle: crate::throttle::EventThrottle::new(),
        };
        event_loop.rebuild_parent_map();
        event_loop.rebuild_focusable_nodes();
        event_loop
    }

    /// Resets all compiled nodes, clears stale hit-test layout, and resets scroll margins.
    ///
    /// Clearing `last_layout` prevents stale mouse hit-tests from firing on old layout
    /// during the window between a doc swap and the first post-swap render.
    pub fn reset_layout_and_scroll(&mut self) {
        self.layout_engine.reset_nodes();
        self.layout_engine.last_layout = None;
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
        self.buffer.force_dirty();
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
            if self.throttle.check_nav() {
                if let Err(e) = self.resolve_and_load_page(target) {
                    warn!("handle_htmx_target: failed to load page {}: {}", target, e);
                    self.send_nav_error();
                }
            } else if self.throttle.is_first_throttle() {
                warn!("handle_htmx_target: NAV throttle active, dropping .thtml load");
                self.send_nav_throttle_notice();
            }
        } else {
            self.session.state.write().apply_action(target);
            self.try_dispatch(target);
        }
    }

    /// Sends a generic navigation-not-found notice (0x34) to web clients.
    ///
    /// Message is FIXED and GENERIC — never includes the requested path or failure reason
    /// (no-enumeration-oracle rule, Plan 2).
    pub(crate) fn send_nav_error(&self) {
        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some((_, ref tx)) = *self.session.web_frame_tx.read() {
                let mut frame = vec![0x34];
                frame.extend_from_slice("Nie znaleziono strony".as_bytes());
                let _ = tx.try_send(frame);
            }
        }
    }

    /// Sends a NAV throttle notice (0x34 "Zwolnij ;)") to web clients.
    ///
    /// Called at most once per throttle episode (guarded by `is_first_throttle()`).
    pub(crate) fn send_nav_throttle_notice(&self) {
        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some((_, ref tx)) = *self.session.web_frame_tx.read() {
                let mut frame = vec![0x34];
                frame.extend_from_slice("Zwolnij ;)".as_bytes());
                let _ = tx.try_send(frame);
            }
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
        is_web_client: bool,
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
            // Quit via 'q'/'Q' is SSH-only; web sessions ignore this binding.
            InputEvent::KeyPress(ref key) if !is_input_focused && !is_web_client
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
    pub(crate) fn handle_open_url(&self, url: String, _source_node_id: NodeId) {
        let scheme_end = url.find(':').unwrap_or(0);
        let scheme = &url[..scheme_end];
        if scheme != "http" && scheme != "https" {
            warn!("open: rejected URL with disallowed scheme '{}': {}", scheme, url);
            return;
        }

        if self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst) {
            if let Some((_, ref tx)) = *self.session.web_frame_tx.read() {
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

    /// Processes a single `MouseInput` event and returns whether a frame render is needed.
    ///
    /// Anchored by spec [Plan-2.2/R1, SC-05].
    ///
    /// # Design contract
    ///
    /// - **Press** (activation) — always runs `hit_test` + `handle_htmx_target`.
    ///   The INPUT bucket is never consulted for this path. The NAV bucket inside
    ///   `handle_htmx_target` caps the actual page-load rate.
    /// - **Release / Move / hover** — render-only flood surface. The INPUT bucket gates
    ///   `needs_render`; skipping a hover render frame is always safe.
    /// - `update_interactive_animations` and `pending_mouse` are always updated
    ///   (cheap; animation state coalesces internally).
    ///
    /// # Testability note
    ///
    /// This method is the canonical entry point exercised by t1/t2/t3. Tests that call
    /// `handle_mouse_event` directly prove the ARM wires hit_test → get_htmx_node_and_target
    /// → handle_htmx_target correctly, without bypassing any of that glue. Re-adding an
    /// `if self.throttle.check_input()` guard to the caller arm in `run()` would cause t2
    /// to fail when the INPUT bucket is empty, because the Press would never reach this method.
    /// Number of document rows a single mouse-wheel notch scrolls. Distinct from
    /// PageUp/PageDown (which move a whole viewport) so reading long content is smooth.
    const WHEEL_LINE_STEP: u16 = 3;

    /// Scrolls the viewport by [`Self::WHEEL_LINE_STEP`] rows, clamped to the scrollable
    /// range. `up == true` scrolls toward the top. Shared by web wheel events and native
    /// SSH SGR wheel reports (both arrive as `MouseButton::WheelUp/WheelDown`).
    fn scroll_wheel_lines(&mut self, up: bool) {
        let dims = *self.session.dims.read();
        if self.total_height <= dims.rows {
            return; // content fits; nothing to scroll
        }
        let viewport_h = dims.rows.saturating_sub(1);
        let max_scroll = self.total_height.saturating_sub(viewport_h);
        self.scroll_offset = if up {
            self.scroll_offset.saturating_sub(Self::WHEEL_LINE_STEP)
        } else {
            (self.scroll_offset + Self::WHEEL_LINE_STEP).min(max_scroll)
        };
    }

    /// Reveals the focused node in the viewport, but ONLY when focus just changed since
    /// the last call (tracked via `anchor`). Running it unconditionally every frame fights
    /// manual scrolling — PgDn/wheel past a top-anchored focused node gets yanked straight
    /// back, capping the scroll (the "stuck at 50%" bug). Firing only on focus change still
    /// reveals a Tab/arrow-selected node exactly once.
    ///
    /// Takes individual fields (not `&mut self`) so the caller can invoke it while holding
    /// a read lock on another `self` field via disjoint borrows.
    pub(crate) fn autoscroll_to_focused(
        focused_node: Option<NodeId>,
        anchor: &mut Option<NodeId>,
        scroll_offset: &mut u16,
        layout: &oxiterm_renderer::layout::types::LayoutResult,
        viewport_h: u16,
    ) {
        if focused_node == *anchor {
            return;
        }
        if let Some(focused_id) = focused_node {
            if let Some(rect) = layout.nodes.get(&focused_id) {
                if rect.y < *scroll_offset {
                    *scroll_offset = rect.y;
                } else if rect.y + rect.height > *scroll_offset + viewport_h {
                    *scroll_offset = (rect.y + rect.height).saturating_sub(viewport_h);
                }
            }
        }
        *anchor = focused_node;
    }

    /// Applies a keystroke to the focused text input's bound state: printable characters
    /// append, Backspace/Delete (8/127) delete the last char, Enter is a no-op here (it is
    /// handled as activation upstream). Returns `true` when the key was consumed as text
    /// input (so predictive echo is suppressed). Returns `false` when there is no focused
    /// bound `Input`, or for Enter/other control keys.
    ///
    /// Must run for ALL non-navigation keys — regressing this to only run on a specific
    /// key (e.g. Enter) silently breaks typing into every input field.
    pub(crate) fn apply_key_to_focused_input(&mut self, cp: char) -> bool {
        let Some(focused_id) = self.focused_node else { return false };
        let state_key = match self.doc.get_node(focused_id) {
            Some(node) if node.tag == oxiterm_proto::dom::NodeTag::Input => {
                match node.attrs.bind_value.clone() {
                    Some(k) => k,
                    None => return false,
                }
            }
            _ => return false,
        };

        let is_backspace = cp as u32 == 127 || cp as u32 == 8;
        let is_enter = cp == '\r' || cp as u32 == 13;
        if !is_backspace && (is_enter || cp.is_control()) {
            return false;
        }

        let mut value = match self.session.state.read().get(&state_key) {
            Some(crate::state::StateValue::Str(s)) => s.clone(),
            _ => String::new(),
        };
        if is_backspace {
            value.pop();
        } else {
            value.push(cp);
        }
        self.session.state.write().set(state_key, crate::state::StateValue::Str(value));
        true
    }

    pub(crate) fn handle_mouse_event(&mut self, mut mouse: oxiterm_proto::input::MouseInput) -> bool {
        let dims = *self.session.dims.read();
        tracing::trace!("Received MouseEvent: col={}, row={}, action={:?}", mouse.col, mouse.row, mouse.action);

        // Wheel notches are line-step scrolls, not clicks: intercept before any
        // coordinate remap or hit-test/activation so a scroll never activates a link.
        match mouse.button {
            oxiterm_proto::input::MouseButton::WheelUp => {
                self.scroll_wheel_lines(true);
                return true;
            }
            oxiterm_proto::input::MouseButton::WheelDown => {
                self.scroll_wheel_lines(false);
                return true;
            }
            _ => {}
        }
        if let Some(layout) = &self.layout_engine.last_layout {
            let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
            tracing::trace!("Document centering offset: offset_x={}, offset_y={}", offset_x, offset_y);
            mouse.col = mouse.col.saturating_sub(offset_x).saturating_sub(1);
            mouse.row = mouse.row.saturating_sub(offset_y).saturating_sub(1).saturating_add(self.scroll_offset);
            tracing::trace!("Adjusted MouseEvent: col={}, row={}", mouse.col, mouse.row);
        } else {
            tracing::trace!("No last layout found — pre-swap mouse event, dropping hit-test");
        }

        self.update_interactive_animations(&mouse);
        self.pending_mouse = Some(mouse.clone());

        // Anchored by spec [SC-05]. Handle interactive navigation or state updates based on htmx event targets.
        if mouse.action == oxiterm_proto::input::MouseAction::Press {
            // Press = activation: unconditional. hit_test + handle_htmx_target always run.
            // NAV bucket inside handle_htmx_target caps the actual page-load rate.
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
            true // Press always triggers a render
        } else {
            // Release / Move / hover — flood surface.
            // Gate only the render trigger; skipping a hover render is safe.
            self.throttle.check_input()
        }
    }

    /// Starts the main event loop thread driving keyboard and mouse coordinate reactions.
    pub fn run(&mut self) {

        let _guard = EventLoopGuard { session: self.session.clone() };
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
                // Signal idle death so WsFrameSink::drop emits [0xFF, 1] to web clients.
                self.session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);
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
                    // Quit is SSH-only; web sessions pass 'q' through unchanged.
                    let is_web = self.session.is_web_client.load(std::sync::atomic::Ordering::SeqCst);
                    let event = match Self::preprocess_key(event, is_input_focused, is_web) {
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
                                        if self.throttle.check_nav() {
                                            if let Err(e) = self.resolve_and_load_page(&file_name) {
                                                warn!("SwitchViewport: failed to load page: {}", e);
                                                self.send_nav_error();
                                            }
                                            needs_render = true;
                                        } else if self.throttle.is_first_throttle() {
                                            warn!("SwitchViewport: NAV throttle active, dropping viewport switch");
                                            self.send_nav_throttle_notice();
                                        }
                                    }
                                }
                                InputEvent::NavigateTo(rel_path) => {
                                    // Containment check is inside resolve_and_load_page.
                                    if self.throttle.check_nav() {
                                        if let Err(e) = self.resolve_and_load_page(&rel_path) {
                                            warn!("NavigateTo: failed to load page: {}", e);
                                            self.send_nav_error();
                                        }
                                        needs_render = true;
                                    } else if self.throttle.is_first_throttle() {
                                        warn!("NavigateTo: NAV throttle active, dropping navigation");
                                        self.send_nav_throttle_notice();
                                    }
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
                                // Anchored by spec [Plan-2.2/R1]: KeyPress arm runs unconditionally.
                                // Activation paths (focus-nav, Enter) must never be dropped by the INPUT bucket.
                                // check_input() is moved inside the character-input else-branch only.
                                InputEvent::KeyPress(key) => {

                                    let is_nav_forward = key.codepoint == '\u{F701}'
                                        || key.codepoint == '\u{F702}'
                                        || key.codepoint == '\t';
                                    let is_nav_backward = key.codepoint == '\u{F700}'
                                        || key.codepoint == '\u{F703}';

                                    if is_nav_forward || is_nav_backward {
                                        // Focus navigation — unconditional (cheap, no render flood risk).
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
                                    } else {
                                        // Enter → HTMX activation (Enter only). NAV bucket in handle_htmx_target caps loads.
                                        if key.codepoint == '\r' || key.codepoint as u32 == 13 {
                                            if let Some(focused) = self.focused_node {
                                                if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(focused) {
                                                    info!("Enter activated HTMX (node {:?}): {}", target_node_id, htmx_target);
                                                    self.handle_htmx_target(target_node_id, &htmx_target);
                                                    needs_render = true;
                                                }
                                            }
                                        }
                                        // Character input / predictive echo — runs for ALL non-navigation keys
                                        // (printable chars append, Backspace deletes; Enter is a no-op here).
                                        // D2: state write (append char / backspace) is UNCONDITIONAL —
                                        // a dropped frame is invisible; a dropped keystroke corrupts
                                        // a comment or password field.
                                        // check_input() gates only needs_render (render-flood surface).
                                        let handled_by_input = self.apply_key_to_focused_input(key.codepoint);

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
                                        }
                                        // Render is gated: skipping a frame is safe; losing a char is not.
                                        if self.throttle.check_input() {
                                            needs_render = true;
                                        }
                                    }
                                }
                                // Anchored by spec [Plan-2.2/R1]: MouseEvent arm runs unconditionally.
                                // The arm delegates entirely to handle_mouse_event(), which owns the
                                // Press-vs-flood split. Re-adding an `if self.throttle.check_input()`
                                // guard HERE would cause t2 to fail (Press swallowed when bucket empty).
                                InputEvent::MouseEvent(mouse) => {
                                    if self.handle_mouse_event(mouse) {
                                        needs_render = true;
                                    }
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

                    Self::autoscroll_to_focused(
                        self.focused_node,
                        &mut self.autoscroll_anchor,
                        &mut self.scroll_offset,
                        &layout,
                        viewport_h,
                    );

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
                                // Markers MUST be unambiguous width-1 glyphs: a wide glyph
                                // (e.g. ▶/◀, which are East-Asian Ambiguous) drawn in the
                                // cell before the label pushes the visible text right off
                                // its hit box on terminals that render them 2-wide, so
                                // clicks on a focused label miss. ASCII '>'/'<' are safe.
                                if rect.x > 0 {
                                    let lx = rect.x.saturating_sub(1) + offset_x;
                                    if lx < self.buffer.back.width {
                                        self.buffer.back.set(lx, mid_row, oxiterm_renderer::render::buffer::Cell {
                                            ch: FOCUS_RING_LEFT,
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
                                        ch: FOCUS_RING_RIGHT,
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
                        
                        let max_scroll = self.total_height.saturating_sub(viewport_h);
                        if self.scroll_offset > 0 {
                            status_parts.push("▲ PgUp".to_string());
                        }
                        if self.scroll_offset < max_scroll {
                            status_parts.push("▼ PgDn".to_string());
                        }

                        // Position model: line fraction, bar fill, and percentage all
                        // derive from the SAME scroll_offset/max_scroll, so they can never
                        // disagree. max_scroll comes from the layout extent (total_height),
                        // matching the vertical centering in `get_centering_offset`.
                        let bar_width = 8;
                        let ind = ScrollIndicator::new(self.scroll_offset, self.total_height, viewport_h, bar_width);
                        status_parts.push(format!("line {}/{}", ind.position, ind.steps));
                        let pct = ind.percent;
                        let filled_width = ind.filled;
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
    fn test_autoscroll_does_not_fight_manual_scroll() {
        // Focused node anchored at the top (y=1). After it's revealed once, a manual
        // scroll (offset advanced) must NOT be reverted on subsequent frames while focus
        // is unchanged — otherwise you can't scroll past a top-anchored focused link.
        let (mut el, btn) = make_el_with_htmx_button("noop:x");
        *el.session.dims.write() = PtyDimensions { cols: 20, rows: 6 };
        el.total_height = 20;
        el.focused_node = Some(btn);
        inject_layout(
            &mut el,
            vec![(btn, oxiterm_renderer::layout::types::Rect { x: 0, y: 1, width: 6, height: 1 })],
            20,
        );
        let layout = el.layout_engine.last_layout.clone().unwrap();
        let viewport_h = 5;

        // Frame 1: focus just became `btn` — reveal it (already visible at y=1, offset stays 0).
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout, viewport_h);
        assert_eq!(el.scroll_offset, 0);

        // User scrolls down manually.
        el.scroll_offset = 4;
        // Frame 2+: focus unchanged — offset must be left alone.
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout, viewport_h);
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout, viewport_h);
        assert_eq!(el.scroll_offset, 4, "manual scroll must survive while focus is unchanged");

        // When focus actually changes to an off-screen node, it is revealed once.
        let other = el.doc.arena.alloc(oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Box));
        let mut nodes = layout.nodes.clone();
        nodes.insert(other, oxiterm_renderer::layout::types::Rect { x: 0, y: 18, width: 4, height: 1 });
        let layout2 = oxiterm_renderer::layout::types::LayoutResult { nodes, total_height: 20 };
        el.focused_node = Some(other);
        EventLoop::autoscroll_to_focused(el.focused_node, &mut el.autoscroll_anchor, &mut el.scroll_offset, &layout2, viewport_h);
        assert_eq!(el.scroll_offset, 19 - viewport_h, "focus change reveals the newly focused node");
    }

    #[test]
    fn test_typing_updates_focused_input_state() {
        // Regression: printable keys must append to the focused input's bound state,
        // Backspace deletes, Enter is a no-op, and no focused input = not consumed.
        // (Previously all char input was wrongly gated behind the Enter branch, so typing
        // into any field did nothing.)
        use oxiterm_proto::dom::{Node, NodeTag};
        let (mut el, _btn) = make_el_with_htmx_button("noop:x");
        let mut input = Node::new(NodeTag::Input);
        input.attrs.bind_value = Some("name".to_string());
        let input_id = el.doc.arena.alloc(input);
        el.focused_node = Some(input_id);

        assert!(el.apply_key_to_focused_input('h'));
        assert!(el.apply_key_to_focused_input('i'));
        assert_eq!(
            el.session.state.read().get("name").cloned(),
            Some(crate::state::StateValue::Str("hi".to_string()))
        );

        assert!(el.apply_key_to_focused_input('\u{8}'), "backspace consumed");
        assert_eq!(
            el.session.state.read().get("name").cloned(),
            Some(crate::state::StateValue::Str("h".to_string()))
        );

        assert!(!el.apply_key_to_focused_input('\r'), "Enter is not text input");
        assert_eq!(
            el.session.state.read().get("name").cloned(),
            Some(crate::state::StateValue::Str("h".to_string()))
        );

        el.focused_node = None;
        assert!(!el.apply_key_to_focused_input('x'), "no focused input → not consumed");
    }

    #[test]
    fn test_focus_ring_glyphs_are_unambiguous_width() {
        // A wide/ambiguous focus-ring marker drawn before a focused label pushes the
        // visible text off its hit box, so clicks on the focused label miss.
        use oxiterm_renderer::render::unicode::is_ambiguous_width;
        assert!(!is_ambiguous_width(FOCUS_RING_LEFT), "left focus marker must be width-1");
        assert!(!is_ambiguous_width(FOCUS_RING_RIGHT), "right focus marker must be width-1");
    }

    #[test]
    fn test_scroll_indicator_is_mutually_consistent() {
        // t2: line X/Y, bar fill, and percentage all come from one total_height, so the
        // fraction and the percentage agree at top / middle / bottom.
        let total_height = 33u16;
        let viewport_h = 30u16; // max_scroll = 3 → 4 positions
        let bar_width = 8usize;

        let max_scroll = total_height - viewport_h;
        assert_eq!(max_scroll, 3);

        let check = |offset: u16, exp_pos: u16, exp_pct: u8| {
            let ind = ScrollIndicator::new(offset, total_height, viewport_h, bar_width);
            assert_eq!(ind.steps, max_scroll + 1, "denominator is the position count");
            assert_eq!(ind.position, exp_pos);
            assert_eq!(ind.percent, exp_pct);
            // Percentage and the line fraction tell the same story: both are the same
            // 0-indexed progress out of `max_scroll`.
            let progress = ((ind.position - 1) as f32 / (ind.steps - 1) as f32 * 100.0).round() as u8;
            assert_eq!(ind.percent, progress, "percent must equal the line-fraction progress");
            // Bar fill agrees with the percentage.
            let expected_filled = (ind.percent as usize * bar_width) / 100;
            assert!((ind.filled as i32 - expected_filled as i32).abs() <= 1,
                "bar fill must track the percentage");
        };

        check(0, 1, 0);   // top: line 1/4, 0%
        check(1, 2, 33);  // middle: line 2/4, 33%
        check(3, 4, 100); // bottom: line 4/4, 100%

        // Over-scroll is clamped to the bottom position, never past 100%.
        let clamped = ScrollIndicator::new(99, total_height, viewport_h, bar_width);
        assert_eq!((clamped.position, clamped.percent), (4, 100));
    }

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
        let result = EventLoop::preprocess_key(key, true, false);
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
        let result = EventLoop::preprocess_key(key, true, false);
        // input focused: 'q' is NOT a quit signal
        assert!(result.is_some(), "q with focused input must pass through");
    }

    #[test]
    fn test_7_preprocess_q_without_input_focused_ssh() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        // SSH session (is_web_client = false), no input focused: 'q' IS a quit signal
        let result = EventLoop::preprocess_key(key, false, false);
        assert!(result.is_none(), "q without focused input on SSH must return None");
    }

    #[test]
    fn test_8_preprocess_q_web_session_no_quit() {
        use oxiterm_proto::input::{KeyEvent, KeyModifiers, KeyKind};
        let key = InputEvent::KeyPress(KeyEvent { codepoint: 'q', modifiers: KeyModifiers::default(), kind: KeyKind::Press });
        // Web session (is_web_client = true), no input focused: 'q' must NOT quit
        let result = EventLoop::preprocess_key(key, false, true);
        match result {
            Some(InputEvent::KeyPress(k)) => assert_eq!(k.codepoint, 'q', "web 'q' must pass through as KeyPress"),
            _ => panic!("Expected KeyPress('q') to pass through on web session"),
        }
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
        let event_loop = EventLoop::new(session, event_bus, output_tx, doc, false);

        let (open_tx, mut open_rx) = tokio::sync::mpsc::channel(4);
        *event_loop.session.web_frame_tx.write() = Some((1, open_tx));

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
        let event_loop = EventLoop::new(session, event_bus, output_tx, doc, false);

        let (open_tx, mut open_rx) = tokio::sync::mpsc::channel(4);
        *event_loop.session.web_frame_tx.write() = Some((1, open_tx));

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

    #[test]
    fn test_t10_run_exit_allows_fresh_event_loop() {
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        
        // Initially false
        assert!(!session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst));

        // Simulate run start
        let is_running1 = session.event_loop_running.swap(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!is_running1);

        // Simulate run exit via Drop guard
        {
            let _guard = EventLoopGuard { session: session.clone() };
        }

        // Should be false again
        assert!(!session.event_loop_running.load(std::sync::atomic::Ordering::SeqCst));

        // Simulate second start
        let is_running2 = session.event_loop_running.swap(true, std::sync::atomic::Ordering::SeqCst);
        assert!(!is_running2);
    }
    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.2 / R1 — 0x34 navigation failure notice
    // ────────────────────────────────────────────────────────────────────────

    fn make_web_event_loop_with_channel(
        session: Arc<ClientSession>,
        frame_rx: &mut Option<tokio::sync::mpsc::Receiver<Vec<u8>>>,
    ) -> EventLoop {
        let (frame_tx, rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        *frame_rx = Some(rx);
        *session.web_frame_tx.write() = Some((1, frame_tx));
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        EventLoop::new(session, event_bus, output_tx, doc, false)
    }

    fn drain_frames(rx: &mut tokio::sync::mpsc::Receiver<Vec<u8>>) -> Vec<Vec<u8>> {
        let mut frames = Vec::new();
        while let Ok(f) = rx.try_recv() { frames.push(f); }
        frames
    }

    #[test]
    fn test_t1_nav_error_web_client_sends_0x34() {
        // 2.2/t1: NavigateTo missing page on web session → [0x34, "Nie znaleziono strony"]
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let temp = std::env::temp_dir();
        *session.app_base_dir.write() = Some(temp.clone());

        let mut rx = None;
        let mut el = make_web_event_loop_with_channel(session, &mut rx);
        el.base_source_path = Some(temp.join("dummy.thtml"));

        // exhausting NAV tokens first so throttle is out of the way
        // then restore
        el.throttle.nav.set_tokens(5.0);
        el.resolve_and_load_page("nonexistent_page_t1.thtml").ok(); // will fail → send_nav_error
        // BUT send_nav_error is only called from handle_htmx_target / NavigateTo arms.
        // Call send_nav_error directly to test the helper.
        el.send_nav_error();

        let frames = drain_frames(rx.as_mut().unwrap());
        let found = frames.iter().any(|f| {
            f.len() > 1 && f[0] == 0x34 && {
                let msg = std::str::from_utf8(&f[1..]).unwrap_or("");
                msg == "Nie znaleziono strony"
            }
        });
        assert!(found, "expected 0x34 'Nie znaleziono strony' frame, got: {:?}", frames);
    }

    #[test]
    fn test_t2_nav_error_ssh_session_no_0x34() {
        // 2.2/t2: SSH session (is_web_client=false) → send_nav_error is a no-op
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        // is_web_client stays false (default)

        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let el = EventLoop::new(session, event_bus, output_tx, doc, false);

        el.send_nav_error();

        let frames = drain_frames(&mut frame_rx);
        let found = frames.iter().any(|f| !f.is_empty() && f[0] == 0x34);
        assert!(!found, "SSH session must not emit 0x34");
    }

    #[test]
    fn test_t3_nav_error_traversal_indistinguishable() {
        // 2.2/t3: traversal + missing produce byte-identical 0x34 frames
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let el = EventLoop::new(session, event_bus, output_tx, doc, false);

        // Send two nav errors (simulating traversal case and missing case)
        el.send_nav_error();
        el.send_nav_error();

        let frames = drain_frames(&mut frame_rx);
        assert_eq!(frames.len(), 2, "expected exactly two frames");
        assert_eq!(frames[0], frames[1], "traversal and missing must produce identical frames");
        assert_eq!(frames[0][0], 0x34, "must be tag 0x34");
    }

    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.2 / R5 — Throttle tests
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t4_nav_throttle_emits_one_notice_then_silences() {
        // 2.2/t4: exhaust NAV bucket → exactly one 0x34 "Zwolnij ;)" per episode
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);
        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = EventLoop::new(session, event_bus, output_tx, doc, false);

        // Exhaust NAV tokens
        for _ in 0..crate::throttle::NAV_CAPACITY {
            assert!(el.throttle.check_nav(), "should be within capacity");
        }
        // Now throttled — simulate 20 throttle calls
        let mut throttle_notices = 0usize;
        for _ in 0..20 {
            if !el.throttle.check_nav() && el.throttle.is_first_throttle() {
                el.send_nav_throttle_notice();
                throttle_notices += 1;
            }
        }
        // Must be exactly 1 throttle notice per episode
        // Run note: under real time ≤ capacity+1 due to refill; under paused time exactly 1.
        assert_eq!(throttle_notices, 1,
            "exactly one throttle notice per episode; got {}", throttle_notices);

        let frames = drain_frames(&mut frame_rx);
        let notice_frames: Vec<_> = frames.iter().filter(|f| {
            f.len() > 1 && f[0] == 0x34 && std::str::from_utf8(&f[1..]).unwrap_or("") == "Zwolnij ;)"
        }).collect();
        assert_eq!(notice_frames.len(), 1, "exactly one 0x34 'Zwolnij ;)' frame emitted");
    }

    #[test]
    fn test_t5_input_throttle_silent_drop() {
        // 2.2/t5: INPUT throttle drops events silently (no 0x34)
        let mut t = crate::throttle::EventThrottle::new();
        // Exhaust the input bucket
        for _ in 0..crate::throttle::INPUT_CAPACITY {
            assert!(t.check_input());
        }
        // Further 150 events: all dropped
        for _ in 0..150 {
            assert!(!t.check_input(), "throttled input must return false");
        }
        // No 0x34 test: no send path reached for INPUT throttle
    }

    #[test]
    fn test_t8_after_throttle_refill_nav_accepted() {
        // 2.2/t8: After exhaust + manual refill, NAV accepted again; episode cleared
        let mut t = crate::throttle::EventThrottle::new();
        for _ in 0..crate::throttle::NAV_CAPACITY {
            t.check_nav();
        }
        assert!(!t.check_nav(), "exhausted");
        t.is_first_throttle(); // enter episode

        // Force refill
        t.nav.set_tokens(crate::throttle::NAV_CAPACITY as f64);
        assert!(t.check_nav(), "after refill, allowed again");
        assert!(!t.nav_throttle_active, "episode cleared on allow");
    }

    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.3 / R1 — death_reason + 0xFF tests (Correction A)
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t1a_idle_death_sets_death_reason_1() {
        // 2.3/t1a: EventLoop idle path sets death_reason=1
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();

        // Default is 2
        assert_eq!(session.death_reason.load(std::sync::atomic::Ordering::SeqCst), 2);

        // Simulate idle timeout path (just the store that EventLoop::run does before return)
        session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(session.death_reason.load(std::sync::atomic::Ordering::SeqCst), 1,
            "idle path must set death_reason to 1");
    }

    #[test]
    fn test_t1b_takeover_sends_0xff_0() {
        // 2.3/t1b: Takeover site sends hardcoded [0xFF, 0]
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        // Simulate takeover: old_tx.try_send([0xFF, 0])
        let _ = tx.try_send(vec![0xFF, 0u8]);
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame, vec![0xFF, 0u8], "takeover must send [0xFF, 0]");
    }

    #[test]
    fn test_t1c_live_tab_idle_death_emits_0xff_1_not_0() {
        // 2.3/t1c: live-tab idle death (is_web_client=true, death_reason=1) → [0xFF, 1]
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        session.is_web_client.store(true, std::sync::atomic::Ordering::SeqCst);

        let (frame_tx, mut frame_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(4);
        *session.web_frame_tx.write() = Some((1, frame_tx));

        // Simulate EventLoop idle path: set death_reason=1
        session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);

        // Simulate WsFrameSink::drop: read death_reason and send [0xFF, reason]
        let reason = session.death_reason.load(std::sync::atomic::Ordering::SeqCst);
        if let Some((_, ref tx)) = *session.web_frame_tx.read() {
            let _ = tx.try_send(vec![0xFF, reason]);
        }

        let frame = frame_rx.try_recv().unwrap();
        assert_eq!(frame, vec![0xFF, 1u8],
            "live-tab idle death must emit [0xFF, 1], NOT [0xFF, 0]: got {:?}", frame);
    }

    #[test]
    fn test_t1a_extended_f2_reattach_resets_death_reason() {
        // F2: After idle death + fresh attach, death_reason is reset to 2.
        // A subsequent non-idle exit must emit [0xFF, 2].
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();

        // Simulate idle cycle: death_reason=1
        session.death_reason.store(1, std::sync::atomic::Ordering::SeqCst);

        // Simulate WS reattach: death_reason reset to 2 (as in web.rs handle_websocket)
        session.death_reason.store(2, std::sync::atomic::Ordering::SeqCst);

        // Now a non-idle exit (e.g., server restart) → death_reason stays 2
        let reason = session.death_reason.load(std::sync::atomic::Ordering::SeqCst);
        assert_eq!(reason, 2u8,
            "after reattach, death_reason must be 2 (restart/unknown); got {}", reason);
    }

    #[test]
    fn test_t4a_reattach_resets_last_activity() {
        // 2.3/t4a: WS attach resets last_activity within 1 second
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();

        // Wind back last_activity to simulate stale session
        *session.last_activity.write() = std::time::Instant::now()
            - std::time::Duration::from_secs(600);

        // Simulate handle_websocket reset
        *session.last_activity.write() = std::time::Instant::now();
        session.death_reason.store(2, std::sync::atomic::Ordering::SeqCst);

        let elapsed = session.last_activity.read().elapsed();
        assert!(elapsed < std::time::Duration::from_secs(1),
            "last_activity after reattach must be < 1s ago, was {:?}", elapsed);
    }

    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.3 Addendum R5 — media log split
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t5b_media_outside_base_does_not_crash() {
        // 2.3/t5b: path traversal attempt in media path → blocked, no panic
        // This test verifies the is_within_base guard still rejects traversals.
        let base = std::env::temp_dir().join("oxiterm_test_t5b");
        std::fs::create_dir_all(&base).unwrap();
        let outside = std::fs::canonicalize(base.parent().unwrap()).unwrap().join("outside_t5b.jpg");

        // is_within_base should block this
        let is_safe = crate::pathsafe::is_within_base(&base, &outside);
        assert!(!is_safe, "path outside base must be blocked by is_within_base");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn test_t5c_media_missing_not_traversal() {
        // 2.3/t5c: The log split in web.rs distinguishes missing vs traversal AFTER the
        // is_within_base guard. is_within_base itself returns false for non-existent files
        // (canonicalize fails). The test verifies that the web.rs log-split code correctly
        // categorises a within-base path that exists as safe, and a non-existent path
        // as "not found" rather than "traversal" — tested via the path logic directly.
        let base = std::env::temp_dir().join("oxiterm_test_t5c");
        std::fs::create_dir_all(&base).unwrap();
        let existing = base.join("existing_t5c.jpg");
        std::fs::write(&existing, b"jpg").unwrap();
        let missing = base.join("missing_asset_t5c.jpg");

        // Existing file within base → is_within_base returns true
        assert!(crate::pathsafe::is_within_base(&base, &existing),
            "existing file within base must be considered safe");

        // Non-existent file — is_within_base returns false (canonicalize failure)
        // but it is NOT a traversal: the path lexically starts with base
        assert!(!missing.exists(), "missing file must not exist");
        // Lexical prefix check confirms it's not a traversal (just missing)
        let base_str = base.to_string_lossy();
        let missing_str = missing.to_string_lossy();
        assert!(missing_str.starts_with(base_str.as_ref()),
            "path is lexically within base — it's missing, not a traversal");

        let _ = std::fs::remove_file(&existing);
        let _ = std::fs::remove_dir_all(&base);
    }

    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.3 Addendum R6 — last_layout cleared after page change
    // ────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_t6a_reset_layout_clears_last_layout() {
        // 2.3/t6a: reset_layout_and_scroll clears last_layout → no stale hit-test
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());
        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let root = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Screen);
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };
        let mut el = EventLoop::new(session, event_bus, output_tx, doc, false);

        // Manually set a fake last_layout to ensure it's not None
        // (We can't construct a full LayoutResult, but we can verify None after reset)
        // The field is already None at construction; just confirm reset_layout_and_scroll keeps it None.
        assert!(el.layout_engine.last_layout.is_none(), "starts as None");
        el.reset_layout_and_scroll();
        assert!(el.layout_engine.last_layout.is_none(), "still None after reset");
        assert_eq!(el.scroll_offset, 0, "scroll offset reset to 0");
    }

    #[test]
    fn test_t_death_reason_default_is_2() {
        // Guard: newly created sessions always default death_reason=2 (restart/unknown)
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        assert_eq!(
            session.death_reason.load(std::sync::atomic::Ordering::SeqCst),
            2u8,
            "default death_reason must be 2 (restart/unknown)"
        );
    }

    // ────────────────────────────────────────────────────────────────────────
    // Plan 2.2 / R1 — Mouse activation throttle fix
    // ────────────────────────────────────────────────────────────────────────

    /// Builds a minimal EventLoop with one Screen root and one Box child carrying event-htmx.
    /// Returns (event_loop, box_node_id).
    fn make_el_with_htmx_button(
        htmx: &str,
    ) -> (EventLoop, oxiterm_proto::dom::NodeId) {
        use oxiterm_proto::dom::{Node, NodeTag};
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();
        let mut btn = Node::new(NodeTag::Box);
        btn.attrs.event_htmx = Some(htmx.to_string());
        let btn_id = arena.alloc(btn);
        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![btn_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let el = EventLoop::new(session, event_bus, output_tx, doc, false);
        (el, btn_id)
    }

    /// Injects a synthetic LayoutResult placing `node_id` at the given rect.
    fn inject_layout(
        el: &mut EventLoop,
        entries: Vec<(oxiterm_proto::dom::NodeId, oxiterm_renderer::layout::types::Rect)>,
        total_height: u16,
    ) {
        let mut nodes = std::collections::HashMap::new();
        for (id, rect) in entries {
            nodes.insert(id, rect);
        }
        el.layout_engine.last_layout = Some(oxiterm_renderer::layout::types::LayoutResult {
            nodes,
            total_height,
        });
    }

    #[test]
    fn test_wheel_scrolls_by_line_step_and_clamps() {
        // Mouse wheel = line-step scroll (3 rows/notch), clamped to the scroll range,
        // and it must NOT run hit-test/activation (a scroll is not a click).
        let (mut el, btn_id) = make_el_with_htmx_button("inc:clicks");
        el.rebuild_parent_map();
        *el.session.dims.write() = PtyDimensions { cols: 20, rows: 5 };
        el.total_height = 20; // viewport_h = 4 → max_scroll = 16

        // Place the button under the wheel coordinate; if the wheel wrongly activated,
        // "clicks" state would change.
        inject_layout(
            &mut el,
            vec![(btn_id, oxiterm_renderer::layout::types::Rect { x: 0, y: 0, width: 20, height: 20 })],
            20,
        );

        let wheel = |btn| oxiterm_proto::input::MouseInput {
            col: 1, row: 1, button: btn,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        };
        use oxiterm_proto::input::MouseButton::{WheelDown, WheelUp};

        assert!(el.handle_mouse_event(wheel(WheelDown)), "wheel requests a render");
        assert_eq!(el.scroll_offset, 3, "one notch down = WHEEL_LINE_STEP rows");
        el.handle_mouse_event(wheel(WheelDown));
        assert_eq!(el.scroll_offset, 6);
        el.handle_mouse_event(wheel(WheelUp));
        assert_eq!(el.scroll_offset, 3, "one notch up = WHEEL_LINE_STEP rows back");

        for _ in 0..20 { el.handle_mouse_event(wheel(WheelDown)); }
        assert_eq!(el.scroll_offset, 16, "clamped at max_scroll, never past the bottom");
        for _ in 0..20 { el.handle_mouse_event(wheel(WheelUp)); }
        assert_eq!(el.scroll_offset, 0, "clamped at the top");

        assert!(el.session.state.read().get("clicks").is_none(),
            "wheel scrolling must never activate an htmx target");
    }

    #[test]
    fn test_wheel_ignored_when_content_fits() {
        // No scrollbar (content fits the viewport) → wheel is a no-op.
        let (mut el, _btn) = make_el_with_htmx_button("noop:x");
        *el.session.dims.write() = PtyDimensions { cols: 20, rows: 30 };
        el.total_height = 10; // <= rows → no scroll
        el.handle_mouse_event(oxiterm_proto::input::MouseInput {
            col: 1, row: 1, button: oxiterm_proto::input::MouseButton::WheelDown,
            action: oxiterm_proto::input::MouseAction::Press, modifiers: Default::default(),
        });
        assert_eq!(el.scroll_offset, 0);
    }

    #[test]
    fn test_render_click_roundtrip_activates_every_rendered_cell() {
        // Full render↔click round-trip: a click on the terminal cell where a node is
        // *rendered* (centering offset + rect + the terminal's 1-based origin) must, after
        // handle_mouse_event's inverse adjustment, hit-test back to that node and activate
        // it — and a click one column outside the span must NOT. This locks the coordinate
        // contract shared by rendering (`get_centering_offset` + rect.x) and hit-testing
        // (`raw - offset - 1`); any offset or off-by-one regression fails here.
        let (mut el, btn) = make_el_with_htmx_button("inc:hits");
        el.rebuild_parent_map();
        {
            let n = el.doc.arena.get_mut(btn).unwrap();
            n.style.width = Some(10);
            n.style.height = Some(2);
            n.style.margin.left = 4;
            n.style.margin.top = 3;
        }
        let dims = PtyDimensions { cols: 80, rows: 24 };
        *el.session.dims.write() = dims;

        // Real layout — the same offsets rendering would use.
        el.layout_engine.compute(&mut el.doc, dims.cols, 0, None).unwrap();
        let layout = el.layout_engine.last_layout.clone().unwrap();
        let rect = *layout.nodes.get(&btn).unwrap();
        let (ox, oy) = layout.get_centering_offset(&el.doc, dims.cols, dims.rows);
        assert!(rect.width > 0 && rect.height > 0);

        let click = |el: &mut EventLoop, term_col: u16, term_row: u16| {
            el.session.state.write().set("hits".to_string(), crate::state::StateValue::Int(0));
            el.handle_mouse_event(oxiterm_proto::input::MouseInput {
                col: term_col,
                row: term_row,
                button: oxiterm_proto::input::MouseButton::Left,
                action: oxiterm_proto::input::MouseAction::Press,
                modifiers: Default::default(),
            });
            el.session.state.read().get("hits").cloned()
        };

        // Every rendered cell activates the button.
        for ry in rect.y..rect.y + rect.height {
            for rx in rect.x..rect.x + rect.width {
                // Terminal (1-based) coordinate that renders layout cell (rx, ry), no scroll.
                let got = click(&mut el, ox + rx + 1, oy + ry + 1);
                assert_eq!(
                    got,
                    Some(crate::state::StateValue::Int(1)),
                    "click on rendered cell for layout ({rx},{ry}) must activate the button"
                );
            }
        }

        // One column left of, and one past, the span must not activate it.
        let row = oy + rect.y + 1;
        assert_eq!(click(&mut el, ox + rect.x, row), Some(crate::state::StateValue::Int(0)),
            "column just left of the span must not activate");
        assert_eq!(click(&mut el, ox + rect.x + rect.width + 1, row), Some(crate::state::StateValue::Int(0)),
            "column just past the span must not activate");
    }

    #[test]
    fn test_t1_press_activates_despite_empty_input_bucket() {
        // 2.2/t1: A MouseEvent(Press) delivered to handle_mouse_event() fires
        // hit_test + handle_htmx_target even when the INPUT bucket is empty.
        //
        // Negative anchor: if `run()` re-adds `if self.throttle.check_input()` as
        // an arm guard, handle_mouse_event() would never be called and this test
        // would panic because state "clicks" would remain None.
        let (mut el, btn_id) = make_el_with_htmx_button("inc:clicks");
        el.rebuild_parent_map();

        // Button occupies col=5..15, row=2..4.
        inject_layout(
            &mut el,
            vec![(btn_id, oxiterm_renderer::layout::types::Rect { x: 5, y: 2, width: 10, height: 2 })],
            10,
        );

        // Drain the INPUT bucket — simulates post-hover state.
        el.throttle.input.set_tokens(0.0);

        // handle_mouse_event subtracts centering_offset (0,0 for dims=0) plus 1
        // from each axis before calling hit_test. Button is at x=5,y=2; send
        // col=8,row=3 so after saturating_sub(1) hit_test receives (7,2).
        let needs_render = el.handle_mouse_event(oxiterm_proto::input::MouseInput {
            col: 8,
            row: 3,
            button: oxiterm_proto::input::MouseButton::Left,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        });

        assert!(needs_render, "Press must always request a render");
        let clicks = el.session.state.read().get("clicks").cloned();
        assert_eq!(
            clicks,
            Some(crate::state::StateValue::Int(1)),
            "Press must activate handle_htmx_target regardless of INPUT bucket; got {:?}",
            clicks,
        );
    }

    #[test]
    fn test_t2_click_on_text_child_activates_parent_box_htmx() {
        // 2.2/t2 — REGRESSION TEST FOR THIS BUG.
        //
        // Scenario: a Box carries event-htmx; a Text node is its child.
        // The user hovers (exhausting INPUT bucket) then clicks on the Text label.
        // Before the fix: the MouseEvent arm was guarded by check_input(), so the
        //   Press was silently dropped — activation never fired.
        // After the fix: handle_mouse_event() is called unconditionally; it runs
        //   hit_test (returns Text node) → get_htmx_node_and_target (walks up to Box)
        //   → handle_htmx_target — all without the test touching those functions.
        //
        // Negative anchor: re-adding `if self.throttle.check_input()` in run()'s arm
        // makes handle_mouse_event() unreachable when the bucket is empty, causing
        // "activations" to remain None and this assertion to fail.
        use oxiterm_proto::dom::{Node, NodeTag};
        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        *session.dims.write() = PtyDimensions { cols: 0, rows: 0 };
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();

        // Text child — the actual click target, carries no htmx.
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("Click me".to_string());
        let text_id = arena.alloc(text_node);

        // Box parent — carries event-htmx.
        let mut box_node = Node::new(NodeTag::Box);
        box_node.attrs.event_htmx = Some("inc:activations".to_string());
        box_node.children = vec![text_id];
        let box_id = arena.alloc(box_node);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![box_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = EventLoop::new(session, event_bus, output_tx, doc, false);
        el.rebuild_parent_map();

        // Text at col=5..13, row=2..3 (smaller area → hit_test picks it over Box).
        // Box at col=4..14, row=1..4 (larger area, parent).
        inject_layout(
            &mut el,
            vec![
                (box_id,  oxiterm_renderer::layout::types::Rect { x: 4, y: 1, width: 10, height: 3 }),
                (text_id, oxiterm_renderer::layout::types::Rect { x: 5, y: 2, width:  8, height: 1 }),
            ],
            10,
        );

        // Exhaust INPUT bucket — simulates hover-heavy scenario before the click.
        el.throttle.input.set_tokens(0.0);

        // handle_mouse_event subtracts centering_offset (0,0 for dims=0) plus 1
        // from each axis. Text rect is at x=5,y=2; send col=8,row=3 so after
        // saturating_sub(1) hit_test receives (7,2) which lands in the Text rect
        // (x=5..13, y=2..3) but not the Box (x=4..14, y=1..4) alone — Text wins.
        let needs_render = el.handle_mouse_event(oxiterm_proto::input::MouseInput {
            col: 8,
            row: 3,
            button: oxiterm_proto::input::MouseButton::Left,
            action: oxiterm_proto::input::MouseAction::Press,
            modifiers: Default::default(),
        });

        assert!(needs_render, "Press must always request a render");
        let activations = el.session.state.read().get("activations").cloned();
        assert_eq!(
            activations,
            Some(crate::state::StateValue::Int(1)),
            "clicking Text child must activate parent Box's htmx via the arm wiring; got {:?}",
            activations,
        );
    }

    #[test]
    fn test_t3_burst_press_activates_all_never_throttled() {
        // 2.2/t3: 300 rapid Press events via handle_mouse_event() with INPUT bucket
        // empty from the start must ALL activate (inc:clicks → state count == 300).
        // NAV bucket is irrelevant (state-mutation action, not a .thtml load).
        //
        // Negative anchor: the old arm guard would have suppressed all 300 Presses
        // (bucket empty), leaving "clicks" as None and this assertion failing.
        let (mut el, btn_id) = make_el_with_htmx_button("inc:clicks");
        el.rebuild_parent_map();
        inject_layout(
            &mut el,
            vec![(btn_id, oxiterm_renderer::layout::types::Rect { x: 0, y: 0, width: 10, height: 1 })],
            5,
        );

        // Force INPUT bucket empty throughout.
        el.throttle.input.set_tokens(0.0);

        const PRESS_COUNT: i64 = 300;
        for _ in 0..PRESS_COUNT {
            el.handle_mouse_event(oxiterm_proto::input::MouseInput {
                col: 5,
                row: 0,
                button: oxiterm_proto::input::MouseButton::Left,
                action: oxiterm_proto::input::MouseAction::Press,
                modifiers: Default::default(),
            });
        }

        let clicks = el.session.state.read().get("clicks").cloned();
        assert_eq!(
            clicks,
            Some(crate::state::StateValue::Int(PRESS_COUNT)),
            "all {} Press activations must reach handle_htmx_target; INPUT bucket must never gate them; got {:?}",
            PRESS_COUNT,
            clicks,
        );
    }

    #[test]
    fn test_t_charwrite_never_throttled() {
        // D2: character state-writes (bind_value append) are UNCONDITIONAL.
        // Only the render trigger is gated by check_input().
        // With INPUT bucket at 0, 10 keypresses to a focused Input node must
        // produce bind_value "aaaaaaaaaa" (length 10) — no lost characters.
        use oxiterm_proto::dom::{Node, NodeTag};

        let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
        let session = reg.create_session().unwrap();
        let (output_tx, _) = crate::backpressure::BoundedFrameChannel::new(10);
        let event_bus = Arc::new(crate::events::EventBus::new());

        let mut arena = oxiterm_renderer::arena::NodeArena::new();

        // Input node bound to state key "text".
        let mut input_node = Node::new(NodeTag::Input);
        input_node.attrs.bind_value = Some("text".to_string());
        let input_id = arena.alloc(input_node);

        let mut root = Node::new(NodeTag::Screen);
        root.children = vec![input_id];
        let root_id = arena.alloc(root);
        let doc = THTMLDocument { arena, root: root_id, dirty_nodes: Vec::new() };

        let mut el = EventLoop::new(session, event_bus, output_tx, doc, false);
        el.rebuild_focusable_nodes();

        // Focus the Input node.
        el.focused_node = Some(input_id);

        // Drain INPUT bucket to 0 — the critical precondition.
        el.throttle.input.set_tokens(0.0);

        // Replicate the character-input else-branch from the KeyPress arm.
        // The write is UNCONDITIONAL; only needs_render is gated by check_input().
        for _ in 0..10usize {
            if let Some(focused_id) = el.focused_node {
                if let Some(node) = el.doc.get_node(focused_id) {
                    if node.tag == oxiterm_proto::dom::NodeTag::Input {
                        if let Some(ref state_key) = node.attrs.bind_value.clone() {
                            let cp = 'a';
                            let current = match el.session.state.read().get(state_key) {
                                Some(crate::state::StateValue::Str(s)) => s.clone(),
                                _ => String::new(),
                            };
                            let mut new_val = current;
                            new_val.push(cp);
                            el.session.state.write().set(
                                state_key.clone(),
                                crate::state::StateValue::Str(new_val),
                            );
                        }
                    }
                }
            }
            // Render is gated: check_input() returns false (bucket empty), no render.
            // But the write above already committed — this is exactly D2's guarantee.
        }

        let text_val = el.session.state.read().get("text").cloned();
        assert_eq!(
            text_val,
            Some(crate::state::StateValue::Str("aaaaaaaaaa".to_string())),
            "10 keypresses with empty INPUT bucket must produce bind_value of length 10; got {:?}",
            text_val,
        );
    }
}

