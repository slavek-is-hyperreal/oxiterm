//! Unit tests per primitive. Geometry at scale 1: cell = 9.6 x 20 px.

use oxiterm_fb::color::{self, DEFAULT_BG, DEFAULT_FG};
use oxiterm_fb::{Rasterizer, Surface};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PathCmd, PointF, RectF, ShapedRun, FACE_MONO, FACE_SANS,
};

const RED: AnsiColor = AnsiColor::TrueColor(255, 0, 0);
const GREEN: AnsiColor = AnsiColor::TrueColor(0, 255, 0);
const RED_RGB: [u8; 3] = [255, 0, 0];
const GREEN_RGB: [u8; 3] = [0, 255, 0];

fn paint(items: Vec<DisplayItem>, cols: u32, rows: u32) -> (Rasterizer, Surface) {
    let mut r = Rasterizer::new(1.0);
    let mut s = r.surface_for(cols, rows).unwrap();
    r.render(&DisplayList { items }, &mut s);
    (r, s)
}

fn fill(rect: RectF, radius: f32, bg: AnsiColor) -> DisplayItem {
    DisplayItem::Fill { node: NodeId(1), rect, radius, fg: AnsiColor::Reset, bg }
}

fn border(rect: RectF, radius: f32, chars: BorderChars, fg: AnsiColor) -> DisplayItem {
    DisplayItem::Border { node: NodeId(1), rect, radius, chars, fg, bg: AnsiColor::Reset }
}

fn text(origin: (f32, f32), clip: RectF, glyphs: Vec<Glyph>, mods: Modifiers, shaped: Option<ShapedRun>) -> DisplayItem {
    DisplayItem::Text {
        node: NodeId(1),
        origin: PointF { x: origin.0, y: origin.1 },
        clip,
        glyphs,
        fg: AnsiColor::TrueColor(255, 255, 255),
        bg: AnsiColor::Reset,
        mods,
        shaped: shaped.map(Box::new),
    }
}

fn shaped_run(face: u8, size: f32) -> ShapedRun {
    ShapedRun {
        face,
        size_px: size,
        line_height: size * 1.25 / 20.0,
        baseline: 0.8,
        glyph_faces: vec![],
        clusters: vec![],
    }
}

fn glyph(ch: char, x: f32, y: f32, advance: f32) -> Glyph {
    Glyph { x, y, ch, advance }
}

