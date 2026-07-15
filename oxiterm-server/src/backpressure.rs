//! Backpressure-aware bounded message channel.
//!
//! Provides a channel that drops the oldest frame on capacity overflow to prevent
//! memory leakage under high latency, supporting asynchronous, blocking, and timed receives.

use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::{Mutex, Condvar};
use tokio::sync::Notify;

/// Outcome of attempting to send a message through the channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    /// Message successfully pushed to the queue.
    Sent,
    /// Message was sent, but the oldest message in the queue was dropped to fit capacity.
    Dropped,
    /// The channel has been closed; message was rejected.
    Closed,
}

struct ChannelInner<T> {
    queue: VecDeque<T>,
    capacity: usize,
    closed: bool,
}

/// A multi-sender, single-receiver channel enforcing real backpressure by dropping the oldest frame on overflow.
///
/// Anchored by spec [S5-47].
#[derive(Clone)]
pub struct BoundedFrameChannel<T> {
    inner: Arc<(Mutex<ChannelInner<T>>, Condvar)>,
    notify: Arc<Notify>,
    senders: Arc<()>,
}

/// Receiving handle for the bounded frame channel.
pub struct Receiver<T> {
    inner: Arc<(Mutex<ChannelInner<T>>, Condvar)>,
    notify: Arc<Notify>,
}

impl<T: Send + 'static> BoundedFrameChannel<T> {
    /// Creates a new bounded frame channel with the specified capacity.
    pub fn new(capacity: usize) -> (Self, Receiver<T>) {
        let inner = Arc::new((
            Mutex::new(ChannelInner {
                queue: VecDeque::with_capacity(capacity),
                capacity,
                closed: false,
            }),
            Condvar::new()
        ));
        let notify = Arc::new(Notify::new());
        let senders = Arc::new(());
        
        let tx = Self { inner: inner.clone(), notify: notify.clone(), senders };
        let rx = Receiver { inner, notify };
        
        (tx, rx)
    }

    /// Attempts to enqueue a message. Drops the oldest message if the queue is full.
    pub fn try_send(&self, item: T) -> SendResult {
        let (lock, cvar) = &*self.inner;
        let mut inner = lock.lock();
        if inner.closed {
            return SendResult::Closed;
        }

        let res = if inner.queue.len() >= inner.capacity {
            inner.queue.pop_front();
            inner.queue.push_back(item);
            SendResult::Dropped
        } else {
            inner.queue.push_back(item);
            SendResult::Sent
        };

        self.notify.notify_one();
        cvar.notify_one();
        res
    }

    /// Returns `true` if the channel is open and has space for new elements.
    pub fn poll_ready(&self) -> bool {
        let (lock, _) = &*self.inner;
        let inner = lock.lock();
        !inner.closed && inner.queue.len() < inner.capacity
    }
}

impl<T> Receiver<T> {
    /// Asynchronously receives the next message, yielding the task if empty.
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            {
                let (lock, _) = &*self.inner;
                let mut inner = lock.lock();
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

    /// Blocks the current thread until a message is available.
    ///
    /// Anchored by spec [S5-48]. Uses a conditional variable (`Condvar`) to park the
    /// thread while waiting, avoiding high CPU consumption spin-locks or arbitrary sleep polling.
    pub fn blocking_recv(&mut self) -> Option<T> {
        let (lock, cvar) = &*self.inner;
        let mut inner = lock.lock();
        loop {
            if let Some(item) = inner.queue.pop_front() {
                return Some(item);
            }
            if inner.closed {
                return None;
            }
            cvar.wait(&mut inner);
        }
    }

    /// Blocks the current thread waiting for a message, up to the specified timeout duration.
    pub fn recv_timeout(&mut self, timeout: std::time::Duration) -> Result<T, std::sync::mpsc::RecvTimeoutError> {
        let (lock, cvar) = &*self.inner;
        let mut inner = lock.lock();
        
        if inner.queue.is_empty() && !inner.closed {
            let _ = cvar.wait_for(&mut inner, timeout);
        }
        
        if let Some(item) = inner.queue.pop_front() {
            return Ok(item);
        }
        if inner.closed {
            return Err(std::sync::mpsc::RecvTimeoutError::Disconnected);
        }
        
        Err(std::sync::mpsc::RecvTimeoutError::Timeout)
    }

    /// Non-blocking attempt to retrieve a message if immediately available.
    pub fn try_recv(&mut self) -> Option<T> {
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock();
        inner.queue.pop_front()
    }

    /// Closes the channel, notifying all blocked receivers.
    pub fn close(&mut self) {
        let (lock, cvar) = &*self.inner;
        let mut inner = lock.lock();
        inner.closed = true;
        self.notify.notify_waiters();
        cvar.notify_all();
    }

    /// Reopens the channel and clears any buffered messages.
    pub fn reopen(&mut self) {
        let (lock, _) = &*self.inner;
        let mut inner = lock.lock();
        inner.closed = false;
        inner.queue.clear();
    }
}

impl<T> Drop for BoundedFrameChannel<T> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.senders) == 1 {
            let (lock, cvar) = &*self.inner;
            let mut inner = lock.lock();
            inner.closed = true;
            self.notify.notify_waiters();
            cvar.notify_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_bounded_channel_capacity() {
        let (tx, mut rx) = BoundedFrameChannel::<i32>::new(2);
        assert_eq!(tx.try_send(1), SendResult::Sent);
        assert_eq!(tx.try_send(2), SendResult::Sent);
        
        assert_eq!(tx.try_send(3), SendResult::Dropped);
        
        assert_eq!(rx.blocking_recv(), Some(2));
        assert_eq!(rx.blocking_recv(), Some(3));
    }

    #[tokio::test]
    async fn test_async_recv() {
        let (tx, mut rx) = BoundedFrameChannel::<i32>::new(10);
        tx.try_send(42);
        assert_eq!(rx.recv().await, Some(42));
    }

    #[test]
    fn test_recv_timeout() {
        let (_tx, mut rx) = BoundedFrameChannel::<i32>::new(1);
        let res = rx.recv_timeout(Duration::from_millis(10));
        assert!(res.is_err());
    }
}
