//! Primitive-level and robustness tests for [`oxiterm_gpu::GpuSink`].
//!
//! Every GPU test skips cleanly (passing with a printed note) when no `wgpu` adapter is available,
//! so CI environments without a GPU or software Vulkan driver stay green.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use oxiterm_gpu::color::DEFAULT_BG;
use oxiterm_gpu::{FrameImage, GpuContext, GpuContextOptions, GpuError, GpuSink};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PointF, RectF, ShapedRun, FACE_MONO,
    FACE_SANS,
};

const RED: AnsiColor = AnsiColor::TrueColor(255, 0, 0);
const GREEN: AnsiColor = AnsiColor::TrueColor(0, 255, 0);
const WHITE: AnsiColor = AnsiColor::TrueColor(255, 255, 255);
const RED_RGB: [u8; 3] = [255, 0, 0];
const GREEN_RGB: [u8; 3] = [0, 255, 0];

fn shared_ctx() -> Option<Arc<GpuContext>> {
    static CTX: OnceLock<Option<Arc<GpuContext>>> = OnceLock::new();
    CTX.get_or_init(|| match GpuContext::new_headless() {
        Ok(ctx) => Some(Arc::new(ctx)),
        Err(GpuError::NoAdapter(msg)) => {
            eprintln!("SKIP oxiterm-gpu tests: no GPU adapter available ({msg})");
            None
        }
        Err(other) => {
            panic!("oxiterm-gpu tests: GPU context initialization failed unexpectedly: {other}");
        }
    })
    .clone()
}

macro_rules! require_gpu {
    () => {
        match shared_ctx() {
            Some(ctx) => ctx,
            None => {
                eprintln!("SKIP: no GPU adapter available");
                return;
            }
        }
    };
}

fn paint(items: Vec<DisplayItem>, cols: u32, rows: u32) -> Option<(GpuSink, FrameImage)> {
    let ctx = shared_ctx()?;
    let mut sink = GpuSink::new(ctx, 1.0);
    let img = sink.render_cells(&DisplayList { items }, cols, rows).unwrap();
    Some((sink, img))
}

fn fill(rect: RectF, radius: f32, bg: AnsiColor) -> DisplayItem {
    DisplayItem::Fill { node: NodeId(1), rect, radius, fg: AnsiColor::Reset, bg }
}

fn border(rect: RectF, radius: f32, chars: BorderChars, fg: AnsiColor) -> DisplayItem {
    DisplayItem::Border { node: NodeId(1), rect, radius, chars, fg, bg: AnsiColor::Reset }
}

