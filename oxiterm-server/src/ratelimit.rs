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
            RateResult::Allow
        }
    }
}
