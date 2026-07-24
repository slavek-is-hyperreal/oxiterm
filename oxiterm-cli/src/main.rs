//! CLI entrypoint for OxiTerm.
//!
//! Provides the Serve, Demo, and Check commands for starting the TUI server and validating templates.

#![allow(clippy::all, clippy::pedantic)]

use clap::{Parser, Subcommand};
use anyhow::Result;
use tracing::{info, warn};
use std::path::PathBuf;
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::time::Duration;
use oxiterm_proto::input::InputEvent;

/// CLI command line parser configuration.
#[derive(Parser)]
#[command(name = "oxiterm")]
#[command(about = "OxiTerm: Build TUI apps like web pages. Serve over SSH.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// Commands supported by the CLI.
#[derive(Subcommand)]
enum Commands {
    /// Start an SSH server to serve a THTML application
    Serve {
        /// Path to the .thtml file
        file: String,
        /// Port to listen on
        #[arg(short, long, default_value_t = 2222)]
        port: u16,
        /// Listen address
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// Disable authentication (for development)
        #[arg(long)]
        no_auth: bool,
        /// Web port to listen on
        #[arg(long, default_value_t = 8080)]
        web_port: u16,
        /// Enable accessible rendering via LinearFrameSink (linear text tree output).
        /// AT-SPI2/D-Bus transport is not yet implemented — no external connections are made.
        #[arg(long)]
        a11y: bool,
    },
    /// Run the built-in weather dashboard demo
    Demo {
        /// Port to listen on
        #[arg(short, long, default_value_t = 2222)]
        port: u16,
        /// Web port to listen on
        #[arg(long, default_value_t = 8080)]
        web_port: u16,
        /// Enable accessibility mode
        #[arg(long)]
        a11y: bool,
    },
    /// Validate a .thtml file for syntax errors
    Check {
        /// Path to the .thtml file
        file: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { file, port, host, no_auth, web_port, a11y } => {
            info!("Serving {} on {}:{} (web on port {}){}", file, host, port, web_port, if a11y { " [a11y mode]" } else { "" });
            
            let doc = oxiterm_server::loader::load_thtml_file(&file)?;
            
            let mut config = oxiterm_server::OxiTermConfig::default();
            config.server.host = host;
            config.server.port = port;
            config.server.web_port = web_port;
            config.server.a11y_mode = a11y;
            if no_auth {
                config.server.no_auth = true;
            }
            if config.server.no_auth {
                warn!("⚠️ SECURITY WARNING: SSH authentication is disabled! Anyone can connect without a password.");
            }

            let prometheus_registry = std::sync::Arc::new(prometheus::Registry::new());
            let registry = std::sync::Arc::new(oxiterm_server::session::SessionRegistry::new(prometheus_registry.clone(), config.session.max_sessions));
            let rate_limiter = std::sync::Arc::new(oxiterm_server::ratelimit::RateLimiter::new(60));

            // Anchored by spec [SC-01]. Setup Hot Reload watcher.
            let registry_clone = registry.clone();
            let file_path = PathBuf::from(&file);
            let file_path_clone = file_path.clone();
            let (tx, rx) = std::sync::mpsc::channel();
            match new_debouncer(Duration::from_millis(100), tx) {
                Ok(mut debouncer) => {
                    if let Err(e) = debouncer.watcher().watch(&file_path, RecursiveMode::NonRecursive) {
                        warn!("Failed to watch file for hot reload (OS limit reached?): {}", e);
                    } else {
                        info!("Hot Reload active for {}", file);
                        tokio::spawn(async move {
                            while let Ok(events) = rx.recv() {
                                if let Ok(_evs) = events {
                                    info!("File change detected, broadcasting Reload signal...");
                                    registry_clone.broadcast_input_event(InputEvent::Reload);
                                }
                            }
                            let _ = debouncer;
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to create hot reload debouncer: {}", e);
                }
            }

            // Start Web/WebSocket server
            let web_host = config.server.host.clone();
            let web_registry = registry.clone();
            oxiterm_server::web::web_impl::start_web_server(web_host, web_port, web_registry, rate_limiter.clone(), Some(doc.clone()), Some(file_path_clone.clone()));

            oxiterm_server::ssh::run_server(config, registry, rate_limiter, Some(doc), Some(file_path_clone)).await?;
        }
        Commands::Demo { port, web_port, a11y } => {
            info!("Starting interactive demo on port {} (web on port {}){}", port, web_port, if a11y { " [a11y mode]" } else { "" });
            let mut config = oxiterm_server::OxiTermConfig::default();
            config.server.port = port;
            config.server.web_port = web_port;
            config.server.a11y_mode = a11y;
            
            let prometheus_registry = std::sync::Arc::new(prometheus::Registry::new());
            let registry = std::sync::Arc::new(oxiterm_server::session::SessionRegistry::new(prometheus_registry.clone(), config.session.max_sessions));
            let rate_limiter = std::sync::Arc::new(oxiterm_server::ratelimit::RateLimiter::new(60));
            
            let doc_path = PathBuf::from("examples/hello.thtml");
            let (doc, final_path) = if doc_path.exists() {
                match oxiterm_server::loader::load_thtml_file(&doc_path) {
                    Ok(d) => (Some(d), Some(doc_path)),
                    Err(e) => {
                        warn!("Failed to load examples/hello.thtml: {}. Using embedded fallback.", e);
                        let embedded = include_str!("../../examples/hello.thtml");
                        (oxiterm_server::loader::load_thtml_str(embedded).ok(), None)
                    }
                }
            } else {
                let embedded = include_str!("../../examples/hello.thtml");
                (oxiterm_server::loader::load_thtml_str(embedded).ok(), None)
            };

            // Start Web/WebSocket server
            let web_host = config.server.host.clone();
            let web_registry = registry.clone();
            oxiterm_server::web::web_impl::start_web_server(web_host, web_port, web_registry, rate_limiter.clone(), doc.clone(), final_path.clone());

            oxiterm_server::ssh::run_server(config, registry, rate_limiter, doc, final_path).await?;
        }
        Commands::Check { file } => {
            info!("Checking file: {}", file);
            match oxiterm_server::loader::load_thtml_file(&file) {
                Ok(_) => info!("File {:?} is valid THTML", file),
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }

    Ok(())
}