/// Bounding box (x0, y0, x1 exclusive, y1 exclusive) of pixels that differ from `bg`, in a region.
fn ink_bbox(s: &Surface, region: (u32, u32, u32, u32), bg: [u8; 3]) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in region.1..region.3.min(s.height()) {
        for x in region.0..region.2.min(s.width()) {
            if s.pixel(x, y) != bg {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > 0).then_some((x0, y0, x1, y1))
}

fn count(s: &Surface, c: [u8; 3]) -> usize {
    (0..s.height()).flat_map(|y| (0..s.width()).map(move |x| (x, y))).filter(|&(x, y)| s.pixel(x, y) == c).count()
}

#[test]
fn surface_is_cleared_to_the_default_background() {
    let (r, s) = paint(vec![], 4, 2);
    assert_eq!((s.width(), s.height()), (39, 40), "ceil(4 * 9.6), 2 * 20");
    assert_eq!(r.cell_px(), (9.6, 20.0));
    assert!(s.rgba().chunks_exact(4).all(|p| p == [15, 23, 42, 255]));
}

#[test]
fn square_fill_snaps_each_edge_to_a_whole_pixel() {
    // x: 0.5 cells = 4.8 -> 5, right edge 1.5 cells = 14.4 -> 14 (width 9 px); y: 1..2 cells.
    let (_, s) = paint(vec![fill(RectF::new(0.5, 1.0, 1.0, 1.0), 0.0, RED)], 3, 3);
    assert_eq!(ink_bbox(&s, (0, 0, 29, 60), DEFAULT_BG), Some((5, 20, 14, 40)));
    assert_eq!(s.pixel(5, 20), RED_RGB);
    assert_eq!(s.pixel(13, 39), RED_RGB);
    assert_eq!(s.pixel(14, 30), DEFAULT_BG);
}

#[test]
fn rounded_fill_has_antialiased_corners_and_the_right_coverage() {
    // 6 x 3 cells = 57.6 x 60 px, radius 0.5 cell = 4.8 px.
    let (_, s) = paint(vec![fill(RectF::new(1.0, 1.0, 6.0, 3.0), 0.5, RED)], 8, 5);
    let (x0, y0) = (10u32, 20u32); // 9.6 rounds to 10 only for snapped fills; the path is exact
    // Square corner pixel is outside the arc.
    assert_eq!(s.pixel(x0 - 0, y0), DEFAULT_BG);
    // The pixel right at the arc's midpoint is partially covered: neither bg nor red.
    let p = s.pixel(11, 21);
    assert!(p != DEFAULT_BG && p != RED_RGB, "{p:?}");
    // Edge midpoints and the centre are solid.
    assert_eq!(s.pixel(40, 21), RED_RGB);
    assert_eq!(s.pixel(40, 50), RED_RGB);
    // Coverage: sum of red-ness equals the analytic area within 1 %.
    let mut cov = 0.0f64;
    for y in 0..s.height() {
        for x in 0..s.width() {
            let px = s.pixel(x, y);
            cov += f64::from(px[0] - DEFAULT_BG[0]) / f64::from(255 - DEFAULT_BG[0]);
        }
    }
    let r = 4.8f64;
    let area = 57.6 * 60.0 - (4.0 - std::f64::consts::PI) * r * r;
    assert!((cov - area).abs() / area < 0.01, "coverage {cov} vs {area}");
    // Radius is clamped to half of the smaller side: a 1-cell-high pill never goes negative.
    let (_, pill) = paint(vec![fill(RectF::new(1.0, 1.0, 4.0, 1.0), 9.0, RED)], 6, 3);
    assert_eq!(pill.pixel(10, 20), DEFAULT_BG);
    assert_eq!(pill.pixel(30, 30), RED_RGB);
}

#[test]
fn single_border_is_a_one_pixel_line_through_the_cell_centres() {
    let (_, s) = paint(vec![border(RectF::new(2.0, 1.0, 5.0, 3.0), 0.0, BorderChars::single(), GREEN)], 8, 5);
    // x: (2 + 0.5) * 9.6 = 24 -> 24.5 centre, so pixel column 24; right (2+5-0.5)*9.6 = 62.4 -> 62 -> 62.5.
    assert_eq!(s.pixel(24, 40), GREEN_RGB);
    assert_eq!(s.pixel(23, 40), DEFAULT_BG);
    assert_eq!(s.pixel(25, 40), DEFAULT_BG);
    assert_eq!(s.pixel(62, 40), GREEN_RGB);
    // y: (1 + 0.5) * 20 = 30 -> row 30; bottom (1+3-0.5)*20 = 70 -> row 70.
    assert_eq!(s.pixel(40, 30), GREEN_RGB);
    assert_eq!(s.pixel(40, 29), DEFAULT_BG);
    assert_eq!(s.pixel(40, 31), DEFAULT_BG);
    assert_eq!(s.pixel(40, 70), GREEN_RGB);
    // The interior stays untouched.
    assert_eq!(s.pixel(40, 50), DEFAULT_BG);
    // No glyph fallback: a vector frame has no ink inside the ring cells besides the line.
    assert_eq!(count(&s, GREEN_RGB), (62 - 24 + 1) * 2 + (70 - 30 - 1) * 2);
}

#[test]
fn border_geometry_matches_the_browser_painter() {
    use oxiterm_web::paint::{plan, Metrics, PaintOp, Seg};
    let m = Metrics { cw: 9.6, ch: 20.0, dpr: 1.0, base_font: "16px monospace".into() };
    let rect = RectF::new(2.0, 1.0, 5.0, 4.0);
    for chars in [BorderChars::single(), BorderChars::rounded(), BorderChars::double()] {
        let list = DisplayList { items: vec![border(rect, 0.0, chars.clone(), GREEN)] };
        let ops = plan(&list, &m);
        let (_, s) = paint(list.items.clone(), 8, 6);
        let strokes: Vec<_> = ops.iter().filter_map(|o| if let PaintOp::Stroke { path, .. } = o { Some(path) } else { None }).collect();
        assert!(!strokes.is_empty());
        for path in strokes {
            // Every straight segment of the planned path lies on pixels we painted green.
            let mut cur = (0.0, 0.0);
            let mut checked = 0;
            for seg in path {
                match seg {
                    Seg::MoveTo(x, y) => cur = (*x, *y),
                    Seg::LineTo(x, y) => {
                        let (mx, my) = ((cur.0 + x) / 2.0, (cur.1 + y) / 2.0);
                        let p = s.pixel(mx.floor() as u32, my.floor() as u32);
                        assert!(p != DEFAULT_BG, "{chars:?}: mid of {cur:?}->({x},{y}) is background");
                        checked += 1;
                        cur = (*x, *y);
                    }
                    Seg::ArcTo(_, _, x, y, _) => cur = (*x, *y),
                    _ => {}
                }
            }
            assert!(checked >= 2);
        }
    }
}

#[test]
fn rounded_border_cuts_the_corner_and_double_has_two_lines() {
    let rect = RectF::new(1.0, 1.0, 6.0, 4.0);
    let (_, sq) = paint(vec![border(rect, 0.0, BorderChars::single(), GREEN)], 8, 6);
    let (_, rd) = paint(vec![border(rect, 0.0, BorderChars::rounded(), GREEN)], 8, 6);
    // single corner: line centre (1.5 cells = 14.4 -> 14.5, 30.5) is inked; rounded leaves it bare
    assert_eq!(sq.pixel(14, 30), GREEN_RGB);
    assert_eq!(rd.pixel(14, 30), DEFAULT_BG);
    // straight parts are the same
    assert_eq!(rd.pixel(40, 30), GREEN_RGB);
    // double: two lines near the top edge row 30 +- d (d = min(cw,ch)/6 = 1.6)
    let (_, db) = paint(vec![border(rect, 0.0, BorderChars::double(), GREEN)], 8, 6);
    let col: Vec<u32> = (20..45).filter(|&y| db.pixel(40, y) == GREEN_RGB).collect();
    assert_eq!(col.len(), 2, "{col:?}");
    assert!(col[1] - col[0] >= 2);
}

#[test]
fn radius_border_draws_a_ring_clipped_to_the_rounded_silhouette() {
    let rect = RectF::new(1.0, 1.0, 6.0, 4.0);
    let item = DisplayItem::Border {
        node: NodeId(1),
        rect,
        radius: 0.8,
        chars: BorderChars::rounded(),
        fg: GREEN,
        bg: RED,
    };
    let (_, s) = paint(vec![item], 8, 6);
    // ring background: red between the silhouette and the one-cell inset, interior untouched
    assert_eq!(s.pixel(12, 40), RED_RGB, "left ring strip");
    assert_eq!(s.pixel(40, 50), DEFAULT_BG, "interior");
    // square corner pixel of the bounding box is outside the rounded silhouette
    assert_eq!(s.pixel(9, 20), DEFAULT_BG);
    // the line itself is the foreground colour
    assert_eq!(s.pixel(40, 30), GREEN_RGB);
}

#[test]
fn glyph_ascii_border_falls_back_to_glyph_cells() {
    let ascii = BorderChars { top_left: '+', top: '-', top_right: '+', left: '|', right: '|', bot_left: '+', bot: '-', bot_right: '+' };
    let (_, s) = paint(vec![border(RectF::new(0.0, 0.0, 4.0, 3.0), 0.0, ascii, GREEN)], 5, 4);
    assert!(count(&s, GREEN_RGB) > 20, "glyph strokes were drawn");
    // interior cell (1,1) has no ink
    assert_eq!(ink_bbox(&s, (10, 20, 19, 40), DEFAULT_BG), None);
}

#[test]
fn path_fill_and_stroke() {
    let p = |x, y| PointF { x, y };
    let item = DisplayItem::Path {
        node: NodeId(1),
        cmds: vec![PathCmd::MoveTo(p(1.0, 1.0)), PathCmd::LineTo(p(5.0, 1.0)), PathCmd::LineTo(p(5.0, 3.0)), PathCmd::Close],
        fill: Some(RED),
        stroke: Some((GREEN, 0.2)),
    };
    let (_, s) = paint(vec![item], 7, 4);
    // inside the triangle (bottom-right half of the box)
    assert_eq!(s.pixel(43, 30), RED_RGB);
    // outside it (top-left of the diagonal)
    assert_eq!(s.pixel(14, 50), DEFAULT_BG);
    // stroke along the top edge y = 20 (2 px wide: 0.2 cells * 9.6)
    assert_eq!(s.pixel(30, 20), GREEN_RGB);
    assert!(count(&s, GREEN_RGB) > 60);
}

#[test]
fn default_text_run_paints_glyphs_in_the_mono_cell() {
    let glyphs = vec![glyph('H', 0.0, 0.0, 1.0), glyph('i', 1.0, 0.0, 1.0)];
    let (_, s) = paint(vec![text((1.0, 1.0), RectF::new(1.0, 1.0, 4.0, 1.0), glyphs, Modifiers::default(), None)], 6, 3);
    let bb = ink_bbox(&s, (0, 0, s.width(), s.height()), DEFAULT_BG).unwrap();
    // Everything stays inside the two cells (x 9.6..28.8) and the row (y 20..40); a cap-height
    // glyph does not touch the very top of the cell.
    assert!(bb.0 >= 9 && bb.2 <= 30, "{bb:?}");
    assert!(bb.1 >= 20 && bb.3 <= 40, "{bb:?}");
    // the white text colour is reached in the stems
    assert!(count(&s, [255, 255, 255]) > 4);
}

#[test]
fn text_is_clipped_to_its_clip_rect() {
    let glyphs: Vec<Glyph> = "HHHH".chars().enumerate().map(|(i, c)| glyph(c, i as f32, 0.0, 1.0)).collect();
    // clip to the first two cells only
    let (_, s) = paint(vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 2.0, 1.0), glyphs, Modifiers::default(), None)], 5, 2);
    let bb = ink_bbox(&s, (0, 0, s.width(), s.height()), DEFAULT_BG).unwrap();
    assert!(bb.2 <= 19, "ink right edge {} past the 19.2 px clip", bb.2);
}

