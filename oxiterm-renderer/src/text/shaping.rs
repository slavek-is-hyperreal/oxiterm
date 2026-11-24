//! Shaping spike on top of `cosmic-text` (built `no_std`: libm math, no system font scan, no
//! memmap). Fonts come only from the byte slices handed to [`Shaper::new`].
//!
//! Contract used by later stages: per glyph an `f32` x offset, an `f32` advance and the source
//! text of its cluster; lines are broken at an optional max width.

use std::sync::Arc;

use cosmic_text::{
    fontdb, Attrs, Buffer, Family, FontSystem, Metrics, Shaping, Weight, Wrap,
};

/// One positioned glyph of a [`ShapedLine`].
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedGlyph {
    /// Left edge of the glyph box, from the start of the line.
    pub x: f32,
    /// Horizontal advance of the glyph box.
    pub advance: f32,
    /// Source text of the cluster this glyph belongs to (a ligature or a base plus combining
    /// marks share one cluster; a glyph may carry the whole cluster text).
    pub cluster: String,
    /// Glyph id in the face; 0 is `.notdef` (the face has no glyph for the cluster).
    pub glyph_id: u16,
}

/// One visual line.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedLine {
    /// Width of the line as laid out.
    pub width: f32,
    /// Top of the line box.
    pub top: f32,
    /// Baseline of the line.
    pub baseline: f32,
    pub glyphs: Vec<ShapedGlyph>,
}

/// Line height as a multiple of the font size (spike default).
pub const LINE_HEIGHT_FACTOR: f32 = 1.25;

pub struct Shaper {
    fs: FontSystem,
    family: String,
}

impl Shaper {
    /// Builds a shaper from embedded font files. The first face's family is the one used.
    /// Returns `None` if no face parses.
    pub fn new(fonts: &[&'static [u8]]) -> Option<Self> {
        let mut db = fontdb::Database::new();
        for f in fonts {
            db.load_font_source(fontdb::Source::Binary(Arc::new(*f)));
        }
        let family = db.faces().next()?.families.first()?.0.clone();
        let fs = FontSystem::new_with_locale_and_db("en-US".to_string(), db);
        Some(Self { fs, family })
    }

    /// Shapes `text` at `size_px`, breaking lines at `max_width` (if any). `\n` starts a new
    /// paragraph. `bold` picks weight 700, else 400.
    pub fn shape(
        &mut self,
        text: &str,
        size_px: f32,
        max_width: Option<f32>,
        bold: bool,
    ) -> Vec<ShapedLine> {
        let metrics = Metrics::new(size_px, size_px * LINE_HEIGHT_FACTOR);
        let mut buf = Buffer::new(&mut self.fs, metrics);
        buf.set_wrap(Wrap::Word);
        buf.set_size(max_width, None);
        let weight = if bold { Weight::BOLD } else { Weight::NORMAL };
        let attrs = Attrs::new().family(Family::Name(&self.family)).weight(weight);
        buf.set_text(text, &attrs, Shaping::Advanced, None);
        buf.shape_until_scroll(&mut self.fs, false);
        buf.layout_runs()
            .map(|run| ShapedLine {
                width: run.line_w,
                top: run.line_top,
                baseline: run.line_y,
                glyphs: run
                    .glyphs
                    .iter()
                    .map(|g| ShapedGlyph {
                        x: g.x,
                        advance: g.w,
                        cluster: run.text.get(g.start..g.end).unwrap_or("").to_string(),
                        glyph_id: g.glyph_id,
                    })
                    .collect(),
            })
            .collect()
    }
}