fn text(
    origin: (f32, f32),
    clip: RectF,
    glyphs: Vec<Glyph>,
    mods: Modifiers,
    shaped: Option<ShapedRun>,
) -> DisplayItem {
    DisplayItem::Text {
        node: NodeId(1),
        origin: PointF { x: origin.0, y: origin.1 },
        clip,
        glyphs,
        fg: WHITE,
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

fn ink_bbox(
    img: &FrameImage,
    region: (u32, u32, u32, u32),
    bg: [u8; 3],
) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for y in region.1..region.3.min(img.height()) {
        for x in region.0..region.2.min(img.width()) {
            if img.pixel(x, y) != bg {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    (x1 > 0).then_some((x0, y0, x1, y1))
}

fn count_color(img: &FrameImage, c: [u8; 3]) -> usize {
    (0..img.height())
        .flat_map(|y| (0..img.width()).map(move |x| (x, y)))
        .filter(|&(x, y)| img.pixel(x, y) == c)
        .count()
}

#[test]
fn surface_is_cleared_to_default_background() {
    let Some((sink, img)) = paint(vec![], 4, 2) else { return };
    assert_eq!((img.width(), img.height()), (39, 40));
    assert_eq!(sink.cell_px(), (9.6, 20.0));
    assert!(img.rgba().as_chunks::<4>().0.iter().all(|p| *p == [15, 23, 42, 255]));
}

#[test]
fn square_fill_snaps_each_edge_to_whole_pixel() {
    let Some((_, img)) = paint(vec![fill(RectF::new(0.5, 1.0, 1.0, 1.0), 0.0, RED)], 3, 3) else {
        return;
    };
    assert_eq!(ink_bbox(&img, (0, 0, 29, 60), DEFAULT_BG), Some((5, 20, 14, 40)));
    assert_eq!(img.pixel(5, 20), RED_RGB);
    assert_eq!(img.pixel(13, 39), RED_RGB);
    assert_eq!(img.pixel(14, 30), DEFAULT_BG);
}

#[test]
fn rounded_fill_has_antialiased_corners_and_right_coverage() {
    let Some((_, img)) = paint(vec![fill(RectF::new(1.0, 1.0, 6.0, 3.0), 0.5, RED)], 8, 5) else {
        return;
    };
    assert_eq!(img.pixel(10, 20), DEFAULT_BG);
    let p = img.pixel(11, 21);
    assert!(p != DEFAULT_BG && p != RED_RGB, "expected antialiased corner pixel, got {p:?}");
    assert_eq!(img.pixel(40, 21), RED_RGB);
    assert_eq!(img.pixel(40, 50), RED_RGB);

    let mut cov = 0.0f64;
    for y in 0..img.height() {
        for x in 0..img.width() {
            let px = img.pixel(x, y);
            cov += f64::from(px[0].saturating_sub(DEFAULT_BG[0])) / f64::from(255 - DEFAULT_BG[0]);
        }
    }
    let r = 4.8f64;
    let area = 57.6 * 60.0 - (4.0 - std::f64::consts::PI) * r * r;
    assert!((cov - area).abs() / area < 0.01, "coverage {cov} vs analytic {area}");

    // Radius clamped to half of smaller side (pill shape).
    let Some((_, pill)) = paint(vec![fill(RectF::new(1.0, 1.0, 4.0, 1.0), 9.0, RED)], 6, 3) else {
        return;
    };
    assert_eq!(pill.pixel(10, 20), DEFAULT_BG);
    assert_eq!(pill.pixel(30, 30), RED_RGB);
}

#[test]
fn single_border_is_one_pixel_line_through_cell_centres() {
    let Some((_, img)) =
        paint(vec![border(RectF::new(2.0, 1.0, 5.0, 3.0), 0.0, BorderChars::single(), GREEN)], 8, 5)
    else {
        return;
    };
    assert_eq!(img.pixel(24, 40), GREEN_RGB);
    assert_eq!(img.pixel(23, 40), DEFAULT_BG);
    assert_eq!(img.pixel(25, 40), DEFAULT_BG);
    assert_eq!(img.pixel(62, 40), GREEN_RGB);
    assert_eq!(img.pixel(40, 30), GREEN_RGB);
    assert_eq!(img.pixel(40, 29), DEFAULT_BG);
    assert_eq!(img.pixel(40, 31), DEFAULT_BG);
    assert_eq!(img.pixel(40, 70), GREEN_RGB);
    assert_eq!(img.pixel(40, 50), DEFAULT_BG);
    assert_eq!(count_color(&img, GREEN_RGB), (62 - 24 + 1) * 2 + (70 - 30 - 1) * 2);
}

#[test]
fn rounded_border_cuts_corner_and_double_has_two_lines() {
    let rect = RectF::new(1.0, 1.0, 6.0, 4.0);
    let Some((_, sq)) = paint(vec![border(rect, 0.0, BorderChars::single(), GREEN)], 8, 6) else {
        return;
    };
    let Some((_, rd)) = paint(vec![border(rect, 0.0, BorderChars::rounded(), GREEN)], 8, 6) else {
        return;
    };
    assert_eq!(sq.pixel(14, 30), GREEN_RGB);
    assert_eq!(rd.pixel(14, 30), DEFAULT_BG);
    assert_eq!(rd.pixel(40, 30), GREEN_RGB);

    let Some((_, db)) = paint(vec![border(rect, 0.0, BorderChars::double(), GREEN)], 8, 6) else {
        return;
    };
    let col: Vec<u32> = (20..45).filter(|&y| db.pixel(40, y) == GREEN_RGB).collect();
    assert_eq!(col.len(), 2, "expected 2 horizontal lines for double border, got {col:?}");
    assert!(col[1] - col[0] >= 2);
}

#[test]
fn radius_border_draws_ring_clipped_to_rounded_silhouette() {
    let rect = RectF::new(1.0, 1.0, 6.0, 4.0);
    let item = DisplayItem::Border {
        node: NodeId(1),
        rect,
        radius: 0.8,
        chars: BorderChars::rounded(),
        fg: GREEN,
        bg: RED,
    };
    let Some((_, img)) = paint(vec![item], 8, 6) else { return };
    assert_eq!(img.pixel(12, 40), RED_RGB, "left ring strip");
    assert_eq!(img.pixel(40, 50), DEFAULT_BG, "interior untouched");
    assert_eq!(img.pixel(9, 20), DEFAULT_BG, "square outer corner clipped away");
    assert_eq!(img.pixel(40, 30), GREEN_RGB, "border stroke");
}

#[test]
fn glyph_ascii_border_falls_back_to_glyph_cells() {
    let ascii = BorderChars {
        top_left: '+',
        top: '-',
        top_right: '+',
        left: '|',
        right: '|',
        bot_left: '+',
        bot: '-',
        bot_right: '+',
    };
    let Some((_, img)) = paint(vec![border(RectF::new(0.0, 0.0, 4.0, 3.0), 0.0, ascii, GREEN)], 5, 4)
    else {
        return;
    };
    assert!(count_color(&img, GREEN_RGB) > 20, "glyph strokes were drawn");
    assert_eq!(ink_bbox(&img, (10, 20, 19, 40), DEFAULT_BG), None);
}

#[test]
fn default_text_run_paints_glyphs_in_mono_cell() {
    let glyphs = vec![glyph('H', 0.0, 0.0, 1.0), glyph('i', 1.0, 0.0, 1.0)];
    let Some((_, img)) = paint(
        vec![text((1.0, 1.0), RectF::new(1.0, 1.0, 4.0, 1.0), glyphs, Modifiers::default(), None)],
        6,
        3,
    ) else {
        return;
    };
    let bb = ink_bbox(&img, (0, 0, img.width(), img.height()), DEFAULT_BG).unwrap();
    assert!(bb.0 >= 9 && bb.2 <= 30, "{bb:?}");
    assert!(bb.1 >= 20 && bb.3 <= 40, "{bb:?}");
    assert!(count_color(&img, [255, 255, 255]) > 4);
}

#[test]
fn text_is_clipped_to_clip_rect() {
    let glyphs: Vec<Glyph> =
        "HHHH".chars().enumerate().map(|(i, c)| glyph(c, i as f32, 0.0, 1.0)).collect();
    let Some((_, img)) = paint(
        vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 2.0, 1.0), glyphs, Modifiers::default(), None)],
        5,
        2,
    ) else {
        return;
    };
    let bb = ink_bbox(&img, (0, 0, img.width(), img.height()), DEFAULT_BG).unwrap();
    assert!(bb.2 <= 19, "ink right edge {} past 19.2 px clip", bb.2);
}

#[test]
fn shaped_clusters_and_modifiers_render_correctly() {
    let mut run = shaped_run(FACE_SANS, 32.0);
    run.clusters = vec![(0, "ffi".into()), (1, "e\u{301}".into())];
    let glyphs = vec![glyph('f', 0.0, 1.0, 1.5), glyph('e', 5.0, 1.0, 1.0)];
    let Some((_, img)) = paint(
        vec![text(
            (0.0, 0.0),
            RectF::new(0.0, 0.0, 6.0, 3.0),
            glyphs,
            Modifiers::default(),
            Some(run),
        )],
        6,
        3,
    ) else {
        return;
    };
    let lig = ink_bbox(&img, (0, 0, 45, 60), DEFAULT_BG).expect("ligature drawn");
    let acc = ink_bbox(&img, (46, 0, 70, 60), DEFAULT_BG).expect("accented e drawn");
    assert!(lig.2 - lig.0 > 10);

    let mut plain_run = shaped_run(FACE_SANS, 32.0);
    plain_run.clusters = vec![];
    let Some((_, plain)) = paint(
        vec![text(
            (0.0, 0.0),
            RectF::new(0.0, 0.0, 6.0, 3.0),
            vec![glyph('e', 5.0, 1.0, 1.0)],
            Modifiers::default(),
            Some(plain_run),
        )],
        6,
        3,
    ) else {
        return;
    };
    let pe = ink_bbox(&plain, (46, 0, 70, 60), DEFAULT_BG).unwrap();
    assert!(acc.1 < pe.1, "accent above base: {acc:?} vs {pe:?}");

    // Bold, italic, underline
    let render_mod = |m: Modifiers| {
        let glyphs = vec![glyph('l', 0.0, 0.0, 1.0)];
        paint(vec![text((0.0, 0.0), RectF::new(0.0, 0.0, 3.0, 1.0), glyphs, m, None)], 3, 1)
            .unwrap()
            .1
    };
    let plain_l = render_mod(Modifiers::default());
    let bold_l = render_mod(Modifiers { bold: true, ..Default::default() });
    let italic_l = render_mod(Modifiers { italic: true, ..Default::default() });
    let under_l = render_mod(Modifiers { underline: true, ..Default::default() });

    let pb = ink_bbox(&plain_l, (0, 0, plain_l.width(), 18), DEFAULT_BG).unwrap();
    let bb = ink_bbox(&bold_l, (0, 0, bold_l.width(), 18), DEFAULT_BG).unwrap();
    assert!(bb.2 - bb.0 > pb.2 - pb.0, "bold is wider: {bb:?} vs {pb:?}");
    assert_ne!(plain_l.rgba(), italic_l.rgba());
    assert_eq!(under_l.pixel(0, 19), [255, 255, 255]);
    assert_eq!(plain_l.pixel(0, 19), DEFAULT_BG);
}

#[test]
fn cell_grid_paints_cells_and_skips_wide_trailers() {
    let cell = |ch, bg| GridCell { ch, fg: AnsiColor::Reset, bg, mods: Modifiers::default(), skip: false };
    let mut cells =
        vec![cell('X', RED), cell(' ', GREEN), cell('Y', AnsiColor::Reset), cell(' ', AnsiColor::Reset)];
    cells[3].skip = true;
    cells[3].bg = RED;
    let Some((_, img)) =
        paint(vec![DisplayItem::CellGrid { rect: RectF::new(0.0, 0.0, 2.0, 2.0), cells }], 2, 2)
    else {
        return;
    };
    assert_eq!(img.pixel(0, 0), RED_RGB);
    assert_eq!(img.pixel(15, 5), GREEN_RGB);
    assert_eq!(img.pixel(15, 25), DEFAULT_BG, "skipped cell untouched");
    assert!(ink_bbox(&img, (0, 20, 10, 40), DEFAULT_BG).is_some(), "'Y' has ink");
}

#[test]
fn push_clip_and_pop_clip_nest_and_clip_primitives() {
    let items = vec![
        DisplayItem::PushClip { rect: RectF::new(0.0, 0.0, 3.0, 2.0) },
        DisplayItem::PushClip { rect: RectF::new(1.0, 0.0, 3.0, 1.0) },
        fill(RectF::new(0.0, 0.0, 5.0, 3.0), 0.0, RED),
        DisplayItem::PopClip,
        fill(RectF::new(0.0, 1.0, 5.0, 1.0), 0.0, GREEN),
        DisplayItem::PopClip,
        // Extra PopClip is ignored safely.
        DisplayItem::PopClip,
    ];
    let Some((_, img)) = paint(items, 5, 3) else { return };
    // Intersection of [0..3, 0..2] and [1..4, 0..1] is [1..3, 0..1] -> x in 10..29, y in 0..20.
    assert_eq!(ink_bbox(&img, (0, 0, img.width(), 20), DEFAULT_BG), Some((10, 0, 29, 20)));
    // Second fill clipped to [0..3, 0..2] -> x in 0..29, y in 20..40.
    assert_eq!(ink_bbox(&img, (0, 20, img.width(), 40), DEFAULT_BG), Some((0, 20, 29, 40)));
}

#[test]
fn shared_gpu_context_serves_multiple_sinks() {
    let ctx = require_gpu!();
    let mut sink_a = GpuSink::new(Arc::clone(&ctx), 1.0);
    let mut sink_b = GpuSink::new(Arc::clone(&ctx), 2.0);
    let list = DisplayList { items: vec![fill(RectF::new(0.0, 0.0, 2.0, 1.0), 0.0, RED)] };
    let img_a = sink_a.render_cells(&list, 4, 2).unwrap();
    let img_b = sink_b.render_cells(&list, 4, 2).unwrap();
    assert_eq!((img_a.width(), img_a.height()), (39, 40));
    assert_eq!((img_b.width(), img_b.height()), (77, 80));
    assert_eq!(img_a.pixel(2, 2), RED_RGB);
    assert_eq!(img_b.pixel(2, 2), RED_RGB);
}

#[test]
fn untrusted_display_list_inputs_do_not_panic_or_hang() {
    let ctx = require_gpu!();
    let mut sink = GpuSink::new(ctx, 1.0);
    let ascii = BorderChars {
        top_left: '+',
        top_right: '+',
        bot_left: '+',
        bot_right: '+',
        top: '-',
        bot: '-',
        left: '|',
        right: '|',
    };
    let cell = GridCell { ch: 'A', fg: WHITE, bg: RED, mods: Modifiers::default(), skip: false };
    let mut cluster_run = shaped_run(FACE_MONO, 0.0);
    cluster_run.clusters = vec![(0, "ffi".into())];

    let items = vec![
        fill(RectF::new(f32::NAN, 0.0, 2.0, 2.0), 0.0, RED),
        fill(RectF::new(0.0, 0.0, f32::INFINITY, 2.0), 0.5, RED),
        border(RectF::new(0.0, 0.0, 1e8, 3.0), 0.0, ascii.clone(), WHITE),
        border(RectF::new(1e20, 0.0, 3.0, 3.0), 0.0, ascii, WHITE),
        text(
            (0.0, 0.0),
            RectF::new(0.0, 0.0, 10.0, 5.0),
            vec![Glyph { x: 0.0, y: f32::NAN, ch: 'A', advance: 1.0 }],
            Modifiers::default(),
            Some(shaped_run(FACE_MONO, 16.0)),
        ),
        text(
            (0.0, 0.0),
            RectF::new(0.0, 0.0, 10.0, 5.0),
            vec![Glyph { x: 0.0, y: 0.0, ch: 'M', advance: 1.0 }],
            Modifiers::default(),
            Some(shaped_run(FACE_MONO, 1e5)),
        ),
        text(
            (0.0, 0.0),
            RectF::new(0.0, 0.0, 10.0, 5.0),
            vec![Glyph { x: 0.0, y: 0.0, ch: 'f', advance: 1.0 }],
            Modifiers::default(),
            Some(cluster_run),
        ),
        text(
            (1e30, -1e30),
            RectF::new(-1e30, -1e30, 2e30, 2e30),
            vec![Glyph { x: 0.0, y: 0.0, ch: 'A', advance: 1.0 }],
            Modifiers::default(),
            None,
        ),
        DisplayItem::CellGrid {
            rect: RectF::new(0.0, 0.0, 9.223_372e18, 2.0),
            cells: vec![cell; 4],
        },
        DisplayItem::PushClip { rect: RectF::new(f32::NAN, 0.0, 10.0, 5.0) },
        fill(RectF::new(0.0, 0.0, 10.0, 5.0), 0.0, GREEN),
        DisplayItem::PopClip,
    ];
    let img = sink.render_cells(&DisplayList { items }, 10, 5).unwrap();
    // Non-finite clip failed closed so green fill did not overwrite cell (0, 0) from CellGrid.
    assert_eq!(img.pixel(0, 0), RED_RGB);

    // Invalid surface sizes return clean errors rather than panicking.
    assert!(matches!(
        sink.render(&DisplayList::new(), 0, 10),
        Err(GpuError::InvalidSurfaceSize { .. })
    ));
    assert!(matches!(
        sink.render(&DisplayList::new(), 100_000, 100_000),
        Err(GpuError::InvalidSurfaceSize { .. })
    ));
}

#[test]
fn png_round_trip_preserves_pixels() {
    let Some((_, img)) = paint(
        vec![
            fill(RectF::new(0.0, 0.0, 4.0, 2.0), 0.5, RED),
            border(RectF::new(0.0, 0.0, 4.0, 2.0), 0.0, BorderChars::rounded(), GREEN),
        ],
        5,
        3,
    ) else {
        return;
    };
    let png_bytes = img.to_png().unwrap();
    let decoded = FrameImage::from_png(&png_bytes).unwrap();
    assert_eq!(decoded, img);
}

#[test]
fn rendering_is_deterministic_across_cold_warm_and_cleared_atlas() {
    let ctx = require_gpu!();
    let list = DisplayList {
        items: vec![
            fill(RectF::new(0.0, 0.0, 12.0, 4.0), 0.5, AnsiColor::TrueColor(30, 41, 59)),
            border(RectF::new(0.0, 0.0, 12.0, 4.0), 0.0, BorderChars::double(), GREEN),
            text(
                (1.0, 1.0),
                RectF::new(1.0, 1.0, 10.0, 2.0),
                "OxiGPU 5.2"
                    .chars()
                    .enumerate()
                    .map(|(i, ch)| glyph(ch, i as f32, 0.0, 1.0))
                    .collect(),
                Modifiers { bold: true, underline: true, italic: false },
                None,
            ),
        ],
    };

    let mut sink = GpuSink::new(Arc::clone(&ctx), 1.0);
    let cold = sink.render_cells(&list, 14, 5).unwrap();
    assert!(sink.atlas().glyph_count() >= 8);
    let misses_after_cold = sink.atlas().misses();

    let warm = sink.render_cells(&list, 14, 5).unwrap();
    assert_eq!(sink.atlas().misses(), misses_after_cold, "warm frame hits glyph atlas");
    assert!(sink.atlas().hits() > 0);
    assert_eq!(cold.rgba(), warm.rgba(), "warm render differs from cold");

    sink.clear_atlas();
    let after_clear = sink.render_cells(&list, 14, 5).unwrap();
    assert_eq!(cold.rgba(), after_clear.rgba(), "post-clear render differs from cold");
}

#[test]
fn context_options_constructs_shared_context() {
    let opts = GpuContextOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        ..GpuContextOptions::default()
    };
    let ctx = match GpuContext::new_headless_with_options(&opts) {
        Ok(c) => Arc::new(c),
        Err(GpuError::NoAdapter(msg)) => {
            eprintln!("SKIP context_options_constructs_shared_context: {msg}");
            return;
        }
        Err(other) => panic!("GpuContext::new_headless_with_options failed: {other}"),
    };
    let mut sink = GpuSink::try_new(ctx, 1.0).expect("try_new");
    let img = sink
        .render_cells(&DisplayList { items: vec![fill(RectF::new(0.0, 0.0, 2.0, 1.0), 0.0, RED)] }, 4, 2)
        .expect("render");
    assert_eq!(img.pixel(2, 2), RED_RGB);
}

