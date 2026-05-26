//! Visual rendering, cell double-buffering, and terminal display protocols.
//!
//! Includes buffer allocation, character cell manipulation, terminal drawing diffs,
//! and kitty/sixel graphics protocol emitters.

pub mod buffer;
pub mod diff;
pub mod unicode;
pub mod emitter;
pub mod renderer;
pub mod kitty;
pub mod sixel;
pub mod cache;
