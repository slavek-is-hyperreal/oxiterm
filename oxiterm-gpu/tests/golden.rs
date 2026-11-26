//! Tolerance-based PNG golden tests for [`oxiterm_gpu::GpuSink`].
//!
//! Goldens live in `oxiterm-gpu/tests/golden/<name>.png`.
//! - When `OXI_BLESS_GPU=1` is set, the test writes the golden PNGs (and cleans up `.new.png`).
//! - When a golden is missing, the test writes `<name>.new.png` next to the expected path and
//!   fails with a clear message so the lead can inspect and bless it.
//! - When a golden exists, decoded RGBA pixels are compared with per-channel tolerance
//!   (`CHANNEL_TOLERANCE`) and percentage-of-differing-pixels tolerance (`MAX_DIFF_PERCENT`).
//! - When no `wgpu` adapter is available (e.g. headless CI without Vulkan/lavapipe), tests skip
//!   cleanly with a printed note.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use oxiterm_gpu::{page, FrameImage, GpuContext, GpuError, GpuSink};

/// Allowed absolute per-channel difference (out of 255) before a pixel counts as differing.
const CHANNEL_TOLERANCE: u8 = 2;
/// Allowed percentage of pixels exceeding `CHANNEL_TOLERANCE` across drivers/lavapipe versions.
const MAX_DIFF_PERCENT: f64 = 0.25;

fn shared_ctx() -> Option<Arc<GpuContext>> {
    static CTX: OnceLock<Option<Arc<GpuContext>>> = OnceLock::new();
    CTX.get_or_init(|| match GpuContext::new_headless() {
        Ok(ctx) => Some(Arc::new(ctx)),
        Err(GpuError::NoAdapter(msg)) => {
            eprintln!("SKIP golden tests: no GPU adapter available ({msg})");
            None
        }
        Err(other) => {
            panic!("golden tests: GPU context initialization failed unexpectedly: {other}");
        }
    })
    .clone()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

fn should_bless() -> bool {
    std::env::var("OXI_BLESS_GPU").as_deref() == Ok("1")
}

fn check_golden_page(name: &str, rel_page: &str) {
    let Some(ctx) = shared_ctx() else {
        eprintln!("SKIP golden {name}: no GPU adapter available");
        return;
    };

    let root = repo_root();
    let examples = root.join("examples").canonicalize().unwrap();
    let list = page::display_list_of_file(&root.join(rel_page), Some(&examples), 80, 24).unwrap();

    let mut sink = GpuSink::new(ctx, 1.0);
    let got = sink.render_cells(&list, 80, 24).unwrap();

    let dir = golden_dir();
    let golden_path = dir.join(format!("{name}.png"));
    let new_path = dir.join(format!("{name}.new.png"));

    if should_bless() {
        std::fs::create_dir_all(&dir).unwrap();
        got.save_png(&golden_path).unwrap();
        let _ = std::fs::remove_file(&new_path);
        return;
    }

    if !golden_path.exists() {
        std::fs::create_dir_all(&dir).unwrap();
        got.save_png(&new_path).unwrap();
        panic!(
            "missing golden {}; wrote candidate output to {}. Inspect it and bless with: OXI_BLESS_GPU=1 cargo test -p oxiterm-gpu --test golden",
            golden_path.display(),
            new_path.display()
        );
    }

    let want = FrameImage::load_png(&golden_path).unwrap_or_else(|e| {
        panic!("failed to load golden {}: {e}", golden_path.display());
    });
    let Some(diff) = got.diff(&want, CHANNEL_TOLERANCE) else {
        got.save_png(&new_path).unwrap();
        panic!(
            "{name}: dimensions {}x{} differ from golden {}x{}; wrote actual to {}",
            got.width(),
            got.height(),
            want.width(),
            want.height(),
            new_path.display()
        );
    };

    if diff.differing_percent > MAX_DIFF_PERCENT {
        got.save_png(&new_path).unwrap();
        panic!(
            "{name}: {}/{} pixels ({:.4}%) differed by > {CHANNEL_TOLERANCE} (max_ch_diff={}, limit {MAX_DIFF_PERCENT}%) vs {}; wrote actual to {}; re-bless with OXI_BLESS_GPU=1",
            diff.differing_pixels,
            diff.total_pixels,
            diff.differing_percent,
            diff.max_channel_diff,
            golden_path.display(),
            new_path.display()
        );
    }
}

#[test]
fn golden_index() {
    check_golden_page("index", "examples/index.thtml");
}

#[test]
fn golden_features() {
    check_golden_page("features", "examples/features.thtml");
}

#[test]
fn golden_counter() {
    check_golden_page("counter", "examples/demos/counter.thtml");
}

#[test]
fn golden_showcase() {
    check_golden_page("showcase", "examples/demos/showcase.thtml");
}

#[test]
fn golden_styles() {
    check_golden_page("styles", "examples/demos/styles.thtml");
}

#[test]
fn golden_input() {
    check_golden_page("input", "examples/demos/input.thtml");
}
