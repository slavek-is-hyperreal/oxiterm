//! Reader configuration (`reader.toml`), every field optional.

use serde::Deserialize;
use std::path::PathBuf;

/// One configured RSS/Atom feed.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct FeedConfig {
    /// Label shown on the start page.
    pub name: String,
    /// Feed URL (http/https).
    pub url: String,
}

/// Reader settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Directory the reader owns: `index.thtml`, `cache/`.
    pub site_dir: PathBuf,
    /// Address of the `/events` endpoint.
    pub listen: String,
    /// Allow the user to type any URL. Set `false` on public instances: only
    /// Hacker News, Wikipedia, the configured feeds and `allowed_hosts` are fetched.
    pub allow_any_url: bool,
    /// Extra hosts (suffix match) allowed when `allow_any_url = false`.
    pub allowed_hosts: Vec<String>,
    /// User actions per session per minute.
    pub actions_per_minute: usize,
    /// Global cap on simultaneous outbound requests.
    pub max_concurrent_fetches: usize,
    /// Outbound requests one user action may make (page, items, images).
    pub max_fetches_per_action: usize,
    /// HTML/JSON/XML body cap in bytes.
    pub max_html_bytes: usize,
    /// Per-image body cap in bytes.
    pub max_image_bytes: usize,
    /// Images downloaded per article.
    pub max_images_per_article: usize,
    /// Redirect hops followed (each re-validated).
    pub max_redirects: usize,
    /// TCP connect timeout, seconds.
    pub connect_timeout_secs: u64,
    /// Whole-request timeout, seconds.
    pub total_timeout_secs: u64,
    /// Reuse a generated page for the same source for this many seconds.
    pub cache_ttl_secs: u64,
    /// Delete cached files older than this many hours at start-up.
    pub cache_max_age_hours: u64,
    /// Hacker News stories listed.
    pub hn_stories: usize,
    /// Content column width in cells.
    pub content_cols: u16,
    /// Configured feeds.
    pub feeds: Vec<FeedConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            site_dir: PathBuf::from("reader-site"),
            listen: "127.0.0.1:3100".to_string(),
            allow_any_url: true,
            allowed_hosts: Vec::new(),
            actions_per_minute: 20,
            max_concurrent_fetches: 8,
            max_fetches_per_action: 60,
            max_html_bytes: 5 * 1024 * 1024,
            max_image_bytes: 5 * 1024 * 1024,
            max_images_per_article: 20,
            max_redirects: 5,
            connect_timeout_secs: 5,
            total_timeout_secs: 15,
            cache_ttl_secs: 600,
            cache_max_age_hours: 24,
            hn_stories: 25,
            content_cols: 74,
            feeds: vec![
                FeedConfig { name: "Rust Blog".into(), url: "https://blog.rust-lang.org/feed.xml".into() },
                FeedConfig { name: "LWN".into(), url: "https://lwn.net/headlines/rss".into() },
            ],
        }
    }
}

impl Config {
    /// Loads a TOML file. Unknown keys are ignored; invalid values are an error.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        let mut cfg: Config = toml::from_str(&text)?;
        cfg.sanitize();
        Ok(cfg)
    }

    /// Clamps values to sane ranges so a typo cannot disable a safety limit.
    pub fn sanitize(&mut self) {
        self.actions_per_minute = self.actions_per_minute.clamp(1, 600);
        self.max_concurrent_fetches = self.max_concurrent_fetches.clamp(1, 64);
        self.max_fetches_per_action = self.max_fetches_per_action.clamp(1, 200);
        self.max_html_bytes = self.max_html_bytes.clamp(1024, 32 * 1024 * 1024);
        self.max_image_bytes = self.max_image_bytes.clamp(1024, 32 * 1024 * 1024);
        self.max_images_per_article = self.max_images_per_article.min(60);
        self.max_redirects = self.max_redirects.min(10);
        self.connect_timeout_secs = self.connect_timeout_secs.clamp(1, 60);
        self.total_timeout_secs = self.total_timeout_secs.clamp(2, 120);
        self.hn_stories = self.hn_stories.clamp(1, 50);
        self.content_cols = self.content_cols.clamp(30, 76);
        self.feeds.truncate(6);
    }

    /// Hosts always allowed when `allow_any_url = false` (built-in sources, feeds, extras).
    pub fn effective_allowed_hosts(&self) -> Option<Vec<String>> {
        if self.allow_any_url {
            return None;
        }
        let mut hosts: Vec<String> = vec!["wikipedia.org".into(), "wikimedia.org".into(), "hacker-news.firebaseio.com".into()];
        for f in &self.feeds {
            if let Ok(u) = url::Url::parse(&f.url) {
                if let Some(h) = u.host_str() {
                    hosts.push(h.to_string());
                }
            }
        }
        hosts.extend(self.allowed_hosts.iter().cloned());
        Some(hosts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_partial_toml() {
        let mut c: Config = toml::from_str("allow_any_url = false\n[[feeds]]\nname='x'\nurl='https://x.org/f'\n").unwrap();
        c.sanitize();
        assert!(!c.allow_any_url);
        assert_eq!(c.feeds.len(), 1);
        assert_eq!(c.max_images_per_article, 20);
        let hosts = c.effective_allowed_hosts().unwrap();
        assert!(hosts.contains(&"x.org".to_string()) && hosts.contains(&"wikipedia.org".to_string()));
        assert!(Config::default().effective_allowed_hosts().is_none());
    }

    #[test]
    fn sanitize_clamps() {
        let mut c = Config { actions_per_minute: 0, max_html_bytes: usize::MAX, content_cols: 500, ..Config::default() };
        c.sanitize();
        assert_eq!(c.actions_per_minute, 1);
        assert_eq!(c.max_html_bytes, 32 * 1024 * 1024);
        assert_eq!(c.content_cols, 76);
    }
}