#[test]
fn shaped_glyph_lands_on_the_positions_the_shaper_gave() {
    // 'H' in sans, size 16, line top at row 2, x = 3.0 cells; baseline 0.8 cells below the top.
    let run = shaped_run(FACE_SANS, 16.0);
    let g = glyph('H', 3.0, 2.0, 1.2);
    let (mut r, s) = paint(
        vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 10.0, 6.0), vec![g], Modifiers::default(), Some(run))],
        10,
        6,
    );
    let x_px = 3.0 * 9.6; // 28.8 -> quarter-pixel phase 3, origin pixel 28
    let base_y = ((2.0f32 + 0.8) * 20.0).round() as i32; // 56
    let (gface, gid) = r.glyphs().char_glyph(FACE_SANS, 'H').unwrap();
    let m = r.glyphs().mask(gface, gid, 16.0, false, false, 3).unwrap();
    let bb = ink_bbox(&s, (0, 0, s.width(), s.height()), DEFAULT_BG).unwrap();
    assert_eq!(bb.0 as i32, 28 + m.left);
    assert_eq!(bb.2 as i32, 28 + m.left + m.width as i32);
    assert_eq!(bb.3 as i32, base_y, "an 'H' sits exactly on the baseline row");
    assert_eq!(bb.1 as i32, base_y - m.top);
    // and inside the advance box (+-1 px): the glyph is where the shaper put it
    assert!(bb.0 as f64 >= x_px - 1.0 && f64::from(bb.2) <= x_px + 1.2 * 9.6 + 1.0, "{bb:?}");
}

