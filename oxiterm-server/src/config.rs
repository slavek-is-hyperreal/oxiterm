use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OxiTermConfig {
    pub server: ServerConfig,
    pub session: SessionConfig,
    pub metrics: MetricsConfig,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub host_key_path: PathBuf,
    pub password: Option<String>,
    pub no_auth: bool,
    pub web_port: u16,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionConfig {
    pub max_sessions: usize,
    pub fps_limit: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub port: u16,
}

impl Default for OxiTermConfig {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                host: "0.0.0.0".to_string(),
                port: 2222,
                host_key_path: PathBuf::from("host_key"),
                password: None,
                no_auth: false,
                web_port: 8080,
            },
            session: SessionConfig {
                max_sessions: 100,
                fps_limit: 60,
            },
            metrics: MetricsConfig {
                enabled: true,
                port: 9090,
            },
        }
    }
}

impl OxiTermConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn from_env() -> Result<Self> {
        // Simple env override implementation
        let mut config = Self::default();
        if let Ok(host) = std::env::var("OXITERM_HOST") {
            config.server.host = host;
        }
        if let Ok(port) = std::env::var("OXITERM_PORT") {
            config.server.port = port.parse()?;
        }
        if let Ok(password) = std::env::var("OXITERM_PASSWORD") {
            config.server.password = Some(password);
        }
        if let Ok(no_auth) = std::env::var("OXITERM_NO_AUTH") {
            config.server.no_auth = no_auth == "true" || no_auth == "1";
        }
        if let Ok(web_port) = std::env::var("OXITERM_WEB_PORT") {
            config.server.web_port = web_port.parse()?;
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            anyhow::bail!("Server port cannot be 0");
        }
        if self.session.fps_limit == 0 {
            anyhow::bail!("FPS limit cannot be 0");
        }
        Ok(())
    }
}
