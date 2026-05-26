//! THTML and TCSS syntax parsing engine.
//!
//! Provides utilities to parse THTML template structures and TCSS stylesheet files.

pub mod thtml;
pub mod tcss;

pub use thtml::{THTMLParser, sanitize_style_raw};