#[test]
fn try_new_and_render_on_lost_device_return_device_lost() {
    let ctx = match GpuContext::new_headless() {
        Ok(c) => Arc::new(c),
        Err(GpuError::NoAdapter(msg)) => {
            eprintln!("SKIP try_new_and_render_on_lost_device_return_device_lost: {msg}");
            return;
        }
        Err(other) => panic!("GpuContext init failed: {other}"),
    };
    let mut sink = GpuSink::try_new(Arc::clone(&ctx), 1.0).expect("try_new");
    let list = DisplayList { items: vec![fill(RectF::new(0.0, 0.0, 2.0, 1.0), 0.0, RED)] };
    let _ = sink.render_cells(&list, 4, 2).expect("initial render");

    let tex = ctx.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("lost-device-test"),
        size: wgpu::Extent3d { width: 32, height: 32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: oxiterm_gpu::TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());

    ctx.device().destroy();
    assert!(matches!(sink.render_cells(&list, 4, 2), Err(GpuError::DeviceLost(_))));
    assert!(matches!(sink.render_into_view(&list, &view, 32, 32), Err(GpuError::DeviceLost(_))));
    assert!(matches!(GpuSink::try_new(Arc::clone(&ctx), 1.0), Err(GpuError::DeviceLost(_))));
    let mut late_sink = GpuSink::new(ctx, 1.0);
    assert!(matches!(late_sink.render_cells(&list, 4, 2), Err(GpuError::DeviceLost(_))));
}

