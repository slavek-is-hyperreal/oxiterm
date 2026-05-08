use std::collections::HashMap;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use parking_lot::Mutex;

pub struct RateLimiter {
    windows: Mutex<HashMap<IpAddr, WindowCounter>>,
    limit_per_min: u32,
}

struct WindowCounter {
    count: u32,
    window_start: Instant,
}

pub enum RateResult {
    Allow,
    Throttle(Duration),
    Deny,
}

impl RateLimiter {
    pub fn new(limit_per_min: u32) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            limit_per_min,
        }
    }

    pub fn check_and_record(&self, ip: IpAddr) -> RateResult {
        let mut windows = self.windows.lock();
        let now = Instant::now();

        // BUG-H06 Fix: Occasional cleanup to prevent memory leak
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
            
            // S5-38: Implement Throttle for sessions approaching the limit (above 80%)
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
pub struct FrameRateLimiter {
    last_frame: Instant,
    min_interval: Duration,
}

impl FrameRateLimiter {
    pub fn new(fps: u32) -> Self {
        Self {
            last_frame: Instant::now(),
            min_interval: Duration::from_secs_f32(1.0 / fps as f32),
        }
    }

    /// Non-blocking check — caller skips render if false, no sleep ever.
    pub fn should_render(&self) -> bool {
        self.last_frame.elapsed() >= self.min_interval
    }

    /// Call after a frame is actually sent to update the timestamp.
    pub fn record_frame(&mut self) {
        self.last_frame = Instant::now();
    }
}
