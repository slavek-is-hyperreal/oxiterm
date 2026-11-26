//! Differential correctness tests comparing [`oxiterm_gpu::GpuSink`] output against the
//! CPU reference software rasterizer (`oxiterm_fb::Rasterizer`) on the exact same `DisplayList`.
//!
//! Skips cleanly with a printed note when no `wgpu` adapter is available.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use oxiterm_fb::Rasterizer;
use oxiterm_gpu::{diff_rgba, page, GpuContext, GpuError, GpuSink};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{AnsiColor, BorderChars};
use oxiterm_renderer::render::display_list::{
    DisplayItem, DisplayList, Glyph, GridCell, Modifiers, PointF, RectF, ShapedRun, FACE_MONO,
    FACE_SANS,
};

/// Maximum per-channel difference tolerated for matching pixels (allows +/-2 for 8-bit GPU blend
/// rounding vs CPU integer blend and analytic SDF vs tiny-skia cubic arc coverage).
const CHANNEL_TOLERANCE: u8 = 4;
/// Maximum fraction of pixels allowed to exceed `CHANNEL_TOLERANCE` (strictly < 0.5%).
const MAX_DIFF_PERCENT: f64 = 0.5;

fn shared_ctx() -> Option<Arc<GpuContext>> {
    static CTX: OnceLock<Option<Arc<GpuContext>>> = OnceLock::new();
    CTX.get_or_init(|| match GpuContext::new_headless() {
        Ok(ctx) => Some(Arc::new(ctx)),
        Err(GpuError::NoAdapter(msg)) => {
            eprintln!("SKIP cpu_compare: no GPU adapter available ({msg})");
            None
        }
        Err(other) => {
            panic!("cpu_compare: GPU context initialization failed unexpectedly: {other}");
        }
    })
    .clone()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn assert_gpu_matches_cpu(label: &str, list: &DisplayList, cols: u32, rows: u32, scale: f64) {
    let Some(ctx) = shared_ctx() else {
        eprintln!("SKIP {label}: no GPU adapter available");
        return;
    };

    let mut cpu = Rasterizer::new(scale);
    let mut cpu_surf = cpu.surface_for(cols, rows).expect("cpu surface");
    cpu.render(list, &mut cpu_surf);

    let mut gpu = GpuSink::new(ctx, scale);
    let gpu_img = gpu.render_cells(list, cols, rows).expect("gpu render");

    assert_eq!(
        (gpu_img.width(), gpu_img.height()),
        (cpu_surf.width(), cpu_surf.height()),
        "{label}: dimension mismatch"
    );

    let diff = diff_rgba(gpu_img.rgba(), cpu_surf.rgba(), CHANNEL_TOLERANCE);
    eprintln!(
        "[cpu_compare] {label} ({}x{} @ {scale:.2}x): differing={}/{} ({:.4}%), max_ch_diff={}",
        gpu_img.width(),
        gpu_img.height(),
        diff.differing_pixels,
        diff.total_pixels,
        diff.differing_percent,
        diff.max_channel_diff
    );

    assert!(
        diff.differing_percent <= MAX_DIFF_PERCENT,
        "{label}: GPU vs CPU differed on {}/{} pixels ({:.4}% > {MAX_DIFF_PERCENT}%, tol={CHANNEL_TOLERANCE}, max_ch_diff={})",
        diff.differing_pixels,
        diff.total_pixels,
        diff.differing_percent,
        diff.max_channel_diff
    );
}

#[test]
fn gpu_matches_cpu_on_example_pages() {
    let root = repo_root();
    let examples = root.join("examples").canonicalize().unwrap();
    let pages = [
        "examples/index.thtml",
        "examples/features.thtml",
        "examples/demos/counter.thtml",
        "examples/demos/showcase.thtml",
        "examples/demos/styles.thtml",
        "examples/demos/input.thtml",
    ];
    for rel in pages {
        let list = page::display_list_of_file(&root.join(rel), Some(&examples), 80, 24).unwrap();
        assert_gpu_matches_cpu(rel, &list, 80, 24, 1.0);
    }
}

#[test]
fn gpu_matches_cpu_at_hidpi_scales() {
    let root = repo_root();
    let examples = root.join("examples").canonicalize().unwrap();
    let list =
        page::display_list_of_file(&root.join("examples/demos/counter.thtml"), Some(&examples), 80, 24)
            .unwrap();
    assert_gpu_matches_cpu("counter@1.5x", &list, 80, 24, 1.5);
    assert_gpu_matches_cpu("counter@2.0x", &list, 80, 24, 2.0);
}

#[test]
#[allow(clippy::too_many_lines)]
fn gpu_matches_cpu_on_synthetic_all_primitives_scene() {
    let list = DisplayList {
        items: vec![
            // Square fill
            DisplayItem::Fill {
                node: NodeId(1),
                rect: RectF::new(1.0, 1.0, 12.0, 4.0),
                radius: 0.0,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(30, 41, 59),
            },
            // Rounded fill
            DisplayItem::Fill {
                node: NodeId(2),
                rect: RectF::new(15.0, 1.0, 12.0, 4.0),
                radius: 0.8,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(56, 189, 248),
            },
            // Single, rounded, double borders
            DisplayItem::Border {
                node: NodeId(3),
                rect: RectF::new(1.0, 6.0, 10.0, 4.0),
                radius: 0.0,
                chars: BorderChars::single(),
                fg: AnsiColor::TrueColor(248, 113, 113),
                bg: AnsiColor::Reset,
            },
            DisplayItem::Border {
                node: NodeId(4),
                rect: RectF::new(12.0, 6.0, 10.0, 4.0),
                radius: 0.0,
                chars: BorderChars::rounded(),
                fg: AnsiColor::TrueColor(74, 222, 128),
                bg: AnsiColor::Reset,
            },
            DisplayItem::Border {
                node: NodeId(5),
                rect: RectF::new(23.0, 6.0, 10.0, 4.0),
                radius: 0.0,
                chars: BorderChars::double(),
                fg: AnsiColor::TrueColor(251, 191, 36),
                bg: AnsiColor::Reset,
            },
            // Border with CSS radius > 0 (rounded ring cutout + rounded stroke)
            DisplayItem::Border {
                node: NodeId(6),
                rect: RectF::new(34.0, 6.0, 10.0, 4.0),
                radius: 0.9,
                chars: BorderChars::rounded(),
                fg: AnsiColor::TrueColor(192, 132, 252),
                bg: AnsiColor::TrueColor(30, 27, 75),
            },
            // Default monospace text with bold/underline
            DisplayItem::Text {
                node: NodeId(7),
                origin: PointF { x: 2.0, y: 2.0 },
                clip: RectF::new(2.0, 2.0, 10.0, 1.0),
                glyphs: "Hello GPU!"
                    .chars()
                    .enumerate()
                    .map(|(i, ch)| Glyph { x: i as f32, y: 0.0, ch, advance: 1.0 })
                    .collect(),
                fg: AnsiColor::TrueColor(241, 245, 249),
                bg: AnsiColor::Reset,
                mods: Modifiers { bold: true, underline: true, italic: false },
                shaped: None,
            },
            // Shaped proportional text with italic, ligature cluster, and combining accent
            DisplayItem::Text {
                node: NodeId(8),
                origin: PointF { x: 2.0, y: 11.0 },
                clip: RectF::new(2.0, 11.0, 30.0, 3.0),
                glyphs: vec![
                    Glyph { x: 0.0, y: 0.0, ch: 'O', advance: 1.4 },
                    Glyph { x: 1.4, y: 0.0, ch: 'f', advance: 2.2 },
                    Glyph { x: 3.6, y: 0.0, ch: 'e', advance: 1.2 },
                ],
                fg: AnsiColor::TrueColor(250, 204, 21),
                bg: AnsiColor::TrueColor(15, 23, 42),
                mods: Modifiers { bold: false, underline: true, italic: true },
                shaped: Some(Box::new(ShapedRun {
                    face: FACE_SANS,
                    size_px: 20.0,
                    line_height: 1.25,
                    baseline: 0.95,
                    glyph_faces: vec![(0, FACE_MONO)],
                    clusters: vec![(1, "ffi".into()), (2, "e\u{301}".into())],
                })),
            },
            // CellGrid overlay
            DisplayItem::CellGrid {
                rect: RectF::new(30.0, 1.0, 4.0, 2.0),
                cells: vec![
                    GridCell {
                        ch: 'G',
                        fg: AnsiColor::TrueColor(255, 255, 255),
                        bg: AnsiColor::Color256(24),
                        mods: Modifiers::default(),
                        skip: false,
                    },
                    GridCell {
                        ch: 'P',
                        fg: AnsiColor::TrueColor(255, 255, 255),
                        bg: AnsiColor::Color256(24),
                        mods: Modifiers { bold: true, ..Default::default() },
                        skip: false,
                    },
                    GridCell {
                        ch: 'U',
                        fg: AnsiColor::TrueColor(255, 255, 255),
                        bg: AnsiColor::Color256(24),
                        mods: Modifiers::default(),
                        skip: false,
                    },
                    GridCell {
                        ch: ' ',
                        fg: AnsiColor::Reset,
                        bg: AnsiColor::Color256(24),
                        mods: Modifiers::default(),
                        skip: false,
                    },
                    GridCell {
                        ch: 'O',
                        fg: AnsiColor::Color256(46),
                        bg: AnsiColor::Reset,
                        mods: Modifiers::default(),
                        skip: false,
                    },
                    GridCell {
                        ch: 'K',
                        fg: AnsiColor::Color256(46),
                        bg: AnsiColor::Reset,
                        mods: Modifiers::default(),
                        skip: false,
                    },
                    // Wide CJK character immediately followed by its skip trailer (F-03).
                    GridCell {
                        ch: '日',
                        fg: AnsiColor::Color256(46),
                        bg: AnsiColor::TrueColor(20, 40, 80),
                        mods: Modifiers::default(),
                        skip: false,
                    },
                    GridCell {
                        ch: ' ',
                        fg: AnsiColor::Reset,
                        bg: AnsiColor::TrueColor(255, 0, 0),
                        mods: Modifiers::default(),
                        skip: true,
                    },
                ],
            },
            // Nested scissor clips (outer + fractional inner) around a fill + border (F-03).
            DisplayItem::PushClip { rect: RectF::new(2.0, 15.0, 8.0, 3.0) },
            DisplayItem::Fill {
                node: NodeId(9),
                rect: RectF::new(0.0, 14.0, 20.0, 6.0),
                radius: 0.4,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(16, 185, 129),
            },
            DisplayItem::PushClip { rect: RectF::new(3.25, 15.5, 5.5, 2.0) },
            DisplayItem::Border {
                node: NodeId(10),
                rect: RectF::new(1.0, 14.5, 12.0, 4.0),
                radius: 0.0,
                chars: BorderChars::double(),
                fg: AnsiColor::TrueColor(255, 255, 255),
                bg: AnsiColor::Reset,
            },
            DisplayItem::PopClip,
            DisplayItem::PopClip,
        ],
    };

    assert_gpu_matches_cpu("synthetic_all_primitives", &list, 46, 20, 1.0);
}

#[test]
fn color_tables_match_oxiterm_fb_exactly() {
    for idx in 0..=255u8 {
        let c = AnsiColor::Color256(idx);
        assert_eq!(oxiterm_gpu::color::fg(&c), oxiterm_fb::color::fg(&c), "fg Color256({idx})");
        assert_eq!(oxiterm_gpu::color::bg(&c), oxiterm_fb::color::bg(&c), "bg Color256({idx})");
    }
    assert_eq!(oxiterm_gpu::color::fg(&AnsiColor::Reset), oxiterm_fb::color::fg(&AnsiColor::Reset));
    assert_eq!(oxiterm_gpu::color::bg(&AnsiColor::Reset), oxiterm_fb::color::bg(&AnsiColor::Reset));
    let tc = AnsiColor::TrueColor(12, 34, 56);
    assert_eq!(oxiterm_gpu::color::fg(&tc), oxiterm_fb::color::fg(&tc));
    assert_eq!(oxiterm_gpu::color::bg(&tc), oxiterm_fb::color::bg(&tc));
}

/// Exact area coverage of pixel `[ix, ix+1] x [iy, iy+1]` by a rounded rectangle
/// `[x0, x1] x [y0, y1]` with corner radius `r`, computed via 256-slice horizontal chord integration.
fn analytic_rounded_rect_coverage(
    ix: u32,
    iy: u32,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    r: f64,
) -> f64 {
    const SLICES: usize = 256;
    let w = x1 - x0;
    let h = y1 - y0;
    if w <= 0.0 || h <= 0.0 {
        return 0.0;
    }
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    let px0 = f64::from(ix);
    let px1 = px0 + 1.0;
    let py0 = f64::from(iy);
    let py1 = py0 + 1.0;
    if px1 <= x0 || px0 >= x1 || py1 <= y0 || py0 >= y1 {
        return 0.0;
    }
    let mut sum = 0.0f64;
    for s in 0..SLICES {
        let y = py0 + (s as f64 + 0.5) / (SLICES as f64);
        if y < y0 || y > y1 {
            continue;
        }
        let inset = if r > 0.0 && y < y0 + r {
            let dy = (y0 + r) - y;
            r - (r * r - dy * dy).max(0.0).sqrt()
        } else if r > 0.0 && y > y1 - r {
            let dy = y - (y1 - r);
            r - (r * r - dy * dy).max(0.0).sqrt()
        } else {
            0.0
        };
        let xl = x0 + inset;
        let xr = x1 - inset;
        let overlap = (xr.min(px1) - xl.max(px0)).max(0.0);
        sum += overlap;
    }
    sum / (SLICES as f64)
}

fn analytic_blend(fg: [u8; 3], bg: [u8; 3], cov: f64) -> [u8; 3] {
    let mut out = [0u8; 3];
    for c in 0..3 {
        let v = f64::from(fg[c]) * cov + f64::from(bg[c]) * (1.0 - cov);
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out
}

#[test]
#[allow(clippy::too_many_lines)]
fn b08_analytic_rounded_coverage_comparison() {
    let Some(ctx) = shared_ctx() else { return };
    let bg = oxiterm_gpu::color::DEFAULT_BG;
    let fill_fg = [56u8, 189, 248];
    let stroke_fg = [74u8, 222, 128];

    // 1. Filled circles / rounded rects at multiple radii and scales.
    for &(label, rect, radius) in &[
        ("circle r1.0 (pure circle)", RectF::new(1.0, 1.0, 2.083_333, 1.0), 1.041_667_f32),
        ("fill r0.5 6x3", RectF::new(1.0, 1.0, 6.0, 3.0), 0.5),
        ("fill r0.8 12x4", RectF::new(1.0, 1.0, 12.0, 4.0), 0.8),
        ("fill r1.5 20x6", RectF::new(1.0, 1.0, 20.0, 6.0), 1.5),
    ] {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            let list = DisplayList {
                items: vec![DisplayItem::Fill {
                    node: NodeId(1),
                    rect,
                    radius,
                    fg: AnsiColor::Reset,
                    bg: AnsiColor::TrueColor(fill_fg[0], fill_fg[1], fill_fg[2]),
                }],
            };
            let cols = 24;
            let rows = 9;
            let mut cpu = Rasterizer::new(scale);
            let mut surf = cpu.surface_for(cols, rows).unwrap();
            cpu.render(&list, &mut surf);
            let mut gpu = GpuSink::new(Arc::clone(&ctx), scale);
            let img = gpu.render_cells(&list, cols, rows).unwrap();

            let (cw, ch) = gpu.cell_px();
            let x0 = f64::from(rect.x) * cw;
            let y0 = f64::from(rect.y) * ch;
            let x1 = x0 + f64::from(rect.w) * cw;
            let y1 = y0 + f64::from(rect.h) * ch;
            let r = (f64::from(radius) * cw).max(0.0).min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);

            let (mut gpu_max, mut cpu_max) = (0u8, 0u8);
            let (mut gpu_gt4, mut cpu_gt4) = (0u32, 0u32);
            let (mut gpu_gt8, mut cpu_gt8) = (0u32, 0u32);
            let (mut gpu_gt16, mut cpu_gt16) = (0u32, 0u32);
            for y in 0..img.height() {
                for x in 0..img.width() {
                    let cov = analytic_rounded_rect_coverage(x, y, x0, y0, x1, y1, r);
                    let ref_px = analytic_blend(fill_fg, bg, cov);
                    let g = img.pixel(x, y);
                    let ci = ((y * surf.width() + x) * 4) as usize;
                    let c = [surf.rgba()[ci], surf.rgba()[ci + 1], surf.rgba()[ci + 2]];
                    let gd = (0..3).map(|k| g[k].abs_diff(ref_px[k])).max().unwrap();
                    let cd = (0..3).map(|k| c[k].abs_diff(ref_px[k])).max().unwrap();
                    gpu_max = gpu_max.max(gd);
                    cpu_max = cpu_max.max(cd);
                    if gd > 4 { gpu_gt4 += 1; }
                    if gd > 8 { gpu_gt8 += 1; }
                    if gd > 16 { gpu_gt16 += 1; }
                    if cd > 4 { cpu_gt4 += 1; }
                    if cd > 8 { cpu_gt8 += 1; }
                    if cd > 16 { cpu_gt16 += 1; }
                }
            }
            eprintln!(
                "[B-08] {label:26} @{scale:.1}x | GPU vs analytic: max={gpu_max:2} (>4:{gpu_gt4:3} >8:{gpu_gt8:3} >16:{gpu_gt16:3}) | CPU vs analytic: max={cpu_max:2} (>4:{cpu_gt4:3} >8:{cpu_gt8:3} >16:{cpu_gt16:3})"
            );
            assert!(
                gpu_max <= 12 && gpu_gt16 == 0,
                "{label}@{scale}: GPU SDF exceeded analytic coverage bound (max={gpu_max}, >16={gpu_gt16})"
            );
            assert!(
                gpu_max < cpu_max,
                "{label}@{scale}: expected GPU SDF ({gpu_max}) to beat tiny-skia ({cpu_max}) vs exact analytic coverage"
            );
        }
    }

    // 2. Stroked rounded borders at multiple scales.
    for &(label, radius, chars_rounded) in &[
        ("border ╭ r0 12x4", 0.0f32, true),
        ("border ┌ r0.9 12x4", 0.9f32, false),
    ] {
        for scale in [1.0, 1.5, 2.0, 3.0] {
            let rect = RectF::new(1.0, 1.0, 12.0, 4.0);
            let list = DisplayList {
                items: vec![DisplayItem::Border {
                    node: NodeId(1),
                    rect,
                    radius,
                    chars: if chars_rounded { BorderChars::rounded() } else { BorderChars::single() },
                    fg: AnsiColor::TrueColor(stroke_fg[0], stroke_fg[1], stroke_fg[2]),
                    bg: AnsiColor::Reset,
                }],
            };
            let cols = 16;
            let rows = 7;
            let mut cpu = Rasterizer::new(scale);
            let mut surf = cpu.surface_for(cols, rows).unwrap();
            cpu.render(&list, &mut surf);
            let mut gpu = GpuSink::new(Arc::clone(&ctx), scale);
            let img = gpu.render_cells(&list, cols, rows).unwrap();

            let (cw, ch) = gpu.cell_px();
            let lw = scale.round().max(1.0);
            let off = if (lw as i64) % 2 == 1 { 0.5 } else { 0.0 };
            let snap = |v: f64| v.round() + off;
            let cx0 = (f64::from(rect.x) + 0.5) * cw;
            let cx1 = (f64::from(rect.x + rect.w) - 0.5) * cw;
            let cy0 = (f64::from(rect.y) + 0.5) * ch;
            let cy1 = (f64::from(rect.y + rect.h) - 0.5) * ch;
            let want = if radius > 0.0 { f64::from(radius) * cw } else { cw.min(ch) / 2.0 };
            let rad_pre = want.min((cx1 - cx0) / 2.0).min((cy1 - cy0) / 2.0);
            let (sx0, sy0, sx1, sy1) = (snap(cx0), snap(cy0), snap(cx1), snap(cy1));
            let r = rad_pre.min((sx1 - sx0) / 2.0).min((sy1 - sy0) / 2.0).max(0.0);
            let hw = 0.5 * lw;

            let (mut gpu_max, mut cpu_max) = (0u8, 0u8);
            let (mut gpu_gt4, mut cpu_gt4) = (0u32, 0u32);
            let (mut gpu_gt16, mut cpu_gt16) = (0u32, 0u32);
            for y in 0..img.height() {
                for x in 0..img.width() {
                    let outer = analytic_rounded_rect_coverage(
                        x, y, sx0 - hw, sy0 - hw, sx1 + hw, sy1 + hw, r + hw,
                    );
                    let inner = analytic_rounded_rect_coverage(
                        x, y, sx0 + hw, sy0 + hw, sx1 - hw, sy1 - hw, (r - hw).max(0.0),
                    );
                    let cov = (outer - inner).clamp(0.0, 1.0);
                    let ref_px = analytic_blend(stroke_fg, bg, cov);
                    let g = img.pixel(x, y);
                    let ci = ((y * surf.width() + x) * 4) as usize;
                    let c = [surf.rgba()[ci], surf.rgba()[ci + 1], surf.rgba()[ci + 2]];
                    let gd = (0..3).map(|k| g[k].abs_diff(ref_px[k])).max().unwrap();
                    let cd = (0..3).map(|k| c[k].abs_diff(ref_px[k])).max().unwrap();
                    gpu_max = gpu_max.max(gd);
                    cpu_max = cpu_max.max(cd);
                    if gd > 4 { gpu_gt4 += 1; }
                    if gd > 16 { gpu_gt16 += 1; }
                    if cd > 4 { cpu_gt4 += 1; }
                    if cd > 16 { cpu_gt16 += 1; }
                }
            }
            eprintln!(
                "[B-08] {label:26} @{scale:.1}x | GPU vs analytic: max={gpu_max:2} (>4:{gpu_gt4:3} >16:{gpu_gt16:3}) | CPU vs analytic: max={cpu_max:2} (>4:{cpu_gt4:3} >16:{cpu_gt16:3})"
            );
            assert!(
                gpu_max <= 12 && gpu_gt16 == 0,
                "{label}@{scale}: GPU rounded border exceeded analytic coverage bound (max={gpu_max}, >16={gpu_gt16})"
            );
            assert!(
                gpu_max < cpu_max,
                "{label}@{scale}: expected GPU SDF ({gpu_max}) to beat tiny-skia ({cpu_max}) vs exact analytic coverage"
            );
        }
    }
}

