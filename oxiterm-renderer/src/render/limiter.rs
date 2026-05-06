use std::time::{Duration, Instant};

/// S5-08: `FrameRateLimiter`
/// Manages frame delivery to stay within target FPS and handles frame dropping under load.
pub struct FrameRateLimiter {
    #[allow(dead_code)]
    target_fps: u32,
    last_frame: Instant,
    frame_interval: Duration,
}

impl FrameRateLimiter {
    pub fn new(target_fps: u32) -> Self {
        Self {
            target_fps,
            last_frame: Instant::now(),
            frame_interval: Duration::from_secs_f32(1.0 / target_fps as f32),
        }
    }

    pub fn should_render(&self) -> bool {
        self.last_frame.elapsed() >= self.frame_interval
    }

    pub fn record_frame(&mut self) {
        self.last_frame = Instant::now();
    }

    pub fn frame_drop(&self) {
        // Log or update metrics for dropped frames
    }
}
