//! BAD ACTOR findings for Stage 7.2 (`oxiterm-fb`). Each test fails today and passes once fixed.
//! Hang tests run the render on a thread and fail on a 5 s timeout (the thread keeps spinning
//! until the test binary exits).

use std::sync::mpsc;
use std::time::Duration;

use oxiterm_fb::pixel::{BitField, PixelFormat};
use oxiterm_fb::{Rasterizer, Surface};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PointF, RectF, ShapedRun, FACE_MONO,
};

const WHITE: AnsiColor = AnsiColor::TrueColor(255, 255, 255);
const RED: AnsiColor = AnsiColor::TrueColor(255, 0, 0);

fn render(items: Vec<DisplayItem>) -> Surface {
    let mut r = Rasterizer::new(1.0);
    let mut s = r.surface_for(10, 5).unwrap();
    r.render(&DisplayList { items }, &mut s);
    s
}

/// Runs `items` on a thread; Err("hang") after 5 s, Err(panic message) on panic.
fn render_timeboxed(items: Vec<DisplayItem>) -> Result<(), String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let res = std::panic::catch_unwind(|| {
            render(items);
        });
        let _ = tx.send(res.map_err(|e| {
            e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()
        }));
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(r) => r,
        Err(_) => Err("hang".into()),
    }
}

fn text(origin: (f32, f32), clip: RectF, glyphs: Vec<Glyph>, shaped: Option<ShapedRun>) -> DisplayItem {
    DisplayItem::Text {
        node: NodeId(1),
        origin: PointF { x: origin.0, y: origin.1 },
        clip,
        glyphs,
        fg: WHITE,
        bg: AnsiColor::Reset,
        mods: Modifiers::default(),
        shaped: shaped.map(Box::new),
    }
}

fn run(face: u8, size: f32) -> ShapedRun {
    ShapedRun { face, size_px: size, line_height: 1.0, baseline: 0.8, glyph_faces: vec![], clusters: vec![] }
}

fn ascii_border(w: f32, h: f32) -> DisplayItem {
    // '+' corners with ASCII edges: `border_style` -> Glyphs (top_left is not a box char).
    let chars = BorderChars { top_left: '+', top_right: '+', bot_left: '+', bot_right: '+', top: '-', bot: '-', left: '|', right: '|' };
    DisplayItem::Border { node: NodeId(1), rect: RectF::new(0.0, 0.0, w, h), radius: 0.0, chars, fg: WHITE, bg: AnsiColor::Reset }
}

/// CRASH (also release): a shaped glyph with NaN `y` makes the line-grouping loop in
/// `Canvas::shaped` produce an empty line, then `line[0]` panics (NaN != NaN).
#[test]
fn shaped_glyph_with_nan_y_must_not_panic() {
    let g = Glyph { x: 0.0, y: f32::NAN, ch: 'A', advance: 1.0 };
    let clip = RectF::new(0.0, 0.0, 10.0, 5.0);
    let r = render_timeboxed(vec![text((0.0, 0.0), clip, vec![g], Some(run(FACE_MONO, 16.0)))]);
    assert!(r.is_ok(), "{r:?}");
}

/// HANG: glyph-style border with f32 loop counter; at i = 2^24, i += 1.0 is a no-op.
#[test]
fn border_glyph_style_huge_width_must_terminate() {
    let r = render_timeboxed(vec![ascii_border(1e8, 3.0)]);
    assert!(r.is_ok(), "{r:?}");
}

/// HANG: same loop with w = +inf (also h loop).
#[test]
fn border_glyph_style_infinite_width_must_terminate() {
    let r = render_timeboxed(vec![ascii_border(f32::INFINITY, 3.0)]);
    assert!(r.is_ok(), "{r:?}");
}

/// HANG: height loop.
#[test]
fn border_glyph_style_infinite_height_must_terminate() {
    let r = render_timeboxed(vec![ascii_border(3.0, f32::INFINITY)]);
    assert!(r.is_ok(), "{r:?}");
}

/// OVERFLOW: `w * h` usize multiply in `Canvas::grid` (debug panic; release wraps to a small
/// number so legitimate cells are dropped).
#[test]
fn cellgrid_huge_rect_must_not_overflow() {
    let cell = GridCell { ch: ' ', fg: AnsiColor::Reset, bg: RED, mods: Modifiers::default(), skip: false };
    let r = render_timeboxed(vec![DisplayItem::CellGrid { rect: RectF::new(0.0, 0.0, 9.223372e18, 2.0), cells: vec![cell; 4] }]);
    assert!(r.is_ok(), "{r:?}");
}