/// Computes worst differing pixel count across `tile x tile` blocks under `tol`.
fn worst_tile_diff(
    lhs: &[u8],
    rhs: &[u8],
    width: u32,
    height: u32,
    tol: u8,
    tile: u32,
) -> (u64, u32) {
    let mut total = 0u64;
    let mut worst = 0u32;
    for ty in 0..height.div_ceil(tile) {
        for tx in 0..width.div_ceil(tile) {
            let mut cnt = 0u32;
            for py in ty * tile..((ty + 1) * tile).min(height) {
                for px in tx * tile..((tx + 1) * tile).min(width) {
                    let idx = ((py * width + px) * 4) as usize;
                    let diff = (0..3).map(|c| lhs[idx + c].abs_diff(rhs[idx + c])).max().unwrap_or(0);
                    if diff > tol {
                        cnt += 1;
                    }
                }
            }
            total += u64::from(cnt);
            worst = worst.max(cnt);
        }
    }
    (total, worst)
}

/// F-02: Per-feature differential scenes with tight per-tile limits (<= 4 px per 16x16 tile)
/// so regressions in bold, italic, underline, clusters, wide `CellGrid`, or nested clips cannot hide
/// behind a whole-frame aggregate percentage.
#[test]
#[allow(clippy::too_many_lines)]
fn gpu_matches_cpu_per_feature_scenes_tight_tiles() {
    let Some(ctx) = shared_ctx() else { return };
    let fg = AnsiColor::TrueColor(248, 250, 252);
    let bg = AnsiColor::TrueColor(20, 30, 90);
    let mono_scene = |mods: Modifiers| {
        let glyphs = "Hello GPU! Feature WWW"
            .chars()
            .enumerate()
            .map(|(i, ch)| Glyph { x: i as f32, y: 0.0, ch, advance: 1.0 })
            .collect();
        DisplayList {
            items: vec![
                DisplayItem::Fill {
                    node: NodeId(1),
                    rect: RectF::new(0.0, 0.0, 36.0, 4.0),
                    radius: 0.0,
                    fg: AnsiColor::Reset,
                    bg,
                },
                DisplayItem::Text {
                    node: NodeId(2),
                    origin: PointF { x: 1.0, y: 1.0 },
                    clip: RectF::new(1.0, 1.0, 34.0, 1.0),
                    glyphs,
                    fg,
                    bg,
                    mods,
                    shaped: None,
                },
            ],
        }
    };
    let sans_scene = |mods: Modifiers, clusters: Vec<(u32, Box<str>)>, glyph_faces: Vec<(u32, u8)>| {
        let glyphs = vec![
            Glyph { x: 0.0, y: 0.0, ch: 'W', advance: 1.8 },
            Glyph { x: 1.8, y: 0.0, ch: 'f', advance: 2.0 },
            Glyph { x: 3.8, y: 0.0, ch: 'e', advance: 1.4 },
            Glyph { x: 5.2, y: 0.0, ch: '┌', advance: 1.2 },
        ];
        DisplayList {
            items: vec![DisplayItem::Text {
                node: NodeId(3),
                origin: PointF { x: 1.0, y: 1.0 },
                clip: RectF::new(0.0, 0.0, 36.0, 4.0),
                glyphs,
                fg,
                bg,
                mods,
                shaped: Some(Box::new(ShapedRun {
                    face: FACE_SANS,
                    size_px: 18.0,
                    line_height: 1.25,
                    baseline: 0.95,
                    glyph_faces,
                    clusters,
                })),
            }],
        }
    };
    let grid_scene = |mods: Modifiers| {
        let mut cells = Vec::new();
        for ch in "日本語ab語c".chars() {
            let wide = (ch as u32) > 0x2000;
            cells.push(GridCell { ch, fg, bg, mods, skip: false });
            if wide {
                cells.push(GridCell { ch: ' ', fg, bg, mods, skip: true });
            }
        }
        DisplayList {
            items: vec![DisplayItem::CellGrid {
                rect: RectF::new(1.0, 1.0, cells.len() as f32, 1.0),
                cells,
            }],
        }
    };
    let nested_clip_scene = DisplayList {
        items: vec![
            DisplayItem::PushClip { rect: RectF::new(1.25, 0.5, 20.5, 3.0) },
            DisplayItem::Fill {
                node: NodeId(1),
                rect: RectF::new(0.0, 0.0, 36.0, 4.0),
                radius: 0.0,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(30, 64, 175),
            },
            DisplayItem::PushClip { rect: RectF::new(3.75, 1.0, 10.25, 2.0) },
            DisplayItem::Fill {
                node: NodeId(2),
                rect: RectF::new(0.0, 0.0, 36.0, 4.0),
                radius: 0.0,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(16, 185, 129),
            },
            DisplayItem::Border {
                node: NodeId(3),
                rect: RectF::new(2.0, 0.5, 14.0, 3.0),
                radius: 0.0,
                chars: BorderChars::double(),
                fg,
                bg: AnsiColor::Reset,
            },
            DisplayItem::PopClip,
            DisplayItem::PopClip,
        ],
    };

    let cases: Vec<(&str, DisplayList)> = vec![
        ("mono plain", mono_scene(Modifiers::default())),
        ("mono bold", mono_scene(Modifiers { bold: true, ..Default::default() })),
        ("mono italic", mono_scene(Modifiers { italic: true, ..Default::default() })),
        ("mono underline", mono_scene(Modifiers { underline: true, ..Default::default() })),
        (
            "mono bold+italic+underline",
            mono_scene(Modifiers { bold: true, italic: true, underline: true }),
        ),
        ("sans plain", sans_scene(Modifiers::default(), vec![], vec![])),
        ("sans bold", sans_scene(Modifiers { bold: true, ..Default::default() }, vec![], vec![])),
        ("sans italic", sans_scene(Modifiers { italic: true, ..Default::default() }, vec![], vec![])),
        (
            "sans underline",
            sans_scene(Modifiers { underline: true, ..Default::default() }, vec![], vec![]),
        ),
        (
            "sans clusters+fallback",
            sans_scene(
                Modifiers::default(),
                vec![(1, "ffi".into()), (2, "e\u{301}".into())],
                vec![(3, FACE_MONO)],
            ),
        ),
        ("grid wide plain", grid_scene(Modifiers::default())),
        ("grid wide bold", grid_scene(Modifiers { bold: true, ..Default::default() })),
        ("grid wide italic", grid_scene(Modifiers { italic: true, ..Default::default() })),
        ("nested fractional clips", nested_clip_scene),
    ];

    for (name, list) in &cases {
        for scale in [1.0, 1.5, 2.0] {
            let mut cpu = Rasterizer::new(scale);
            let mut surf = cpu.surface_for(36, 4).unwrap();
            cpu.render(list, &mut surf);
            let mut gpu = GpuSink::new(Arc::clone(&ctx), scale);
            let img = gpu.render_cells(list, 36, 4).unwrap();
            let (total, worst_tile) =
                worst_tile_diff(img.rgba(), surf.rgba(), img.width(), img.height(), CHANNEL_TOLERANCE, 16);
            assert!(
                worst_tile <= 4,
                "{name}@{scale}: worst 16x16 tile had {worst_tile} differing px (total={total}, limit=4)"
            );
        }
    }
}

