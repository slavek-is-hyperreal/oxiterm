use std::collections::HashMap;
use std::sync::Arc;
use crate::ssh::reactor::ReactorMessage;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::{info, warn};

pub use oxiterm_renderer::document::THTMLDocument;
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::render::buffer::DoubleBuffer;
use oxiterm_renderer::render::renderer::Renderer;

pub type SessionId = usize;

pub struct SessionRegistry {
    pub sessions: RwLock<HashMap<SessionId, Arc<ClientSession>>>,
    pub next_id: RwLock<SessionId>,
    pub prometheus_registry: Arc<prometheus::Registry>,
    pub max_sessions: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct PtyDimensions {
    pub cols: u16,
    pub rows: u16,
}

use std::sync::mpsc;
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::input::InputEvent;

/// S5-21: `PredictiveEcho` for local feedback.
#[derive(Debug, Default)]
pub struct PredictiveEcho {
    pub buffer: String,
    pub active_node: Option<NodeId>,
}

impl PredictiveEcho {
    pub fn push(&mut self, ch: char) {
        self.buffer.push(ch);
    }

    pub fn confirm(&mut self, ch: char) {
        // BUG-H01: Proper FIFO confirmation
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

    pub fn flush_to_server(&mut self) -> String {
        self.buffer.drain(..).collect()
    }
}

#[derive(Debug)]
pub struct ResizeDebouncer {
    pub pending: Option<PtyDimensions>,
    pub pushed_at: std::time::Instant,
    pub last_update: std::time::Instant,
}

impl ResizeDebouncer {
    pub fn new() -> Self {
        Self {
            pending: None,
            pushed_at: std::time::Instant::now(),
            last_update: std::time::Instant::now(),
        }
    }

    pub fn push(&mut self, dims: PtyDimensions) {
        self.pending = Some(dims);
        self.pushed_at = std::time::Instant::now();
    }

    pub fn poll(&mut self) -> Option<PtyDimensions> {
        if let Some(dims) = self.pending {
            // BUG-M01: Check since push, not last update
            if self.pushed_at.elapsed() > Duration::from_millis(100) {
                self.pending = None;
                self.last_update = std::time::Instant::now();
                return Some(dims);
            }
        }
        None
    }
}

pub struct ClientSession {
    pub id: SessionId,
    pub dims: RwLock<PtyDimensions>,
    pub metrics: Arc<crate::metrics::SessionMetrics>,
    /// Channel to send raw bytes or commands to the `ReactorThread`
    pub raw_input_tx: mpsc::Sender<ReactorMessage>,
    /// Channel to receive processed `InputEvents` from the `ReactorThread`
    pub event_rx: Arc<parking_lot::Mutex<crate::backpressure::Receiver<InputEvent>>>,
    pub event_tx: crate::backpressure::BoundedFrameChannel<InputEvent>,
    pub predictive_echo: RwLock<PredictiveEcho>,
    pub resize_debouncer: RwLock<ResizeDebouncer>,
    pub terminal_profile: RwLock<crate::ssh::negotiator::TerminalProfile>,
    pub last_activity: RwLock<std::time::Instant>,
    pub state: RwLock<crate::state::StateManager>,
}

impl ClientSession {
    pub fn close(&self) {
        self.event_rx.lock().close();
    }
}

impl SessionRegistry {
    pub fn new(prometheus_registry: Arc<prometheus::Registry>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            next_id: RwLock::new(0),
            prometheus_registry,
            max_sessions: 20, // Default for demo
        }
    }

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
        });
        self.sessions.write().insert(id, session.clone());
        Some(session)
    }

    pub fn remove_session(&self, id: SessionId) {
        let count_before = self.sessions.read().len();
        if let Some(session) = self.sessions.write().remove(&id) {
            session.close();
        }
        let count_after = self.sessions.read().len();
        if count_after == 0 && count_before > 0 {
            info!("No active sessions remaining. Cleaning up video players.");
            oxiterm_renderer::render::cache::VideoPlayerRegistry::get().cleanup();
        }
    }

    pub fn broadcast_input_event(&self, event: InputEvent) {
        let sessions = self.sessions.read();
        for session in sessions.values() {
            let _ = session.event_tx.try_send(event.clone());
        }
    }

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