#[test]
fn default_mono_text_equals_a_shaped_16px_mono_run() {
    let cell = vec![glyph('H', 0.0, 0.0, 1.0)];
    let (_, a) = paint(vec![text((2.0, 1.0), RectF::new(0.0, 0.0, 8.0, 4.0), cell, Modifiers::default(), None)], 8, 4);
    let mut run = shaped_run(FACE_MONO, 16.0);
    let probe = oxiterm_fb::glyphs::GlyphCache::new();
    run.baseline = probe.mono_baseline_px() / 20.0;
    run.line_height = 1.0;
    let (_, b) = paint(
        vec![text((2.0, 1.0), RectF::new(0.0, 0.0, 8.0, 4.0), vec![glyph('H', 0.0, 0.0, 1.0)], Modifiers::default(), Some(run))],
        8,
        4,
    );
    assert_eq!(a.rgba(), b.rgba());
}

#[test]
fn clusters_ligatures_and_combining_marks() {
    let mut run = shaped_run(FACE_SANS, 32.0);
    run.clusters = vec![(0, "ffi".into()), (1, "e\u{301}".into())];
    let glyphs = vec![glyph('f', 0.0, 1.0, 1.5), glyph('e', 5.0, 1.0, 1.0)];
    let (_, s) = paint(
        vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 6.0, 3.0), glyphs, Modifiers::default(), Some(run))],
        6,
        3,
    );
    let lig = ink_bbox(&s, (0, 0, 45, 60), DEFAULT_BG).expect("ligature drawn");
    let acc = ink_bbox(&s, (46, 0, 70, 60), DEFAULT_BG).expect("accented e drawn");
    // the accent rises above the x-height of the base: taller than a plain 'e' at that size
    let mut run2 = shaped_run(FACE_SANS, 32.0);
    run2.clusters = vec![];
    let (_, plain) = paint(
        vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 6.0, 3.0), vec![glyph('e', 5.0, 1.0, 1.0)], Modifiers::default(), Some(run2))],
        6,
        3,
    );
    let pe = ink_bbox(&plain, (46, 0, 70, 60), DEFAULT_BG).unwrap();
    assert!(acc.1 < pe.1, "accent above the base: {acc:?} vs {pe:?}");
    assert!(lig.2 - lig.0 > 10);
}

