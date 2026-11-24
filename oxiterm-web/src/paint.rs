//! Display-list painter planning (OxiUI phase 3a).
//!
//! [`plan`] turns a decoded [`DisplayList`] into a flat sequence of abstract [`PaintOp`]s in
//! CSS pixels. It is pure (no canvas, no browser), so it is unit-tested on the host; the
//! `WebTerminal::draw_display_list` executor only maps ops 1:1 to Canvas2D calls.

use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, GridCell, Modifiers, PathCmd, PointF, RectF, ShapedRun, FACE_MONO,
};
use oxiterm_renderer::font::CELL_W_PX;
use oxiterm_renderer::render::unicode::UnicodeWidthCache;

use crate::get_color_str;

/// Pixel metrics used to map cell units to CSS pixels.
#[derive(Debug, Clone)]
pub struct Metrics {
    /// Cell width in CSS pixels.
    pub cw: f64,
    /// Cell height in CSS pixels.
    pub ch: f64,
    /// Device pixel ratio (the canvas has a `dpr` scale transform applied).
    pub dpr: f64,
    /// Base CSS font, e.g. `16px monospace`.
    pub base_font: String,
}

/// One path segment of a [`PaintOp::Stroke`].
#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    MoveTo(f64, f64),
    LineTo(f64, f64),
    /// `arc_to(x1, y1, x2, y2, radius)`
    ArcTo(f64, f64, f64, f64, f64),
    /// `quadratic_curve_to(cx, cy, x, y)`
    QuadTo(f64, f64, f64, f64),
    /// `bezier_curve_to(c1x, c1y, c2x, c2y, x, y)`
    CubicTo(f64, f64, f64, f64, f64, f64),
    Close,
}

/// Abstract paint operation (CSS pixels).
#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    Fill { x: f64, y: f64, w: f64, h: f64, color: String },
    Stroke { color: String, width: f64, path: Vec<Seg> },
    /// Fills a (possibly curved) path with the non-zero rule.
    FillPath { color: String, path: Vec<Seg> },
    /// Intersects the clip with a path (must be inside a `Save`/`Restore` pair).
    ClipPath { path: Vec<Seg> },
    /// Glyph drawn with baseline `top` at (x, y).
    Text { ch: char, x: f64, y: f64, font: String, color: String },
    /// Proportional glyph (cluster) drawn with the `alphabetic` baseline at (x, y): `y` is the
    /// baseline, not the line top. The executor restores `top` afterwards.
    Run { text: String, x: f64, y: f64, font: String, color: String },
    Save,
    ClipRect { x: f64, y: f64, w: f64, h: f64 },
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BorderStyle {
    Single,
    Rounded,
    Double,
    /// Unrecognised set (ASCII, custom): drawn as glyphs like the cell renderer.
    Glyphs,
}

fn border_style(c: &BorderChars, w: f32, h: f32) -> BorderStyle {
    // Vector lines need a real rectangle; 1-cell-wide frames fall back to glyphs.
    if w < 2.0 || h < 2.0 {
        return BorderStyle::Glyphs;
    }
    let sides = [c.top, c.bot, c.left, c.right];
    if sides.contains(&' ') {
        return BorderStyle::Glyphs;
    }
    match c.top_left {
        '╭' => BorderStyle::Rounded,
        '╔' => BorderStyle::Double,
        '┌' => BorderStyle::Single,
        _ => BorderStyle::Glyphs,
    }
}

fn font_for(m: &Metrics, mods: Modifiers) -> String {
    format!(
        "{}{}{}",
        if mods.bold { "bold " } else { "" },
        if mods.italic { "italic " } else { "" },
        m.base_font
    )
}

impl Metrics {
    /// `true` when the server's bundled faces (`oxi-mono`, `oxi-sans`) are what the page paints
    /// with; `false` after the browser-monospace fallback of stage 6.1.
    fn bundled_fonts(&self) -> bool {
        self.base_font.contains("oxi-mono")
    }

    /// On-screen scale of the reference cell: `cw / 9.6`. The client rounds the cell to device
    /// pixels (10 px at DPR 1, 9.5 at DPR 2), and proportional glyphs must scale with the very
    /// same factor as their positions, or spacing drifts.
    fn scale(&self) -> f64 {
        self.cw / f64::from(CELL_W_PX)
    }
}

/// CSS font of a shaped glyph: `[bold ][italic ]<size * scale>px "<face>", <generic>`. Without
/// the bundled faces only the generic family is used.
fn run_font(m: &Metrics, mods: Modifiers, face: u8, size_px: f32) -> String {
    let px = (f64::from(size_px) * m.scale() * 100.0).round() / 100.0;
    let family = match (m.bundled_fonts(), face == FACE_MONO) {
        (true, true) => "\"oxi-mono\", monospace",
        (true, false) => "\"oxi-sans\", sans-serif",
        (false, true) => "monospace",
        (false, false) => "sans-serif",
    };
    format!("{}{}{px}px {family}", if mods.bold { "bold " } else { "" }, if mods.italic { "italic " } else { "" })
}