pub struct AnsiFrameSink {
    output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
}

impl AnsiFrameSink {
    pub fn new(output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>) -> Self {
        Self { output_tx }
    }
}

impl oxiterm_renderer::FrameSink for AnsiFrameSink {
    fn send_frame(&mut self, front: &oxiterm_renderer::CellBuffer, back: &oxiterm_renderer::CellBuffer) -> anyhow::Result<bool> {
        let commands = oxiterm_renderer::DiffEngine::diff(front, back);
        let transitioning_graphics_cleanup = !front.graphics.is_empty() && back.graphics.is_empty();
        if commands.is_empty() && back.graphics.is_empty() && !transitioning_graphics_cleanup {
            return Ok(false);
        }

        let mut out = Vec::new();
        // BSU: CSI ? 2026 h
        out.extend_from_slice(b"\x1b[?2026h");

        // BUG-KITTY-GHOST-01: clear image placements when transitioning from graphics to no graphics
        if !front.graphics.is_empty() && back.graphics.is_empty() {
            out.extend_from_slice(&oxiterm_renderer::render::kitty::KittyImageManager::delete_all_placements());
        }
        
        let bytes = oxiterm_renderer::DiffEngine::encode_ansi(&commands);
        out.extend_from_slice(&bytes);
        
        for g in &back.graphics {
            out.extend_from_slice(g);
        }
        
        // ESU: CSI ? 2026 l
        out.extend_from_slice(b"\x1b[?2026l");
        
        let _ = self.output_tx.try_send(out);
        Ok(true)
    }

