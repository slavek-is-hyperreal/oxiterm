//! OxiTerm SSH and Web Application Server.
//!
//! Provides the terminal user interface server, including SSH session handlers,
//! HTTP/WS web interfaces, rate-limiting, and template loaders.

#![allow(clippy::all, clippy::pedantic)]

pub mod config;
pub use oxiterm_core::action;
pub mod dispatcher;
pub mod metrics;
pub mod ratelimit;
pub use oxiterm_core::frame_limiter;
pub use oxiterm_core::throttle;
pub mod session;
pub use oxiterm_core::events;
pub mod ssh;
pub mod backpressure;
pub mod loader;
pub mod placeholder;
pub mod url_validator;
pub use oxiterm_core::state;
pub mod web;
pub mod local_mode;
pub mod pathsafe;
pub use oxiterm_core::page_source;
pub mod fs_page_source;
pub use oxiterm_core::identity;
pub use oxiterm_core::expand;
pub use oxiterm_core::animation;
pub use oxiterm_core::clock;
pub mod host_clock;
pub use oxiterm_core::session_state;
pub use oxiterm_core::loop_io;
pub mod host_queue;


pub use config::OxiTermConfig;
pub use metrics::SessionMetrics;
pub use ratelimit::RateLimiter;

#[cfg(test)]
pub mod test_env;

#[cfg(test)]
mod event_loop_trace;
