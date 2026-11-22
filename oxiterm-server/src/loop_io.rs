//! The event loop's I/O seam (OxiUI Phase 4, step 6): how it takes input events in, puts events
//! back, and reports to its host.
//!
//! Everything here is core-destined: only core-owned types, no queue implementation. The host
//! owns the concrete queue (see `host_queue.rs`); the loop sees [`EventSource`] and
//! [`EventPush`]. [`MemoryQueue`] is a tiny single-threaded-friendly in-memory implementation
//! for tests and for hosts without threads (wasm).

use crate::session_state::SessionState;
use oxiterm_proto::input::InputEvent;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

/// What the event loop needs from the transport side of a session, besides the pure state.
///
/// The idle reaper's wall-clock state and the "loop is running" flag stay in the server; the loop
/// only reports through this trait.
pub trait LoopHost: Send + Sync {
    /// Records input activity now (feeds the transport's idle reaper).
    fn touch_activity(&self);
    /// Time since the last recorded activity.
    fn idle_for(&self) -> Duration;
    /// Called exactly once when `EventLoop::run` returns, however it exits.
    fn loop_exited(&self);
}

/// Outcome of waiting for the next input event.
#[derive(Debug, Clone, PartialEq)]
pub enum WaitResult {
    /// An event arrived.
    Event(InputEvent),
    /// The wait elapsed with nothing queued.
    Timeout,
    /// The queue is closed and drained; no more events will come.
    Disconnected,
}

/// Where the loop takes its input events from (single consumer).
pub trait EventSource: Send {
    /// Waits up to `timeout` for the next event. Queued events are returned before a closed
    /// queue reports `Disconnected`.
    fn wait_event(&mut self, timeout: Duration) -> WaitResult;
    /// Non-blocking attempt to take the next event.
    fn try_recv(&mut self) -> Option<InputEvent>;
}

/// Where the loop (and replies of the action handler) put events back into the queue.
pub trait EventPush: Send + Sync {
    /// Enqueues `event`; never blocks, may drop it when the queue is full or closed.
    fn push(&self, event: InputEvent);
}

/// The loop's handles on the transport half of its session.
pub struct LoopHandles {
    /// The pure session state.
    pub state: Arc<SessionState>,
    /// Input event source. Locked for the duration of a receive batch, as before the split.
    pub events_in: Arc<parking_lot::Mutex<dyn EventSource>>,
    /// Event enqueueing for action replies.
    pub events_out: Arc<dyn EventPush>,
    /// Activity and lifecycle reporting.
    pub host: Arc<dyn LoopHost>,
}

struct MemoryInner {
    queue: VecDeque<InputEvent>,
    capacity: usize,
    closed: bool,
}

/// In-memory bounded queue: drop-oldest when full, never blocks (a wait on an empty open queue
/// returns `Timeout` at once). Cloning shares the queue; one clone is the source, others push.
#[derive(Clone)]
pub struct MemoryQueue {
    inner: Arc<parking_lot::Mutex<MemoryInner>>,
}

impl MemoryQueue {
    /// Creates an empty open queue holding at most `capacity` events.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(parking_lot::Mutex::new(MemoryInner {
                queue: VecDeque::new(),
                capacity: capacity.max(1),
                closed: false,
            })),
        }
    }

    /// Closes the queue: pushes are dropped, queued events can still be drained.
    pub fn close(&self) {
        self.inner.lock().closed = true;
    }

    /// Reopens the queue and clears anything buffered.
    pub fn reopen(&self) {
        let mut g = self.inner.lock();
        g.closed = false;
        g.queue.clear();
    }

    /// Number of queued events.
    pub fn len(&self) -> usize {
        self.inner.lock().queue.len()
    }

    /// True when nothing is queued.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl EventSource for MemoryQueue {
    fn wait_event(&mut self, _timeout: Duration) -> WaitResult {
        let mut g = self.inner.lock();
        match g.queue.pop_front() {
            Some(e) => WaitResult::Event(e),
            None if g.closed => WaitResult::Disconnected,
            None => WaitResult::Timeout,
        }
    }

    fn try_recv(&mut self) -> Option<InputEvent> {
        self.inner.lock().queue.pop_front()
    }
}

impl EventPush for MemoryQueue {
    fn push(&self, event: InputEvent) {
        let mut g = self.inner.lock();
        if g.closed {
            return;
        }
        if g.queue.len() >= g.capacity {
            g.queue.pop_front();
        }
        g.queue.push_back(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_proto::input::AppPatch;

    fn ev(n: i64) -> InputEvent {
        InputEvent::AppResponse(AppPatch(serde_json::json!(n)))
    }

    #[test]
    fn order_and_timeout() {
        let mut q = MemoryQueue::new(4);
        assert_eq!(q.wait_event(Duration::from_millis(1)), WaitResult::Timeout);
        q.push(ev(1));
        q.push(ev(2));
        assert_eq!(q.wait_event(Duration::ZERO), WaitResult::Event(ev(1)));
        assert_eq!(q.try_recv(), Some(ev(2)));
        assert_eq!(q.try_recv(), None);
    }

    #[test]
    fn drops_oldest_at_capacity() {
        let mut q = MemoryQueue::new(2);
        q.push(ev(1));
        q.push(ev(2));
        q.push(ev(3));
        assert_eq!(q.len(), 2);
        assert_eq!(q.try_recv(), Some(ev(2)));
        assert_eq!(q.try_recv(), Some(ev(3)));
    }

    #[test]
    fn closed_drains_then_disconnects_and_reopen_clears() {
        let mut q = MemoryQueue::new(4);
        q.push(ev(1));
        q.close();
        q.push(ev(2)); // dropped: closed
        assert_eq!(q.wait_event(Duration::ZERO), WaitResult::Event(ev(1)));
        assert_eq!(q.wait_event(Duration::ZERO), WaitResult::Disconnected);
        q.push(ev(3));
        q.reopen();
        assert!(q.is_empty());
        assert_eq!(q.wait_event(Duration::ZERO), WaitResult::Timeout);
        q.push(ev(4));
        assert_eq!(q.try_recv(), Some(ev(4)));
    }
}
