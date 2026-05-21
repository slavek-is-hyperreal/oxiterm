pub mod keys;
pub mod reactor;
pub mod negotiator;
pub mod server;
pub use server::OxiServer;

use std::sync::Arc;
use std::path::PathBuf;
use anyhow::Result;
use crate::config::OxiTermConfig;
use crate::session::{SessionRegistry, THTMLDocument};
use crate::ratelimit::RateLimiter;
use tracing::{info, warn};
use std::collections::HashMap;

pub async fn run_server(
    config: OxiTermConfig, 
    registry: Arc<SessionRegistry>,
    rate_limiter: Arc<RateLimiter>,
    initial_document: Option<THTMLDocument>,
    source_path: Option<PathBuf>,
) -> Result<()> {
    let mut ssh_config = russh::server::Config {
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(3)),
        methods: russh::MethodSet::PUBLICKEY | russh::MethodSet::PASSWORD,
        ..Default::default()
    };
    ssh_config.keys.push(keys::load_host_key(&config.server.host_key_path)?);

    let auth_keys = Arc::new(keys::AuthorizedKeys::load(std::path::Path::new("authorized_keys")).unwrap_or_else(|e| {
        warn!("Failed to load authorized_keys: {e:?}. Starting with empty list.");
        keys::AuthorizedKeys { keys: Vec::new() }
    }));

    let ssh_config = Arc::new(ssh_config);
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("SSH server listening on {addr}");

    loop {
        let (stream, peer_addr) = listener.accept().await?;
        
        // Rate limiting (QUAL-004)
        match rate_limiter.check_and_record(peer_addr.ip()) {
            crate::ratelimit::RateResult::Allow => {
                let russh_config_ref = ssh_config.clone();
                let handler = OxiServer {
                    config: config.clone(),
                    registry: registry.clone(),
                    auth_keys: auth_keys.clone(),
                    rate_limiter: rate_limiter.clone(),
                    peer_addr,
                    channels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
                    initial_document: initial_document.clone(),
                    source_path: source_path.clone(),
                };
                let session_registry = registry.clone();
                let session_channels = handler.channels.clone();
                tokio::spawn(async move {
                    if let Err(e) = russh::server::run_stream(russh_config_ref, stream, handler).await {
                        warn!("SSH session error for {peer_addr}: {e:?}");
                    }
                    // QUAL-006: Ensure all sessions are removed even on abrupt disconnect
                    let mut channels = session_channels.lock();
                    for (_, sid) in channels.drain() {
                        info!("Cleanup: Removing session {sid} from registry");
                        session_registry.remove_session(sid);
                    }
                });
            }
            crate::ratelimit::RateResult::Deny => {
                warn!("Rate limit exceeded for {peer_addr}");
                // Just drop the connection
            }
            crate::ratelimit::RateResult::Throttle(delay) => {
                warn!("Throttling {peer_addr} for {delay:?}");
                let russh_config_ref = ssh_config.clone();
                let handler = OxiServer {
                    config: config.clone(),
                    registry: registry.clone(),
                    auth_keys: auth_keys.clone(),
                    rate_limiter: rate_limiter.clone(),
                    peer_addr,
                    channels: Arc::new(parking_lot::Mutex::new(HashMap::new())),
                    initial_document: initial_document.clone(),
                    source_path: source_path.clone(),
                };
                let session_registry = registry.clone();
                let session_channels = handler.channels.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    if let Err(e) = russh::server::run_stream(russh_config_ref, stream, handler).await {
                        warn!("SSH session error for {peer_addr}: {e:?}");
                    }
                    // QUAL-006: Ensure all sessions are removed even on abrupt disconnect
                    let mut channels = session_channels.lock();
                    for (_, sid) in channels.drain() {
                        info!("Cleanup: Removing session {sid} from registry");
                        session_registry.remove_session(sid);
                    }
                });
            }
        }
    }
}
