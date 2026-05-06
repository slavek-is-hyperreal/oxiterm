use std::sync::Arc;
use std::time::Instant;
use prometheus::{Encoder, TextEncoder, Opts, Counter, Registry};

pub struct SessionMetrics {
    pub connected_at: Instant,
    pub bytes_sent: Counter,
    pub bytes_recv: Counter,
    pub frame_count: Counter,
    pub drop_count: Counter,
}

impl SessionMetrics {
    pub fn new(id: &str, registry: &Registry) -> Arc<Self> {
        let bytes_sent = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{id}_bytes_sent"),
            "Bytes sent in session"
        )).expect("Failed to create metrics counter");
        
        let bytes_recv = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{id}_bytes_recv"),
            "Bytes received in session"
        )).expect("Failed to create metrics counter");
        
        let frame_count = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{id}_frames"),
            "Frames rendered in session"
        )).expect("Failed to create metrics counter");
        
        let drop_count = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{id}_drops"),
            "Frames dropped in session"
        )).expect("Failed to create metrics counter");

        registry.register(Box::new(bytes_sent.clone())).expect("Failed to register metric");
        registry.register(Box::new(bytes_recv.clone())).expect("Failed to register metric");
        registry.register(Box::new(frame_count.clone())).expect("Failed to register metric");
        registry.register(Box::new(drop_count.clone())).expect("Failed to register metric");

        Arc::new(Self {
            connected_at: Instant::now(),
            bytes_sent,
            bytes_recv,
            frame_count,
            drop_count,
        })
    }
}

pub fn emit_prometheus_metrics(registry: &Registry) -> Vec<u8> {
    let mut buffer = Vec::new();
    let encoder = TextEncoder::new();
    let metric_families = registry.gather();
    // In memory encode usually doesn't fail unless OOM
    if let Err(e) = encoder.encode(&metric_families, &mut buffer) {
        tracing::error!("Failed to encode metrics: {e}");
    }
    buffer
}
