//! Document layout calculation and spatial query processing.
//!
//! Integrates the Taffy library to compute element locations and dimensions
//! based on Flexbox styling, and exposes a hit-testing helper for cursor interactions.

pub mod engine;
pub mod types;

pub use engine::LayoutEngine;
pub use types::{Rect, LayoutResult, HitTester};