#[test]
fn bold_italic_underline_modifiers() {
    let g = |m: Modifiers| {
        let glyphs = vec![glyph('l', 0.0, 0.0, 1.0)];
        let (_, s) = paint(vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 3.0, 1.0), glyphs, m, None)], 3, 1);
        s
    };
    let plain = g(Modifiers::default());
    let bold = g(Modifiers { bold: true, ..Default::default() });
    let italic = g(Modifiers { italic: true, ..Default::default() });
    let under = g(Modifiers { underline: true, ..Default::default() });
    let w = |s: &Surface| ink_bbox(s, (0, 0, s.width(), 18), DEFAULT_BG).unwrap();
    let (pb, bb, ib) = (w(&plain), w(&bold), w(&italic));
    assert!(bb.2 - bb.0 > pb.2 - pb.0, "bold is wider: {bb:?} vs {pb:?}");
    assert_ne!(plain.rgba(), italic.rgba());
    // the top of an italic stem sits further right than the top of the upright one
    let top_x = |s: &Surface| {
        let xs: Vec<u32> = (0..s.width()).filter(|&x| s.pixel(x, 4) != DEFAULT_BG).collect();
        f64::from(xs.iter().sum::<u32>()) / xs.len() as f64
    };
    assert!(top_x(&italic) > top_x(&plain) + 1.0, "{ib:?} {pb:?}");
    // underline: the bottom two rows of the cell, full cell width (9.6 -> 10 px)
    assert_eq!(under.pixel(0, 19), [255, 255, 255]);
    assert_eq!(under.pixel(9, 18), [255, 255, 255]);
    assert_eq!(plain.pixel(0, 19), DEFAULT_BG);
}

