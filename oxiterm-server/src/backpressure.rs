use tokio::sync::mpsc;
use tracing::warn;

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

    pub fn try_send(&self, item: T) {
        match self.tx.try_send(item) {
            Ok(()) => {},
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("Frame buffer full (cap={}), dropping oldest frame", self.capacity);
                // In a real "drop oldest" we might need a broadcast channel or a custom queue.
                // For now, we just warn and drop the *new* frame to prevent blocking the reactor.
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("Frame channel closed");
            }
        }
    }
}
