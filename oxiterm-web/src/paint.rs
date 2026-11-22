//! Display-list painter planning (OxiUI phase 3a).
//!
//! [`plan`] turns a decoded [`DisplayList`] into a flat sequence of abstract [`PaintOp`]s in
//! CSS pixels. It is pure (no canvas, no browser), so it is unit-tested on the host; the
//! `WebTerminal::draw_display_list` executor only maps ops 1:1 to Canvas2D calls.

use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList, GridCell, Modifiers, RectF};
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
    Close,
}

/// Abstract paint operation (CSS pixels).
#[derive(Debug, Clone, PartialEq)]
pub enum PaintOp {
    Fill { x: f64, y: f64, w: f64, h: f64, color: String },
    Stroke { color: String, width: f64, path: Vec<Seg> },
    /// Glyph drawn with baseline `top` at (x, y).
    Text { ch: char, x: f64, y: f64, font: String, color: String },
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

fn cell_rect(m: &Metrics, x: f32, y: f32, w: f32, h: f32) -> (f64, f64, f64, f64) {
    (x as f64 * m.cw, y as f64 * m.ch, w as f64 * m.cw, h as f64 * m.ch)
}

fn push_fill(ops: &mut Vec<PaintOp>, m: &Metrics, r: RectF, color: &AnsiColor) {
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = cell_rect(m, r.x, r.y, r.w, r.h);
    ops.push(PaintOp::Fill { x, y, w, h, color: get_color_str(color, false) });
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
    chars: &BorderChars,
    fg: &AnsiColor,
    bg: &AnsiColor,
) {
    if rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = (rect.x, rect.y, rect.w, rect.h);
    // Background under the frame ring only, so interior content is never covered.
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

    let style = border_style(chars, w, h);
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
            let r = (m.cw.min(m.ch) / 2.0).min((cx1 - cx0) / 2.0).min((cy1 - cy0) / 2.0);
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
            DisplayItem::Fill { rect, bg, .. } => push_fill(&mut ops, m, *rect, bg),
            DisplayItem::Border { rect, chars, fg, bg, .. } => {
                plan_border(&mut ops, m, *rect, chars, fg, bg)
            }
            DisplayItem::Text { origin, clip, glyphs, fg, bg, mods, .. } => {
                let (cx, cy, cw, ch) = cell_rect(m, clip.x, clip.y, clip.w, clip.h);
                ops.push(PaintOp::Save);
                ops.push(PaintOp::ClipRect { x: cx, y: cy, w: cw, h: ch });
                for g in glyphs {
                    push_glyph_cell(
                        &mut ops, m, origin.x + g.x, origin.y + g.y, g.advance, g.ch, fg, bg, *mods,
                    );
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
}
