//! Prometheus instrumentation and sessions metrics.
//!
//! Registers session activity trackers (network usage, frame rates, packet drops)
//! and encodes metric families to standard Prometheus scrape formats.

use std::sync::Arc;
use std::time::Instant;
use prometheus::{Encoder, TextEncoder, Opts, Counter, CounterVec, Registry};
use std::sync::LazyLock;

static BYTES_SENT: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(Opts::new("oxiterm_bytes_sent", "Bytes sent in session"), &["session"]).unwrap()
});
static BYTES_RECV: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(Opts::new("oxiterm_bytes_recv", "Bytes received in session"), &["session"]).unwrap()
});
static FRAMES: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(Opts::new("oxiterm_frames", "Frames rendered in session"), &["session"]).unwrap()
});
static DROPS: LazyLock<CounterVec> = LazyLock::new(|| {
    CounterVec::new(Opts::new("oxiterm_drops", "Frames dropped in session"), &["session"]).unwrap()
});

/// Metrics trackers associated with an active connection session.
pub struct SessionMetrics {
    /// Timestamp when the connection was established.
    pub connected_at: Instant,
    /// Counter tracking outgoing payload bytes written.
    pub bytes_sent: Counter,
    /// Counter tracking incoming input bytes read.
    pub bytes_recv: Counter,
    /// Counter tracking the count of frames painted to screen.
    pub frame_count: Counter,
    /// Counter tracking frames dropped due to network congestion or latency.
    pub drop_count: Counter,
}

impl SessionMetrics {
    /// Creates and registers a new session tracker within the provided Prometheus registry.
    pub fn new(id: &str, registry: &Registry) -> Arc<Self> {
        let _ = registry.register(Box::new(BYTES_SENT.clone()));
        let _ = registry.register(Box::new(BYTES_RECV.clone()));
        let _ = registry.register(Box::new(FRAMES.clone()));
        let _ = registry.register(Box::new(DROPS.clone()));

        Arc::new(Self {
            connected_at: Instant::now(),
            bytes_sent: BYTES_SENT.with_label_values(&[id]),
            bytes_recv: BYTES_RECV.with_label_values(&[id]),
            frame_count: FRAMES.with_label_values(&[id]),
            drop_count: DROPS.with_label_values(&[id]),
        })
    }
}

/// Gathers registry metrics and formats them to Prometheus text payload bytes.
pub fn emit_prometheus_metrics(registry: &Registry) -> Vec<u8> {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!("Failed to encode metrics: {e}");
    }
    buffer
}
