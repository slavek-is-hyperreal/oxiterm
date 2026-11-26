//! Read-back RGBA8 image buffer, PNG encoding/decoding, and tolerance-based comparison.

use std::io::Cursor;
use std::path::Path;

use crate::color::Rgb;
use crate::context::GpuError;

/// Maximum pixels accepted when allocating or decoding an image (64 Mpx = 256 MiB RGBA8).
pub const MAX_PIXELS: u64 = 64 << 20;
/// Maximum side length accepted for a surface or PNG (matches `oxiterm-fb`).
pub const MAX_DIM: u32 = 32_768;

/// Returns `true` when `width` x `height` is non-empty and within [`MAX_DIM`] / [`MAX_PIXELS`].
pub fn size_ok(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_DIM
        && height <= MAX_DIM
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

/// Tightly packed RGBA8 image read back from the GPU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrameImage {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Summary of comparing two [`FrameImage`]s under a per-channel tolerance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDiff {
    /// Total number of pixels in the image.
    pub total_pixels: u64,
    /// Pixels where any RGBA channel differed by more than `channel_tolerance`.
    pub differing_pixels: u64,
    /// Percentage of differing pixels (`0.0..=100.0`).
    pub differing_percent: f64,
    /// Maximum absolute per-channel difference across the entire image.
    pub max_channel_diff: u8,
}

impl FrameImage {
    /// Wraps a tightly packed RGBA8 buffer (`rgba.len() == width * height * 4`).
    pub fn from_rgba(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, GpuError> {
        if !size_ok(width, height) {
            return Err(GpuError::InvalidSurfaceSize { width, height, max_dim: MAX_DIM });
        }
        let expected = (width as usize).saturating_mul(height as usize).saturating_mul(4);
        if rgba.len() != expected {
            return Err(GpuError::Readback(format!(
                "RGBA buffer size {} does not match {width}x{height} ({expected} bytes)",
                rgba.len()
            )));
        }
        Ok(Self { width, height, rgba })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// Tightly packed RGBA8 rows (`width * 4` bytes each).
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    /// RGB value of pixel `(x, y)`.
    pub fn pixel(&self, x: u32, y: u32) -> Rgb {
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        [self.rgba[i], self.rgba[i + 1], self.rgba[i + 2]]
    }

    /// Encodes the image as an RGBA8 PNG byte vector.
    pub fn to_png(&self) -> Result<Vec<u8>, GpuError> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(|e| GpuError::Png(e.to_string()))?;
            writer
                .write_image_data(&self.rgba)
                .map_err(|e| GpuError::Png(e.to_string()))?;
        }
        Ok(out)
    }

    /// Writes the image to `path` as a PNG file.
    pub fn save_png(&self, path: &Path) -> Result<(), GpuError> {
        let bytes = self.to_png()?;
        std::fs::write(path, bytes).map_err(|e| GpuError::Io(format!("{}: {e}", path.display())))
    }

    /// Decodes a PNG byte slice into an RGBA8 [`FrameImage`].
    pub fn from_png(bytes: &[u8]) -> Result<Self, GpuError> {
        // Validate IHDR dimensions before decoding to bound memory usage.
        if bytes.len() >= 24 {
            let be = |i: usize| u32::from_be_bytes([bytes[i], bytes[i + 1], bytes[i + 2], bytes[i + 3]]);
            let w = be(16);
            let h = be(20);
            if !size_ok(w, h) {
                return Err(GpuError::Png(format!("PNG dimensions {w}x{h} outside limits")));
            }
        }
        let mut decoder = png::Decoder::new(Cursor::new(bytes));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().map_err(|e| GpuError::Png(e.to_string()))?;
        let mut buf = vec![0u8; reader.output_buffer_size().unwrap_or(0)];
        let info = reader.next_frame(&mut buf).map_err(|e| GpuError::Png(e.to_string()))?;
        if !size_ok(info.width, info.height) {
            return Err(GpuError::Png(format!(
                "decoded PNG dimensions {}x{} outside limits",
                info.width, info.height
            )));
        }
        let src = &buf[..info.buffer_size()];
        let rgba = match info.color_type {
            png::ColorType::Rgba => src.to_vec(),
            png::ColorType::Rgb => {
                let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
                for px in src.as_chunks::<3>().0 {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
                out
            }
            png::ColorType::GrayscaleAlpha => {
                let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
                for px in src.as_chunks::<2>().0 {
                    out.extend_from_slice(&[px[0], px[0], px[0], px[1]]);
                }
                out
            }
            png::ColorType::Grayscale => {
                let mut out = Vec::with_capacity((info.width * info.height * 4) as usize);
                for &g in src {
                    out.extend_from_slice(&[g, g, g, 255]);
                }
                out
            }
            other @ png::ColorType::Indexed => {
                return Err(GpuError::Png(format!("unsupported PNG color type {other:?}")));
            }
        };
        Self::from_rgba(info.width, info.height, rgba)
    }

    /// Loads a PNG file from disk into a [`FrameImage`].
    pub fn load_png(path: &Path) -> Result<Self, GpuError> {
        let bytes = std::fs::read(path).map_err(|e| GpuError::Io(format!("{}: {e}", path.display())))?;
        Self::from_png(&bytes)
    }

    /// Compares two same-sized images with a per-channel tolerance.
    ///
    /// Returns `None` if dimensions differ.
    pub fn diff(&self, other: &Self, channel_tolerance: u8) -> Option<ImageDiff> {
        if self.width != other.width || self.height != other.height {
            return None;
        }
        Some(diff_rgba(&self.rgba, &other.rgba, channel_tolerance))
    }
}

/// Compares two equal-length tightly packed RGBA8 slices under `channel_tolerance`.
pub fn diff_rgba(a: &[u8], b: &[u8], channel_tolerance: u8) -> ImageDiff {
    let total_pixels = (a.len().min(b.len()) / 4) as u64;
    let mut differing_pixels = 0u64;
    let mut max_channel_diff = 0u8;

    for (pa, pb) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0) {
        let mut px_diff = 0u8;
        for k in 0..4 {
            let d = pa[k].abs_diff(pb[k]);
            if d > px_diff {
                px_diff = d;
            }
        }
        if px_diff > max_channel_diff {
            max_channel_diff = px_diff;
        }
        if px_diff > channel_tolerance {
            differing_pixels += 1;
        }
    }

    let differing_percent = if total_pixels == 0 {
        0.0
    } else {
        (differing_pixels as f64) * 100.0 / (total_pixels as f64)
    };

    ImageDiff { total_pixels, differing_pixels, differing_percent, max_channel_diff }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encode_decode_round_trip() {
        let img = FrameImage::from_rgba(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 15, 23, 42, 255,
            ],
        )
        .unwrap();
        let bytes = img.to_png().unwrap();
        let decoded = FrameImage::from_png(&bytes).unwrap();
        assert_eq!(decoded, img);
    }

    #[test]
    fn diff_rgba_counts_per_channel_tolerance() {
        let a = vec![10, 20, 30, 255, 100, 100, 100, 255];
        let b = vec![12, 19, 30, 255, 110, 100, 100, 255];
        let d = diff_rgba(&a, &b, 2);
        assert_eq!(d.total_pixels, 2);
        assert_eq!(d.differing_pixels, 1);
        assert_eq!(d.max_channel_diff, 10);
        assert!((d.differing_percent - 50.0).abs() < 1e-6);
    }
}
