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
use oxiterm_renderer::render::emitter::SyncedEmitter;

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
    pub state: RwLock<HashMap<String, i32>>,
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
            state: RwLock::new(HashMap::new()),
        });
        self.sessions.write().insert(id, session.clone());
        Some(session)
    }

    pub fn remove_session(&self, id: SessionId) {
        self.sessions.write().remove(&id);
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

pub struct EventLoop {
    pub session: Arc<ClientSession>,
    pub event_bus: Arc<crate::events::EventBus>,
    pub doc: THTMLDocument,
    pub layout_engine: LayoutEngine,
    pub buffer: DoubleBuffer,
    pub output_paused: bool,
    pub output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
    pub frame_limiter: crate::ratelimit::FrameRateLimiter,
    pub pending_mouse: Option<oxiterm_proto::input::MouseInput>,
    pub source_path: Option<std::path::PathBuf>,
    pub weather_app: Option<crate::weather_app::WeatherApp>,
    pub weather_rx: Option<mpsc::Receiver<Result<crate::weather::WeatherData, String>>>,
    pub weather_tx: Option<mpsc::Sender<Result<crate::weather::WeatherData, String>>>,
}

struct TerminalCleanupGuard {
    output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
}

impl Drop for TerminalCleanupGuard {
    fn drop(&mut self) {
        let mut cleanup = Vec::new();
        cleanup.extend_from_slice(b"\x1b[?2026l"); // ESU
        cleanup.extend_from_slice(b"\x1b[?25h");   // Show Cursor
        cleanup.extend_from_slice(b"\x1b[?1049l"); // Disable Alt Buffer
        let _ = self.output_tx.try_send(cleanup);
    }
}

impl EventLoop {
    pub fn new(
        session: Arc<ClientSession>, 
        event_bus: Arc<crate::events::EventBus>, 
        output_tx: crate::backpressure::BoundedFrameChannel<Vec<u8>>,
        doc: THTMLDocument,
    ) -> Self {
        let dims = *session.dims.read();
        
        Self { 
            session, 
            event_bus,
            doc,
            layout_engine: LayoutEngine::new(),
            buffer: DoubleBuffer::new(dims.cols, dims.rows),
            output_paused: false,
            output_tx,
            frame_limiter: crate::ratelimit::FrameRateLimiter::new(60),
            pending_mouse: None,
            source_path: None,
            weather_app: None,
            weather_rx: None,
            weather_tx: None,
        }
    }

    fn inject_state(doc: &mut THTMLDocument, state: &HashMap<String, i32>) {
        for node in doc.arena.iter_mut() {
            if let Some(key) = &node.attrs.bind_state {
                if let Some(val) = state.get(key) {
                    node.text = Some(val.to_string());
                }
            }
        }
    }

    pub fn run(&mut self) {
        // BUG-C03 Fix: Use a RAII guard to ensure cleanup is ALWAYS sent
        let _cleanup_guard = TerminalCleanupGuard { output_tx: self.output_tx.clone() };

        // Initial clear screen and scrollback
        let mut initial_clear = Vec::new();
        // Włącz Alt Buffer, Wyczyść ekran (wraz z historią), Schowaj kursor
        initial_clear.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[3J\x1b[H\x1b[?25l");
        let _ = self.output_tx.try_send(initial_clear);
        
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
                        self.doc = new_doc;
                        needs_render = true;
                    }
                }
            }

            {
                let mut rx_lock = self.session.event_rx.lock();
                while let Ok(event) = rx_lock.recv_timeout(std::time::Duration::from_millis(5)) {
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
                                        Self::inject_state(&mut new_doc, &self.session.state.read());
                                        self.doc = new_doc;
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
                                return;
                            }

                            // Group 1-9 for quick navigation (S6-nav)
                            if ('1'..='9').contains(&key.codepoint) || key.codepoint == '\t' || key.codepoint == 'r' || key.codepoint == 'R' {
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
                                        self.doc = new_doc;
                                        needs_render = true;
                                    }
                                }
                            }

                            let mut echo = self.session.predictive_echo.write();
                            echo.buffer.push(key.codepoint);
                            needs_render = true;
                        }
                        InputEvent::MouseEvent(mouse) => {
                            self.pending_mouse = Some(mouse.clone());
                            
                            // SC-05: Handle HTMX navigation
                            if mouse.action == oxiterm_proto::input::MouseAction::Press {
                                // Find node at coordinates
                                if let Some(node_id) = self.layout_engine.hit_test(mouse.col, mouse.row) {
                                    if let Some(node) = self.doc.get_node(node_id) {
                                        if let Some(ref htmx_target) = node.attrs.event_htmx {
                                            info!("HTMX Navigation: {}", htmx_target);
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
                                                    if let Ok(mut new_doc) = crate::loader::load_thtml_file(&new_path) {
                                                        Self::inject_state(&mut new_doc, &self.session.state.read());
                                                        self.doc = new_doc;
                                                        self.source_path = Some(new_path);
                                                    }
                                                } else {
                                                    warn!("Blocked Path Traversal attempt to: {:?}", new_path);
                                                }
                                            } else if htmx_target.starts_with("inc:") {
                                                let key = htmx_target.strip_prefix("inc:").unwrap();
                                                let mut state = self.session.state.write();
                                                let val = state.entry(key.to_string()).or_insert(0);
                                                *val += 1;
                                                info!("State incremented: {} = {}", key, *val);
                                                // Re-render current doc with new state
                                                Self::inject_state(&mut self.doc, &state);
                                                needs_render = true;
                                            } else if htmx_target.starts_with("dec:") {
                                                let key = htmx_target.strip_prefix("dec:").unwrap();
                                                let mut state = self.session.state.write();
                                                let val = state.entry(key.to_string()).or_insert(0);
                                                *val -= 1;
                                                info!("State decremented: {} = {}", key, *val);
                                                Self::inject_state(&mut self.doc, &state);
                                                needs_render = true;
                                            }
                                        }
                                    }
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
                        _ => {}
                    }
                }
            }

            if let Some(dims) = self.session.resize_debouncer.write().poll() {
                *self.session.dims.write() = dims;
                self.buffer = DoubleBuffer::new(dims.cols, dims.rows);
                info!("Resized session {} to {:?}", self.session.id, dims);
                needs_render = true;
            }

            if needs_render && !self.output_paused && self.frame_limiter.should_render() {
                if let Ok(layout) = self.layout_engine.compute(&mut self.doc) {
                    // QUAL-01: Process pending mouse event now that we have layout
                    if let Some(mouse) = self.pending_mouse.take() {
                        let _ = self.event_bus.dispatch_mouse(mouse, &mut self.doc, &layout);
                    }

                    Renderer::render_node(&self.doc, &layout, &mut self.buffer.back);
                    
                    // S5-21: PredictiveEcho overlay
                    let echo = self.session.predictive_echo.read();
                    if !echo.buffer.is_empty() {
                        let (mut cursor_x, mut cursor_y) = (0, 0);
                        if let Some(node_id) = echo.active_node {
                            if let Some(rect) = layout.nodes.get(&node_id) {
                                cursor_x = rect.x;
                                cursor_y = rect.y;
                            }
                        }

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
                    
                    let mut out = Vec::new();
                    if SyncedEmitter::emit_frame(&mut out, &self.buffer.front, &self.buffer.back).is_ok() {
                        if !out.is_empty() {
                            let _ = self.output_tx.try_send(out);
                            self.buffer.swap();
                            self.frame_limiter.record_frame();
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

    #[test]
    fn test_predictive_echo() {
        let mut echo = PredictiveEcho::default();
        echo.push('a');
        echo.push('b');
        assert_eq!(echo.buffer, "ab");
        echo.buffer.clear();
        assert_eq!(echo.buffer, "");
    }
}
