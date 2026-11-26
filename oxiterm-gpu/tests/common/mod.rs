//! Shared skip/require logic for the `oxiterm-gpu` integration tests.
//!
//! Default (developer machine): a missing GPU adapter makes a test print `SKIP ...` and pass.
//! With `OXI_GPU_REQUIRED=1` (set by the `gpu` CI job) a missing adapter is a hard failure, so a
//! green CI run proves the GPU code really ran. Only `GpuError::NoAdapter` is ever skipped; any
//! other context error is always a failure.
#![allow(dead_code)]

use oxiterm_gpu::{GpuContext, GpuError};

/// True when `OXI_GPU_REQUIRED` is `1` or `true` (case-insensitive).
pub fn gpu_required() -> bool {
    required_from(std::env::var("OXI_GPU_REQUIRED").ok().as_deref())
}

pub fn required_from(value: Option<&str>) -> bool {
    matches!(value.map(|v| v.trim().to_ascii_lowercase()).as_deref(), Some("1") | Some("true"))
}

/// Turn a context-creation result into `Some(ctx)`, a printed skip (`None`), or a panic.
pub fn ctx_or_skip(label: &str, result: Result<GpuContext, GpuError>) -> Option<GpuContext> {
    match result {
        Ok(ctx) => Some(ctx),
        Err(GpuError::NoAdapter(msg)) => {
            skip_or_fail(label, &format!("no GPU adapter available ({msg})"));
            None
        }
        Err(other) => panic!("{label}: GPU context initialization failed unexpectedly: {other}"),
    }
}

/// Skip with a note, or panic when `OXI_GPU_REQUIRED` is set.
pub fn skip_or_fail(label: &str, reason: &str) {
    if gpu_required() {
        panic!(
            "{label}: {reason}. OXI_GPU_REQUIRED is set, so skipping is not allowed. \
             Install a Vulkan software driver (Mesa lavapipe) and set VK_ICD_FILENAMES, \
             or unset OXI_GPU_REQUIRED."
        );
    }
    eprintln!("SKIP {label}: {reason}");
}
