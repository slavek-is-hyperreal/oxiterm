use clap::{Parser, Subcommand};
use anyhow::Result;
use tracing::info;
use std::path::PathBuf;
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::time::Duration;
use oxiterm_proto::input::InputEvent;

#[derive(Parser)]
#[command(name = "oxiterm")]
#[command(about = "OxiTerm: Build TUI apps like web pages. Serve over SSH.", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

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
        #[arg(short, long, default_value = "0.0.0.0")]
        host: String,
        /// Disable authentication (for development)
        #[arg(long)]
        no_auth: bool,
    },
    /// Run the built-in weather dashboard demo
    Demo {
        /// Port to listen on
        #[arg(short, long, default_value_t = 2222)]
        port: u16,
    },
    /// Validate a .thtml file for syntax errors
    Check {
        /// Path to the .thtml file
        file: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        Commands::Serve { file, port, host, no_auth } => {
            info!("Serving {} on {}:{}", file, host, port);
            
            let doc = oxiterm_server::loader::load_thtml_file(&file)?;
            
            let mut config = oxiterm_server::OxiTermConfig::default();
            config.server.host = host;
            config.server.port = port;
            if no_auth {
                config.server.no_auth = true;
            }

            let prometheus_registry = std::sync::Arc::new(prometheus::Registry::new());
            let registry = std::sync::Arc::new(oxiterm_server::session::SessionRegistry::new(prometheus_registry.clone()));
            let rate_limiter = std::sync::Arc::new(oxiterm_server::ratelimit::RateLimiter::new(60));

            // SC-01: Setup Hot Reload watcher
            let registry_clone = registry.clone();
            let file_path = PathBuf::from(&file);
            let (tx, rx) = std::sync::mpsc::channel();
            let mut debouncer = new_debouncer(Duration::from_millis(100), tx)?;
            debouncer.watcher().watch(&file_path, RecursiveMode::NonRecursive)?;

            info!("Hot Reload active for {}", file);
            
            let file_path_clone = file_path.clone();
            tokio::spawn(async move {
                while let Ok(events) = rx.recv() {
                    if let Ok(_evs) = events {
                        info!("File change detected, broadcasting Reload signal...");
                        registry_clone.broadcast_input_event(InputEvent::Reload);
                    }
                }
                // Keep debouncer alive
                let _ = debouncer;
            });

            oxiterm_server::ssh::run_server(config, registry, rate_limiter, Some(doc), Some(file_path_clone)).await?;
        }
        Commands::Demo { port } => {
            info!("Starting built-in weather demo on port {}", port);
            let mut config = oxiterm_server::OxiTermConfig::default();
            config.server.port = port;
            
            let prometheus_registry = std::sync::Arc::new(prometheus::Registry::new());
            let registry = std::sync::Arc::new(oxiterm_server::session::SessionRegistry::new(prometheus_registry.clone()));
            let rate_limiter = std::sync::Arc::new(oxiterm_server::ratelimit::RateLimiter::new(60));
            
            oxiterm_server::ssh::run_server(config, registry, rate_limiter, None, None).await?;
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
