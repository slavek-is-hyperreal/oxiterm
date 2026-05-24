use std::collections::VecDeque;
use std::sync::Arc;
use parking_lot::{Mutex, Condvar};
use tokio::sync::Notify;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendResult {
    Sent,
    Dropped,
    Closed,
}

struct ChannelInner<T> {
    queue: VecDeque<T>,
    capacity: usize,
    closed: bool,
}

/// S5-47: `BoundedFrameChannel` with real backpressure (drop oldest on overflow).
#[derive(Clone)]
pub struct BoundedFrameChannel<T> {
    inner: Arc<(Mutex<ChannelInner<T>>, Condvar)>,
    notify: Arc<Notify>,
    senders: Arc<()>,
}

pub struct Receiver<T> {
    inner: Arc<(Mutex<ChannelInner<T>>, Condvar)>,
    notify: Arc<Notify>,
}

impl<T: Send + 'static> BoundedFrameChannel<T> {
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

    pub fn poll_ready(&self) -> bool {
        let (lock, _) = &*self.inner;
        let inner = lock.lock();
        !inner.closed && inner.queue.len() < inner.capacity
    }
}

impl<T> Receiver<T> {
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

    /// S5-48: Blocking receive for use in dedicated threads (like EventLoop).
    /// BUG-SPINLOCK-01 Fix: Using Condvar instead of sleep(5ms).
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

    pub fn close(&mut self) {
        let (lock, cvar) = &*self.inner;
        let mut inner = lock.lock();
        inner.closed = true;
        self.notify.notify_waiters();
        cvar.notify_all();
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
        
        // This should drop 1 and keep 2, 3
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