/// F-04: Forces the glyph atlas to grow (1024 -> 2048) and flush+reset multiple times mid-frame,
/// asserting `resets >= 2`, `passes > 1`, and exact CPU parity.
#[test]
fn gpu_atlas_grow_and_reset_mid_frame_matches_cpu() {
    let Some(ctx) = shared_ctx() else { return };
    let chars: Vec<char> = (33u8..=126u8).map(char::from).collect();
    let mut items = Vec::new();
    let sizes = [110.0f32, 130.0f32, 150.0f32];
    for (si, &size_px) in sizes.iter().enumerate() {
        for bold in [false, true] {
            for italic in [false, true] {
                for (ci, &ch) in chars.iter().enumerate() {
                    let col = ((ci % 10) * 14) as f32;
                    let row = ((si * 12 + (ci / 10)) % 60) as f32;
                    items.push(DisplayItem::Text {
                        node: NodeId((ci as u32) + 1),
                        origin: PointF { x: col, y: row },
                        clip: RectF::new(0.0, 0.0, 140.0, 70.0),
                        glyphs: vec![Glyph { x: 0.0, y: 0.0, ch, advance: 10.0 }],
                        fg: AnsiColor::TrueColor(
                            (80 + (ci * 5) as u8).wrapping_add(si as u8 * 40),
                            200,
                            240,
                        ),
                        bg: AnsiColor::Reset,
                        mods: Modifiers { bold, italic, underline: false },
                        shaped: Some(Box::new(ShapedRun {
                            face: if ci % 2 == 0 { FACE_SANS } else { FACE_MONO },
                            size_px,
                            line_height: size_px * 1.25 / 20.0,
                            baseline: size_px * 0.95 / 20.0,
                            glyph_faces: vec![],
                            clusters: vec![],
                        })),
                    });
                }
            }
        }
    }
    let list = DisplayList { items };
    let (cols, rows) = (140, 70);

    let mut cpu = Rasterizer::new(1.0);
    let mut surf = cpu.surface_for(cols, rows).unwrap();
    cpu.render(&list, &mut surf);

    let mut gpu = GpuSink::new(ctx, 1.0);
    let img = gpu.render_cells(&list, cols, rows).unwrap();

    assert!(
        gpu.atlas().resets() >= 2,
        "expected >= 2 mid-frame atlas resets (grow + reset), got {}",
        gpu.atlas().resets()
    );
    assert!(
        gpu.last_stats().passes > 1,
        "expected multiple render passes on atlas overflow, got {:?}",
        gpu.last_stats()
    );
    let diff = diff_rgba(img.rgba(), surf.rgba(), CHANNEL_TOLERANCE);
    eprintln!(
        "[cpu_compare] atlas_mid_frame_overflow: size={}, resets={}, passes={}, differing={}/{} (max_ch_diff={})",
        gpu.atlas().size(),
        gpu.atlas().resets(),
        gpu.last_stats().passes,
        diff.differing_pixels,
        diff.total_pixels,
        diff.max_channel_diff
    );
    assert_eq!(
        diff.differing_pixels, 0,
        "mid-frame atlas grow/reset diverged from CPU on {} pixels (max_ch_diff={})",
        diff.differing_pixels, diff.max_channel_diff
    );
}
