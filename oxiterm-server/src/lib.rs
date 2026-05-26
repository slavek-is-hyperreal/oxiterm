//! OxiTerm SSH and Web Application Server.
//!
//! Provides the terminal user interface server, including SSH session handlers,
//! HTTP/WS web interfaces, rate-limiting, and template loaders.

#![allow(clippy::all, clippy::pedantic)]

pub mod config;
pub mod dispatcher;
pub mod metrics;
pub mod ratelimit;
pub mod session;
pub mod events;
pub mod ssh;
pub mod backpressure;
pub mod loader;
pub mod placeholder;
pub mod url_validator;
pub mod state;
pub mod web;

pub use config::OxiTermConfig;
pub use metrics::SessionMetrics;
pub use ratelimit::RateLimiter;
