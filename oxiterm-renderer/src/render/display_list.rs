//! Backend-neutral paint commands (OxiUI phase 1, see `docs/oxi-ui.md`).
//!
//! The renderer turns DOM + layout into a [`DisplayList`]: a flat, paint-ordered list of
//! primitives in `f32` logical units. Backends ("sinks") rasterize it. In phase 1 the only
//! sink is [`crate::render::cell_sink::CellSink`], which must reproduce the historical cell
//! output exactly (guarded by `tests/golden_cells.rs`).
//!
//! # Coordinate system
//! In phase 1, one unit = one terminal cell, and every coordinate produced by the builder is
//! integer-valued. Coordinates are absolute (scroll and centering offsets already applied)
//! and may be negative or exceed the viewport; sinks clip to their surface bounds.
//!
//! # Sink semantics (normative for the cell sink)
//! Items are painted strictly in list order; a later item overwrites earlier cells.
//! - [`DisplayItem::Fill`]: every cell in `rect` becomes `' '` with `fg`/`bg`.
//! - [`DisplayItem::Border`]: draws the frame of `rect` with `chars`, `fg`, `bg`, in exactly
//!   this order: the four corners (top-left; top-right if w > 1; bottom-left if h > 1;
//!   bottom-right if w > 1 && h > 1), then top/bottom edges for x in 1..w-1 (bottom only if
//!   h > 1), then left/right edges for y in 1..h-1 (right only if w > 1). Nothing is drawn
//!   if w == 0 || h == 0.
//! - [`DisplayItem::Text`]: each glyph's `ch` is written at `(origin.x + g.x, origin.y + g.y)`;
//!   for `g.advance > 1`, continuation cells `x + 1 .. x + advance` are written as `' '` with
//!   the same colours, but only those whose x is `< clip.x + clip.w`. The builder only emits
//!   glyphs whose origin cell lies inside `clip`.
//! - [`DisplayItem::Legacy`]: painted by the legacy cell code for that node (img, video,
//!   diagram) into `content`. GUI sinks will get dedicated items for these later.
//! - [`DisplayItem::CellGrid`]: paints `rect.w * rect.h` cells verbatim, row-major, starting
//!   at `(rect.x, rect.y)`. Every cell is written (glyph, colours, modifiers, `skip` flag),
//!   clamped to the surface. A grid whose `cells.len()` differs from `w * h` paints only the
//!   cells that exist. Produced by `Renderer::render_frame` in place of `Legacy` items and
//!   for session overlays (focus ring, predictive echo, scroll bar) in GUI sessions.
//! - [`DisplayItem::PushClip`] / [`DisplayItem::PopClip`]: reserved. Not emitted in phase 1,
//!   because the historical renderer does not clip children to their parents.

use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};

/// Axis-aligned rectangle in logical units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct RectF {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl RectF {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
}

/// A point in logical units.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PointF {
    pub x: f32,
    pub y: f32,
}

/// One positioned glyph inside a [`DisplayItem::Text`] run.
///
/// Positions are relative to the run's `origin`. `advance` is the glyph width in cells
/// (1 for normal glyphs, 2 for wide CJK/emoji).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    pub x: f32,
    pub y: f32,
    pub ch: char,
    pub advance: f32,
}

/// Text modifiers carried to the sink.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Modifiers {
    pub bold: bool,
    pub underline: bool,
    pub italic: bool,
}

/// One verbatim cell of a [`DisplayItem::CellGrid`].
///
/// `skip` marks the trailing half of a wide glyph (see `Cell::skip`); GUI painters should
/// not draw it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridCell {
    pub ch: char,
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    pub mods: Modifiers,
    pub skip: bool,
}

impl From<crate::render::buffer::Cell> for GridCell {
    fn from(c: crate::render::buffer::Cell) -> Self {
        Self {
            ch: c.ch,
            fg: c.fg,
            bg: c.bg,
            mods: Modifiers { bold: c.bold, underline: c.underline, italic: c.italic },
            skip: c.skip,
        }
    }
}

/// One paint command.
#[derive(Debug, Clone, PartialEq)]
pub enum DisplayItem {
    /// Solid background fill of a node's border box.
    Fill { node: NodeId, rect: RectF, fg: AnsiColor, bg: AnsiColor },
    /// Node frame. `chars` keeps the TUI glyph set; GUI sinks may draw vector lines instead.
    Border { node: NodeId, rect: RectF, chars: BorderChars, fg: AnsiColor, bg: AnsiColor },
    /// Already laid-out, positioned glyphs (wrapping and masking are resolved by the builder).
    Text {
        node: NodeId,
        origin: PointF,
        clip: RectF,
        glyphs: Vec<Glyph>,
        fg: AnsiColor,
        bg: AnsiColor,
        mods: Modifiers,
    },
    /// Content painted by legacy cell code (img / video / diagram in phase 1).
    Legacy { node: NodeId, content: RectF },
    /// Cells painted verbatim (see the module docs). `cells` is row-major, `rect.w * rect.h`.
    CellGrid { rect: RectF, cells: Vec<GridCell> },
    /// Reserved: start clipping subsequent items to `rect`.
    PushClip { rect: RectF },
    /// Reserved: end the innermost clip.
    PopClip,
}

/// Paint-ordered list of [`DisplayItem`]s for one frame.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
}

impl DisplayList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, item: DisplayItem) {
        self.items.push(item);
    }
}
