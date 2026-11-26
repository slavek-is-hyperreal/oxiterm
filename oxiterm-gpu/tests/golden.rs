//! Tolerance-based PNG golden tests for [`oxiterm_gpu::GpuSink`].
//!
//! Goldens live in `oxiterm-gpu/tests/golden/<name>.png`.
//! - When `OXI_BLESS_GPU=1` is set, the test writes the golden PNGs (and cleans up `.new.png`).
//! - When a golden is missing, the test writes `<name>.new.png` next to the expected path and
//!   fails with a clear message so the lead can inspect and bless it.
//! - When a golden exists, decoded RGBA pixels are compared with per-channel tolerance
//!   (`CHANNEL_TOLERANCE`) and percentage-of-differing-pixels tolerance (`MAX_DIFF_PERCENT`).
//! - When no `wgpu` adapter is available, or the adapter is not the llvmpipe build the goldens were
//!   blessed on, tests skip with a printed note. With `OXI_GPU_REQUIRED=1` (CI) a missing adapter
//!   or a non-llvmpipe adapter is a hard failure; a different llvmpipe version still skips (see
//!   `docs/stages/5.2/ci.md`).

mod common;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use oxiterm_gpu::{page, FrameImage, GpuContext, GpuSink};

/// Allowed absolute per-channel difference (out of 255) before a pixel counts as differing.
const CHANNEL_TOLERANCE: u8 = 2;
/// Allowed percentage of pixels exceeding `CHANNEL_TOLERANCE` across drivers/lavapipe versions.
const MAX_DIFF_PERCENT: f64 = 0.25;

/// Blessed driver identity: the goldens were produced on Mesa llvmpipe (lavapipe) with this
/// LLVM major and Mesa major.minor. SRU patch bumps (25.2.8 -> 25.2.9) do not change the gate.
const BLESSED_LLVM_MAJOR: u32 = 20;
const BLESSED_MESA: (u32, u32) = (25, 2);
/// llvmpipe builds on which the blessed goldens were shown to match within the tolerance:
/// the owner's machine (LLVM 20 / Mesa 25.2) and GitHub's ubuntu-latest (LLVM 21 / Mesa 26.0,
/// CI run 45781692407, advisory step: 6/6 goldens within tolerance). Add a pair only after such
/// evidence; any other build skips loudly and the advisory CI step compares anyway.
const VERIFIED_LLVMPIPE: [(u32, (u32, u32)); 2] = [(BLESSED_LLVM_MAJOR, BLESSED_MESA), (21, (26, 0))];

#[derive(Debug, PartialEq, Eq)]
enum Gate {
    Run,
    Skip(String),
    Fail(String),
}

