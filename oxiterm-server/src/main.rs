//! OxiTerm Server daemon binary entrypoint.
//!
//! Initializes logging subscriptions, loads configurations, binds auxiliary prometheus
//! metrics and HTTP/WS web servers, and runs the main SSH connection coordinator under OS signal handlers.

#![allow(clippy::all, clippy::pedantic, clippy::doc_markdown)]

use oxiterm_server::OxiTermConfig;
use oxiterm_server::metrics::emit_prometheus_metrics;
use oxiterm_server::session::SessionRegistry;
use oxiterm_server::ratelimit::RateLimiter;
use oxiterm_server::ssh::run_server;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use prometheus::Registry;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stdout)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init()
        .ok();

    let config = OxiTermConfig::from_env().unwrap_or_default();
    info!("Starting OxiTerm server with config: {:?}", config);
    if config.server.no_auth {
        warn!("⚠️ SECURITY WARNING: SSH authentication is disabled! Anyone can connect without a password.");
    }

    let prometheus_registry = Arc::new(Registry::new());
    let registry = Arc::new(SessionRegistry::new(prometheus_registry.clone(), config.session.max_sessions));
    let rate_limiter = Arc::new(RateLimiter::new(60)); // 60 connection attempts per minute
    
    // Start metrics server if enabled
    if config.metrics.enabled {
        let registry_clone = prometheus_registry.clone();
        let metrics_host = config.metrics.host.clone();
        let addr_str = format!("{}:{}", metrics_host, config.metrics.port);
        let addr = match addr_str.parse::<std::net::SocketAddr>() {
            Ok(a) => a,
            Err(e) => {
                warn!("Invalid metrics address {}: {}. Falling back to 127.0.0.1", addr_str, e);
                std::net::SocketAddr::from(([127, 0, 0, 1], config.metrics.port))
            }
        };
        if metrics_host == "0.0.0.0" {
            warn!("╔══════════════════════════════════════════════════╗");
            warn!("║  ⚠️  METRICS SERVER BOUND TO 0.0.0.0              ║");
            warn!("║  This exposes metrics to the public network!     ║");
            warn!("╚══════════════════════════════════════════════════╝");
        }
        tokio::spawn(async move {
            let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
            info!("Metrics server listening on http://{}", addr);
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let io = TokioIo::new(stream);
                let registry = registry_clone.clone();
                tokio::task::spawn(async move {
                    if let Err(err) = http1::Builder::new()
                        .serve_connection(io, service_fn(move |_req: Request<hyper::body::Incoming>| {
                            let registry = registry.clone();
                            async move {
                                let body = emit_prometheus_metrics(&registry);
                                Ok::<_, Infallible>(Response::new(Full::new(Bytes::from(body))))
                            }
                        }))
                        .await
                    {
                        warn!("Error serving metrics connection: {:?}", err);
                    }
                });
            }
        });
    }

    // Start Web/WebSocket server
    let web_host = config.server.host.clone();
    let web_port = config.server.web_port;
    let web_registry = registry.clone();
    oxiterm_server::web::web_impl::start_web_server(web_host, web_port, web_registry, rate_limiter.clone(), None, None);

    // Start SSH server
    let ssh_config = config.clone();
    let ssh_registry = registry.clone();
    let ssh_rate_limiter = rate_limiter.clone();
    tokio::spawn(async move {
        if let Err(e) = run_server(ssh_config, ssh_registry, ssh_rate_limiter, None, None).await {
            warn!("SSH server error: {:?}", e);
        }
    });

    // Signal handlers
    let mut sigusr1 = signal(SignalKind::user_defined1())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    tokio::select! {
        res = sigusr1.recv() => {
            if res.is_some() {
                info!("Received SIGUSR1, initiating graceful drain...");
                registry.drain_sessions(Duration::from_secs(30)).await;
            }
        }
        res = sigterm.recv() => {
            if res.is_some() {
                info!("Received SIGTERM, shutting down...");
            }
        }
        res = sigint.recv() => {
            if res.is_some() {
                info!("Received SIGINT, shutting down...");
            }
        }
    }

    Ok(())
}
