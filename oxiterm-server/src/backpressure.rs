use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::Mutex;
use tokio::sync::Notify;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    Sent,
    Dropped,
    Closed,
}

/// S5-47: `BoundedFrameChannel` with real backpressure (drop oldest on overflow).
/// Uses a Mutex-protected VecDeque and a Notify handle for async coordination.
pub struct BoundedFrameChannel<T> {
    inner: Arc<Mutex<ChannelInner<T>>>,
    notify: Arc<Notify>,
}

struct ChannelInner<T> {
    queue: VecDeque<T>,
    capacity: usize,
    closed: bool,
}

pub struct Receiver<T> {
    inner: Arc<Mutex<ChannelInner<T>>>,
    notify: Arc<Notify>,
}

impl<T: Send + 'static> BoundedFrameChannel<T> {
    pub fn new(capacity: usize) -> (Self, Receiver<T>) {
        let inner = Arc::new(Mutex::new(ChannelInner {
            queue: VecDeque::with_capacity(capacity),
            capacity,
            closed: false,
        }));
        let notify = Arc::new(Notify::new());
        
        let tx = Self { inner: inner.clone(), notify: notify.clone() };
        let rx = Receiver { inner, notify };
        
        (tx, rx)
    }

    pub fn try_send(&self, item: T) -> SendResult {
        let mut inner = self.inner.lock();
        if inner.closed {
            return SendResult::Closed;
        }

        if inner.queue.len() >= inner.capacity {
            // Drop oldest
            inner.queue.pop_front();
            warn!("Frame buffer full, dropped oldest frame");
            inner.queue.push_back(item);
            self.notify.notify_one();
            SendResult::Dropped
        } else {
            inner.queue.push_back(item);
            self.notify.notify_one();
            SendResult::Sent
        }
    }

    pub fn poll_ready(&self) -> bool {
        let inner = self.inner.lock();
        !inner.closed && inner.queue.len() < inner.capacity
    }
}

impl<T> Receiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            {
                let mut inner = self.inner.lock();
                if let Some(item) = inner.queue.pop_front() {
                    return Some(item);
                }
                if inner.closed {
                    return None;
                }
            }
            self.notify.notified().await;
        }
    }

    pub fn close(&mut self) {
        let mut inner = self.inner.lock();
        inner.closed = true;
        self.notify.notify_waiters();
    }
}

impl<T> Drop for BoundedFrameChannel<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inner) <= 2 {
            let mut inner = self.inner.lock();
            inner.closed = true;
            self.notify.notify_waiters();
        }
    }
}
