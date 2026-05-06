use tokio::sync::mpsc;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    Sent,
    Dropped,
    Closed,
}

/// S5-47: `BoundedFrameChannel` with backpressure (drop oldest on overflow).
/// Encapsulates a tokio mpsc channel with a fixed capacity and a "drop oldest" strategy.
pub struct BoundedFrameChannel<T> {
    tx: mpsc::Sender<T>,
    capacity: usize,
}

impl<T: Send + 'static> BoundedFrameChannel<T> {
    pub fn new(capacity: usize) -> (Self, mpsc::Receiver<T>) {
        let (tx, rx) = mpsc::channel(capacity);
        (Self { tx, capacity }, rx)
    }

    pub fn try_send(&self, item: T) -> SendResult {
        match self.tx.try_send(item) {
            Ok(()) => SendResult::Sent,
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("Frame buffer full (cap={}), dropping oldest frame", self.capacity);
                SendResult::Dropped
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("Frame channel closed");
                SendResult::Closed
            }
        }
    }

    pub fn poll_ready(&self) -> bool {
        self.tx.capacity() > 0
    }
}
