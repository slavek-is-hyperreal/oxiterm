//! OxiTerm core: the pure, host-independent session logic (no transport, threads, real clock or
//! filesystem). Hosts (SSH/web server, desktop, Android, wasm) supply time and I/O.

#![allow(clippy::all, clippy::pedantic)]

pub mod action;
pub mod animation;
pub mod clock;
pub mod event_loop;
pub mod events;
pub mod expand;
pub mod frame_limiter;
pub mod identity;
pub mod loader;
pub mod loop_io;
pub mod page_source;
pub mod session_state;
pub mod state;
pub mod throttle;