    fn setup(&mut self) -> anyhow::Result<()> {
        let mut initial_clear = Vec::new();
        // Włącz Alt Buffer, Wyczyść ekran (wraz z historią), Schowaj kursor
        initial_clear.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[3J\x1b[H\x1b[?25l");
        let _ = self.output_tx.try_send(initial_clear);
        Ok(())
    }

    fn clear_screen(&mut self) -> anyhow::Result<()> {
        let mut clear_seq = Vec::new();
        // Send delete_all_placements before ESC[2J
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

pub struct EventLoop {
    pub session: Arc<ClientSession>,
    pub event_bus: Arc<crate::events::EventBus>,
    pub doc: THTMLDocument,
    pub layout_engine: LayoutEngine,
    pub buffer: DoubleBuffer,
    pub output_paused: bool,
    pub output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
    pub frame_sink: Box<dyn oxiterm_renderer::FrameSink>,
    pub frame_limiter: crate::ratelimit::FrameRateLimiter,
    pub pending_mouse: Option<oxiterm_proto::input::MouseInput>,
    pub source_path: Option<std::path::PathBuf>,
    pub weather_app: Option<crate::weather_app::WeatherApp>,
    pub weather_rx: Option<mpsc::Receiver<Result<crate::weather::WeatherData, String>>>,
    pub weather_tx: Option<mpsc::Sender<Result<crate::weather::WeatherData, String>>>,
    pub dbus_bridge: Option<oxiterm_a11y::DBusBridge>,
    pub parent_map: HashMap<NodeId, NodeId>,
    pub focusable_nodes: Vec<NodeId>,
    pub focused_node: Option<NodeId>,
    pub scroll_offset: u16,
    pub total_height: u16,
    /// Optional dispatcher for external app server notifications.
    pub app_dispatcher: Option<crate::dispatcher::AppDispatcher>,
}

impl EventLoop {
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
            weather_app: None,
            weather_rx: None,
            weather_tx: None,
            dbus_bridge,
            parent_map: HashMap::new(),
            focusable_nodes: Vec::new(),
            focused_node: None,
            scroll_offset: 0,
            total_height: 0,
            app_dispatcher: None,
        };
        event_loop.rebuild_parent_map();
        event_loop.rebuild_focusable_nodes();
        event_loop
    }

    pub fn reset_layout_and_scroll(&mut self) {
        self.layout_engine.reset_nodes();
        self.scroll_offset = 0;
    }

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

    pub fn update_doc(&mut self, new_doc: THTMLDocument) {
        self.doc = new_doc;
        self.rebuild_parent_map();
        self.rebuild_focusable_nodes();
    }

    pub fn rebuild_focusable_nodes(&mut self) {
        let mut nodes = Vec::new();
        let mut stack = vec![self.doc.root];
        while let Some(id) = stack.pop() {
            if let Some(node) = self.doc.arena.get(id) {
                // Focusable: any node with event_htmx OR an <input> with bind_value
                let is_interactive = node.attrs.event_htmx.is_some()
                    || (node.tag == oxiterm_proto::dom::NodeTag::Input
                        && node.attrs.bind_value.is_some());
                if is_interactive {
                    nodes.push(id);
                }
                // push children in reverse so left-to-right / top-to-bottom order
                for &child in node.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        // If current focused node is gone, reset focus
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
        }
    }

    fn sync_dirty_state(doc: &mut THTMLDocument, state: &mut crate::state::StateManager) {
        let dirty_nodes = state.get_dirty_nodes();
        for node_id in dirty_nodes {
            if let Some(node) = doc.arena.get_mut(node_id) {
                if let Some(key) = &node.attrs.bind_state {
                    if let Some(val) = state.get(key) {
                        node.text = Some(val.to_string());
                        doc.mark_dirty(node_id);
                    }
                }
            }
        }
    }

    fn inject_initial_state(doc: &mut THTMLDocument, state: &crate::state::StateManager) {
        let mut dirty = Vec::new();
        for (id, node) in doc.arena.iter_mut() {
            if let Some(key) = &node.attrs.bind_state {
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

    /// If an `AppDispatcher` is configured, snapshot the current state and fire a POST.
    fn try_dispatch(&self, action: &str) {
        if let Some(ref dispatcher) = self.app_dispatcher {
            let state_guard = self.session.state.read();
            let mut state_snapshot = std::collections::HashMap::new();
            // Collect all known keys referenced in the DOM as bind_state or bind_value.
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
                    // bind_show condition: extract key before '='
                    let key_part = key.split('=').next().unwrap_or(key);
                    if let Some(val) = state_guard.get(key_part) {
                        state_snapshot.insert(key_part.to_string(), val.to_string());
                    }
                }
            }
            drop(state_guard);
            let payload = crate::dispatcher::DispatchPayload {
                action: action.to_string(),
                state: state_snapshot,
                session_id: self.session.id,
            };
            dispatcher.dispatch(payload);
        }
    }

    pub fn run(&mut self) {
        {
            let mut state = self.session.state.write();
            Self::setup_state_subscriptions(&self.doc, &mut *state);
            Self::inject_initial_state(&mut self.doc, &*state);
        }

        // Initial clear screen and scrollback
        let _ = self.frame_sink.setup();
        
        let mut first_frame = true;
        loop {
            // SA-10: Idle timeout check (10 minutes). Explicitly return to close the session.
            let elapsed = self.session.last_activity.read().elapsed();
            if elapsed > std::time::Duration::from_secs(600) {
                info!("Session {} timed out ({}s idle)", self.session.id, elapsed.as_secs());
                return;
            }

            let mut needs_render = first_frame;
            first_frame = false;

            // Poll for weather updates
            if let Some(ref rx) = self.weather_rx {
                if let Ok(result) = rx.try_recv() {
                    if let Some(ref mut app) = self.weather_app {
                        match result {
                            Ok(data) => {
                                app.data = Some(data);
                                app.error = None;
                            }
                            Err(e) => {
                                app.error = Some(e);
                            }
                        }
                        app.loading = false;
                        let dims = *self.session.dims.read();
                        let (new_doc, _) = app.build_document(dims.cols, dims.rows);
                        self.update_doc(new_doc);
                        self.reset_layout_and_scroll();
                        needs_render = true;
                    }
                }
            }

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
                loop {
                    match rx_lock.recv_timeout(sleep_dur) {
                        Ok(event) => {
                            *self.session.last_activity.write() = std::time::Instant::now();
                            let event = match event {
                                InputEvent::KeyPress(key) if key.codepoint == '\u{F72E}' || key.codepoint == 'b' => InputEvent::ScrollUp,
                                InputEvent::KeyPress(key) if key.codepoint == '\u{F72D}' || key.codepoint == ' ' => InputEvent::ScrollDown,
                                e => e,
                            };
                            match event {
                                InputEvent::Resize { cols, rows } => {
                                    info!("Reactor notified resize to {}x{}", cols, rows);
                                    // BUG-RESIZE-01: We handle buffer recreation in resize_debouncer.poll()
                                    // to avoid double allocation. We just signal that a render might be needed.
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
                                                // TODO: Show error overlay in TUI
                                            }
                                        }
                                    }
                                }
                                InputEvent::KeyPress(key) => {
                                    if key.codepoint == 'q' || key.codepoint == 'Q' {
                                        info!("Quit requested via keyboard: {}", key.codepoint);
                                        disconnected = true;
                                        break;
                                    }

                                    // Arrow keys / Tab: move focus between event-htmx nodes
                                    // Browser sends 0xF700=Up, 0xF701=Down, 0xF702=Right, 0xF703=Left
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
                                        // Enter: activate focused node's HTMX action
                                        if let Some(focused) = self.focused_node {
                                            if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(focused) {
                                                info!("Enter activated HTMX (node {:?}): {}", target_node_id, htmx_target);
                                                if htmx_target.ends_with(".thtml") || !htmx_target.contains(':') {
                                                    let mut filename = htmx_target.to_string();
                                                    if !filename.ends_with(".thtml") {
                                                        filename.push_str(".thtml");
                                                    }
                                                    let base = self.source_path.as_ref()
                                                        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                                                        .unwrap_or_else(|| std::path::PathBuf::from("."));
                                                    let new_path = base.join(&filename);
                                                    let is_safe = (|| -> Option<bool> {
                                                        let canonical_base = base.canonicalize().ok()?;
                                                        let canonical_target = new_path.canonicalize().ok()?;
                                                        Some(canonical_target.starts_with(canonical_base))
                                                    })().unwrap_or(false);
                                                    if is_safe {
                                                        match crate::loader::load_thtml_file(&new_path) {
                                                            Ok(mut new_doc) => {
                                                                let mut state = self.session.state.write();
                                                                Self::setup_state_subscriptions(&new_doc, &mut *state);
                                                                Self::inject_initial_state(&mut new_doc, &*state);
                                                                drop(state);
                                                                self.update_doc(new_doc);
                                                                self.reset_layout_and_scroll();
                                                                self.source_path = Some(new_path);
                                                                needs_render = true;
                                                            }
                                                            Err(e) => warn!("Enter: failed to load {:?}: {}", new_path, e),
                                                        }
                                                    }
                                                } else {
                                                    self.session.state.write().apply_action(&htmx_target);
                                                    self.try_dispatch(&htmx_target);
                                                    needs_render = true;
                                                }
                                            }
                                        }
                                    } else if ('1'..='9').contains(&key.codepoint) || key.codepoint == 'r' || key.codepoint == 'R' {
                                        // Group 1-9 for quick navigation (S6-nav / weather)
                                        if let Some(ref mut app) = self.weather_app {
                                            if key.codepoint == 'r' || key.codepoint == 'R' {
                                                if !app.loading {
                                                    if let Some(ref tx) = self.weather_tx {
                                                        app.loading = true;
                                                        let tx_clone = tx.clone();
                                                        std::thread::spawn(move || {
                                                            let res = crate::weather::fetch_krakow()
                                                                .map_err(|e| e.to_string());
                                                            let _ = tx_clone.send(res);
                                                        });
                                                        needs_render = true;
                                                    }
                                                }
                                            } else if app.handle_key(key.codepoint) {
                                                info!("WeatherApp handled key: {}", key.codepoint);
                                                let dims = *self.session.dims.read();
                                                let (new_doc, _) = app.build_document(dims.cols, dims.rows);
                                                self.update_doc(new_doc);
                                                self.reset_layout_and_scroll();
                                                needs_render = true;
                                            }
                                        }
                                    } else {
                                        // Check if focused node is an <input> with bind_value
                                        let handled_by_input = if let Some(focused_id) = self.focused_node {
                                            if let Some(node) = self.doc.get_node(focused_id) {
                                                if node.tag == oxiterm_proto::dom::NodeTag::Input {
                                                    if let Some(ref state_key) = node.attrs.bind_value.clone() {
                                                        let cp = key.codepoint;
                                                        if cp as u32 == 127 || cp as u32 == 8 {
                                                            // Backspace: remove last char from state
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
                                                            // Enter on input: fire event_htmx if present, but don't consume
                                                            // (let the outer Enter branch handle it via get_htmx_node_and_target)
                                                            false
                                                        } else if !cp.is_control() {
                                                            // Printable char: append to state
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
                                            let mut echo = self.session.predictive_echo.write();
                                            echo.buffer.push(key.codepoint);
                                            needs_render = true;
                                        }
                                    }
                                }
                                InputEvent::MouseEvent(mut mouse) => {
                                    let dims = *self.session.dims.read();
                                    info!("Received MouseEvent: col={}, row={}, action={:?}", mouse.col, mouse.row, mouse.action);
                                    if let Some(layout) = &self.layout_engine.last_layout {
                                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
                                        info!("Document centering offset: offset_x={}, offset_y={}", offset_x, offset_y);
                                        mouse.col = mouse.col.saturating_sub(offset_x).saturating_sub(1);
                                        mouse.row = mouse.row.saturating_sub(offset_y).saturating_sub(1).saturating_add(self.scroll_offset);
                                        info!("Adjusted MouseEvent: col={}, row={}", mouse.col, mouse.row);
                                    } else {
                                        warn!("No last layout found!");
                                    }
                                    
                                    // Handle hover and click coordinates mapping for Rive animations
                                    self.update_interactive_animations(&mouse);

                                    self.pending_mouse = Some(mouse.clone());
                                    
                                    // SC-05: Handle HTMX navigation
                                    if mouse.action == oxiterm_proto::input::MouseAction::Press {
                                        // Find node at coordinates
                                        if let Some(node_id) = self.layout_engine.hit_test(mouse.col, mouse.row) {
                                            info!("Hit test found node: {:?}", node_id);
                                            if let Some(node) = self.doc.get_node(node_id) {
                                                info!("Node details: tag={:?}, attrs={:?}", node.tag, node.attrs);
                                            }
                                            if let Some((target_node_id, htmx_target)) = self.get_htmx_node_and_target(node_id) {
                                                info!("Found HTMX target (node {:?}): {}", target_node_id, htmx_target);
                                                if htmx_target.ends_with(".thtml") || !htmx_target.contains(':') {
                                                    // BUG-SEC-01: Path Traversal prevention
                                                    let base_dir = self.source_path.as_ref()
                                                        .and_then(|p| p.parent())
                                                        .map(|p| p.to_path_buf())
                                                        .unwrap_or_else(|| std::env::current_dir().unwrap());
                                                    
                                                    let mut filename = htmx_target.to_string();
                                                    if !filename.ends_with(".thtml") {
                                                        filename.push_str(".thtml");
                                                    }
                                                    
                                                    let new_path = base_dir.join(filename);
                                                    
                                                    let is_safe = (|| -> Option<bool> {
                                                        let canonical_base = base_dir.canonicalize().ok()?;
                                                        let canonical_target = new_path.canonicalize().ok()?;
                                                        Some(canonical_target.starts_with(canonical_base))
                                                    })().unwrap_or(false);

                                                    if is_safe {
                                                        match crate::loader::load_thtml_file(&new_path) {
                                                            Ok(mut new_doc) => {
                                                                let mut state = self.session.state.write();
                                                                Self::setup_state_subscriptions(&new_doc, &mut *state);
                                                                Self::inject_initial_state(&mut new_doc, &*state);
                                                                drop(state);
                                                                self.update_doc(new_doc);
                                                                self.reset_layout_and_scroll();
                                                                self.source_path = Some(new_path);
                                                            }
                                                            Err(e) => {
                                                                warn!("Failed to load THTML file {:?}: {}", new_path, e);
                                                            }
                                                        }
                                                    } else {
                                                        warn!("Blocked Path Traversal attempt to: {:?}", new_path);
                                                    }
                                                } else {
                                                    // Handle general HTMX action (inc, dec, toggle, set, etc.)
                                                    self.session.state.write().apply_action(&htmx_target);
                                                    self.try_dispatch(&htmx_target);
                                                }
                                            } else {
                                                info!("No HTMX target found for node {:?}", node_id);
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
                                    // TextInput is generated when an external dispatcher routes
                                    // typed text to a focused input node. Currently a no-op at
                                    // session level; dispatching happens via AppDispatcher.
                                    info!("TextInput received (len={}): {:?}", text.len(), text);
                                }
                                _ => {}
                            }
                        }
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

                if let Some(ref mut app) = self.weather_app {
                    let (new_doc, _) = app.build_document(dims.cols, dims.rows);
                    self.update_doc(new_doc);
                    self.reset_layout_and_scroll();
                }

                // Clear physical screen and cursor to home to match clean front buffer
                let _ = self.frame_sink.clear_screen();

                needs_render = true;
            }

            if needs_render && !self.output_paused && self.frame_limiter.should_render() {
                // Sync reactive state before render
                Self::sync_dirty_state(&mut self.doc, &mut *self.session.state.write());

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

                    // BUG-HTMX-ENTER-MISSING-SCROLL-01: Auto-scroll to focused node if it exists and is out of the viewport
                    if let Some(focused_id) = self.focused_node {
                        if let Some(rect) = layout.nodes.get(&focused_id) {
                            if rect.y < self.scroll_offset {
                                self.scroll_offset = rect.y;
                            } else if rect.y + rect.height > self.scroll_offset + viewport_h {
                                self.scroll_offset = (rect.y + rect.height).saturating_sub(viewport_h);
                            }
                        }
                    }

                    // Clamp scroll_offset to valid bounds
                    let max_scroll = self.total_height.saturating_sub(viewport_h);
                    self.scroll_offset = self.scroll_offset.min(max_scroll);

                    if let Some(ref mut bridge) = self.dbus_bridge {
                        let tree = oxiterm_a11y::build_a11y_tree(&self.doc);
                        let _ = bridge.register_at_spi(&tree);
                        if let Some(active_id) = self.session.predictive_echo.read().active_node {
                            let _ = bridge.update_focus(active_id, &tree);
                        }
                    }

                    // QUAL-01: Process pending mouse event now that we have layout
                    if let Some(mouse) = self.pending_mouse.take() {
                        let _ = self.event_bus.dispatch_mouse(mouse, &mut self.doc, &layout);
                    }

                    let profile = self.session.terminal_profile.read().clone();
                    let base_dir = self.source_path.as_ref().and_then(|p| p.parent());
                    self.buffer.back.clear();
                    Renderer::render_node(&self.doc, &layout, &mut self.buffer.back, &profile, base_dir, self.scroll_offset);

                    // Focus ring: draw bright-cyan ▶/◀ markers on left/right edge of focused node
                    if let Some(focused_id) = self.focused_node {
                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);
                        if let Some(rect) = layout.nodes.get(&focused_id) {
                            let mid_row_offset = (rect.y + rect.height / 2 + offset_y) as i32 - self.scroll_offset as i32;
                            // BUG-FOCUS-RING-SCROLLED-01: Restrict drawing to viewport_h and prevent overwriting status bar
                            if mid_row_offset >= 0 && mid_row_offset < viewport_h as i32 {
                                let mid_row = mid_row_offset as u16;
                                let focus_fg = oxiterm_proto::style::AnsiColor::Color256(51); // bright cyan
                                let focus_bg = oxiterm_proto::style::AnsiColor::Reset;
                                // Left bracket
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
                                // Right bracket
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
                    let echo = self.session.predictive_echo.read();
                    if !echo.buffer.is_empty() {
                        let (offset_x, offset_y) = layout.get_centering_offset(&self.doc, dims.cols, dims.rows);

                        let (mut cursor_x, mut cursor_y) = (offset_x, offset_y.saturating_sub(self.scroll_offset));
                        if let Some(node_id) = echo.active_node {
                            if let Some(rect) = layout.nodes.get(&node_id) {
                                cursor_x = rect.x + offset_x;
                                cursor_y = (rect.y + offset_y).saturating_sub(self.scroll_offset);
                            }
                        }

                        // Prevent drawing predictive echo on the status bar row
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

                    // BUG-SCROLL-NO-INDICATOR-01: Draw scroll status bar if total_height > dims.rows
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
                        status_parts.push(format!("wiersz {}/{}", current_row, self.total_height));
                        
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
            }
        }
    }

    fn has_active_animations(&self) -> bool {
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

        // Test 4: empty frame with no graphics changes
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(!res);

        // Test 3: transition from graphics to no graphics -> should contain delete_all_placements
        front.graphics.push(b"some_img".to_vec());
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(res);
        let frame = rx.try_recv().expect("Should receive frame");
        // Frame should contain delete_all_placements sequence: \x1b_Ga=d,d=A\x1b\\
        let delete_seq = b"\x1b_Ga=d,d=A\x1b\\";
        let seq_pos = frame.windows(delete_seq.len()).position(|w| w == delete_seq);
        assert!(seq_pos.is_some(), "delete_all_placements sequence not found in frame");

        // Test 5: transition from no graphics to graphics -> no delete_all_placements, but contains back graphics
        front.graphics.clear();
        back.graphics.push(b"new_img_data".to_vec());
        let res = sink.send_frame(&front, &back).unwrap();
        assert!(res);
        let frame2 = rx.try_recv().expect("Should receive frame 2");
        let seq_pos2 = frame2.windows(delete_seq.len()).position(|w| w == delete_seq);
        assert!(seq_pos2.is_none(), "delete_all_placements should not be sent on transition to graphics");
        let img_pos = frame2.windows(12).position(|w| w == b"new_img_data");
        assert!(img_pos.is_some(), "new_img_data not found in frame");

        // Test 6: clear_screen -> should contain delete_all_placements before ESC[2J
        sink.clear_screen().unwrap();
        let frame3 = rx.try_recv().expect("Should receive clear screen");
        let seq_pos3 = frame3.windows(delete_seq.len()).position(|w| w == delete_seq);
        let clear_pos = frame3.windows(4).position(|w| w == b"\x1b[2J");
        assert!(seq_pos3.is_some(), "delete_all_placements not found in clear_screen");
        assert!(clear_pos.is_some(), "ESC[2J not found in clear_screen");
        assert!(seq_pos3.unwrap() < clear_pos.unwrap(), "delete_all_placements must be before ESC[2J");
    }
}
