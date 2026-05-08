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

pub struct SessionMetrics {
    pub connected_at: Instant,
    pub bytes_sent: Counter,
    pub bytes_recv: Counter,
    pub frame_count: Counter,
    pub drop_count: Counter,
}

impl SessionMetrics {
    pub fn new(id: &str, registry: &Registry) -> Arc<Self> {
        // Register vectors once. ignore errors if already registered
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

pub fn emit_prometheus_metrics(registry: &Registry) -> Vec<u8> {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!("Failed to encode metrics: {e}");
    }
    buffer
}
