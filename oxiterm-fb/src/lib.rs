//! OxiDE software sink: rasterizes an OxiUI `DisplayList` into an RGBA8 buffer (Phase 7.2).
//!
//! ```no_run
//! use oxiterm_fb::{page, Rasterizer};
//! let list = page::display_list("<screen><text>hi</text></screen>", None, None, 80, 24).unwrap();
//! let mut r = Rasterizer::new(1.0);
//! let mut surface = r.surface_for(80, 24).unwrap();
//! r.render(&list, &mut surface);
//! surface.save_png("out.png".as_ref()).unwrap();
//! ```
//!
//! - [`Rasterizer`]: the sink (see `raster.rs` for geometry and determinism rules).
//! - [`Surface`]: the RGBA8 buffer; PNG in and out.
//! - [`page`]: THTML source to a GUI display list.
//! - [`pixel`]: pure pixel-format conversion for scan-out backends.
//! - `device` (feature `device`): `/dev/fb0` and DRM dumb-buffer output. Never run by tests.

pub mod color;
pub mod glyphs;
pub mod page;
pub mod pixel;
pub mod raster;
pub mod surface;

#[cfg(feature = "device")]
pub mod device;

pub use raster::Rasterizer;
pub use surface::Surface;