/// Plans one shaped text run (inside the item's clip): a background box per line when the
/// background is not the default, each glyph or cluster with its own `fillText` at its exact
/// position (alphabetic baseline = line top + `run.baseline`), and an underline per line.
#[allow(clippy::too_many_arguments)]
fn plan_shaped(
    ops: &mut Vec<PaintOp>,
    m: &Metrics,
    origin: PointF,
    glyphs: &[oxiterm_renderer::render::display_list::Glyph],
    run: &ShapedRun,
    fg: &AnsiColor,
    bg: &AnsiColor,
    mods: Modifiers,
) {
    let color = get_color_str(fg, true);
    // Lines are consecutive glyphs sharing a `y`.
    let mut i = 0;
    while i < glyphs.len() {
        let mut j = i;
        while j < glyphs.len() && glyphs[j].y == glyphs[i].y {
            j += 1;
        }
        let line = &glyphs[i..j];
        let top = origin.y + line[0].y;
        let left = origin.x + line[0].x;
        let right = line.iter().map(|g| origin.x + g.x + g.advance).fold(left, f32::max);
        if *bg != AnsiColor::Reset {
            push_fill(ops, m, RectF::new(left, top, right - left, run.line_height), bg);
        }
        let base_y = (top + run.baseline) as f64 * m.ch;
        for (k, g) in line.iter().enumerate() {
            let text = match run.cluster_of(i + k) {
                Some(t) => t.to_string(),
                None => g.ch.to_string(),
            };
            if text.chars().all(char::is_whitespace) {
                continue;
            }
            ops.push(PaintOp::Run {
                text,
                x: (origin.x + g.x) as f64 * m.cw,
                y: base_y,
                font: run_font(m, mods, run.face_of(i + k), run.size_px),
                color: color.clone(),
            });
        }
        if mods.underline {
            let px = f64::from(run.size_px) * m.scale();
            let th = (px * 0.06).max(1.0);
            ops.push(PaintOp::Fill {
                x: left as f64 * m.cw,
                y: base_y + px * 0.12,
                w: (right - left) as f64 * m.cw,
                h: th,
                color: color.clone(),
            });
        }
        i = j;
    }
}

fn cell_rect(m: &Metrics, x: f32, y: f32, w: f32, h: f32) -> (f64, f64, f64, f64) {
    (x as f64 * m.cw, y as f64 * m.ch, w as f64 * m.cw, h as f64 * m.ch)
}

fn push_fill(ops: &mut Vec<PaintOp>, m: &Metrics, r: RectF, color: &AnsiColor) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = cell_rect(m, r.x, r.y, r.w, r.h);
    // Fractional (GUI) rects: round each edge to a device pixel (never the size itself) so
    // adjacent fills share edges without seams or blurry half-covered pixels.
    let sx = |v: f64| (v * m.dpr).round() / m.dpr;
    let (x0, y0, x1, y1) = (sx(x), sx(y), sx(x + w), sx(y + h));
    ops.push(PaintOp::Fill { x: x0, y: y0, w: x1 - x0, h: y1 - y0, color: get_color_str(color, false) });
}

/// Pixel rect of a cell-unit rect plus its corner radius (cells along x -> px), clamped to
/// half of the smaller side so the arcs always fit.
fn rounded_px(m: &Metrics, r: RectF, radius: f32) -> (f64, f64, f64, f64, f64) {
    let (x, y, w, h) = cell_rect(m, r.x, r.y, r.w, r.h);
    let rad = (radius as f64 * m.cw).max(0.0).min(w / 2.0).min(h / 2.0);
    (x, y, w, h, rad)
}

fn push_rounded_fill(ops: &mut Vec<PaintOp>, m: &Metrics, r: RectF, radius: f32, color: &AnsiColor) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let (x, y, w, h, rad) = rounded_px(m, r, radius);
    ops.push(PaintOp::FillPath {
        color: get_color_str(color, false),
        path: rect_path(x, y, x + w, y + h, rad),
    });
}

fn pt(m: &Metrics, p: &PointF) -> (f64, f64) {
    (p.x as f64 * m.cw, p.y as f64 * m.ch)
}

fn path_segs(m: &Metrics, cmds: &[PathCmd]) -> Vec<Seg> {
    cmds.iter()
        .map(|c| match c {
            PathCmd::MoveTo(p) => {
                let (x, y) = pt(m, p);
                Seg::MoveTo(x, y)
            }
            PathCmd::LineTo(p) => {
                let (x, y) = pt(m, p);
                Seg::LineTo(x, y)
            }
            PathCmd::QuadTo(c, p) => {
                let (cx, cy) = pt(m, c);
                let (x, y) = pt(m, p);
                Seg::QuadTo(cx, cy, x, y)
            }
            PathCmd::CubicTo(c1, c2, p) => {
                let (c1x, c1y) = pt(m, c1);
                let (c2x, c2y) = pt(m, c2);
                let (x, y) = pt(m, p);
                Seg::CubicTo(c1x, c1y, c2x, c2y, x, y)
            }
            PathCmd::Close => Seg::Close,
        })
        .collect()
}

fn plan_path(
    ops: &mut Vec<PaintOp>,
    m: &Metrics,
    cmds: &[PathCmd],
    fill: &Option<AnsiColor>,
    stroke: &Option<(AnsiColor, f32)>,
) {
    if cmds.is_empty() {
        return;
    }
    let path = path_segs(m, cmds);
    if let Some(c) = fill {
        // Background colour mode (Reset = page bg), like every other filled area.
        ops.push(PaintOp::FillPath { color: get_color_str(c, false), path: path.clone() });
    }
    if let Some((c, w)) = stroke {
        // Width is in cells along x. lineWidth is CSS px (the canvas carries the DPR scale).
        let width = *w as f64 * m.cw;
        if width > 0.0 {
            // Foreground colour mode (Reset = default text colour) for lines.
            ops.push(PaintOp::Stroke { color: get_color_str(c, true), width, path });
        }
    }
}

