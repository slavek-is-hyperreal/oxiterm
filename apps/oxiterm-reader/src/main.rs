//! oxiterm-reader binary.

use oxiterm_reader::{app::App, config::Config, fetch::SafeFetcher, server};
use std::path::PathBuf;
use std::sync::Arc;

fn usage() -> ! {
    eprintln!("usage: oxiterm-reader [--config reader.toml] [--site-dir DIR] [--listen ADDR]\n\nEnvironment: OXITERM_APP_TOKEN (required, shared with OxiTerm)");
    std::process::exit(2)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    let mut cfg_path: Option<PathBuf> = None;
    let (mut site, mut listen) = (None, None);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--config" => cfg_path = Some(args.next().unwrap_or_else(|| usage()).into()),
            "--site-dir" => site = Some(PathBuf::from(args.next().unwrap_or_else(|| usage()))),
            "--listen" => listen = Some(args.next().unwrap_or_else(|| usage())),
            _ => usage(),
        }
    }
    let mut cfg = match cfg_path {
        Some(p) => Config::load(&p)?,
        None if std::path::Path::new("reader.toml").exists() => Config::load(std::path::Path::new("reader.toml"))?,
        None => Config::default(),
    };
    if let Some(s) = site {
        cfg.site_dir = s;
    }
    if let Some(l) = listen {
        cfg.listen = l;
    }
    let token = std::env::var("OXITERM_APP_TOKEN").unwrap_or_default();
    let listen = cfg.listen.clone();
    let site_dir = cfg.site_dir.clone();
    let fetcher = SafeFetcher::new(&cfg);
    let app = Arc::new(App::new(cfg, fetcher)?);
    tracing::info!(
        "site dir: {}\nstart OxiTerm with:\n  OXITERM_APP_SERVER=http://{listen}/events OXITERM_APP_TOKEN=... oxiterm-cli serve {}/index.thtml --web-port 8080",
        site_dir.display(),
        site_dir.display()
    );
    server::run(app, token, &listen).await
}
