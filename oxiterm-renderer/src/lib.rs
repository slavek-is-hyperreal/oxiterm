//! Rendering and layout engine for OxiTerm.
//!
//! Handles building and defragmenting the DOM tree (arena allocation),
//! parsing THTML templates and TCSS stylesheets, computing flexbox layouts via Taffy,
//! and generating optimized ANSI screen update sequences.

#![allow(clippy::all, clippy::pedantic)]

pub mod arena;
pub mod document;
pub mod parser;
pub mod render;
pub mod layout;

pub use arena::NodeArena;
pub use document::THTMLDocument;
pub use render::buffer::{CellBuffer, DoubleBuffer};
pub use render::diff::{DiffEngine, AnsiCommand};
pub use render::emitter::FrameSink;
pub use layout::types::{HitTester, LayoutResult, Rect};
pub use layout::engine::LayoutEngine;
