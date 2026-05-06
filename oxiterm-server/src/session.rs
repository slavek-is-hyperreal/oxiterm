use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::{info, warn};

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

#[derive(Debug)]
pub struct ResizeDebouncer {
    pub pending: Option<PtyDimensions>,
    pub last_update: std::time::Instant,
}

impl ResizeDebouncer {
    pub fn push(&mut self, dims: PtyDimensions) {
        self.pending = Some(dims);
    }

    pub fn poll(&mut self) -> Option<PtyDimensions> {
        if let Some(dims) = self.pending.take() {
            if self.last_update.elapsed() > Duration::from_millis(100) {
                self.last_update = std::time::Instant::now();
                return Some(dims);
            }
            self.pending = Some(dims);
        }
        None
    }
}

pub struct ClientSession {
    pub id: SessionId,
    pub dims: RwLock<PtyDimensions>,
    pub metrics: Arc<crate::metrics::SessionMetrics>,
    /// Channel to send raw bytes to the `ReactorThread`
    pub raw_input_tx: mpsc::Sender<Vec<u8>>,
    /// Channel to receive processed `InputEvents` from the `ReactorThread`
    pub event_rx: Arc<parking_lot::Mutex<mpsc::Receiver<InputEvent>>>,
    pub predictive_echo: RwLock<PredictiveEcho>,
    pub resize_debouncer: RwLock<ResizeDebouncer>,
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
                last_update: std::time::Instant::now(),
            }),
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
}

impl EventLoop {
    pub fn new(session: Arc<ClientSession>, event_bus: Arc<crate::events::EventBus>) -> Self {
        Self { session, event_bus }
    }

    pub fn run(&self) {
        info!("Starting EventLoop for session {}", self.session.id);
        let rx_lock = self.session.event_rx.lock();
        
        while let Ok(event) = rx_lock.recv() {
            match event {
                InputEvent::Resize { cols, rows } => {
                    self.session.resize_debouncer.write().push(PtyDimensions { cols, rows });
                }
                InputEvent::KeyPress(key) => {
                    info!("Key: {:?}", key);
                    let mut echo = self.session.predictive_echo.write();
                    echo.buffer.push(key.codepoint);
                }
                InputEvent::MouseEvent(m) => {
                    info!("Mouse: {:?}", m);
                }
                InputEvent::Unknown(raw) => {
                    warn!("Unknown input: {}", String::from_utf8_lossy(&raw));
                }
            }
            
            if let Some(dims) = self.session.resize_debouncer.write().poll() {
                *self.session.dims.write() = dims;
                info!("Resized session {} to {:?}", self.session.id, dims);
            }
        }
        info!("EventLoop for session {} terminated", self.session.id);
    }
}
