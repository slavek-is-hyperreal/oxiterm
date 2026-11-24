//! Proportional text (Phase 6). `bundle` defines the GUI reference cell (6.1); `layout` is the
//! shaped text layout that `LayoutEngine` measures with (6.3).
#[cfg(feature = "text-shaping")]
pub mod bundle;
#[cfg(feature = "text-shaping")]
pub mod layout;
#[cfg(feature = "text-shaping")]
pub mod shaping;

/// Without `text-shaping` there is nothing to shape with: every cache lookup misses and layout
/// keeps measuring in whole cells.
#[cfg(not(feature = "text-shaping"))]
pub mod layout {
    use crate::font::ResolvedFont;
    use std::sync::Arc;

    /// Placeholder; never constructed.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TextLayout;

    impl TextLayout {
        pub fn min_content_cells(&self) -> f32 {
            0.0
        }
        pub fn max_content_cells(&self) -> f32 {
            0.0
        }
        pub fn height_cells(&self) -> f32 {
            0.0
        }
        pub fn line_count(&self) -> usize {
            0
        }
    }

    /// Placeholder cache.
    #[derive(Default)]
    pub struct TextCache;

    impl TextCache {
        pub fn begin_frame(&mut self) {}
        pub fn base(&mut self, _text: &str, _font: &ResolvedFont) -> Option<Arc<TextLayout>> {
            None
        }
        pub fn at_width(&mut self, base: &Arc<TextLayout>, _width_cells: f32) -> Arc<TextLayout> {
            base.clone()
        }
    }
}