/// Paints one glyph cell: bg box (`advance` cells wide), glyph, optional underline.
#[allow(clippy::too_many_arguments)]
fn push_glyph_cell(
    ops: &mut Vec<PaintOp>,
    m: &Metrics,
    cx: f32,
    cy: f32,
    advance: f32,
    ch: char,
    fg: &AnsiColor,
    bg: &AnsiColor,
    mods: Modifiers,
) {
    push_fill(ops, m, RectF::new(cx, cy, advance, 1.0), bg);
    if ch == ' ' || ch == '\0' {
        return;
    }
    let (x, y, w, _) = cell_rect(m, cx, cy, advance, 1.0);
    let color = get_color_str(fg, true);
    ops.push(PaintOp::Text { ch, x, y, font: font_for(m, mods), color: color.clone() });
    if mods.underline {
        ops.push(PaintOp::Fill { x, y: y + m.ch - 2.0, w, h: 2.0, color });
    }
}

fn snap(v: f64, dpr: f64, lw_dev: f64) -> f64 {
    let off = if (lw_dev as i64) % 2 == 1 { 0.5 } else { 0.0 };
    ((v * dpr).round() + off) / dpr
}

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> Vec<Seg> {
    if r <= 0.0 {
        return vec![
            Seg::MoveTo(x0, y0),
            Seg::LineTo(x1, y0),
            Seg::LineTo(x1, y1),
            Seg::LineTo(x0, y1),
            Seg::Close,
        ];
    }
    vec![
        Seg::MoveTo(x0 + r, y0),
        Seg::LineTo(x1 - r, y0),
        Seg::ArcTo(x1, y0, x1, y0 + r, r),
        Seg::LineTo(x1, y1 - r),
        Seg::ArcTo(x1, y1, x1 - r, y1, r),
        Seg::LineTo(x0 + r, y1),
        Seg::ArcTo(x0, y1, x0, y1 - r, r),
        Seg::LineTo(x0, y0 + r),
        Seg::ArcTo(x0, y0, x0 + r, y0, r),
        Seg::Close,
    ]
}

fn plan_border(
    ops: &mut Vec<PaintOp>,
    m: &Metrics,
    rect: RectF,
    radius: f32,
    chars: &BorderChars,
    fg: &AnsiColor,
    bg: &AnsiColor,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
    // Rounded frames (radius > 0, big enough and not a blank-sided set) are drawn as one
    // vector stroke; otherwise the radius-0 logic below applies unchanged.
    let vector_round = radius > 0.0
        && w >= 2.0
        && h >= 2.0
        && ![chars.top, chars.bot, chars.left, chars.right].contains(&' ');
    // Background under the frame ring only, so interior content is never covered. For a
    // rounded frame the ring is clipped to the rounded outer silhouette (the same shape a
    // rounded Fill uses), so no square corners are painted outside the stroke.
    if vector_round {
        let (px, py, pw, ph, rad) = rounded_px(m, rect, radius);
        ops.push(PaintOp::Save);
        ops.push(PaintOp::ClipPath { path: rect_path(px, py, px + pw, py + ph, rad) });
    }
    push_fill(ops, m, RectF::new(x, y, w, 1.0), bg);
    if h > 1.0 {
        push_fill(ops, m, RectF::new(x, y + h - 1.0, w, 1.0), bg);
    }
    if h > 2.0 {
        push_fill(ops, m, RectF::new(x, y + 1.0, 1.0, h - 2.0), bg);
        if w > 1.0 {
            push_fill(ops, m, RectF::new(x + w - 1.0, y + 1.0, 1.0, h - 2.0), bg);
        }
    }

    if vector_round {
        ops.push(PaintOp::Restore);
    }
    let style = if vector_round { BorderStyle::Rounded } else { border_style(chars, w, h) };
    if style == BorderStyle::Glyphs {
        // Same cell order as the cell sink; each frame cell drawn once.
        let g = |ops: &mut Vec<PaintOp>, cx: f32, cy: f32, ch: char| {
            push_glyph_cell(ops, m, cx, cy, 1.0, ch, fg, bg, Modifiers::default());
        };
        g(ops, x, y, chars.top_left);
        if w > 1.0 {
            g(ops, x + w - 1.0, y, chars.top_right);
        }
        if h > 1.0 {
            g(ops, x, y + h - 1.0, chars.bot_left);
        }
        if w > 1.0 && h > 1.0 {
            g(ops, x + w - 1.0, y + h - 1.0, chars.bot_right);
        }
        let mut i = 1.0;
        while i < w - 1.0 {
            g(ops, x + i, y, chars.top);
            if h > 1.0 {
                g(ops, x + i, y + h - 1.0, chars.bot);
            }
            i += 1.0;
        }
        let mut j = 1.0;
        while j < h - 1.0 {
            g(ops, x, y + j, chars.left);
            if w > 1.0 {
                g(ops, x + w - 1.0, y + j, chars.right);
            }
            j += 1.0;
        }
        return;
    }

    let lw_dev = m.dpr.round().max(1.0);
    let width = lw_dev / m.dpr;
    let color = get_color_str(fg, true);
    // Through the centres of the border cells, where the glyph strokes used to be.
    let cx0 = (x as f64 + 0.5) * m.cw;
    let cx1 = (x as f64 + w as f64 - 0.5) * m.cw;
    let cy0 = (y as f64 + 0.5) * m.ch;
    let cy1 = (y as f64 + h as f64 - 0.5) * m.ch;
    let mut stroke = |x0: f64, y0: f64, x1: f64, y1: f64, r: f64| {
        let path = rect_path(
            snap(x0, m.dpr, lw_dev),
            snap(y0, m.dpr, lw_dev),
            snap(x1, m.dpr, lw_dev),
            snap(y1, m.dpr, lw_dev),
            r,
        );
        ops.push(PaintOp::Stroke { color: color.clone(), width, path });
    };
    match style {
        BorderStyle::Single => stroke(cx0, cy0, cx1, cy1, 0.0),
        BorderStyle::Rounded => {
            let want = if vector_round { radius as f64 * m.cw } else { m.cw.min(m.ch) / 2.0 };
            let r = want.min((cx1 - cx0) / 2.0).min((cy1 - cy0) / 2.0);
            stroke(cx0, cy0, cx1, cy1, r);
        }
        BorderStyle::Double => {
            let d = m.cw.min(m.ch) / 6.0;
            stroke(cx0 - d, cy0 - d, cx1 + d, cy1 + d, 0.0);
            stroke(cx0 + d, cy0 + d, cx1 - d, cy1 - d, 0.0);
        }
        BorderStyle::Glyphs => unreachable!(),
    }
}

