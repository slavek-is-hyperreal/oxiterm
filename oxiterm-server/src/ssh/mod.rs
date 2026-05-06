pub mod server;
pub mod keys;
pub use server::OxiServer;

use std::sync::Arc;
use anyhow::Result;
use crate::config::OxiTermConfig;
use crate::session::SessionRegistry;
use tracing::{info, warn};

pub async fn run_server(config: OxiTermConfig, registry: Arc<SessionRegistry>) -> Result<()> {
    let mut ssh_config = russh::server::Config {
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(3)),
        ..Default::default()
    };
    ssh_config.keys.push(keys::load_host_key(&config.server.host_key_path)?);

    let ssh_config = Arc::new(ssh_config);
    let addr = format!("{}:{}", config.server.host, config.server.port);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    info!("SSH server listening on {}", addr);

    loop {
        let (stream, _) = listener.accept().await?;
        let config = ssh_config.clone();
        let handler = OxiServer {
            registry: registry.clone(),
            current_session: None,
        };
        tokio::spawn(async move {
            if let Err(e) = russh::server::run_stream(config, stream, handler).await {
                warn!("SSH session error: {:?}", e);
            }
        });
    }
}