/// Release-visible variant of the same bug: w*h wraps to 0 so no cell is painted although cells
/// exist and the first one is on screen.
#[test]
fn cellgrid_huge_rect_must_still_paint_existing_cells() {
    let cell = GridCell { ch: ' ', fg: AnsiColor::Reset, bg: RED, mods: Modifiers::default(), skip: false };
    let item = DisplayItem::CellGrid { rect: RectF::new(0.0, 0.0, 9.223372e18, 2.0), cells: vec![cell; 4] };
    let res = std::panic::catch_unwind(|| render(vec![item]));
    match res {
        Ok(s) => assert_eq!(s.pixel(0, 0), [255, 0, 0]),
        Err(_) => panic!("panicked (overflow)"),
    }
}

/// OVERFLOW (debug panic): `ix as i32` saturates, then `+ m.left` / `+ m.width` overflow.
#[test]
fn unshaped_glyph_at_huge_x_must_not_panic() {
    let g = Glyph { x: 0.0, y: 0.0, ch: 'A', advance: 1.0 };
    let clip = RectF::new(-1e30, -1e30, 2e30, 2e30);
    let r = render_timeboxed(vec![text((1e30, 0.0), clip, vec![g], None)]);
    assert!(r.is_ok(), "{r:?}");
}

/// OVERFLOW (debug panic): `by - m.top` with y = -1e30 (by saturates to i32::MIN).
#[test]
fn unshaped_glyph_at_huge_negative_y_must_not_panic() {
    let g = Glyph { x: 0.0, y: 0.0, ch: 'A', advance: 1.0 };
    let clip = RectF::new(-1e30, -1e30, 2e30, 2e30);
    let r = render_timeboxed(vec![text((0.0, -1e30), clip, vec![g], None)]);
    assert!(r.is_ok(), "{r:?}");
}

/// MEMORY: no cap on the glyph pixel size; one glyph mask is width*height bytes (quadratic).
/// size 8000 px is only ~64 MB here, but `size_px` is attacker data (1e5 px = 10 GB).
#[test]
fn glyph_mask_size_must_be_capped() {
    let mut r = Rasterizer::new(1.0);
    let (face, gid) = r.glyphs().char_glyph(0, 'M').unwrap();
    let m = r.glyphs().mask(face, gid, 8000.0, false, false, 0);
    let bytes = m.map(|m| m.data.len()).unwrap_or(0);
    assert!(bytes <= 16 << 20, "one glyph mask is {bytes} bytes");
}

/// MEMORY/ALLOC: shaped run with absurd size must not be rendered at full size. We only probe the
/// cheap, bounded part: scale * size_px should be clamped before shaping (size_px = 1e9).
/// (Formerly ignored: would OOM-kill the machine.) Reproduce: text with shaped run size_px = 1e5.
#[test]
fn shaped_huge_size_px_oom() {
    let g = Glyph { x: 0.0, y: 0.0, ch: 'M', advance: 1.0 };
    let clip = RectF::new(0.0, 0.0, 10.0, 5.0);
    render(vec![text((0.0, 0.0), clip, vec![g], Some(run(FACE_MONO, 1e5)))]);
}

/// PANIC in pure fn: `transp` is never validated by `is_supported`, and `pack` shifts by
/// `transp.offset` (debug panic for offset >= 32; silently wraps shift in release).
#[test]
fn pixelformat_pack_with_huge_transp_offset_must_not_panic() {
    let f = PixelFormat { transp: BitField { offset: 40, length: 8 }, ..PixelFormat::XRGB8888 };
    let ok = f.is_supported();
    if ok {
        let _ = f.pack(1, 2, 3);
    }
}

/// PANIC in pure fn: `offset + length` u32 add overflows in `is_supported` for hostile structs.
#[test]
fn pixelformat_is_supported_overflow() {
    let f = PixelFormat { red: BitField { offset: u32::MAX, length: 8 }, ..PixelFormat::XRGB8888 };
    assert!(!f.is_supported());
}

/// DISPLAY: `convert_image` panics when `rgba.len() < width*height*4` (public API).
#[test]
fn convert_image_short_buffer_must_not_panic() {
    let mut dst = vec![0u8; 64];
    let r = std::panic::catch_unwind(move || {
        oxiterm_fb::pixel::convert_image(&PixelFormat::XRGB8888, &[0u8; 8], 4, 4, &mut dst, 16, 4, 4)
    });
    assert!(r.is_ok());
}

/// ALLOC: surface sizes are uncapped. `surface_for` with a 4.2e9 cell viewport must return None,
/// not try to allocate (formerly ignored: aborted the process).
#[test]
fn surface_for_huge_viewport_must_be_none() {
    let r = Rasterizer::new(16.0);
    assert!(r.surface_for(60_000, 60_000).is_none());
}


fn shaped_text_with(size: f32, ch: char, cluster: Option<&str>) -> DisplayItem {
    let mut sr = run(FACE_MONO, size);
    if let Some(c) = cluster {
        sr.clusters = vec![(0, c.into())];
    }
    let g = Glyph { x: 0.0, y: 0.0, ch, advance: 1.0 };
    text((0.0, 0.0), RectF::new(0.0, 0.0, 10.0, 5.0), vec![g], Some(sr))
}

