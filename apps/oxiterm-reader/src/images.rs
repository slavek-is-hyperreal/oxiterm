//! Image pipeline: size-capped download -> decode-bomb guard -> re-encode to PNG -> cell sizing.
//!
//! Only PNG/JPEG/GIF/WebP are decoded (SVG and everything else is rejected). The stored
//! file is a fresh PNG produced by us, so metadata and format quirks of the original never
//! reach the renderer.

use crate::fetch::{Budget, Kind, SafeFetcher};
use crate::model::{Block, LocalImage};
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;
use std::path::Path;
use url::Url;

/// Largest accepted source edge.
pub const MAX_EDGE: u32 = 8000;
/// Largest accepted source area in pixels (40 MP).
pub const MAX_PIXELS: u64 = 40_000_000;
/// Smaller images are decoration/tracking and skipped.
pub const MIN_EDGE: u32 = 16;
/// Stored images are downscaled to fit this box (px).
pub const STORE_MAX_EDGE: u32 = 800;
/// Renderer assumption: one cell is 10 x 20 px (1:2).
pub const CELL_W_PX: u32 = 10;
/// See [`CELL_W_PX`].
pub const CELL_H_PX: u32 = 20;

/// Why an image was rejected.
#[derive(Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not PNG/JPEG/GIF/WebP (includes SVG).
    UnsupportedFormat,
    /// Edge or pixel count over the guard.
    TooBig,
    /// Smaller than [`MIN_EDGE`].
    TooSmall,
    /// Decoder failure.
    Corrupt,
}

/// Cells needed for a `px_w` x `px_h` image: natural size (10x20 px cells), shrunk to
/// `max_cols` x `max_rows` keeping the aspect ratio.
pub fn cell_size(px_w: u32, px_h: u32, max_cols: u16, max_rows: u16) -> (u16, u16) {
    let (pw, ph) = (px_w.max(1) as f64, px_h.max(1) as f64);
    let mut cols = (pw / CELL_W_PX as f64).round().clamp(2.0, max_cols as f64);
    // A cell is twice as tall as wide, so rows = cols * h / (2 * w).
    let mut rows = (cols * ph / (2.0 * pw)).round().max(1.0);
    if rows > max_rows as f64 {
        rows = max_rows as f64;
        cols = (rows * 2.0 * pw / ph).round().clamp(2.0, max_cols as f64);
    }
    (cols as u16, rows.max(1.0) as u16)
}

/// Validates and re-encodes. Returns `(png bytes, stored width, stored height, source width, source height)`.
pub fn reencode(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32), ImageError> {
    let probe = || ImageReader::new(Cursor::new(bytes)).with_guessed_format().map_err(|_| ImageError::Corrupt);
    let reader = probe()?;
    match reader.format() {
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP) => {}
        _ => return Err(ImageError::UnsupportedFormat),
    }
    let (w, h) = reader.into_dimensions().map_err(|_| ImageError::Corrupt)?;
    if w > MAX_EDGE || h > MAX_EDGE || (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(ImageError::TooBig);
    }
    if w < MIN_EDGE || h < MIN_EDGE {
        return Err(ImageError::TooSmall);
    }
    let mut reader = probe()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_EDGE);
    limits.max_image_height = Some(MAX_EDGE);
    limits.max_alloc = Some(320 * 1024 * 1024);
    reader.limits(limits);
    let img = reader.decode().map_err(|_| ImageError::Corrupt)?;
    let img = if w > STORE_MAX_EDGE || h > STORE_MAX_EDGE { img.thumbnail(STORE_MAX_EDGE, STORE_MAX_EDGE) } else { img };
    let rgba = img.to_rgba8();
    let (sw, sh) = (rgba.width(), rgba.height());
    let mut out = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(rgba).write_to(&mut out, ImageFormat::Png).map_err(|_| ImageError::Corrupt)?;
    Ok((out.into_inner(), sw, sh))
}

