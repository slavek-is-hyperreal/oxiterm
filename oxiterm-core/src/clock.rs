//! Injected monotonic time (OxiUI Phase 4, step 1).
//!
//! Code that will move into `oxiterm-core` must never read the wall/monotonic clock itself:
//! `Instant::now()` panics on `wasm32-unknown-unknown` at runtime. Instead the host supplies a
//! [`Clock`], and everything below the event loop works with [`MonoTime`] values.
//!
//! * [`MonoTime`] is a duration measured from an arbitrary, host-chosen origin. It is plain
//!   data (`Copy`, `Ord`), constructible in tests and wasm-safe. Subtraction saturates, like
//!   `Instant::duration_since`.
//! * `host_clock::SystemClock` is the host clock of the server (`Instant` at creation + elapsed),
//!   kept in its own module because it is the one place that reads a real clock; a wasm host
//!   provides its own `Clock`. The idle reaper's wall clock is transport state, not part of this.
//! * [`FakeClock`] is a manually advanced clock for tests and the trace harness.

use std::ops::{Add, Sub};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// A monotonic timestamp: time elapsed since an arbitrary origin chosen by the host.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MonoTime(Duration);

impl MonoTime {
    /// The origin.
    pub const ZERO: MonoTime = MonoTime(Duration::ZERO);

    /// A timestamp `d` after the origin.
    pub const fn from_duration(d: Duration) -> Self {
        MonoTime(d)
    }

    /// A timestamp `ms` milliseconds after the origin.
    pub const fn from_millis(ms: u64) -> Self {
        MonoTime(Duration::from_millis(ms))
    }

    /// Time since the origin.
    pub const fn as_duration(self) -> Duration {
        self.0
    }

    /// Time from `earlier` to `self`, zero if `earlier` is later (like `Instant::duration_since`).
    pub fn saturating_duration_since(self, earlier: MonoTime) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

impl Add<Duration> for MonoTime {
    type Output = MonoTime;
    fn add(self, rhs: Duration) -> MonoTime {
        MonoTime(self.0 + rhs)
    }
}

/// Saturates at the origin instead of panicking.
impl Sub<Duration> for MonoTime {
    type Output = MonoTime;
    fn sub(self, rhs: Duration) -> MonoTime {
        MonoTime(self.0.saturating_sub(rhs))
    }
}

/// Saturating difference of two timestamps.
impl Sub<MonoTime> for MonoTime {
    type Output = Duration;
    fn sub(self, rhs: MonoTime) -> Duration {
        self.saturating_duration_since(rhs)
    }
}

/// A source of monotonic time, supplied by the host.
pub trait Clock: Send + Sync {
    /// The current monotonic time.
    fn now(&self) -> MonoTime;
}

/// A manually driven clock for tests. Clones share the same time, so a test keeps one handle
/// and gives another (boxed) to the event loop running on its own thread.
#[derive(Clone, Default)]
pub struct FakeClock {
    nanos: Arc<AtomicU64>,
}

impl FakeClock {
    /// A fake clock starting at the origin.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the time.
    pub fn set(&self, t: MonoTime) {
        self.nanos.store(t.0.as_nanos() as u64, Ordering::SeqCst);
    }

    /// Moves the time forward by `d`.
    pub fn advance(&self, d: Duration) {
        self.nanos.fetch_add(d.as_nanos() as u64, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> MonoTime {
        MonoTime(Duration::from_nanos(self.nanos.load(Ordering::SeqCst)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_time_arithmetic_saturates() {
        let a = MonoTime::from_millis(100);
        let b = MonoTime::from_millis(30);
        assert_eq!(a - b, Duration::from_millis(70));
        assert_eq!(b - a, Duration::ZERO);
        assert_eq!(b - Duration::from_millis(500), MonoTime::ZERO);
        assert_eq!(b + Duration::from_millis(20), MonoTime::from_millis(50));
    }

    #[test]
    fn fake_clock_is_shared_between_clones() {
        let c = FakeClock::new();
        let d = c.clone();
        c.advance(Duration::from_millis(5));
        d.advance(Duration::from_millis(7));
        assert_eq!(c.now(), MonoTime::from_millis(12));
        c.set(MonoTime::from_millis(1));
        assert_eq!(d.now(), MonoTime::from_millis(1));
    }
}
