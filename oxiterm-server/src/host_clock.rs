//! The server's real clock. Lives outside the core-bound modules because it reads `Instant`.

use crate::clock::{Clock, MonoTime};

/// Host clock of the server process: `Instant` at creation plus elapsed.
///
/// HOST CLOCK: the only `Instant::now()` behind the loop's logic clock. A wasm host provides its
/// own `Clock` (e.g. `performance.now()`); the core never names this type.
pub struct SystemClock {
    origin: std::time::Instant,
}

impl SystemClock {
    /// Creates a clock whose origin is now.
    pub fn new() -> Self {
        Self { origin: std::time::Instant::now() }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now(&self) -> MonoTime {
        MonoTime::from_duration(self.origin.elapsed())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_clock_is_monotonic() {
        let c = SystemClock::new();
        let a = c.now();
        assert!(c.now() >= a);
    }
}
