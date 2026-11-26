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
            eprintln!("SKIP cpu_compare: GPU init failed ({other})");
            None
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
                    GridCell {
                        ch: '!',
                        fg: AnsiColor::Color256(46),
                        bg: AnsiColor::Reset,
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
            // Scissor clip around a fill + border
            DisplayItem::PushClip { rect: RectF::new(2.0, 15.0, 8.0, 3.0) },
            DisplayItem::Fill {
                node: NodeId(9),
                rect: RectF::new(0.0, 14.0, 20.0, 6.0),
                radius: 0.4,
                fg: AnsiColor::Reset,
                bg: AnsiColor::TrueColor(16, 185, 129),
            },
            DisplayItem::Border {
                node: NodeId(10),
                rect: RectF::new(1.0, 14.5, 12.0, 4.0),
                radius: 0.0,
                chars: BorderChars::double(),
                fg: AnsiColor::TrueColor(255, 255, 255),
                bg: AnsiColor::Reset,
            },
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
