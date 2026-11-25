//! The pixel buffer the rasterizer paints into.

use tiny_skia::Pixmap;

use crate::color::Rgb;

/// Most pixels a surface may hold (64 Mpx = 256 MiB of RGBA).
pub const MAX_PIXELS: u64 = 64 << 20;
/// Longest side of a surface in px. The rasterizer's coordinate margin must exceed this.
pub const MAX_DIM: u32 = 32_768;

/// `true` when a `width` x `height` surface is within the caps (and non-empty).
pub fn size_ok(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && width <= MAX_DIM && height <= MAX_DIM && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// An opaque RGBA8 surface. Every pixel the rasterizer writes has alpha 255, so the premultiplied
/// storage of the backing pixmap is byte-identical to straight RGBA.
pub struct Surface {
    pub(crate) pixmap: Pixmap,
}

impl Surface {
    /// A surface of `width` x `height` px, cleared to opaque black. `None` for a zero size or one
    /// over [`MAX_PIXELS`] / [`MAX_DIM`] (checked before anything is allocated).
    pub fn new(width: u32, height: u32) -> Option<Self> {
        if !size_ok(width, height) {
            return None;
        }
        let mut pixmap = Pixmap::new(width, height)?;
        pixmap.fill(tiny_skia::Color::BLACK);
        Some(Self { pixmap })
    }

    pub fn width(&self) -> u32 {
        self.pixmap.width()
    }

    pub fn height(&self) -> u32 {
        self.pixmap.height()
    }

    /// Tightly packed RGBA8 rows (`width * 4` bytes each).
    pub fn rgba(&self) -> &[u8] {
        self.pixmap.data()
    }

    /// Fills every pixel with `c`.
    pub fn clear(&mut self, c: Rgb) {
        self.pixmap.fill(tiny_skia::Color::from_rgba8(c[0], c[1], c[2], 255));
    }

    /// The pixel at (`x`, `y`) as RGB.
    pub fn pixel(&self, x: u32, y: u32) -> Rgb {
        let i = ((y * self.width() + x) * 4) as usize;
        let d = self.pixmap.data();
        [d[i], d[i + 1], d[i + 2]]
    }

    /// PNG bytes of the surface.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        self.pixmap.encode_png().map_err(|e| e.to_string())
    }

    /// Writes the surface as a PNG file.
    pub fn save_png(&self, path: &std::path::Path) -> Result<(), String> {
        std::fs::write(path, self.to_png()?).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Decodes a PNG into a surface (goldens).
    pub fn from_png(bytes: &[u8]) -> Result<Self, String> {
        // IHDR: 8-byte signature, 4-byte length, "IHDR", then width and height (big endian).
        if bytes.len() >= 24 {
            let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            if !size_ok(be(16), be(20)) {
                return Err("PNG size outside the surface caps".into());
            }
        }
        let pixmap = Pixmap::decode_png(bytes).map_err(|e| e.to_string())?;
        if !size_ok(pixmap.width(), pixmap.height()) {
            return Err("PNG size outside the surface caps".into());
        }
        Ok(Self { pixmap })
    }
}
