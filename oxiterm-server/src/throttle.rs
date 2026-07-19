//! Per-session token-bucket throttle for DoR (denial-of-render) guard.
//!
//! Anchored by spec [SEC-DoR-1].
//! NAV bucket covers: NavigateTo, SwitchViewport, htmx .thtml loads.
//! INPUT bucket covers: KeyPress, MouseEvent, TextInput.

use std::time::Instant;

/// NAV bucket capacity (events).
pub const NAV_CAPACITY: u32 = 5;
/// NAV bucket refill rate (events per second).
pub const NAV_REFILL_PER_SEC: f64 = 2.0;
/// INPUT bucket capacity (events).
pub const INPUT_CAPACITY: u32 = 200;
/// INPUT bucket refill rate (events per second).
pub const INPUT_REFILL_PER_SEC: f64 = 100.0;

/// Simple token-bucket rate limiter.
pub struct TokenBucket {
    capacity: u32,
    refill_per_sec: f64,
    tokens: f64,
    last_refill: Instant,
}

impl TokenBucket {
    /// Creates a new token bucket, starting full.
    pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
        Self {
            capacity,
            refill_per_sec,
            tokens: capacity as f64,
            last_refill: Instant::now(),
        }
    }

    /// Returns true if a token is available and consumes it; false if empty.
    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec)
            .min(self.capacity as f64);
        self.last_refill = now;
    }

    /// Force-sets token count (for testing only).
    #[cfg(test)]
    pub fn set_tokens(&mut self, t: f64) {
        self.tokens = t.min(self.capacity as f64);
    }
}

/// Throttle state owned by EventLoop.
pub struct EventThrottle {
    /// NAV event bucket (NavigateTo, SwitchViewport, htmx .thtml loads).
    pub nav: TokenBucket,
    /// INPUT event bucket (KeyPress, MouseEvent, TextInput).
    pub input: TokenBucket,
    /// True while a NAV throttle episode is active.
    /// Used to emit exactly one 0x34 "Zwolnij ;)" per episode.
    pub nav_throttle_active: bool,
}

impl EventThrottle {
    /// Creates a new throttle with full buckets.
    pub fn new() -> Self {
        Self {
            nav: TokenBucket::new(NAV_CAPACITY, NAV_REFILL_PER_SEC),
            input: TokenBucket::new(INPUT_CAPACITY, INPUT_REFILL_PER_SEC),
            nav_throttle_active: false,
        }
    }

    /// Checks the NAV budget.
    ///
    /// Returns `true` if the event is allowed (token consumed).
    /// Returns `false` if throttled.
    pub fn check_nav(&mut self) -> bool {
        if self.nav.try_consume() {
            self.nav_throttle_active = false;
            true
        } else {
            false
        }
    }

    /// Returns true if this is the FIRST event of a new throttle episode.
    /// Must be called immediately after `check_nav()` returned false.
    /// Caller should send 0x34 notice exactly when this returns true.
    pub fn is_first_throttle(&mut self) -> bool {
        if !self.nav_throttle_active {
            self.nav_throttle_active = true;
            true
        } else {
            false
        }
    }

    /// Checks the INPUT budget for render-side effects. Returns true if allowed.
    ///
    /// Covers: hover/Move/Release render trigger, character-input state write + echo.
    /// Never gates `Press` activation or navigation keypresses — those paths are unconditional
    /// (see [Plan-2.2/R1]). Expensive work (nav, render) is capped by the NAV bucket and
    /// frame coalescing respectively.
    pub fn check_input(&mut self) -> bool {
        self.input.try_consume()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_throttle_nav_capacity_and_refill() {
        let mut t = EventThrottle::new();
        // Exhaust the bucket
        for _ in 0..NAV_CAPACITY {
            assert!(t.check_nav(), "should allow up to capacity");
        }
        assert!(!t.check_nav(), "should be throttled after capacity exhausted");
        assert!(t.is_first_throttle(), "first call after throttle is first episode");
        assert!(!t.is_first_throttle(), "second call is NOT first");

        // Force refill
        t.nav.set_tokens(NAV_CAPACITY as f64);
        assert!(t.check_nav(), "after refill, allowed again");
        assert!(!t.nav_throttle_active, "episode cleared on allow");
    }

    #[test]
    fn test_throttle_input_silent_drop() {
        let mut t = EventThrottle::new();
        for _ in 0..INPUT_CAPACITY {
            assert!(t.check_input(), "within capacity");
        }
        for _ in 0..50 {
            assert!(!t.check_input(), "beyond capacity: silently dropped");
        }
    }
}
