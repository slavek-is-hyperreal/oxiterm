//! Rate limiter implementation for client connection and frame updates.
//!
//! Provides IP-based rate limiting with throttling thresholds, and frame rate controllers
//! to prevent terminal display congestion.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use parking_lot::Mutex;

/// Rate limiter for connection requests, tracking request frequencies.
pub struct RateLimiter {
    windows: Mutex<HashMap<IpAddr, WindowCounter>>,
    limit_per_min: u32,
}

struct WindowCounter {
    count: u32,
    window_start: Instant,
}

/// Result of evaluating a request against the rate limits.
pub enum RateResult {
    /// Request is allowed immediately.
    Allow,
    /// Request is allowed, but client should be throttled by the specified delay.
    Throttle(Duration),
    /// Request is denied as it exceeded limits.
    Deny,
}

impl RateLimiter {
    /// Creates a new IP connection rate limiter with the specified maximum hits per minute.
    pub fn new(limit_per_min: u32) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            limit_per_min,
        }
    }

    /// Evaluates if an request from an IP should be allowed, throttled, or denied.
    pub fn check_and_record(&self, ip: IpAddr) -> RateResult {
        let mut windows = self.windows.lock();
        let now = Instant::now();

        // Evict expired rate limit windows when registry grows large to prevent memory leaks/exhaustion.
        if windows.len() > 1000 {
            windows.retain(|_, v| now.duration_since(v.window_start) < Duration::from_secs(60));
        }

        let counter = windows.entry(ip).or_insert_with(|| WindowCounter {
            count: 0,
            window_start: now,
        });

        if now.duration_since(counter.window_start) > Duration::from_secs(60) {
            counter.count = 1;
            counter.window_start = now;
            RateResult::Allow
        } else if counter.count >= self.limit_per_min {
            RateResult::Deny
        } else {
            counter.count += 1;
            
            // Anchored by spec [S5-38]. Implement dynamic throttling when approaching maximum limit (> 80% capacity).
            let threshold = (self.limit_per_min as f32 * 0.8) as u32;
            if counter.count > threshold {
                let delay = Duration::from_millis(100 * (counter.count - threshold) as u64);
                RateResult::Throttle(delay)
            } else {
                RateResult::Allow
            }
        }
    }
}

/// Rate limiter for rendering loop iterations.
pub struct FrameRateLimiter {
    last_frame: Instant,
    min_interval: Duration,
}

impl FrameRateLimiter {
    /// Creates a frame rate limiter mapping target FPS.
    pub fn new(fps: u32) -> Self {
        let min_interval = Duration::from_secs_f32(1.0 / fps as f32);
        Self {
            last_frame: Instant::now() - min_interval,
            min_interval,
        }
    }

    /// Non-blocking check returning true if enough time has passed since the last frame was rendered.
    pub fn should_render(&self) -> bool {
        self.last_frame.elapsed() >= self.min_interval
    }

    /// Records the execution of a frame paint, updating the timestamp.
    pub fn record_frame(&mut self) {
        self.last_frame = Instant::now();
    }
}
