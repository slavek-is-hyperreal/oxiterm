//! DisplayList to pixels. The visual semantics mirror the browser painter
//! (`oxiterm-web/src/paint.rs`, `plan`): the same geometry, colour modes and fallbacks, in device
//! pixels, with every primitive mapped to `tiny-skia` (paths, strokes, masks) or to exact integer
//! writes (square fills, glyph blits).
//!
//! # Geometry
//! One cell is `9.6 * scale` x `20 * scale` px (`CELL_W_PX` / `CELL_H_PX`, the bundled mono face's
//! advance and 1.25 x 16 px). Unlike the browser client the cell is NOT rounded to whole pixels, so
//! a shaped glyph at `x_cells * 9.6` lands on the very pixel position the shaper computed; only
//! edges of square fills are snapped. Glyph pixel size is `size_px * scale`.
//!
//! # Determinism
//! No threads, no time, no hash-ordered iteration that affects output, quarter-pixel glyph phases
//! and a cache that stores exactly what a fresh render would produce. Same input, same platform,
//! same bytes. (tiny-skia picks its SIMD lowp/highp pipelines at compile time, so a different
//! `target-cpu` may differ in the last bit of an anti-aliased edge pixel.)

use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::font::{CELL_H_PX, CELL_W_PX};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PathCmd, PointF, RectF, ShapedRun, FACE_MONO,
};
use oxiterm_renderer::render::unicode::UnicodeWidthCache;
use tiny_skia::{FillRule, Mask, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

use crate::color::{self, Rgb};
use crate::glyphs::{GlyphCache, SUBPIXEL_STEPS};
use crate::surface::Surface;

/// Default text size (px at scale 1) of the cell-wise font.
const CELL_FONT_PX: f32 = 16.0;
/// Bezier handle length of a quarter circle, as a fraction of the radius.
const KAPPA: f64 = 0.552_284_749_8;

/// The software rasterizer. Owns the glyph cache, so keep one per output and reuse it.
pub struct Rasterizer {
    scale: f64,
    cw: f64,
    ch: f64,
    cache: GlyphCache,
}

impl Rasterizer {
    /// `scale` multiplies the reference cell (9.6 x 20 px). Integer scales are crispest; fractional
    /// ones work (HiDPI, small panels). Clamped to `0.25..=16`.
    pub fn new(scale: f64) -> Self {
        let scale = if scale.is_finite() { scale.clamp(0.25, 16.0) } else { 1.0 };
        Self {
            scale,
            // f32 9.6 widened to f64 is 9.600000381: use the decimal value the constant means.
            cw: (f64::from(CELL_W_PX) * 1e4).round() / 1e4 * scale,
            ch: (f64::from(CELL_H_PX) * 1e4).round() / 1e4 * scale,
            cache: GlyphCache::new(),
        }
    }

    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// Cell size in px.
    pub fn cell_px(&self) -> (f64, f64) {
        (self.cw, self.ch)
    }

    /// Pixel size of a `cols` x `rows` cell viewport (rounded up).
    pub fn pixel_size(&self, cols: u32, rows: u32) -> (u32, u32) {
        // 80 * 9.6 is 768.0000000000001 in f64: strip the noise before rounding up.
        let up = |v: f64| ((v * 1e6).round() / 1e6).ceil() as u32;
        (up(f64::from(cols) * self.cw), up(f64::from(rows) * self.ch))
    }

    /// A fresh surface for a `cols` x `rows` viewport.
    pub fn surface_for(&self, cols: u32, rows: u32) -> Option<Surface> {
        let (w, h) = self.pixel_size(cols, rows);
        Surface::new(w, h)
    }

    /// The glyph cache (stats, tests).
    pub fn glyphs(&mut self) -> &mut GlyphCache {
        &mut self.cache
    }

    /// Clears `surface` to the default background and paints `list` on it.
    pub fn render(&mut self, list: &DisplayList, surface: &mut Surface) {
        surface.clear(color::DEFAULT_BG);
        let (w, h) = (i64::from(surface.width()), i64::from(surface.height()));
        let mut cv = Canvas {
            pm: &mut surface.pixmap,
            clips: vec![ClipEntry { rect: IRect { x0: 0, y0: 0, x1: w, y1: h }, mask: None }],
            cw: self.cw,
            ch: self.ch,
            scale: self.scale,
            cache: &mut self.cache,
        };
        cv.paint(list);
    }
}

// ---- sanitising ----------------------------------------------------------------------------
//
// The display list is untrusted. Every number is checked at this boundary: a non-finite
// coordinate, size, radius or width skips the item (or the glyph); finite ones are clamped to
// +-LIM px before they reach tiny-skia or integer maths. LIM exceeds the largest surface side
// plus the largest glyph, so a clamped item can never land on screen when the original did not.

/// Coordinate margin in px.
const LIM: f64 = 1.0e5;
/// Widest stroke handed to tiny-skia, px.
const MAX_STROKE_PX: f64 = 1.0e4;
const _: () = assert!(LIM > crate::surface::MAX_DIM as f64 + 4.0 * crate::glyphs::MAX_GLYPH_PX as f64);

fn fin(v: &[f32]) -> bool {
    v.iter().all(|x| x.is_finite())
}

fn lim(v: f64) -> f64 {
    v.clamp(-LIM, LIM)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IRect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl IRect {
    fn intersect(self, o: IRect) -> IRect {
        let r = IRect { x0: self.x0.max(o.x0), y0: self.y0.max(o.y0), x1: self.x1.min(o.x1), y1: self.y1.min(o.y1) };
        if r.x1 < r.x0 || r.y1 < r.y0 {
            IRect { x0: r.x0, y0: r.y0, x1: r.x0, y1: r.y0 }
        } else {
            r
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderStyle {
    Single,
    Rounded,
    Double,
    Glyphs,
}

/// Same rules as the web painter's `border_style`.
fn border_style(c: &BorderChars, w: f32, h: f32) -> BorderStyle {
    if w < 2.0 || h < 2.0 {
        return BorderStyle::Glyphs;
    }
    if [c.top, c.bot, c.left, c.right].contains(&' ') {
        return BorderStyle::Glyphs;
    }
    match c.top_left {
        '╭' => BorderStyle::Rounded,
        '╔' => BorderStyle::Double,
        '┌' => BorderStyle::Single,
        _ => BorderStyle::Glyphs,
    }
}

fn paint_of(c: Rgb) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], 255);
    p.anti_alias = true;
    p
}

/// Rounded rectangle path (quarter-circle cubics), `r` already clamped. Inputs within +-LIM.
fn rr_path(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let f = |v: f64| v as f32;
    if r <= 0.0 {
        pb.move_to(f(x0), f(y0));
        pb.line_to(f(x1), f(y0));
        pb.line_to(f(x1), f(y1));
        pb.line_to(f(x0), f(y1));
        pb.close();
        return pb.finish();
    }
    let k = r * KAPPA;
    pb.move_to(f(x0 + r), f(y0));
    pb.line_to(f(x1 - r), f(y0));
    pb.cubic_to(f(x1 - r + k), f(y0), f(x1), f(y0 + r - k), f(x1), f(y0 + r));
    pb.line_to(f(x1), f(y1 - r));
    pb.cubic_to(f(x1), f(y1 - r + k), f(x1 - r + k), f(y1), f(x1 - r), f(y1));
    pb.line_to(f(x0 + r), f(y1));
    pb.cubic_to(f(x0 + r - k), f(y1), f(x0), f(y1 - r + k), f(x0), f(y1 - r));
    pb.line_to(f(x0), f(y0 + r));
    pb.cubic_to(f(x0), f(y0 + r - k), f(x0 + r - k), f(y0), f(x0 + r), f(y0));
    pb.close();
    pb.finish()
}

/// One entry of the clip stack. The AA mask is built at most once per clip, on first use by a
/// path/stroke (rect fills and glyph blits clip by integer intersection and never need it).
struct ClipEntry {
    rect: IRect,
    mask: Option<Option<Mask>>,
}

struct Canvas<'a> {
    pm: &'a mut Pixmap,
    clips: Vec<ClipEntry>,
    cw: f64,
    ch: f64,
    scale: f64,
    cache: &'a mut GlyphCache,
}

impl Canvas<'_> {
    fn clip(&self) -> IRect {
        self.clips.last().expect("clip stack is never empty").rect
    }

    fn push_clip(&mut self, r: IRect) {
        let rect = self.clip().intersect(r);
        self.clips.push(ClipEntry { rect, mask: None });
    }

    fn paint(&mut self, list: &DisplayList) {
        let base_depth = self.clips.len();
        for item in &list.items {
            match item {
                DisplayItem::Fill { rect, radius, bg, .. } => {
                    if !fin(&[rect.x, rect.y, rect.w, rect.h, *radius]) {
                        continue;
                    }
                    if *radius > 0.0 {
                        self.rounded_fill(*rect, *radius, bg);
                    } else {
                        self.push_fill(*rect, color::bg(bg));
                    }
                }
                DisplayItem::Border { rect, radius, chars, fg, bg, .. } => {
                    if fin(&[rect.x, rect.y, rect.w, rect.h, *radius]) {
                        self.border(*rect, *radius, chars, fg, bg);
                    }
                }
                DisplayItem::Text { origin, clip, glyphs, fg, bg, mods, shaped, .. } => {
                    if !fin(&[origin.x, origin.y, clip.x, clip.y, clip.w, clip.h]) {
                        continue;
                    }
                    let r = self.cell_irect(*clip);
                    self.push_clip(r);
                    match shaped {
                        Some(run) => self.shaped(*origin, glyphs, run, fg, bg, *mods),
                        None => {
                            for g in glyphs {
                                if fin(&[g.x, g.y, g.advance]) {
                                    self.glyph_cell(origin.x + g.x, origin.y + g.y, g.advance, g.ch, fg, bg, *mods);
                                }
                            }
                        }
                    }
                    self.clips.pop();
                }
                DisplayItem::Legacy { .. } => {}
                DisplayItem::CellGrid { rect, cells } => {
                    if fin(&[rect.x, rect.y, rect.w, rect.h]) {
                        self.grid(*rect, cells);
                    }
                }
                DisplayItem::PushClip { rect } => {
                    // A non-finite clip clips everything away (fail closed).
                    let r = if fin(&[rect.x, rect.y, rect.w, rect.h]) {
                        self.cell_irect(*rect)
                    } else {
                        IRect { x0: 0, y0: 0, x1: 0, y1: 0 }
                    };
                    self.push_clip(r);
                }
                DisplayItem::Path { cmds, fill, stroke, .. } => self.path(cmds, fill, stroke),
                DisplayItem::PopClip => {
                    if self.clips.len() > base_depth {
                        self.clips.pop();
                    }
                }
            }
        }
        self.clips.truncate(base_depth);
    }

    // ---- geometry helpers -------------------------------------------------------------------

    /// Pixel edges (x0, y0, x1, y1) of a finite cell rect, each clamped to +-LIM.
    fn cell_edges(&self, x: f32, y: f32, w: f32, h: f32) -> (f64, f64, f64, f64) {
        let (x0, y0) = (f64::from(x) * self.cw, f64::from(y) * self.ch);
        let (x1, y1) = (x0 + f64::from(w) * self.cw, y0 + f64::from(h) * self.ch);
        (lim(x0), lim(y0), lim(x1), lim(y1))
    }

    fn cell_irect(&self, r: RectF) -> IRect {
        let (x0, y0, x1, y1) = self.cell_edges(r.x, r.y, r.w, r.h);
        IRect { x0: x0.round() as i64, y0: y0.round() as i64, x1: x1.round() as i64, y1: y1.round() as i64 }
    }

    /// `true` when the box (padded) can touch the current clip.
    fn visible(&self, x0: f64, y0: f64, x1: f64, y1: f64, pad: f64) -> bool {
        let c = self.clip();
        x1 + pad > c.x0 as f64 && x0 - pad < c.x1 as f64 && y1 + pad > c.y0 as f64 && y0 - pad < c.y1 as f64
    }

    // ---- fills ------------------------------------------------------------------------------

    /// Solid rectangle of whole pixels (clipped). Opaque colours only, so a plain write.
    fn fill_irect(&mut self, r: IRect, c: Rgb) {
        let r = r.intersect(self.clip());
        if r.x1 <= r.x0 || r.y1 <= r.y0 {
            return;
        }
        let w = self.pm.width() as usize;
        let data = self.pm.data_mut();
        let (x0, n) = (r.x0 as usize, (r.x1 - r.x0) as usize);
        let first = (r.y0 as usize * w + x0) * 4;
        for px in data[first..first + n * 4].chunks_exact_mut(4) {
            px.copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
        // the remaining rows are memcpy's of the first
        for y in (r.y0 + 1)..r.y1 {
            data.copy_within(first..first + n * 4, (y as usize * w + x0) * 4);
        }
    }

    /// Pixel rect given by its edges, each rounded to a whole pixel (adjacent fills share edges).
    fn fill_edges(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, c: Rgb) {
        let (x0, y0, x1, y1) = (lim(x0), lim(y0), lim(x1), lim(y1));
        self.fill_irect(IRect { x0: x0.round() as i64, y0: y0.round() as i64, x1: x1.round() as i64, y1: y1.round() as i64 }, c);
    }

    fn fill_frect(&mut self, x: f64, y: f64, w: f64, h: f64, c: Rgb) {
        if x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite() {
            self.fill_edges(x, y, x + w, y + h, c);
        }
    }

    fn push_fill(&mut self, r: RectF, c: Rgb) {
        if !fin(&[r.x, r.y, r.w, r.h]) || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (x0, y0, x1, y1) = self.cell_edges(r.x, r.y, r.w, r.h);
        self.fill_edges(x0, y0, x1, y1, c);
    }

    /// Pixel rect of a finite cell rect plus its corner radius (cells along x to px), clamped to
    /// half the smaller side.
    fn rounded_px(&self, r: RectF, radius: f32) -> (f64, f64, f64, f64, f64) {
        let (x0, y0, x1, y1) = self.cell_edges(r.x, r.y, r.w, r.h);
        let (w, h) = (x1 - x0, y1 - y0);
        let rad = (f64::from(radius) * self.cw).max(0.0).min(w / 2.0).min(h / 2.0);
        (x0, y0, w, h, rad)
    }

    fn rounded_fill(&mut self, r: RectF, radius: f32, bg: &AnsiColor) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (x, y, w, h, rad) = self.rounded_px(r, radius);
        if !self.visible(x, y, x + w, y + h, 2.0) {
            return;
        }
        if let Some(p) = rr_path(x, y, x + w, y + h, rad) {
            self.fill_path(&p, color::bg(bg), FillRule::Winding);
        }
    }

    /// Runs `f` with the pixmap and the (lazily built, once per clip) mask of the current clip;
    /// `None` when the clip covers the whole surface.
    fn with_mask<R>(&mut self, f: impl FnOnce(&mut Pixmap, Option<&Mask>) -> R) -> R {
        let (w, h) = (self.pm.width(), self.pm.height());
        let e = self.clips.last_mut().expect("clip stack is never empty");
        if e.mask.is_none() {
            let c = e.rect;
            let full = c == IRect { x0: 0, y0: 0, x1: i64::from(w), y1: i64::from(h) };
            e.mask = Some(if full {
                None
            } else {
                Mask::new(w, h).map(|mut m| {
                    if c.x1 > c.x0 && c.y1 > c.y0 {
                        if let Some(p) = rr_path(c.x0 as f64, c.y0 as f64, c.x1 as f64, c.y1 as f64, 0.0) {
                            m.fill_path(&p, FillRule::Winding, false, Transform::identity());
                        }
                    }
                    m
                })
            });
        }
        f(self.pm, e.mask.as_ref().and_then(Option::as_ref))
    }

    fn fill_path(&mut self, p: &Path, c: Rgb, rule: FillRule) {
        let paint = paint_of(c);
        self.with_mask(|pm, m| pm.fill_path(p, &paint, rule, Transform::identity(), m));
    }

    fn stroke_path(&mut self, p: &Path, c: Rgb, width: f64) {
        // tiny-skia draws strokes of width <= 1 as coverage-scaled hairlines, which smear a
        // pixel-aligned 1 px line over two rows; a hair wider takes the exact-coverage path.
        let width = width.min(MAX_STROKE_PX);
        let w = if width <= 1.0 { width + 0.001 } else { width };
        let stroke = Stroke { width: w as f32, ..Stroke::default() };
        let paint = paint_of(c);
        self.with_mask(|pm, m| pm.stroke_path(p, &paint, &stroke, Transform::identity(), m));
    }

    // ---- paths ------------------------------------------------------------------------------

    fn path(&mut self, cmds: &[PathCmd], fill: &Option<AnsiColor>, stroke: &Option<(AnsiColor, f32)>) {
        if cmds.is_empty() {
            return;
        }
        let mut pts: Vec<f32> = Vec::with_capacity(cmds.len() * 6);
        for c in cmds {
            match c {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => pts.extend([p.x, p.y]),
                PathCmd::QuadTo(a, p) => pts.extend([a.x, a.y, p.x, p.y]),
                PathCmd::CubicTo(a, b, p) => pts.extend([a.x, a.y, b.x, b.y, p.x, p.y]),
                PathCmd::Close => {}
            }
        }
        let stroke_w = stroke.as_ref().map(|s| s.1);
        if !fin(&pts) || stroke_w.is_some_and(|w| !w.is_finite()) {
            return;
        }
        let pad = stroke_w.map_or(0.0, |w| (f64::from(w) * self.cw).clamp(0.0, MAX_STROKE_PX)) / 2.0 + 4.0;
        let (mut bx0, mut by0, mut bx1, mut by1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for xy in pts.chunks_exact(2) {
            let (x, y) = (lim(f64::from(xy[0]) * self.cw), lim(f64::from(xy[1]) * self.ch));
            bx0 = bx0.min(x);
            bx1 = bx1.max(x);
            by0 = by0.min(y);
            by1 = by1.max(y);
        }
        if !pts.is_empty() && !self.visible(bx0, by0, bx1, by1, pad) {
            return;
        }
        let pt = |p: &PointF| (lim(f64::from(p.x) * self.cw) as f32, lim(f64::from(p.y) * self.ch) as f32);
        let mut pb = PathBuilder::new();
        for c in cmds {
            match c {
                PathCmd::MoveTo(p) => {
                    let (x, y) = pt(p);
                    pb.move_to(x, y);
                }
                PathCmd::LineTo(p) => {
                    let (x, y) = pt(p);
                    pb.line_to(x, y);
                }
                PathCmd::QuadTo(c1, p) => {
                    let (cx, cy) = pt(c1);
                    let (x, y) = pt(p);
                    pb.quad_to(cx, cy, x, y);
                }
                PathCmd::CubicTo(c1, c2, p) => {
                    let (c1x, c1y) = pt(c1);
                    let (c2x, c2y) = pt(c2);
                    let (x, y) = pt(p);
                    pb.cubic_to(c1x, c1y, c2x, c2y, x, y);
                }
                PathCmd::Close => pb.close(),
            }
        }
        let Some(path) = pb.finish() else { return };
        if let Some(c) = fill {
            self.fill_path(&path, color::bg(c), FillRule::Winding);
        }
        if let Some((c, w)) = stroke {
            let width = f64::from(*w) * self.cw;
            if width > 0.0 {
                self.stroke_path(&path, color::fg(c), width);
            }
        }
    }

    // ---- borders ----------------------------------------------------------------------------

    fn border(&mut self, rect: RectF, radius: f32, chars: &BorderChars, fg: &AnsiColor, bg: &AnsiColor) {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let (bx0, by0, bx1, by1) = self.cell_edges(rect.x, rect.y, rect.w, rect.h);
        if !self.visible(bx0, by0, bx1, by1, self.cw + self.ch + 4.0) {
            return;
        }
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let bgc = color::bg(bg);
        let fgc = color::fg(fg);
        let vector_round = radius > 0.0
            && w >= 2.0
            && h >= 2.0
            && ![chars.top, chars.bot, chars.left, chars.right].contains(&' ');
        if vector_round {
            // Ring between the rounded outer silhouette and the rect one cell inside it.
            let (px, py, pw, ph, rad) = self.rounded_px(rect, radius);
            let mut pb = PathBuilder::new();
            if let Some(outer) = rr_path(px, py, px + pw, py + ph, rad) {
                pb.push_path(&outer);
                let (ix0, iy0) = (px + self.cw, py + self.ch);
                let (ix1, iy1) = (px + pw - self.cw, py + ph - self.ch);
                if ix1 > ix0 && iy1 > iy0 {
                    if let Some(inner) = rr_path(ix0, iy0, ix1, iy1, 0.0) {
                        pb.push_path(&inner);
                    }
                }
                if let Some(ring) = pb.finish() {
                    self.fill_path(&ring, bgc, FillRule::EvenOdd);
                }
            }
        } else {
            self.push_fill(RectF::new(x, y, w, 1.0), bgc);
            if h > 1.0 {
                self.push_fill(RectF::new(x, y + h - 1.0, w, 1.0), bgc);
            }
            if h > 2.0 {
                self.push_fill(RectF::new(x, y + 1.0, 1.0, h - 2.0), bgc);
                if w > 1.0 {
                    self.push_fill(RectF::new(x + w - 1.0, y + 1.0, 1.0, h - 2.0), bgc);
                }
            }
        }

        let style = if vector_round { BorderStyle::Rounded } else { border_style(chars, w, h) };
        if style == BorderStyle::Glyphs {
            self.glyph_frame(rect, chars, fg, bg);
            return;
        }

        // Vector lines through the centres of the border cells, snapped to the pixel grid.
        let lw_dev = self.scale.round().max(1.0);
        let off = if (lw_dev as i64) % 2 == 1 { 0.5 } else { 0.0 };
        let snap = |v: f64| lim(v).round() + off;
        let (xf, yf, wf, hf) = (f64::from(x), f64::from(y), f64::from(w), f64::from(h));
        let cx0 = (xf + 0.5) * self.cw;
        let cx1 = (xf + wf - 0.5) * self.cw;
        let cy0 = (yf + 0.5) * self.ch;
        let cy1 = (yf + hf - 0.5) * self.ch;
        let stroke = |s: &mut Self, x0: f64, y0: f64, x1: f64, y1: f64, r: f64| {
            let (x0, y0, x1, y1) = (snap(x0), snap(y0), snap(x1), snap(y1));
            // the radius is re-clamped against the clamped (finite) box
            let r = r.min((x1 - x0) / 2.0).min((y1 - y0) / 2.0).max(0.0);
            if let Some(p) = rr_path(x0, y0, x1, y1, r) {
                s.stroke_path(&p, fgc, lw_dev);
            }
        };
        match style {
            BorderStyle::Single => stroke(self, cx0, cy0, cx1, cy1, 0.0),
            BorderStyle::Rounded => {
                let want = if vector_round { f64::from(radius) * self.cw } else { self.cw.min(self.ch) / 2.0 };
                let r = want.min((cx1 - cx0) / 2.0).min((cy1 - cy0) / 2.0);
                stroke(self, cx0, cy0, cx1, cy1, r);
            }
            BorderStyle::Double => {
                let d = self.cw.min(self.ch) / 6.0;
                stroke(self, cx0 - d, cy0 - d, cx1 + d, cy1 + d, 0.0);
                stroke(self, cx0 + d, cy0 + d, cx1 - d, cy1 - d, 0.0);
            }
            BorderStyle::Glyphs => unreachable!("handled above"),
        }
    }

    /// Frame drawn as glyph cells, in the cell sink's order. The edge loops run over integer
    /// counters restricted to the cells that can reach the clip.
    fn glyph_frame(&mut self, rect: RectF, chars: &BorderChars, fg: &AnsiColor, bg: &AnsiColor) {
        let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
        let m = Modifiers::default();
        self.glyph_cell(x, y, 1.0, chars.top_left, fg, bg, m);
        if w > 1.0 {
            self.glyph_cell(x + w - 1.0, y, 1.0, chars.top_right, fg, bg, m);
        }
        if h > 1.0 {
            self.glyph_cell(x, y + h - 1.0, 1.0, chars.bot_left, fg, bg, m);
        }
        if w > 1.0 && h > 1.0 {
            self.glyph_cell(x + w - 1.0, y + h - 1.0, 1.0, chars.bot_right, fg, bg, m);
        }
        let c = self.clip();
        // Visible index range of `v + i`: cells whose box can touch [lo_px, hi_px).
        let range = |lo_px: i64, hi_px: i64, cell: f64, origin: f32, n_max: i64| {
            let lo = ((lo_px as f64 / cell).floor() - 1.0 - f64::from(origin)).ceil() as i64;
            let hi = ((hi_px as f64 / cell).ceil() + 1.0 - f64::from(origin)).floor() as i64;
            (lo.max(1), hi.min(n_max))
        };
        // `i` runs 1.. while i < w - 1 (f32 arithmetic as before), i.e. up to ceil(w - 1) - 1.
        let n_cols = (w - 1.0).ceil() as i64 - 1;
        let n_rows = (h - 1.0).ceil() as i64 - 1;
        let (i0, i1) = range(c.x0, c.x1, self.cw, x, n_cols);
        for i in i0..=i1 {
            let cx = x + i as f32;
            self.glyph_cell(cx, y, 1.0, chars.top, fg, bg, m);
            if h > 1.0 {
                self.glyph_cell(cx, y + h - 1.0, 1.0, chars.bot, fg, bg, m);
            }
        }
        let (j0, j1) = range(c.y0, c.y1, self.ch, y, n_rows);
        for j in j0..=j1 {
            let cy = y + j as f32;
            self.glyph_cell(x, cy, 1.0, chars.left, fg, bg, m);
            if w > 1.0 {
                self.glyph_cell(x + w - 1.0, cy, 1.0, chars.right, fg, bg, m);
            }
        }
    }

    // ---- text -------------------------------------------------------------------------------

    /// One cell-wise glyph: bg box (`advance` cells), the char in the mono face, underline.
    #[allow(clippy::too_many_arguments)]
    fn glyph_cell(&mut self, cx: f32, cy: f32, advance: f32, ch: char, fg: &AnsiColor, bg: &AnsiColor, mods: Modifiers) {
        if !fin(&[cx, cy, advance]) {
            return;
        }
        self.push_fill(RectF::new(cx, cy, advance, 1.0), color::bg(bg));
        if ch == ' ' || ch == '\0' {
            return;
        }
        let (x0, y0, x1, _) = self.cell_edges(cx, cy, advance, 1.0);
        let (x, y, w) = (x0, y0, x1 - x0);
        let fgc = color::fg(fg);
        let base_y = y + f64::from(self.cache.mono_baseline_px()) * self.scale;
        let px = CELL_FONT_PX * self.scale as f32;
        self.text_at(FACE_MONO, ch, None, px, x, base_y, fgc, mods);
        if mods.underline {
            let t = (2.0 * self.scale).round().max(1.0);
            self.fill_frect(x, y + self.ch - t, w, t, fgc);
        }
    }

    /// Paints `ch` (or the multi-char `cluster`) with its pen origin at (`x`, `base_y`).
    #[allow(clippy::too_many_arguments)]
    fn text_at(&mut self, face: u8, ch: char, cluster: Option<&str>, px: f32, x: f64, base_y: f64, c: Rgb, mods: Modifiers) {
        match cluster {
            Some(t) if t.chars().nth(crate::glyphs::MAX_CLUSTER_CHARS).is_none() => {
                let gl = self.cache.cluster_glyphs(face, t, px);
                for g in gl.iter() {
                    self.draw_glyph(g.face, g.gid, px, x + f64::from(g.dx), base_y, c, mods);
                }
            }
            // no cluster, or one too long to re-shape: the glyph's own char
            _ => {
                if let Some((f, gid)) = self.cache.char_glyph(face, ch) {
                    self.draw_glyph(f, gid, px, x, base_y, c, mods);
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_glyph(&mut self, face: u8, gid: u16, px: f32, x: f64, base_y: f64, c: Rgb, mods: Modifiers) {
        if !x.is_finite() || !base_y.is_finite() {
            return;
        }
        let (x, base_y) = (lim(x), lim(base_y));
        let steps = f64::from(SUBPIXEL_STEPS);
        let mut ix = x.floor() as i64;
        let mut sub = ((x - x.floor()) * steps).round() as u8;
        if sub >= SUBPIXEL_STEPS {
            ix += 1;
            sub = 0;
        }
        let by = base_y.round() as i64;
        let Some(m) = self.cache.mask(face, gid, px, mods.bold, mods.italic, sub) else { return };
        let dx = ix + i64::from(m.left);
        let dy = by - i64::from(m.top);
        let clip = self.clip();
        let r = IRect { x0: dx, y0: dy, x1: dx + i64::from(m.width), y1: dy + i64::from(m.height) }.intersect(clip);
        if r.x1 <= r.x0 || r.y1 <= r.y0 {
            return;
        }
        let pw = self.pm.width() as usize;
        let data = self.pm.data_mut();
        for y in r.y0..r.y1 {
            let mrow = ((y - dy) as usize) * m.width as usize;
            for x in r.x0..r.x1 {
                let a = u32::from(m.data[mrow + (x - dx) as usize]);
                if a == 0 {
                    continue;
                }
                let i = (y as usize * pw + x as usize) * 4;
                for k in 0..3 {
                    let d = u32::from(data[i + k]);
                    data[i + k] = ((u32::from(c[k]) * a + d * (255 - a) + 127) / 255) as u8;
                }
            }
        }
    }

    fn shaped(&mut self, origin: PointF, glyphs: &[Glyph], run: &ShapedRun, fg: &AnsiColor, bg: &AnsiColor, mods: Modifiers) {
        let px = run.size_px * self.scale as f32;
        // A run with an unusable size or line geometry is skipped whole, background included.
        if !crate::glyphs::size_ok(px) || !fin(&[run.line_height, run.baseline]) {
            return;
        }
        let face_of = |i: usize| run.face_of(i).min(oxiterm_renderer::render::display_list::FACE_SANS);
        let fgc = color::fg(fg);
        let mut i = 0;
        while i < glyphs.len() {
            // A line is the consecutive glyphs sharing a `y`; it always holds at least glyph `i`.
            let mut j = i + 1;
            while j < glyphs.len() && glyphs[j].y == glyphs[i].y {
                j += 1;
            }
            let line = &glyphs[i..j];
            let top = origin.y + line[0].y;
            let left = origin.x + line[0].x;
            let right = line
                .iter()
                .filter(|g| fin(&[g.x, g.advance]))
                .map(|g| origin.x + g.x + g.advance)
                .fold(left, f32::max);
            if !fin(&[top, left, right]) {
                i = j;
                continue;
            }
            if *bg != AnsiColor::Reset {
                self.push_fill(RectF::new(left, top, right - left, run.line_height), color::bg(bg));
            }
            let base_y = f64::from(top + run.baseline) * self.ch;
            for (k, g) in line.iter().enumerate() {
                if !fin(&[g.x, g.advance]) {
                    continue;
                }
                let cluster = run.cluster_of(i + k);
                let text_blank = match cluster {
                    Some(t) => t.chars().all(char::is_whitespace),
                    None => g.ch.is_whitespace(),
                };
                if text_blank {
                    continue;
                }
                let x = f64::from(origin.x + g.x) * self.cw;
                self.text_at(face_of(i + k), g.ch, cluster, px, x, base_y, fgc, mods);
            }
            if mods.underline {
                let pxf = f64::from(run.size_px) * self.scale;
                let th = (pxf * 0.06).max(1.0);
                self.fill_frect(
                    f64::from(left) * self.cw,
                    base_y + pxf * 0.12,
                    f64::from(right - left) * self.cw,
                    th,
                    fgc,
                );
            }
            i = j;
        }
    }

    fn grid(&mut self, rect: RectF, cells: &[GridCell]) {
        let w = rect.w.max(0.0) as usize;
        if w == 0 {
            return;
        }
        let max = w.saturating_mul(rect.h.max(0.0) as usize);
        for (i, c) in cells.iter().take(max).enumerate() {
            if c.skip {
                continue;
            }
            let adv = UnicodeWidthCache::get().width(c.ch);
            let adv = if adv == 0 { 1 } else { adv } as f32;
            self.glyph_cell(rect.x + (i % w) as f32, rect.y + (i / w) as f32, adv, c.ch, &c.fg, &c.bg, c.mods);
        }
    }
}