fn plan_grid(ops: &mut Vec<PaintOp>, m: &Metrics, rect: RectF, cells: &[GridCell]) {
    let w = rect.w.max(0.0) as usize;
    if w == 0 {
        return;
    }
    let max = w * rect.h.max(0.0) as usize;
    for (i, c) in cells.iter().take(max).enumerate() {
        if c.skip {
            continue;
        }
        let col = (i % w) as f32;
        let row = (i / w) as f32;
        let adv = UnicodeWidthCache::get().width(c.ch);
        let adv = if adv == 0 { 1 } else { adv } as f32;
        push_glyph_cell(ops, m, rect.x + col, rect.y + row, adv, c.ch, &c.fg, &c.bg, c.mods);
    }
}

/// Maps a display list to paint operations. Balanced: every `Save` has a `Restore`.
pub fn plan(list: &DisplayList, m: &Metrics) -> Vec<PaintOp> {
    let mut ops = Vec::new();
    let mut clip_depth = 0usize;
    for item in &list.items {
        match item {
            DisplayItem::Fill { rect, radius, bg, .. } => {
                if *radius > 0.0 {
                    push_rounded_fill(&mut ops, m, *rect, *radius, bg)
                } else {
                    push_fill(&mut ops, m, *rect, bg)
                }
            }
            DisplayItem::Border { rect, radius, chars, fg, bg, .. } => {
                plan_border(&mut ops, m, *rect, *radius, chars, fg, bg)
            }
            DisplayItem::Text { origin, clip, glyphs, fg, bg, mods, shaped, .. } => {
                let (cx, cy, cw, ch) = cell_rect(m, clip.x, clip.y, clip.w, clip.h);
                ops.push(PaintOp::Save);
                ops.push(PaintOp::ClipRect { x: cx, y: cy, w: cw, h: ch });
                match shaped {
                    Some(run) => plan_shaped(&mut ops, m, *origin, glyphs, run, fg, bg, *mods),
                    None => {
                        for g in glyphs {
                            push_glyph_cell(
                                &mut ops, m, origin.x + g.x, origin.y + g.y, g.advance, g.ch, fg, bg, *mods,
                            );
                        }
                    }
                }
                ops.push(PaintOp::Restore);
            }
            // Never sent by the server (replaced by CellGrid); nothing to paint.
            DisplayItem::Legacy { .. } => {}
            DisplayItem::CellGrid { rect, cells } => plan_grid(&mut ops, m, *rect, cells),
            DisplayItem::PushClip { rect } => {
                let (x, y, w, h) = cell_rect(m, rect.x, rect.y, rect.w, rect.h);
                ops.push(PaintOp::Save);
                ops.push(PaintOp::ClipRect { x, y, w, h });
                clip_depth += 1;
            }
            DisplayItem::Path { cmds, fill, stroke, .. } => {
                plan_path(&mut ops, m, cmds, fill, stroke)
            }
            DisplayItem::PopClip => {
                if clip_depth > 0 {
                    clip_depth -= 1;
                    ops.push(PaintOp::Restore);
                }
            }
        }
    }
    for _ in 0..clip_depth {
        ops.push(PaintOp::Restore);
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_proto::dom::NodeId;
    use oxiterm_renderer::render::display_list::{Glyph, PointF};
    use oxiterm_renderer::render::wire;

    fn m() -> Metrics {
        Metrics { cw: 10.0, ch: 20.0, dpr: 1.0, base_font: "16px monospace".into() }
    }

    fn border(chars: BorderChars, w: f32, h: f32) -> DisplayList {
        DisplayList {
            items: vec![DisplayItem::Border {
                node: NodeId(1),
                rect: RectF::new(2.0, 1.0, w, h),
                radius: 0.0,
                chars,
                fg: AnsiColor::TrueColor(1, 2, 3),
                bg: AnsiColor::Reset,
            }],
        }
    }

    fn strokes(ops: &[PaintOp]) -> Vec<&Vec<Seg>> {
        ops.iter().filter_map(|o| if let PaintOp::Stroke { path, .. } = o { Some(path) } else { None }).collect()
    }

    #[test]
    fn roundtrip_and_fill() {
        let list = DisplayList {
            items: vec![DisplayItem::Fill {
                node: NodeId(0),
                rect: RectF::new(0.0, 0.0, 4.0, 2.0),
                radius: 0.0,
                fg: AnsiColor::Reset,
                bg: AnsiColor::Reset,
            }],
        };
        let decoded = wire::decode(&wire::encode(&list)).unwrap();
        let ops = plan(&decoded, &m());
        assert_eq!(
            ops,
            vec![PaintOp::Fill { x: 0.0, y: 0.0, w: 40.0, h: 40.0, color: "#0f172a".into() }]
        );
    }

    #[test]
    fn single_border_is_one_snapped_rect_through_cell_centres() {
        let ops = plan(&border(BorderChars::single(), 5.0, 3.0), &m());
        let s = strokes(&ops);
        assert_eq!(s.len(), 1);
        // x: (2+0.5)*10=25 -> 25.5 ; right (2+5-0.5)*10=65 -> 65.5 ; y: 30 -> 30.5, (1+3-0.5)*20=70 -> 70.5
        assert_eq!(s[0][0], Seg::MoveTo(25.5, 30.5));
        assert_eq!(s[0][2], Seg::LineTo(65.5, 70.5));
        assert!(!ops.iter().any(|o| matches!(o, PaintOp::Text { .. })));
    }

    #[test]
    fn rounded_border_uses_arcs() {
        let ops = plan(&border(BorderChars::rounded(), 5.0, 4.0), &m());
        let s = strokes(&ops);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].iter().filter(|g| matches!(g, Seg::ArcTo(..))).count(), 4);
    }

    #[test]
    fn double_border_has_two_parallel_paths() {
        let ops = plan(&border(BorderChars::double(), 6.0, 4.0), &m());
        assert_eq!(strokes(&ops).len(), 2);
    }

    #[test]
    fn ascii_and_one_cell_borders_fall_back_to_glyphs() {
        let ascii = BorderChars {
            top_left: '+', top: '-', top_right: '+', left: '|', right: '|',
            bot_left: '+', bot: '-', bot_right: '+',
        };
        let ops = plan(&border(ascii, 4.0, 3.0), &m());
        assert!(strokes(&ops).is_empty());
        // 4 corners + 2*2 horizontal + 2*1 vertical = 10 frame cells.
        assert_eq!(ops.iter().filter(|o| matches!(o, PaintOp::Text { .. })).count(), 10);
        let one = plan(&border(BorderChars::single(), 1.0, 3.0), &m());
        assert!(strokes(&one).is_empty());
        assert!(one.iter().any(|o| matches!(o, PaintOp::Text { .. })));
    }

    #[test]
    fn border_bg_covers_ring_only() {
        let ops = plan(&border(BorderChars::single(), 5.0, 4.0), &m());
        let area: f64 = ops
            .iter()
            .filter_map(|o| if let PaintOp::Fill { w, h, .. } = o { Some(w * h) } else { None })
            .sum();
        // Ring = 5*4 - 3*2 = 14 cells of 10x20.
        assert_eq!(area, 14.0 * 200.0);
    }

    #[test]
    fn hidpi_line_is_device_pixel_aligned() {
        let mut mm = m();
        mm.dpr = 2.0;
        let ops = plan(&border(BorderChars::single(), 5.0, 3.0), &mm);
        if let PaintOp::Stroke { width, path, .. } = ops.iter().find(|o| matches!(o, PaintOp::Stroke { .. })).unwrap() {
            assert_eq!(*width, 1.0);
            assert_eq!(path[0], Seg::MoveTo(25.0, 30.0));
        }
    }

    #[test]
    fn text_is_clipped_and_painted_per_glyph() {
        let list = DisplayList {
            items: vec![DisplayItem::Text {
                node: NodeId(1),
                origin: PointF { x: 1.0, y: 2.0 },
                clip: RectF::new(1.0, 2.0, 3.0, 1.0),
                glyphs: vec![
                    Glyph { x: 0.0, y: 0.0, ch: 'a', advance: 1.0 },
                    Glyph { x: 1.0, y: 0.0, ch: '漢', advance: 2.0 },
                ],
                fg: AnsiColor::Reset,
                bg: AnsiColor::Reset,
                mods: Modifiers { bold: true, underline: true, italic: false },
                shaped: None,
            }],
        };
        let ops = plan(&list, &m());
        assert_eq!(ops[0], PaintOp::Save);
        assert_eq!(ops[1], PaintOp::ClipRect { x: 10.0, y: 40.0, w: 30.0, h: 20.0 });
        assert_eq!(*ops.last().unwrap(), PaintOp::Restore);
        assert!(ops.contains(&PaintOp::Fill { x: 20.0, y: 40.0, w: 20.0, h: 20.0, color: "#0f172a".into() }));
        assert!(ops.iter().any(|o| matches!(o, PaintOp::Text { ch: '漢', font, .. } if font == "bold 16px monospace")));
        // One underline per drawn glyph.
        assert_eq!(ops.iter().filter(|o| matches!(o, PaintOp::Fill { h, .. } if *h == 2.0)).count(), 2);
    }

    fn bundled(cw: f64) -> Metrics {
        Metrics { cw, ch: 20.0, dpr: 1.0, base_font: "16px \"oxi-mono\"".into() }
    }

    fn shaped_list(bg: AnsiColor, mods: Modifiers) -> DisplayList {
        DisplayList {
            items: vec![DisplayItem::Text {
                node: NodeId(1),
                origin: PointF { x: 2.0, y: 3.0 },
                clip: RectF::new(2.0, 3.0, 30.0, 3.0),
                glyphs: vec![
                    Glyph { x: 0.0, y: 0.0, ch: 'o', advance: 1.5 },
                    Glyph { x: 1.5, y: 0.0, ch: 'f', advance: 1.0 },
                    Glyph { x: 2.5, y: 0.0, ch: ' ', advance: 0.5 },
                    Glyph { x: 3.0, y: 0.0, ch: '\u{2500}', advance: 1.0 },
                    Glyph { x: 0.0, y: 1.5, ch: 'e', advance: 1.0 },
                ],
                fg: AnsiColor::TrueColor(9, 8, 7),
                bg,
                mods,
                shaped: Some(Box::new(ShapedRun {
                    face: 1,
                    size_px: 24.0,
                    line_height: 1.5,
                    baseline: 1.2,
                    glyph_faces: vec![(3, 0)],
                    clusters: vec![(1, "fi".into())],
                })),
            }],
        }
    }

    fn runs(ops: &[PaintOp]) -> Vec<(&str, f64, f64, &str)> {
        ops.iter()
            .filter_map(|o| match o {
                PaintOp::Run { text, x, y, font, .. } => Some((text.as_str(), *x, *y, font.as_str())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shaped_glyphs_are_drawn_at_their_exact_position_and_face() {
        // cell width 10 px = 9.6 * 1.0417: glyphs scale with the same factor as positions
        let k = 10.0 / 9.6;
        let ops = plan(&shaped_list(AnsiColor::Reset, Modifiers { bold: true, underline: false, italic: true }), &bundled(10.0));
        let size = ((24.0 * k * 100.0f64).round()) / 100.0;
        let sans = format!("bold italic {size}px \"oxi-sans\", sans-serif");
        let r = runs(&ops);
        // the space is not drawn; the cluster "fi" replaces its glyph's char; glyph 3 is a mono
        // fallback inside the sans run
        assert_eq!(r.len(), 4, "{r:?}");
        // baseline y = (origin.y + line top + baseline) * ch
        let near = |a: f64, b: f64| assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        assert_eq!(r[0].0, "o");
        near(r[0].1, 20.0);
        near(r[0].2, (3.0 + 1.2) * 20.0);
        assert_eq!(r[0].3, sans);
        assert_eq!(r[1].0, "fi");
        near(r[1].1, 35.0);
        assert_eq!(r[1].3, sans);
        assert_eq!(r[2].0, "\u{2500}");
        near(r[2].1, 50.0);
        assert_eq!(r[2].3, format!("bold italic {size}px \"oxi-mono\", monospace"));
        // second line: its own top
        assert_eq!(r[3].0, "e");
        near(r[3].2, (3.0 + 1.5 + 1.2) * 20.0);
        // alphabetic runs are inside the clip pair
        assert_eq!(ops[0], PaintOp::Save);
        assert_eq!(*ops.last().unwrap(), PaintOp::Restore);
    }

    #[test]
    fn shaped_font_scale_follows_the_device_rounded_cell() {
        // DPR 2: the client rounds 9.6 px to 9.5 px; 16 px text becomes 15.83 px
        let ops = plan(&shaped_list(AnsiColor::Reset, Modifiers::default()), &bundled(9.5));
        let f = &runs(&ops)[0].3.to_string();
        assert_eq!(f, &format!("{}px \"oxi-sans\", sans-serif", ((24.0 * 9.5 / 9.6) * 100.0f64).round() / 100.0));
        // at exactly 9.6 the size is untouched
        let ops = plan(&shaped_list(AnsiColor::Reset, Modifiers::default()), &bundled(9.6));
        assert_eq!(runs(&ops)[0].3, "24px \"oxi-sans\", sans-serif");
    }

    #[test]
    fn shaped_run_falls_back_to_generic_families_without_the_bundled_fonts() {
        let ops = plan(&shaped_list(AnsiColor::Reset, Modifiers::default()), &Metrics { cw: 9.6, ch: 20.0, dpr: 1.0, base_font: "16px monospace".into() });
        let r = runs(&ops);
        assert_eq!(r[0].3, "24px sans-serif");
        assert_eq!(r[2].3, "24px monospace");
    }

    #[test]
    fn shaped_background_is_one_box_per_line_and_only_when_not_default() {
        let fills = |ops: &[PaintOp]| ops.iter().filter(|o| matches!(o, PaintOp::Fill { .. })).count();
        let none = plan(&shaped_list(AnsiColor::Reset, Modifiers::default()), &m());
        assert_eq!(fills(&none), 0);
        let ops = plan(&shaped_list(AnsiColor::TrueColor(1, 2, 3), Modifiers::default()), &m());
        let boxes: Vec<_> = ops.iter().filter_map(|o| if let PaintOp::Fill { x, y, w, h, .. } = o { Some((*x, *y, *w, *h)) } else { None }).collect();
        // line 1 spans x 2.0 .. 2.0+4.0 cells, 1.5 cells high; line 2 starts 1.5 cells lower
        assert_eq!(boxes, vec![(20.0, 60.0, 40.0, 30.0), (20.0, 90.0, 10.0, 30.0)]);
    }

    #[test]
    fn shaped_underline_is_one_per_line() {
        let ops = plan(&shaped_list(AnsiColor::Reset, Modifiers { bold: false, underline: true, italic: false }), &m());
        let ul: Vec<_> = ops.iter().filter_map(|o| if let PaintOp::Fill { x, w, .. } = o { Some((*x, *w)) } else { None }).collect();
        assert_eq!(ul, vec![(20.0, 40.0), (20.0, 10.0)]);
    }

    #[test]
    fn shaped_list_survives_the_wire() {
        let list = shaped_list(AnsiColor::Reset, Modifiers::default());
        let decoded = wire::decode(&wire::encode_for(&list, 3)).unwrap();
        assert_eq!(plan(&decoded, &bundled(10.0)), plan(&list, &bundled(10.0)));
    }

    #[test]
    fn grid_skips_trailer_cells_and_clips_are_balanced() {
        let cell = |ch, skip| GridCell { ch, fg: AnsiColor::Reset, bg: AnsiColor::Reset, mods: Modifiers::default(), skip };
        let list = DisplayList {
            items: vec![
                DisplayItem::PopClip, // unbalanced, ignored
                DisplayItem::PushClip { rect: RectF::new(0.0, 0.0, 2.0, 1.0) },
                DisplayItem::CellGrid {
                    rect: RectF::new(0.0, 0.0, 2.0, 1.0),
                    cells: vec![cell('漢', false), cell(' ', true)],
                },
            ],
        };
        let ops = plan(&list, &m());
        assert_eq!(ops.iter().filter(|o| matches!(o, PaintOp::Text { .. })).count(), 1);
        assert_eq!(ops.iter().filter(|o| matches!(o, PaintOp::Fill { .. })).count(), 1);
        let saves = ops.iter().filter(|o| **o == PaintOp::Save).count();
        let restores = ops.iter().filter(|o| **o == PaintOp::Restore).count();
        assert_eq!(saves, restores);
        assert_eq!(saves, 1);
    }

    fn fill_item(rect: RectF, radius: f32) -> DisplayList {
        DisplayList {
            items: vec![DisplayItem::Fill {
                node: NodeId(0),
                rect,
                radius,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(9, 8, 7),
            }],
        }
    }

    fn arcs(p: &[Seg]) -> usize {
        p.iter().filter(|g| matches!(g, Seg::ArcTo(..))).count()
    }

    #[test]
    fn rounded_fill_is_a_filled_path_with_scaled_radius() {
        // radius 0.5 cells = 5px; rect 2x1 cells = 20x20 px.
        let ops = plan(&fill_item(RectF::new(1.0, 1.0, 2.0, 1.0), 0.5), &m());
        assert_eq!(ops.len(), 1);
        let PaintOp::FillPath { color, path } = &ops[0] else { panic!("{ops:?}") };
        assert_eq!(color, "rgb(9,8,7)");
        assert_eq!(path[0], Seg::MoveTo(15.0, 20.0));
        assert_eq!(arcs(path), 4);
        assert!(matches!(path[2], Seg::ArcTo(_, _, _, _, r) if r == 5.0));
    }

    #[test]
    fn square_fill_keeps_plain_fill_op() {
        let ops = plan(&fill_item(RectF::new(1.0, 1.0, 2.0, 1.0), 0.0), &m());
        assert_eq!(
            ops,
            vec![PaintOp::Fill { x: 10.0, y: 20.0, w: 20.0, h: 20.0, color: "rgb(9,8,7)".into() }]
        );
    }

    #[test]
    fn radius_is_clamped_to_half_the_smaller_side() {
        // 4x2 cells = 40x40 px at (0,0); radius 10 cells = 100px -> clamped to 20.
        let ops = plan(&fill_item(RectF::new(0.0, 0.0, 4.0, 2.0), 10.0), &m());
        let PaintOp::FillPath { path, .. } = &ops[0] else { panic!() };
        assert!(path.iter().all(|g| !matches!(g, Seg::ArcTo(_, _, _, _, r) if *r != 20.0)));
        assert_eq!(arcs(path), 4);
        // Wide rect: 8x1 cells = 80x20 px -> clamp is h/2 = 10.
        let ops = plan(&fill_item(RectF::new(0.0, 0.0, 8.0, 1.0), 10.0), &m());
        let PaintOp::FillPath { path, .. } = &ops[0] else { panic!() };
        assert_eq!(path[0], Seg::MoveTo(10.0, 0.0));
    }

    fn rborder(chars: BorderChars, w: f32, h: f32, radius: f32) -> DisplayList {
        let mut l = border(chars, w, h);
        if let DisplayItem::Border { radius: r, .. } = &mut l.items[0] {
            *r = radius;
        }
        l
    }

    #[test]
    fn rounded_border_strokes_with_radius_and_clips_ring_to_silhouette() {
        // Single set but radius 1 cell (10px): rounded vector frame, ring clipped.
        let ops = plan(&rborder(BorderChars::single(), 6.0, 4.0, 1.0), &m());
        assert_eq!(ops[0], PaintOp::Save);
        let PaintOp::ClipPath { path } = &ops[1] else { panic!("{ops:?}") };
        assert_eq!(arcs(path), 4);
        assert!(matches!(ops[2], PaintOp::Fill { .. }));
        let restore = ops.iter().position(|o| *o == PaintOp::Restore).unwrap();
        assert!(ops[2..restore].iter().all(|o| matches!(o, PaintOp::Fill { .. })));
        let s = strokes(&ops);
        assert_eq!(s.len(), 1);
        assert_eq!(arcs(s[0]), 4);
        assert!(matches!(s[0][2], Seg::ArcTo(_, _, _, _, r) if r == 10.0));
        // Same ring area as the square frame: 6*4 - 4*2 = 16 cells.
        let area: f64 = ops
            .iter()
            .filter_map(|o| if let PaintOp::Fill { w, h, .. } = o { Some(w * h) } else { None })
            .sum();
        assert_eq!(area, 16.0 * 200.0);
    }

    #[test]
    fn rounded_border_radius_clamps_and_small_frames_fall_back() {
        let ops = plan(&rborder(BorderChars::single(), 4.0, 2.0, 50.0), &m());
        let s = strokes(&ops);
        // Stroke rect is 30x20 px (centres) -> radius <= 10.
        assert!(matches!(s[0][2], Seg::ArcTo(_, _, _, _, r) if r == 10.0));
        let one = plan(&rborder(BorderChars::single(), 1.0, 3.0, 1.0), &m());
        assert!(strokes(&one).is_empty());
        assert!(!one.iter().any(|o| matches!(o, PaintOp::ClipPath { .. })));
    }

    #[test]
    fn zero_radius_border_is_unchanged() {
        let ops = plan(&rborder(BorderChars::single(), 5.0, 3.0, 0.0), &m());
        assert!(!ops.iter().any(|o| matches!(o, PaintOp::ClipPath { .. } | PaintOp::Save)));
    }

    fn p(x: f32, y: f32) -> PointF {
        PointF { x, y }
    }

    fn path_item(fill: Option<AnsiColor>, stroke: Option<(AnsiColor, f32)>) -> DisplayList {
        DisplayList {
            items: vec![DisplayItem::Path {
                node: NodeId(3),
                cmds: vec![
                    PathCmd::MoveTo(p(1.0, 1.0)),
                    PathCmd::LineTo(p(2.5, 1.0)),
                    PathCmd::QuadTo(p(3.0, 1.5), p(2.5, 2.0)),
                    PathCmd::CubicTo(p(2.0, 2.5), p(1.5, 2.5), p(1.0, 2.0)),
                    PathCmd::Close,
                ],
                fill,
                stroke,
            }],
        }
    }

    fn expected_segs() -> Vec<Seg> {
        vec![
            Seg::MoveTo(10.0, 20.0),
            Seg::LineTo(25.0, 20.0),
            Seg::QuadTo(30.0, 30.0, 25.0, 40.0),
            Seg::CubicTo(20.0, 50.0, 15.0, 50.0, 10.0, 40.0),
            Seg::Close,
        ]
    }

    #[test]
    fn path_fill_only() {
        let ops = plan(&path_item(Some(AnsiColor::TrueColor(1, 2, 3)), None), &m());
        assert_eq!(ops, vec![PaintOp::FillPath { color: "rgb(1,2,3)".into(), path: expected_segs() }]);
    }

    #[test]
    fn path_stroke_only_width_is_cells_times_cell_width() {
        let ops = plan(&path_item(None, Some((AnsiColor::TrueColor(4, 5, 6), 0.25))), &m());
        assert_eq!(
            ops,
            vec![PaintOp::Stroke { color: "rgb(4,5,6)".into(), width: 2.5, path: expected_segs() }]
        );
    }

    #[test]
    fn path_fill_then_stroke_and_empty_paths() {
        let both = path_item(
            Some(AnsiColor::TrueColor(1, 2, 3)),
            Some((AnsiColor::TrueColor(4, 5, 6), 1.0)),
        );
        let ops = plan(&both, &m());
        assert_eq!(ops.len(), 2);
        assert!(matches!(ops[0], PaintOp::FillPath { .. }));
        assert!(matches!(&ops[1], PaintOp::Stroke { width, .. } if *width == 10.0));
        // Neither fill nor stroke: nothing to paint.
        assert!(plan(&path_item(None, None), &m()).is_empty());
        let empty = DisplayList {
            items: vec![DisplayItem::Path {
                node: NodeId(1),
                cmds: vec![],
                fill: Some(AnsiColor::Reset),
                stroke: None,
            }],
        };
        assert!(plan(&empty, &m()).is_empty());
    }

    #[test]
    fn fractional_rect_maps_to_pixels() {
        let ops = plan(&fill_item(RectF::new(0.5, 0.25, 1.5, 1.5), 0.0), &m());
        assert_eq!(
            ops,
            vec![PaintOp::Fill { x: 5.0, y: 5.0, w: 15.0, h: 30.0, color: "rgb(9,8,7)".into() }]
        );
        // Adjacent fractional fills share an exact device-pixel edge (no seam).
        let mut mm = m();
        mm.dpr = 2.0;
        let a = plan(&fill_item(RectF::new(0.0, 0.0, 1.55, 1.0), 0.0), &mm);
        let b = plan(&fill_item(RectF::new(1.55, 0.0, 1.0, 1.0), 0.0), &mm);
        let (PaintOp::Fill { x: ax, w: aw, .. }, PaintOp::Fill { x: bx, .. }) = (&a[0], &b[0]) else {
            panic!()
        };
        assert_eq!(ax + aw, *bx);
    }
}
