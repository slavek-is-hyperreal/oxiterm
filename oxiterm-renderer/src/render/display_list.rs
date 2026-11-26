//! Backend-neutral paint commands (OxiUI phase 1, see `docs/history/oxi-ui-plan-history.md`).
//!
//! The renderer turns DOM + layout into a [`DisplayList`]: a flat, paint-ordered list of
//! primitives in `f32` logical units. Backends ("sinks") rasterize it. In phase 1 the only
//! sink is [`crate::render::cell_sink::CellSink`], which must reproduce the historical cell
//! output exactly (guarded by `tests/golden_cells.rs`).
//!
//! # Coordinate system
//! One unit = one terminal cell. In TUI mode every coordinate produced by the builder is
//! integer-valued; in GUI mode (display-list sessions, fractional layout) rects and origins
//! may be fractional, while glyph offsets inside a default-font text run stay whole cells (shaped runs,
//! see [`ShapedRun`], carry fractional offsets). Coordinates are
//! absolute (scroll and centering offsets already applied) and may be negative or exceed the
//! viewport; sinks clip to their surface bounds. `radius` and stroke widths are in cells
//! measured along the horizontal axis (GUI painters multiply by the cell width); path x/y
//! scale by cell width/height respectively.
//!
//! # Sink semantics (normative for the cell sink)
//! Items are painted strictly in list order; a later item overwrites earlier cells.
//! - [`DisplayItem::Fill`]: every cell in `rect` becomes `' '` with `fg`/`bg`. `radius` is
//!   ignored (the cell sink always draws square corners).
//! - [`DisplayItem::Border`]: draws the frame of `rect` with `chars`, `fg`, `bg`, in exactly
//!   this order: the four corners (top-left; top-right if w > 1; bottom-left if h > 1;
//!   bottom-right if w > 1 && h > 1), then top/bottom edges for x in 1..w-1 (bottom only if
//!   h > 1), then left/right edges for y in 1..h-1 (right only if w > 1). Nothing is drawn
//!   if w == 0 || h == 0. `radius` is ignored (square corners, exactly as before).
//! - [`DisplayItem::Text`]: each glyph's `ch` is written at `(origin.x + g.x, origin.y + g.y)`;
//!   for `g.advance > 1`, continuation cells `x + 1 .. x + advance` are written as `' '` with
//!   the same colours, but only those whose x is `< clip.x + clip.w`. The builder only emits
//!   glyphs whose origin cell lies inside `clip`.
//!   A text run with `shaped: Some(run)` (proportional text, Phase 6.4; GUI sessions only, the
//!   TUI never shapes) is written differently: each glyph's FIRST char (`ch`) goes to the single
//!   cell `(floor(origin.x + g.x), floor(origin.y + g.y))`, nothing else (no continuation
//!   cells, no cluster tail), clamped to the surface. Later glyphs overwrite earlier ones that
//!   land in the same cell. This is an approximation for the cell buffer GUI overlays are diffed
//!   against; the real painting is the client's.
//! - [`DisplayItem::Path`]: ignored entirely. A terminal cannot draw vectors; paths are
//!   GUI-only.
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

/// One vector path command (absolute coordinates).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PathCmd {
    MoveTo(PointF),
    LineTo(PointF),
    /// Quadratic Bezier: control point, end point.
    QuadTo(PointF, PointF),
    /// Cubic Bezier: two control points, end point.
    CubicTo(PointF, PointF, PointF),
    Close,
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

/// Face id of the bundled monospace face in [`ShapedRun::face`] (`FontBundle` id `oxi-mono`).
pub const FACE_MONO: u8 = 0;
/// Face id of the bundled proportional face (`FontBundle` id `oxi-sans`).
pub const FACE_SANS: u8 = 1;

/// What a painter needs to draw a proportional (shaped) text run (Phase 6.4). Present only on
/// text whose font is not the root default (mono, 16 px, 20 px line); default text has
/// `shaped: None` and keeps the cell-wise glyph run it always had.
///
/// Geometry of a shaped run (all positions are cells, relative to the item's `origin`):
/// - `Glyph::x` is `px / 9.6`, the left edge of the glyph's advance box on its line;
/// - `Glyph::y` is the **top of the glyph's line box** (`line.top_px / 20`), so `floor(y)` is the
///   row the line starts in, exactly like the cell path;
/// - the baseline of every line is `y + baseline`; a line box is `line_height` cells high;
/// - `Glyph::advance` is `advance_px / 9.6`;
/// - `Glyph::ch` is the first char of the glyph's cluster; the full cluster text is in
///   `clusters` when it has more than one char (ligature `ffi`, base + combining mark).
#[derive(Debug, Clone, PartialEq)]
pub struct ShapedRun {
    /// Face of the run ([`FACE_MONO`] / [`FACE_SANS`]).
    pub face: u8,
    /// Font size in px (CSS px at the reference cell; painters scale it by `cell_w / 9.6`).
    pub size_px: f32,
    /// Line box height in cells (`line_height_px / 20`).
    pub line_height: f32,
    /// Distance from the line top to the baseline in cells.
    pub baseline: f32,
    /// Glyphs drawn in another face than `face` (fallback, e.g. box drawing inside sans):
    /// `(glyph index in Text::glyphs, face)`, ascending by index.
    pub glyph_faces: Vec<(u32, u8)>,
    /// Cluster text of glyphs that cover more than their `ch`: `(glyph index, text)`, ascending.
    pub clusters: Vec<(u32, Box<str>)>,
}

impl ShapedRun {
    /// Face of glyph `i` (the override, or the run's face).
    pub fn face_of(&self, i: usize) -> u8 {
        match self.glyph_faces.binary_search_by_key(&(i as u32), |e| e.0) {
            Ok(k) => self.glyph_faces[k].1,
            Err(_) => self.face,
        }
    }

    /// Cluster text of glyph `i`, when it differs from the glyph's single `ch`.
    pub fn cluster_of(&self, i: usize) -> Option<&str> {
        match self.clusters.binary_search_by_key(&(i as u32), |e| e.0) {
            Ok(k) => Some(&self.clusters[k].1),
            Err(_) => None,
        }
    }
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
    ///
    /// `radius` is the corner radius in cells (0.0 = square); only GUI painters honour it.
    Fill { node: NodeId, rect: RectF, radius: f32, fg: AnsiColor, bg: AnsiColor },
    /// Node frame. `chars` keeps the TUI glyph set; GUI sinks may draw vector lines instead
    /// (rounded when `radius > 0`).
    Border { node: NodeId, rect: RectF, radius: f32, chars: BorderChars, fg: AnsiColor, bg: AnsiColor },
    /// General vector path (GUI only; the cell sink ignores it). Coordinates are absolute.
    /// `fill` paints the closed shape, `stroke` is `(colour, width in cells)`.
    Path {
        node: NodeId,
        cmds: Vec<PathCmd>,
        fill: Option<AnsiColor>,
        stroke: Option<(AnsiColor, f32)>,
    },
    /// Already laid-out, positioned glyphs (wrapping and masking are resolved by the builder).
    /// `shaped` is `Some` for proportional text (see [`ShapedRun`]); `None` for the cell-wise
    /// default font.
    Text {
        node: NodeId,
        origin: PointF,
        clip: RectF,
        glyphs: Vec<Glyph>,
        fg: AnsiColor,
        bg: AnsiColor,
        mods: Modifiers,
        shaped: Option<Box<ShapedRun>>,
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
