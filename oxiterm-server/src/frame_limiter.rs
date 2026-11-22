//! Frame rate limiter for the rendering loop. Pure: time comes in as a `MonoTime`.

use std::time::Duration;
use crate::clock::MonoTime;

/// Rate limiter for rendering loop iterations.
pub struct FrameRateLimiter {
    /// `None` until the first frame: the first render is always allowed.
    last_frame: Option<MonoTime>,
    min_interval: Duration,
}

impl FrameRateLimiter {
    /// Creates a frame rate limiter mapping target FPS.
    pub fn new(fps: u32) -> Self {
        let min_interval = Duration::from_secs_f32(1.0 / fps as f32);
        Self {
            last_frame: None,
            min_interval,
        }
    }

    /// Non-blocking check returning true if enough time has passed since the last frame was rendered.
    pub fn should_render(&self, now: MonoTime) -> bool {
        match self.last_frame {
            Some(last) => now.saturating_duration_since(last) >= self.min_interval,
            None => true,
        }
    }

    /// Records the execution of a frame paint, updating the timestamp.
    pub fn record_frame(&mut self, now: MonoTime) {
        self.last_frame = Some(now);
    }
}
