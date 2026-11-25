//! Pixel-format conversion from the rasterizer's RGBA8 to what a scan-out buffer wants. Pure
//! functions on byte slices (no device access), unit-tested; the device backends call them.

/// One colour channel of a packed pixel (Linux `fb_bitfield` without `msb_right`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BitField {
    pub offset: u32,
    pub length: u32,
}

/// Packed little-endian pixel layout of 16, 24 or 32 bits per pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelFormat {
    pub bpp: u32,
    pub red: BitField,
    pub green: BitField,
    pub blue: BitField,
    /// Length 0 = no alpha channel (the bits stay 0, which fbdev ignores).
    pub transp: BitField,
}

impl PixelFormat {
    /// `XRGB8888` (DRM `DRM_FORMAT_XRGB8888`, the usual fbdev 32 bpp layout): B, G, R, X in memory.
    pub const XRGB8888: PixelFormat = PixelFormat {
        bpp: 32,
        red: BitField { offset: 16, length: 8 },
        green: BitField { offset: 8, length: 8 },
        blue: BitField { offset: 0, length: 8 },
        transp: BitField { offset: 24, length: 0 },
    };
    /// `RGB565`.
    pub const RGB565: PixelFormat = PixelFormat {
        bpp: 16,
        red: BitField { offset: 11, length: 5 },
        green: BitField { offset: 5, length: 6 },
        blue: BitField { offset: 0, length: 5 },
        transp: BitField { offset: 0, length: 0 },
    };

    pub fn bytes_per_pixel(&self) -> usize {
        (self.bpp as usize).div_ceil(8)
    }

    /// `true` for layouts this module can write: 16, 24 or 32 bpp, colour channels of 1..=8 bits
    /// inside the pixel word, and (when present) an alpha field inside the word too.
    pub fn is_supported(&self) -> bool {
        let inside = |f: &BitField| f.offset.checked_add(f.length).is_some_and(|end| end <= self.bpp);
        matches!(self.bpp, 16 | 24 | 32)
            && [self.red, self.green, self.blue].iter().all(|f| f.length > 0 && f.length <= 8 && inside(f))
            && (self.transp.length == 0 || (self.transp.length <= 32 && inside(&self.transp)))
    }

    fn channel(v: u8, f: BitField) -> u32 {
        let v = u32::from(v) >> (8 - f.length.min(8));
        v.checked_shl(f.offset).unwrap_or(0)
    }

    /// The packed pixel word of an opaque colour. Total: a field that does not fit the word
    /// contributes nothing (use [`PixelFormat::is_supported`] to reject such formats first).
    pub fn pack(&self, r: u8, g: u8, b: u8) -> u32 {
        let mut w = Self::channel(r, self.red) | Self::channel(g, self.green) | Self::channel(b, self.blue);
        if self.transp.length > 0 {
            let ones = if self.transp.length >= 32 { u32::MAX } else { (1u32 << self.transp.length) - 1 };
            w |= ones.checked_shl(self.transp.offset).unwrap_or(0);
        }
        w
    }
}

/// Converts one row of RGBA8 (`src`, 4 bytes per pixel) into `dst` (`bytes_per_pixel` per pixel,
/// little-endian), for `min(src pixels, dst pixels)` pixels. Returns the pixel count written.
pub fn convert_row(fmt: &PixelFormat, src: &[u8], dst: &mut [u8]) -> usize {
    let bpp = fmt.bytes_per_pixel();
    let n = (src.len() / 4).min(dst.len() / bpp);
    for (s, d) in src.chunks_exact(4).zip(dst.chunks_exact_mut(bpp)).take(n) {
        let w = fmt.pack(s[0], s[1], s[2]).to_le_bytes();
        d.copy_from_slice(&w[..bpp]);
    }
    n
}