/// CRASH: zero shaped size + a cluster reaches cosmic-text `Metrics{line_height: 0}`, which
/// asserts "line height cannot be 0" (also reached by any size so small that size*1.25 == 0,
/// or scale-clamped products).
#[test]
fn shaped_zero_size_cluster_must_not_panic() {
    let r = render_timeboxed(vec![shaped_text_with(0.0, 'f', Some("ffi"))]);
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn shaped_negative_size_cluster_must_not_hang_or_panic() {
    let r = render_timeboxed(vec![shaped_text_with(-2.2, 'f', Some("ffi"))]);
    assert!(r.is_ok(), "{r:?}");
}

/// Same draw_glyph overflow reached through Border (glyph style) at a huge x.
#[test]
fn border_at_huge_x_must_not_panic() {
    let chars = BorderChars { top_left: '+', top_right: '+', bot_left: '+', bot_right: '+', top: '-', bot: '-', left: '|', right: '|' };
    let b = DisplayItem::Border { node: NodeId(1), rect: RectF::new(1e20, 0.0, 3.0, 3.0), radius: 0.0, chars, fg: WHITE, bg: AnsiColor::Reset };
    let r = render_timeboxed(vec![b]);
    assert!(r.is_ok(), "{r:?}");
}

/// Same through CellGrid.
#[test]
fn cellgrid_at_huge_x_must_not_panic() {
    let cell = GridCell { ch: 'A', fg: WHITE, bg: RED, mods: Modifiers::default(), skip: false };
    let r = render_timeboxed(vec![DisplayItem::CellGrid { rect: RectF::new(1e20, 1e20, 2.0, 1.0), cells: vec![cell; 2] }]);
    assert!(r.is_ok(), "{r:?}");
}

fn pth(cmds: Vec<oxiterm_renderer::render::display_list::PathCmd>, fill: Option<AnsiColor>, stroke: Option<(AnsiColor, f32)>) -> DisplayItem {
    DisplayItem::Path { node: NodeId(1), cmds, fill, stroke }
}
fn p(x: f32, y: f32) -> PointF { PointF { x, y } }

/// CRASH (tiny-skia 0.12 `scan/path.rs:221` debug_assert `edges[curr_idx].last_y >= curr_y`; in release the
/// follow-on is the `alpha_runs.rs:189` unwrap): fill of a
/// cubic with 1e20 coordinates (fuzz seed 11400714819323199479). Coordinates are not clamped
/// before being handed to tiny-skia.
#[test]
fn path_fill_with_1e20_coordinates_must_not_panic() {
    use oxiterm_renderer::render::display_list::PathCmd::*;
    let r = render_timeboxed(vec![pth(
        vec![MoveTo(p(48.775, 7.725)), Close, CubicTo(p(27.05, -1e20), p(-1e20, 1.3333334), p(1.2e-38, -1e20)), Close, Close],
        Some(AnsiColor::Reset),
        Some((AnsiColor::Reset, 1e20)),
    )]);
    assert!(r.is_ok(), "{r:?}");
}

/// CRASH (seed 11400714819323199215): quad to y = 16777215 with a 1e20-cell stroke width. Debug:
/// tiny-skia `scan/path.rs:221` debug_assert; release: the debug_asserts vanish and the scan goes on
/// to `alpha_runs.rs:189` `runs[..].unwrap()` on None (release crash, per source reading).
#[test]
fn path_stroke_with_1e20_width_must_not_panic() {
    use oxiterm_renderer::render::display_list::PathCmd::*;
    let r = render_timeboxed(vec![pth(
        vec![LineTo(p(0.0, 0.0)), MoveTo(p(-0.0, 14.333333)), QuadTo(p(-5.0, 18.425), p(20.75, 16777215.0))],
        Some(AnsiColor::Reset),
        Some((AnsiColor::Reset, 1e20)),
    )]);
    assert!(r.is_ok(), "{r:?}");
}

/// HANG (>8 s; seed 11400714819323200825): stroke width f32::MAX cells over a path with a 3.4e38
/// coordinate and a NaN point.
#[test]
fn path_stroke_with_f32_max_width_must_terminate() {
    use oxiterm_renderer::render::display_list::PathCmd::*;
    let r = render_timeboxed(vec![pth(
        vec![MoveTo(p(1.2e-38, 3.4028235e38)), MoveTo(p(f32::NAN, 1.2e-38)), CubicTo(p(0.05, -14.325), p(-5.33, 49.875), p(47.7, 43.67))],
        Some(AnsiColor::TrueColor(2, 190, 187)),
        Some((AnsiColor::Color256(64), f32::MAX)),
    )]);
    assert!(r.is_ok(), "{r:?}");
}
