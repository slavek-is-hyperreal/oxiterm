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
    tracing_subscriber::fmt::init();

    let config = OxiTermConfig::from_env().unwrap_or_default();
    info!("Starting OxiTerm server with config: {:?}", config);

    let prometheus_registry = Arc::new(Registry::new());
    let registry = Arc::new(SessionRegistry::new(prometheus_registry.clone()));
    let rate_limiter = Arc::new(RateLimiter::new(60)); // 60 conn/min
    
    // Start metrics server if enabled
    if config.metrics.enabled {
        let registry_clone = prometheus_registry.clone();
        let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.metrics.port));
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

    // Start SSH server
    let ssh_config = config.clone();
    let ssh_registry = registry.clone();
    let ssh_rate_limiter = rate_limiter.clone();
    tokio::spawn(async move {
        if let Err(e) = run_server(ssh_config, ssh_registry, ssh_rate_limiter).await {
            warn!("SSH server error: {:?}", e);
        }
    });

    // Signal handlers
    let mut sigusr1 = signal(SignalKind::user_defined1())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    tokio::select! {
        _ = sigusr1.recv() => {
            info!("Received SIGUSR1, initiating graceful drain...");
            registry.drain_sessions(Duration::from_secs(30)).await;
        }
        _ = sigterm.recv() => {
            info!("Received SIGTERM, shutting down...");
        }
        _ = sigint.recv() => {
            info!("Received SIGINT, shutting down...");
        }
    }

    Ok(())
}
