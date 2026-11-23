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

    /// The first instant at which [`should_render`](Self::should_render) returns true, or `None`
    /// when a frame is allowed right away (nothing rendered yet). `should_render` compares with
    /// `>=`, so this is exactly `last_frame + min_interval` (no extra nanosecond, unlike
    /// `ResizeDebouncer::due_at`).
    pub fn next_allowed(&self) -> Option<MonoTime> {
        self.last_frame.map(|last| last + self.min_interval)
    }

    /// Records the execution of a frame paint, updating the timestamp.
    pub fn record_frame(&mut self, now: MonoTime) {
        self.last_frame = Some(now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_allowed_is_the_first_instant_should_render_is_true() {
        let mut l = FrameRateLimiter::new(60);
        assert_eq!(l.next_allowed(), None);
        assert!(l.should_render(MonoTime::from_millis(0)));
        let t = MonoTime::from_millis(500);
        l.record_frame(t);
        let next = l.next_allowed().unwrap();
        assert!(next > t + Duration::from_millis(16) && next < t + Duration::from_millis(17));
        assert!(!l.should_render(next - Duration::from_nanos(1)));
        assert!(l.should_render(next));
    }
}
