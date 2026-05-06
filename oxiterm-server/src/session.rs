use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::{info, warn};

use oxiterm_renderer::document::THTMLDocument;
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::render::buffer::DoubleBuffer;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::render::emitter::SyncedEmitter;

pub type SessionId = usize;

pub struct SessionRegistry {
    pub sessions: RwLock<HashMap<SessionId, Arc<ClientSession>>>,
    pub next_id: RwLock<SessionId>,
    pub prometheus_registry: Arc<prometheus::Registry>,
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
    pub cursor_pos: usize,
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
    pub raw_input_tx: mpsc::Sender<crate::ssh::reactor::ReactorMessage>,
    /// Channel to receive processed `InputEvents` from the `ReactorThread`
    pub event_rx: Arc<parking_lot::Mutex<mpsc::Receiver<InputEvent>>>,
    pub predictive_echo: RwLock<PredictiveEcho>,
    pub resize_debouncer: RwLock<ResizeDebouncer>,
    pub terminal_profile: RwLock<crate::ssh::negotiator::TerminalProfile>,
}

impl SessionRegistry {
    pub fn new(prometheus_registry: Arc<prometheus::Registry>) -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            next_id: RwLock::new(0),
            prometheus_registry,
        }
    }

    pub fn create_session(&self) -> Arc<ClientSession> {
        let mut id_lock = self.next_id.write();
        let id = *id_lock;
        *id_lock += 1;

        let (raw_tx, raw_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();

        // Spawn the RRT (Resilient Reactor Thread)
        crate::ssh::reactor::ReactorThread::spawn(raw_rx, event_tx);

        let session = Arc::new(ClientSession { 
            id,
            dims: RwLock::new(PtyDimensions { cols: 80, rows: 24 }),
            metrics: crate::metrics::SessionMetrics::new(&id.to_string(), &self.prometheus_registry),
            raw_input_tx: raw_tx,
            event_rx: Arc::new(parking_lot::Mutex::new(event_rx)),
            predictive_echo: RwLock::new(PredictiveEcho::default()),
            resize_debouncer: RwLock::new(ResizeDebouncer {
                pending: None,
                pushed_at: std::time::Instant::now(),
                last_update: std::time::Instant::now(),
            }),
            terminal_profile: RwLock::new(crate::ssh::negotiator::TerminalProfile::default()),
        });
        self.sessions.write().insert(id, session.clone());
        session
    }

    pub fn remove_session(&self, id: SessionId) {
        self.sessions.write().remove(&id);
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
    pub output_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    pub frame_limiter: crate::ratelimit::FrameRateLimiter,
    pub pending_mouse: Option<oxiterm_proto::input::MouseInput>,
}

impl EventLoop {
    pub fn new(session: Arc<ClientSession>, event_bus: Arc<crate::events::EventBus>, output_tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>) -> Self {
        let dims = *session.dims.read();
        let mut document = oxiterm_renderer::document::THTMLDocument::new();
        
        // Dodaj powitalne UI
        let mut box_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Box);
        box_node.style.width = Some(80);
        box_node.style.height = Some(24);
        box_node.style.flex_direction = oxiterm_proto::style::FlexDirection::Column;
        let box_id = document.arena.alloc(box_node);
        document.append_child(document.root, box_id).unwrap();

        let mut text_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Text);
        text_node.text_content = Some("Witaj w OxiTerm! Demo dla Inwestora (Sprint 6). Wpisz coś:".to_string());
        text_node.style.fg = oxiterm_proto::style::AnsiColor::Color256(14);
        text_node.style.width = Some(80);
        text_node.style.height = Some(2);
        let text_id = document.arena.alloc(text_node);
        document.append_child(box_id, text_id).unwrap();

        let mut input_node = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Input);
        input_node.style.width = Some(80);
        input_node.style.height = Some(1);
        let input_id = document.arena.alloc(input_node);
        document.append_child(box_id, input_id).unwrap();

        Self { 
            session, 
            event_bus,
            doc: document,
            layout_engine: LayoutEngine::new(),
            buffer: DoubleBuffer::new(dims.cols, dims.rows),
            output_paused: false,
            output_tx,
            frame_limiter: crate::ratelimit::FrameRateLimiter::new(60),
            pending_mouse: None,
        }
    }

    pub fn run(&mut self) {
        info!("Starting EventLoop for session {}", self.session.id);
        
        // BUG-C03: Removed sleep(200ms). The SSH handshake handles readiness.

        // Initial render before blocking on input
        if let Ok(layout) = self.layout_engine.compute(&mut self.doc) {
            Renderer::render_node(&self.doc, &layout, &mut self.buffer.back);
            let mut out = Vec::new();
            // Włącz Alt Buffer, Wyczyść ekran, Schowaj kursor
            out.extend_from_slice(b"\x1b[?1049h\x1b[2J\x1b[?25l");
            if SyncedEmitter::emit_frame(&mut out, &self.buffer.front, &self.buffer.back).is_ok() {
                if !out.is_empty() {
                    let _ = self.output_tx.send(out);
                    self.buffer.swap();
                }
            }
        }

        let rx_lock = self.session.event_rx.lock();
        
        while let Ok(event) = rx_lock.recv() {
            let mut needs_render = false;
            match event {
                InputEvent::Resize { cols, rows } => {
                    info!("Resizing EventLoop buffer to {}x{}", cols, rows);
                    self.buffer = DoubleBuffer::new(cols, rows);
                    needs_render = true;
                }
                InputEvent::KeyPress(key) => {
                    info!("Key: {:?}", key);
                    let mut echo = self.session.predictive_echo.write();
                    echo.buffer.push(key.codepoint);
                    needs_render = true;
                }
                InputEvent::MouseEvent(mouse) => {
                    // QUAL-01: Store mouse event for processing during render (requires layout)
                    self.pending_mouse = Some(mouse);
                    needs_render = true;
                }
                InputEvent::CapabilityResponse(raw) => {
                    info!("Received DA1 response: {}", String::from_utf8_lossy(&raw));
                    self.session.terminal_profile.write().parse_da1_response(&raw);
                }
                InputEvent::Xoff => {
                    warn!("Received XOFF - pausing output");
                    self.output_paused = true;
                }
                InputEvent::Xon => {
                    info!("Received XON - resuming output");
                    self.output_paused = false;
                    needs_render = true; // force render to catch up
                }
                InputEvent::Refresh => {
                    needs_render = true;
                }
                InputEvent::Unknown(raw) => {
                    warn!("Unknown input: {}", String::from_utf8_lossy(&raw));
                }
            }

            if let Some(dims) = self.session.resize_debouncer.write().poll() {
                *self.session.dims.write() = dims;
                self.buffer = DoubleBuffer::new(dims.cols, dims.rows);
                info!("Resized session {} to {:?}", self.session.id, dims);
                needs_render = true;
            }

            if needs_render && !self.output_paused && self.frame_limiter.should_render() {
                // Non-blocking rate limit — skip render if too soon, no sleep

                if let Ok(layout) = self.layout_engine.compute(&mut self.doc) {
                    // QUAL-01: Process pending mouse event now that we have layout
                    if let Some(mouse) = self.pending_mouse.take() {
                        let _ = self.event_bus.dispatch_mouse(mouse, &mut self.doc, &layout);
                    }

                    Renderer::render_node(&self.doc, &layout, &mut self.buffer.back);
                    
                    // S5-21: PredictiveEcho overlay
                    let echo = self.session.predictive_echo.read();
                    if !echo.buffer.is_empty() {
                        // BUG-H02: Use active node position if available
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
                            let _ = self.output_tx.send(out);
                            self.buffer.swap();
                            self.frame_limiter.record_frame();
                        }
                    }
                }
            }
        }

        // BUG-M04: Send ESU and Show Cursor on close
        let mut cleanup = Vec::new();
        cleanup.extend_from_slice(b"\x1b[?2026l"); // ESU
        cleanup.extend_from_slice(b"\x1b[?25h");   // Show Cursor
        cleanup.extend_from_slice(b"\x1b[?1049l"); // Disable Alt Buffer
        let _ = self.output_tx.send(cleanup);

        info!("EventLoop for session {} finished", self.session.id);
    }
}
