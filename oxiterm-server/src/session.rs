use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use std::time::Duration;
use tracing::info;

pub type SessionId = usize;

pub struct SessionRegistry {
    pub sessions: RwLock<HashMap<SessionId, Arc<ClientSession>>>,
    pub next_id: RwLock<SessionId>,
}

#[derive(Debug, Clone, Copy)]
pub struct PtyDimensions {
    pub cols: u16,
    pub rows: u16,
}

pub struct ClientSession {
    pub id: SessionId,
    pub dims: RwLock<PtyDimensions>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            next_id: RwLock::new(0),
        }
    }

    pub fn create_session(&self) -> Arc<ClientSession> {
        let mut id_lock = self.next_id.write();
        let id = *id_lock;
        *id_lock += 1;

        let session = Arc::new(ClientSession { 
            id,
            dims: RwLock::new(PtyDimensions { cols: 80, rows: 24 }),
        });
        self.sessions.write().insert(id, session.clone());
        session
    }

    pub fn remove_session(&self, id: SessionId) {
        self.sessions.write().remove(&id);
    }

    pub async fn drain_sessions(&self, timeout: Duration) {
        info!("Draining sessions with timeout: {:?}", timeout);
        // In a real implementation, we would signal all sessions to close
        // and wait for them to finish or timeout.
        let start = std::time::Instant::now();
        while !self.sessions.read().is_empty() && start.elapsed() < timeout {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}