#[test]
fn cell_grid_paints_cells_and_skips_wide_trailers() {
    let cell = |ch, bg| GridCell { ch, fg: AnsiColor::Reset, bg, mods: Modifiers::default(), skip: false };
    let mut cells = vec![cell('X', RED), cell(' ', GREEN), cell('Y', AnsiColor::Reset), cell(' ', AnsiColor::Reset)];
    cells[3].skip = true;
    cells[3].bg = RED; // never painted: skipped
    let (_, s) = paint(vec![DisplayItem::CellGrid { rect: RectF::new(0.0, 0.0, 2.0, 2.0), cells }], 2, 2);
    assert_eq!(s.pixel(0, 0), RED_RGB);
    assert_eq!(s.pixel(15, 5), GREEN_RGB);
    assert_eq!(s.pixel(15, 25), DEFAULT_BG, "skipped cell keeps the cleared background");
    assert!(ink_bbox(&s, (0, 20, 10, 40), DEFAULT_BG).is_some(), "'Y' has ink");
}

#[test]
fn push_clip_clips_paths_and_fills() {
    let items = vec![
        DisplayItem::PushClip { rect: RectF::new(0.0, 0.0, 2.0, 1.0) },
        fill(RectF::new(0.0, 0.0, 5.0, 3.0), 0.0, RED),
        DisplayItem::PopClip,
        fill(RectF::new(0.0, 2.0, 1.0, 1.0), 0.0, GREEN),
    ];
    let (_, s) = paint(items, 5, 3);
    assert_eq!(ink_bbox(&s, (0, 0, s.width(), 40), DEFAULT_BG), Some((0, 0, 19, 20)));
    assert_eq!(s.pixel(2, 45), GREEN_RGB);
}

#[test]
fn colours_agree_with_the_web_painter() {
    use oxiterm_web::paint::{plan, Metrics, PaintOp};
    let m = Metrics { cw: 10.0, ch: 20.0, dpr: 1.0, base_font: "16px monospace".into() };
    let parse = |s: &str| -> [u8; 3] {
        if let Some(h) = s.strip_prefix('#') {
            let v = u32::from_str_radix(h, 16).unwrap();
            [(v >> 16) as u8, (v >> 8) as u8, v as u8]
        } else {
            let n: Vec<u8> = s.trim_start_matches("rgb(").trim_end_matches(')').split(',').map(|p| p.trim().parse().unwrap()).collect();
            [n[0], n[1], n[2]]
        }
    };
    let mut colours: Vec<AnsiColor> = (0..=255u8).map(AnsiColor::Color256).collect();
    colours.push(AnsiColor::Reset);
    colours.push(AnsiColor::TrueColor(12, 34, 56));
    for c in colours {
        // background mode: a Fill
        let list = DisplayList { items: vec![fill(RectF::new(0.0, 0.0, 1.0, 1.0), 0.0, c)] };
        let PaintOp::Fill { color, .. } = plan(&list, &m).remove(0) else { panic!() };
        assert_eq!(parse(&color), color::bg(&c), "bg {c:?}");
        // foreground mode: a glyph
        let t = DisplayItem::Text {
            node: NodeId(1),
            origin: PointF::default(),
            clip: RectF::new(0.0, 0.0, 1.0, 1.0),
            glyphs: vec![glyph('x', 0.0, 0.0, 1.0)],
            fg: c,
            bg: AnsiColor::Reset,
            mods: Modifiers::default(),
            shaped: None,
        };
        let ops = plan(&DisplayList { items: vec![t] }, &m);
        let fg = ops.iter().find_map(|o| if let PaintOp::Text { color, .. } = o { Some(color.clone()) } else { None }).unwrap();
        assert_eq!(parse(&fg), color::fg(&c), "fg {c:?}");
    }
    assert_eq!(color::fg(&AnsiColor::Reset), DEFAULT_FG);
}

#[test]
fn scale_two_doubles_the_geometry() {
    let mut r = Rasterizer::new(2.0);
    let mut s = r.surface_for(4, 2).unwrap();
    assert_eq!((s.width(), s.height()), (77, 80));
    r.render(&DisplayList { items: vec![fill(RectF::new(1.0, 0.0, 1.0, 1.0), 0.0, RED)] }, &mut s);
    assert_eq!(ink_bbox(&s, (0, 0, 77, 80), DEFAULT_BG), Some((19, 0, 38, 40)));
}
