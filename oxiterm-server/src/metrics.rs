use std::sync::Arc;
use parking_lot::RwLock;
use std::time::Instant;
use prometheus::{Encoder, TextEncoder, Opts, Counter, Gauge, Registry};
use std::io::Write;

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
            format!("oxiterm_session_{}_bytes_sent", id),
            "Bytes sent in session"
        )).unwrap();
        let bytes_recv = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{}_bytes_recv", id),
            "Bytes received in session"
        )).unwrap();
        let frame_count = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{}_frames", id),
            "Frames rendered in session"
        )).unwrap();
        let drop_count = Counter::with_opts(Opts::new(
            format!("oxiterm_session_{}_drops", id),
            "Frames dropped in session"
        )).unwrap();

        registry.register(Box::new(bytes_sent.clone())).unwrap();
        registry.register(Box::new(bytes_recv.clone())).unwrap();
        registry.register(Box::new(frame_count.clone())).unwrap();
        registry.register(Box::new(drop_count.clone())).unwrap();

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
    encoder.encode(&metric_families, &mut buffer).unwrap();
    buffer
}
