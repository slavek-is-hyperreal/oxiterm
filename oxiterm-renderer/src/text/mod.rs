//! Proportional text (Phase 6). Layout and rendering do not use shaping yet; the bundle defines
//! the GUI reference cell (Stage 6.1).
#[cfg(feature = "text-shaping")]
pub mod bundle;
#[cfg(feature = "text-shaping")]
pub mod shaping;
