use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use anyhow::{Context, Result};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OxiTermConfig {
    pub server: ServerConfig,
    pub session: SessionConfig,
    pub metrics: MetricsConfig,
    pub app_server_url: Option<String>,
    pub media_base_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
    pub host_key_path: PathBuf,
    pub password: Option<String>,
    pub no_auth: bool,
    pub web_port: u16,
    /// When true, EventLoop uses LinearFrameSink (plain-text a11y output) instead
    /// of the default AnsiFrameSink. Activate via `oxiterm serve --a11y`.
    #[serde(default)]
    pub a11y_mode: bool,
    pub app_server_url: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SessionConfig {
    pub max_sessions: usize,
    pub fps_limit: u32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MetricsConfig {
    pub enabled: bool,
    pub host: String,
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

