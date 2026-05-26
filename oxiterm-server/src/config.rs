//! OxiTerm configuration and validation schemas.
//!
//! Handles parsing of configuration structures from TOML configuration files or
//! environment variable overrides, and validates inputs to prevent SSRF and security risks.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

/// Overall OxiTerm server and session configuration.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OxiTermConfig {
    /// Networking and authentication config for the SSH service.
    pub server: ServerConfig,
    /// Limits and frame rate config for interactive sessions.
    pub session: SessionConfig,
    /// Settings for the metrics collection endpoint.
    pub metrics: MetricsConfig,
    /// Target application backend server URL.
    pub app_server_url: Option<String>,
    /// Base URL path prefix for resolving media assets.
    pub media_base_url: Option<String>,
}

/// Server network binding and authentication configuration.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    /// Bind address (IP or hostname).
    pub host: String,
    /// Bind port of the SSH terminal service.
    pub port: u16,
    /// Path to the SSH host private key file.
    pub host_key_path: PathBuf,
    /// Optional SSH password fallback.
    pub password: Option<String>,
    /// Bypass SSH authentication checking entirely (WARNING: insecure!).
    pub no_auth: bool,
    /// Bind port of the auxiliary HTTP/WebSocket web service.
    pub web_port: u16,
    /// Enable plain-text accessibility visual streams instead of default ANSI escape sequences.
    #[serde(default)]
    pub a11y_mode: bool,
    /// Target application backend server URL override.
    pub app_server_url: Option<String>,
}

/// Dynamic session configuration limits.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionConfig {
    /// Maximum concurrent active client connection sessions.
    pub max_sessions: usize,
    /// Maximum frame update rendering rate.
    pub fps_limit: u32,
}

/// System metrics collector endpoint configuration.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MetricsConfig {
    /// Enable metrics collection and exposes endpoints.
    pub enabled: bool,
    /// Bind address of the metrics reporting service.
    pub host: String,
    /// Bind port of the metrics service.
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
                a11y_mode: false,
                app_server_url: None,
            },
            session: SessionConfig {
                max_sessions: 100,
                fps_limit: 60,
            },
            metrics: MetricsConfig {
                enabled: true,
                host: "0.0.0.0".to_string(),
                port: 9090,
            },
            app_server_url: None,
            media_base_url: None,
        }
    }
}

impl OxiTermConfig {
    /// Loads and parses configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        config.validate()?;
        Ok(config)
    }

    /// Loads configuration overrides from process environment variables.
    pub fn from_env() -> Result<Self> {
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
        if let Ok(url) = std::env::var("OXITERM_APP_SERVER") {
            config.server.app_server_url = Some(url.clone());
            config.app_server_url = Some(url);
        }
        if let Ok(media_url) = std::env::var("OXITERM_MEDIA_BASE_URL") {
            config.media_base_url = Some(media_url);
        }
        if let Ok(max_sess) = std::env::var("OXITERM_MAX_SESSIONS") {
            config.session.max_sessions = max_sess.parse()?;
        }
        if let Ok(metrics_host) = std::env::var("OXITERM_METRICS_HOST") {
            config.metrics.host = metrics_host;
        }
        config.validate()?;
        Ok(config)
    }

    /// Performs validation checks on ports, constraints, and backend URLs.
    pub fn validate(&self) -> Result<()> {
        if self.server.port == 0 {
            anyhow::bail!("Server port cannot be 0");
        }
        if self.session.fps_limit == 0 {
            anyhow::bail!("FPS limit cannot be 0");
        }
        if let Some(ref pw) = self.server.password {
            if pw.is_empty() {
                anyhow::bail!("Server password cannot be empty string");
            }
        }
        if let Some(ref url) = self.app_server_url {
            crate::url_validator::validate_app_server_url(url)?;
        }
        if let Some(ref url) = self.server.app_server_url {
            crate::url_validator::validate_app_server_url(url)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_base_url_from_env() {
        std::env::set_var("OXITERM_MEDIA_BASE_URL", "/tmp/media_test");
        let config = OxiTermConfig::from_env().unwrap();
        assert_eq!(config.media_base_url, Some("/tmp/media_test".to_string()));
        std::env::remove_var("OXITERM_MEDIA_BASE_URL");
    }

    #[test]
    fn test_sec_ssrf_metadata_blocked() {
        use crate::url_validator::validate_app_server_url;
        assert!(validate_app_server_url("http://169.254.169.254/").is_err());
        assert!(validate_app_server_url("http://169.254.10.20/").is_err());
        assert!(validate_app_server_url("http://metadata.google.internal/").is_err());
        assert!(validate_app_server_url("http://127.0.0.1/").is_ok());
        assert!(validate_app_server_url("http://172.20.0.5:3000/").is_ok());
        assert!(validate_app_server_url("https://example.com/").is_ok());
        assert!(validate_app_server_url("file:///etc/passwd").is_err());
    }
}