/// First integer that follows `marker` in `s` (`"LLVM 20.1.2"` with `"LLVM "` gives 20).
fn number_after(s: &str, marker: &str) -> Option<u32> {
    let rest = &s[s.find(marker)? + marker.len()..];
    let end = rest.find(|c: char| !c.is_ascii_digit()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

/// `"Mesa 25.2.8-0ubuntu0.24.04.5"` gives `(25, 2)`.
fn mesa_major_minor(driver_info: &str) -> Option<(u32, u32)> {
    let major = number_after(driver_info, "Mesa ")?;
    let tail = &driver_info[driver_info.find("Mesa ")? + 5..];
    let minor = number_after(tail, &format!("{major}."))?;
    Some((major, minor))
}

/// Decide whether the PNG comparison may run on this adapter. Pure, so it is unit-tested below.
/// `any_version` is the `OXI_GPU_GOLDEN_ANY_LLVM=1` override (force the comparison on a
/// different llvmpipe build, for inspection).
fn golden_gate(name: &str, driver_info: &str, required: bool, any_version: bool) -> Gate {
    if !name.to_ascii_lowercase().contains("llvmpipe") {
        let why = format!(
            "goldens are blessed on llvmpipe, this adapter is {name:?} (driver {driver_info:?})"
        );
        return if required { Gate::Fail(why) } else { Gate::Skip(why) };
    }
    let llvm = number_after(name, "LLVM ");
    let mesa = mesa_major_minor(driver_info);
    if any_version || VERIFIED_LLVMPIPE.iter().any(|&(l, m)| llvm == Some(l) && mesa == Some(m)) {
        return Gate::Run;
    }
    Gate::Skip(format!(
        "goldens are verified on llvmpipe {VERIFIED_LLVMPIPE:?} (LLVM major, Mesa major.minor); this is \
         {name:?} (driver {driver_info:?}). Set OXI_GPU_GOLDEN_ANY_LLVM=1 to compare anyway. The \
         CPU-vs-GPU differential tests still ran."
    ))
}

fn shared_ctx() -> Option<Arc<GpuContext>> {
    static CTX: OnceLock<Option<Arc<GpuContext>>> = OnceLock::new();
    CTX.get_or_init(|| {
        let ctx = common::ctx_or_skip("golden tests", GpuContext::new_headless())?;
        let info = ctx.adapter_info();
        eprintln!(
            "golden tests adapter: name={:?} driver={:?} driver_info={:?} backend={:?}",
            info.name, info.driver, info.driver_info, info.backend
        );
        let any = std::env::var("OXI_GPU_GOLDEN_ANY_LLVM").as_deref() == Ok("1");
        let gate = golden_gate(
            &info.name,
            &format!("{} {}", info.driver, info.driver_info),
            common::gpu_required(),
            any,
        );
        match gate {
            Gate::Run => Some(Arc::new(ctx)),
            Gate::Skip(why) => {
                eprintln!("SKIP golden tests: {why}");
                if let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") {
                    use std::io::Write;
                    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(path) {
                        let _ = writeln!(f, "**oxiterm-gpu goldens SKIPPED:** {why}\n");
                    }
                }
                None
            }
            Gate::Fail(why) => panic!("golden tests: {why}; OXI_GPU_REQUIRED is set"),
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
        eprintln!("SKIP golden {name}: see the earlier note (no adapter, or goldens gated off)");
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

#[cfg(test)]
mod gate_tests {
    use super::*;

    const LP: &str = "llvmpipe (LLVM 20.1.2, 256 bits)";
    const MESA: &str = "Mesa 25.2.8-0ubuntu0.24.04.5 (LLVM 20.1.2)";

    #[test]
    fn blessed_llvmpipe_runs_and_patch_bumps_do_not_change_it() {
        assert_eq!(golden_gate(LP, MESA, true, false), Gate::Run);
        assert_eq!(golden_gate("llvmpipe (LLVM 20.1.8, 256 bits)", "Mesa 25.2.9-1", true, false), Gate::Run);
        // GitHub ubuntu-latest, verified in CI run 45781692407.
        assert_eq!(
            golden_gate("llvmpipe (LLVM 21.1.8, 256 bits)", "llvmpipe Mesa 26.0.8-1ubuntu0.3 (LLVM 21.1.8)", true, false),
            Gate::Run
        );
        // An unverified pairing (LLVM 21 with Mesa 25.2) still skips.
        assert!(matches!(golden_gate("llvmpipe (LLVM 21.1.8, 256 bits)", "Mesa 25.2.8", true, false), Gate::Skip(_)));
    }

    #[test]
    fn other_llvmpipe_version_skips_even_when_required() {
        assert!(matches!(golden_gate("llvmpipe (LLVM 19.1.1, 256 bits)", MESA, true, false), Gate::Skip(_)));
        assert!(matches!(golden_gate(LP, "Mesa 25.0.7-0ubuntu0.24.04.2", true, false), Gate::Skip(_)));
        assert!(matches!(golden_gate(LP, "unparsable", false, false), Gate::Skip(_)));
    }

    #[test]
    fn any_version_override_forces_the_comparison() {
        assert_eq!(golden_gate("llvmpipe (LLVM 19.1.1, 256 bits)", "Mesa 25.0.7", true, true), Gate::Run);
    }

    #[test]
    fn non_llvmpipe_adapter_skips_by_default_and_fails_when_required() {
        let name = "AMD Radeon RX 7900 XTX (RADV NAVI31)";
        assert!(matches!(golden_gate(name, "Mesa 25.2.8", false, false), Gate::Skip(_)));
        assert!(matches!(golden_gate(name, "Mesa 25.2.8", true, false), Gate::Fail(_)));
        assert!(matches!(golden_gate(name, "Mesa 25.2.8", true, true), Gate::Fail(_)));
    }

    #[test]
    fn required_switch_parsing() {
        assert!(common::required_from(Some("1")));
        assert!(common::required_from(Some("TRUE")));
        assert!(!common::required_from(Some("0")));
        assert!(!common::required_from(Some("")));
        assert!(!common::required_from(None));
    }
}