/// Converts a `width` x `height` RGBA8 image into a destination of `dst_stride` bytes per row,
/// cropping to `dst_w` x `dst_h` pixels (anything beyond the source is left untouched). A short
/// `rgba` buffer or an unsupported format converts nothing.
pub fn convert_image(
    fmt: &PixelFormat,
    rgba: &[u8],
    width: usize,
    height: usize,
    dst: &mut [u8],
    dst_stride: usize,
    dst_w: usize,
    dst_h: usize,
) {
    let bpp = fmt.bytes_per_pixel();
    // `rgba` must really hold width * height pixels; otherwise nothing is converted.
    let Some(need) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else { return };
    if rgba.len() < need || !fmt.is_supported() {
        return;
    }
    let cols = dst_w.min(width);
    for y in 0..height.min(dst_h) {
        let src = &rgba[y * width * 4..(y * width + cols) * 4];
        let Some(start) = y.checked_mul(dst_stride) else { break };
        let Some(end) = cols.checked_mul(bpp).and_then(|n| start.checked_add(n)) else { break };
        let Some(row) = dst.get_mut(start..end) else { break };
        convert_row(fmt, src, row);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xrgb8888_is_bgrx_in_memory() {
        let mut out = [0u8; 4];
        convert_row(&PixelFormat::XRGB8888, &[0x11, 0x22, 0x33, 0xff], &mut out);
        assert_eq!(out, [0x33, 0x22, 0x11, 0x00]);
    }

    #[test]
    fn rgb565_truncates_channels() {
        let mut out = [0u8; 2];
        convert_row(&PixelFormat::RGB565, &[0xff, 0x00, 0xff, 0xff], &mut out);
        // r=31<<11, g=0, b=31 -> 0xF81F little-endian
        assert_eq!(out, [0x1f, 0xf8]);
        convert_row(&PixelFormat::RGB565, &[0xff, 0xff, 0xff, 0xff], &mut out);
        assert_eq!(u16::from_le_bytes(out), 0xffff);
    }

    #[test]
    fn argb8888_sets_alpha_ones_and_24bpp_has_three_bytes() {
        let argb = PixelFormat { transp: BitField { offset: 24, length: 8 }, ..PixelFormat::XRGB8888 };
        assert_eq!(argb.pack(1, 2, 3), 0xff01_0203);
        let rgb24 = PixelFormat { bpp: 24, transp: BitField { offset: 0, length: 0 }, ..PixelFormat::XRGB8888 };
        let mut out = [0u8; 6];
        assert_eq!(convert_row(&rgb24, &[1, 2, 3, 255, 4, 5, 6, 255], &mut out), 2);
        assert_eq!(out, [3, 2, 1, 6, 5, 4]);
    }

    #[test]
    fn image_honours_stride_and_crops() {
        // 3x2 source, 2x2 destination crop, stride 12 bytes (padding must stay untouched).
        let src: Vec<u8> = (0..6u8).flat_map(|i| [i, i, i, 255]).collect();
        let mut dst = vec![0xAAu8; 24];
        convert_image(&PixelFormat::XRGB8888, &src, 3, 2, &mut dst, 12, 2, 2);
        assert_eq!(&dst[0..8], &[0, 0, 0, 0, 1, 1, 1, 0]);
        assert_eq!(&dst[8..12], &[0xAA; 4], "padding untouched");
        assert_eq!(&dst[12..20], &[3, 3, 3, 0, 4, 4, 4, 0]);
    }

    #[test]
    fn hostile_layouts_never_panic() {
        let f = PixelFormat { transp: BitField { offset: 40, length: 8 }, ..PixelFormat::XRGB8888 };
        assert!(!f.is_supported());
        let _ = f.pack(1, 2, 3);
        let g = PixelFormat { red: BitField { offset: u32::MAX, length: 8 }, ..PixelFormat::XRGB8888 };
        assert!(!g.is_supported());
        let _ = g.pack(1, 2, 3);
        let mut dst = [0u8; 16];
        convert_image(&PixelFormat::XRGB8888, &[0u8; 8], 4, 4, &mut dst, 16, 4, 4);
        assert_eq!(dst, [0u8; 16]);
    }

    #[test]
    fn unsupported_layouts_are_rejected() {
        assert!(PixelFormat::XRGB8888.is_supported());
        assert!(PixelFormat::RGB565.is_supported());
        assert!(!PixelFormat { bpp: 8, ..PixelFormat::XRGB8888 }.is_supported());
    }
}