/// Downloads, validates and stores up to `max_images` images referenced by `blocks`.
/// Failures leave `local = None` (the renderer then shows the alt text).
pub fn process_images(
    blocks: &mut [Block],
    fetcher: &SafeFetcher,
    budget: &Budget,
    cache_dir: &Path,
    ids: &crate::store::Ids,
    max_images: usize,
    max_cols: u16,
) {
    let idx: Vec<usize> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| matches!(b, Block::Image { src, .. } if !src.is_empty()))
        .map(|(i, _)| i)
        .take(max_images)
        .collect();
    // Download in small parallel batches, decode sequentially (decode memory is the risk).
    for batch in idx.chunks(4) {
        let results: Vec<(usize, Option<Vec<u8>>)> = std::thread::scope(|s| {
            let handles: Vec<_> = batch
                .iter()
                .map(|&i| {
                    let src = match &blocks[i] {
                        Block::Image { src, .. } => src.clone(),
                        _ => String::new(),
                    };
                    s.spawn(move || {
                        let bytes = Url::parse(&src)
                            .ok()
                            .and_then(|u| fetcher.fetch(&u, Kind::Image, budget).ok())
                            .map(|f| f.body);
                        (i, bytes)
                    })
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for (i, bytes) in results {
            let Some(bytes) = bytes else { continue };
            let Ok((png, w, h)) = reencode(&bytes) else { continue };
            let file = format!("{}.png", ids.hash_bytes(&png));
            let path = cache_dir.join(&file);
            if !path.exists() && crate::store::write_atomic(&path, &png).is_err() {
                continue;
            }
            let (cols, rows) = cell_size(w, h, max_cols, 22);
            if let Block::Image { local, .. } = &mut blocks[i] {
                *local = Some(LocalImage { file, cols, rows });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([10, 20, 30, 255]));
        let mut c = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img).write_to(&mut c, ImageFormat::Png).unwrap();
        c.into_inner()
    }

    #[test]
    fn sizing_math_keeps_aspect_on_1_to_2_cells() {
        // 400x200 px: natural 40 cols; rows = 40*200/(2*400) = 10.
        assert_eq!(cell_size(400, 200, 74, 22), (40, 10));
        // Wide image shrinks to the column cap: 1000x500 -> 74 cols x 18 rows (37 -> round 18.5 -> 19? check rule).
        let (c, r) = cell_size(1000, 500, 74, 40);
        assert_eq!(c, 74);
        assert_eq!(r, (74.0f64 * 500.0 / 2000.0).round() as u16);
        // Tall image: row cap binds and columns are recomputed.
        let (c, r) = cell_size(200, 2000, 74, 20);
        assert_eq!(r, 20);
        assert_eq!(c, (20.0f64 * 2.0 * 200.0 / 2000.0).round() as u16);
        // Never zero.
        let (c, r) = cell_size(16, 16, 74, 22);
        assert!(c >= 2 && r >= 1);
        // Square 200x200 px is 20 cols x 10 rows (cells are 10x20 px).
        assert_eq!(cell_size(200, 200, 74, 22), (20, 10));
    }

    #[test]
    fn reencode_roundtrip_and_downscale() {
        let (out, w, h) = reencode(&png(1600, 400)).unwrap();
        assert_eq!((w, h), (800, 200));
        assert_eq!(&out[..8], b"\x89PNG\r\n\x1a\n");
        let (_, w, h) = reencode(&png(100, 50)).unwrap();
        assert_eq!((w, h), (100, 50));
    }

    #[test]
    fn rejects_svg_garbage_and_tiny() {
        assert_eq!(reencode(b"<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'/>").unwrap_err(), ImageError::UnsupportedFormat);
        assert_eq!(reencode(b"hello world, definitely not an image").unwrap_err(), ImageError::UnsupportedFormat);
        assert_eq!(reencode(&png(4, 4)).unwrap_err(), ImageError::TooSmall);
        let mut bad = png(100, 100);
        bad.truncate(60);
        assert!(reencode(&bad).is_err());
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut c = 0xFFFF_FFFFu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }

    /// A PNG whose header claims huge dimensions but carries almost no data: the guard must
    /// reject it from the header alone, without allocating.
    fn fake_png_header(w: u32, h: u32) -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&ihdr);
        let mut crc_in = b"IHDR".to_vec();
        crc_in.extend_from_slice(&ihdr);
        v.extend_from_slice(&crc32(&crc_in).to_be_bytes());
        // The decoder reads up to the first IDAT before reporting dimensions.
        let idat = [b'I', b'D', b'A', b'T', 0x78, 0x9c];
        v.extend_from_slice(&2u32.to_be_bytes());
        v.extend_from_slice(&idat);
        v.extend_from_slice(&crc32(&idat).to_be_bytes());
        v
    }

    #[test]
    fn decode_bomb_guard_rejects_from_header() {
        assert_eq!(reencode(&fake_png_header(8001, 100)).unwrap_err(), ImageError::TooBig);
        assert_eq!(reencode(&fake_png_header(100, 8001)).unwrap_err(), ImageError::TooBig);
        // The decoder itself may refuse absurd headers first; either way nothing is allocated.
        assert!(matches!(reencode(&fake_png_header(60000, 60000)).unwrap_err(), ImageError::TooBig | ImageError::Corrupt));
        // Under the edge cap but over 40 MP: 7000 x 7000 = 49 MP.
        assert_eq!(reencode(&fake_png_header(7000, 7000)).unwrap_err(), ImageError::TooBig);
    }
}