#[test]
fn error_scopes_isolate_destroyed_target_from_foreign_scope_and_handler() {
    let ctx = match GpuContext::new_headless() {
        Ok(c) => Arc::new(c),
        Err(GpuError::NoAdapter(msg)) => {
            eprintln!("SKIP error_scopes_isolate_destroyed_target: {msg}");
            return;
        }
        Err(other) => panic!("GpuContext init failed: {other}"),
    };
    let handler_count = Arc::new(AtomicUsize::new(0));
    let hc = Arc::clone(&handler_count);
    ctx.device().on_uncaptured_error(Arc::new(move |_err| {
        hc.fetch_add(1, Ordering::SeqCst);
    }));

    let mut sink = GpuSink::new(Arc::clone(&ctx), 1.0);
    let tex = ctx.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("destroyed-target"),
        size: wgpu::Extent3d { width: 64, height: 64, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: oxiterm_gpu::TARGET_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    // Destroy the texture so it passes metadata checks in `validate_target_view` and triggers a
    // real wgpu Validation error inside the render pass.
    tex.destroy();

    let foreign_scope = ctx.device().push_error_scope(wgpu::ErrorFilter::Validation);
    let list = DisplayList { items: vec![fill(RectF::new(0.0, 0.0, 2.0, 1.0), 0.0, RED)] };
    let res = sink.render_into_view(&list, &view, 64, 64);
    let foreign_captured = pollster::block_on(foreign_scope.pop());

    assert!(matches!(res, Err(GpuError::Validation(_))), "expected Validation error, got {res:?}");
    assert!(foreign_captured.is_none(), "sink error leaked into foreign error scope");
    assert_eq!(handler_count.load(Ordering::SeqCst), 0, "sink error leaked to uncaptured handler");
}

#[test]
fn oversized_glyphs_above_512px_are_reported_in_stats_and_atlas() {
    let ctx = require_gpu!();
    let mut sink = GpuSink::new(ctx, 3.0);
    let run = shaped_run(FACE_SANS, 175.0); // 175 * 3 = 525 px > 512 px cap
    let list = DisplayList {
        items: vec![text(
            (0.0, 0.0),
            RectF::new(0.0, 0.0, 20.0, 10.0),
            vec![glyph('W', 0.0, 0.0, 4.0)],
            Modifiers::default(),
            Some(run),
        )],
    };
    let _ = sink.render_cells(&list, 20, 10).unwrap();
    assert!(
        sink.last_stats().dropped_oversized_glyphs >= 1,
        "expected dropped_oversized_glyphs >= 1, got {:?}",
        sink.last_stats()
    );
    assert!(sink.atlas().dropped_oversized_glyphs() >= 1);
}

#[test]
fn cell_grid_and_shaped_runs_batch_per_pipeline() {
    let ctx = require_gpu!();
    let mut sink = GpuSink::new(ctx, 1.0);

    // 40x10 = 400 non-bold/non-italic cells should batch into 2 draw calls (SDF bg + Glyph),
    // not 800 alternating batches (F-08).
    let cells: Vec<GridCell> = (0..400usize)
        .map(|i| GridCell {
            ch: (b'A' + (i % 26) as u8) as char,
            fg: WHITE,
            bg: AnsiColor::TrueColor(10, 20, 40),
            mods: Modifiers::default(),
            skip: false,
        })
        .collect();
    let grid_list = DisplayList {
        items: vec![DisplayItem::CellGrid { rect: RectF::new(0.0, 0.0, 40.0, 10.0), cells }],
    };
    let _ = sink.render_cells(&grid_list, 40, 10).unwrap();
    assert!(
        sink.last_stats().draw_batches <= 2,
        "expected <= 2 batches for 400-cell CellGrid, got {:?}",
        sink.last_stats()
    );

    // 10-line shaped run with background + underline should batch into <= 3 draw calls (bg + glyphs + underline).
    let mut glyphs = Vec::new();
    for line in 0..10 {
        for (i, ch) in "BatchedLine".chars().enumerate() {
            glyphs.push(glyph(ch, i as f32 * 0.9, line as f32 * 1.5, 0.9));
        }
    }
    let mut run = shaped_run(FACE_SANS, 16.0);
    run.line_height = 1.25;
    let shaped_list = DisplayList {
        items: vec![DisplayItem::Text {
            node: NodeId(1),
            origin: PointF { x: 0.0, y: 0.0 },
            clip: RectF::new(0.0, 0.0, 40.0, 20.0),
            glyphs,
            fg: WHITE,
            bg: AnsiColor::TrueColor(10, 20, 40),
            mods: Modifiers { underline: true, ..Default::default() },
            shaped: Some(Box::new(run)),
        }],
    };
    let _ = sink.render_cells(&shaped_list, 40, 20).unwrap();
    assert!(
        sink.last_stats().draw_batches <= 3,
        "expected <= 3 batches for 10-line shaped run, got {:?}",
        sink.last_stats()
    );
}

#[test]
fn five_million_tiny_fills_render_across_multiple_passes_without_panic() {
    let ctx = require_gpu!();
    let mut sink = GpuSink::new(ctx, 1.0);
    let mut items = Vec::with_capacity(5_000_000);
    for _ in 0..4_999_999 {
        items.push(fill(RectF::new(0.0, 0.0, 0.2, 0.1), 0.0, RED));
    }
    items.push(fill(RectF::new(0.0, 0.0, 0.5, 0.5), 0.0, GREEN));
    let list = DisplayList { items };
    let img = sink.render(&list, 8, 8).expect("5M fills must render without panicking");
    let stats = sink.last_stats();
    assert_eq!(stats.sdf_instances, 5_000_000);
    assert!(stats.passes > 100, "expected multi-pass splitting, got {stats:?}");
    assert_eq!(img.pixel(1, 1), GREEN_RGB);
}
