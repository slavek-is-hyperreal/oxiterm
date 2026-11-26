//! `OxiUI` headless GPU sink on `wgpu` (Phase 5.2).
//!
//! Renders an `OxiUI` [`oxiterm_renderer::render::display_list::DisplayList`] into an RGBA8
//! [`FrameImage`] (and PNG) using a custom GLES 3.0 / WebGL2-level pipeline (instanced SDF
//! rounded-rect / border quads, instanced `etagere` glyph-atlas quads, and hardware scissor
//! clips; vertex + fragment shaders only, no compute shaders).
//!
//! The `wgpu` instance, adapter, device, and queue live in a separate shared [`GpuContext`] so
//! future `OxiScript` GPU kernels (Stage 8.3b), 3D rendering (Stage 9.3), and window surfaces
//! (Stage 5.3) share one GPU device without CPU round-trips.
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxiterm_gpu::{page, GpuContext, GpuSink};
//!
//! let ctx = Arc::new(GpuContext::new_headless().expect("GPU adapter"));
//! let list = page::display_list("<box><text>Hello GPU</text></box>", None, None, 80, 24).unwrap();
//! let mut sink = GpuSink::new(ctx, 1.0);
//! let frame = sink.render_cells(&list, 80, 24).unwrap();
//! frame.save_png("out.png".as_ref()).unwrap();
//! ```

pub mod atlas;
pub mod color;
pub mod context;
pub mod image;
pub mod page;
pub mod sink;

pub use atlas::GlyphAtlas;
pub use context::{GpuContext, GpuContextOptions, GpuError};
pub use image::{diff_rgba, FrameImage, ImageDiff};
pub use sink::{FrameStats, GpuSink, TARGET_FORMAT};
