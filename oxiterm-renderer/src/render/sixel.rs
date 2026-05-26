//! Sixel graphics protocol encoder.
//!
//! Provides color quantization, RLE compression, palette declaration, and
//! sixel band formatting to output images on compatible DEC terminals.

use std::collections::HashMap;
use image::RgbaImage;

#[derive(Hash, PartialEq, Eq, Clone, Copy, Debug)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

/// Codec to translate pixel images into Sixel graphics data blocks.
pub struct SixelCodec;

impl SixelCodec {
    /// Translates an RGBA image buffer to a quantized Sixel byte stream.
    ///
    /// Anchored by spec [S6-10]. Maps the image pixels to the most frequent colors
    /// up to the configured palette size, defining DEC palette color register codes.
    pub fn encode_sixel(img: &RgbaImage, palette_size: u16) -> Vec<u8> {
        let width = img.width();
        let height = img.height();
        
        let mut color_counts: HashMap<Rgb, usize> = HashMap::new();
        for pixel in img.pixels() {
            if pixel[3] >= 128 {
                let rgb = Rgb {
                    r: pixel[0],
                    g: pixel[1],
                    b: pixel[2],
                };
                *color_counts.entry(rgb).or_insert(0) += 1;
            }
        }
        
        let mut colors: Vec<(Rgb, usize)> = color_counts.into_iter().collect();
        colors.sort_by(|a, b| b.1.cmp(&a.1));
        
        let mut palette: Vec<Rgb> = colors
            .into_iter()
            .take(palette_size as usize)
            .map(|x| x.0)
            .collect();
            
        if palette.is_empty() {
            palette.push(Rgb { r: 0, g: 0, b: 0 });
        }
        
        let get_color_index = |r: u8, g: u8, b: u8, a: u8| -> Option<usize> {
            if a < 128 {
                return None;
            }
            let mut best_idx = 0;
            let mut min_dist = u32::MAX;
            for (i, pal_rgb) in palette.iter().enumerate() {
                let dr = r as i32 - pal_rgb.r as i32;
                let dg = g as i32 - pal_rgb.g as i32;
                let db = b as i32 - pal_rgb.b as i32;
                let dist = (dr * dr + dg * dg + db * db) as u32;
                if dist == 0 {
                    return Some(i);
                }
                if dist < min_dist {
                    min_dist = dist;
                    best_idx = i;
                }
            }
            Some(best_idx)
        };
        
        let mut output = Vec::new();
        
        // DCS P q " 1 ; 1 ; width ; height
        let header = format!("\x1bPq\"1;1;{};{}", width, height);
        output.extend_from_slice(header.as_bytes());
        
        // Define color palette in percentages (0..100)
        for (i, color) in palette.iter().enumerate() {
            let r_pct = (color.r as u32 * 100) / 255;
            let g_pct = (color.g as u32 * 100) / 255;
            let b_pct = (color.b as u32 * 100) / 255;
            let color_def = format!("#{};2;{};{};{}", i, r_pct, g_pct, b_pct);
            output.extend_from_slice(color_def.as_bytes());
        }
        
        let num_bands = (height + 5) / 6;
        for band_idx in 0..num_bands {
            let y_start = band_idx * 6;
            
            let mut band_has_colors = false;
            for color_idx in 0..palette.len() {
                let mut char_row = Vec::with_capacity(width as usize);
                let mut color_present_in_band = false;
                
                for x in 0..width {
                    let mut sixel_val = 0u8;
                    for bit_idx in 0..6 {
                        let y = y_start + bit_idx;
                        if y < height {
                            let pixel = img.get_pixel(x, y);
                            if let Some(idx) = get_color_index(pixel[0], pixel[1], pixel[2], pixel[3]) {
                                if idx == color_idx {
                                    sixel_val |= 1 << bit_idx;
                                    color_present_in_band = true;
                                }
                            }
                        }
                    }
                    char_row.push(63 + sixel_val);
                }
                
                if color_present_in_band {
                    band_has_colors = true;
                    output.extend_from_slice(format!("#{}", color_idx).as_bytes());
                    let compressed = Self::sixel_rle_compress(&char_row);
                    output.extend_from_slice(&compressed);
                    output.push(b'$');
                }
            }
            
            if band_has_colors {
                if output.last() == Some(&b'$') {
                    output.pop();
                }
            }
            output.push(b'-');
        }
        
        output.extend_from_slice(b"\x1b\\");
        output
    }

    /// Encodes an image with a static, pre-defined 240-color palette.
    ///
    /// Ideal for systems requesting static palette declarations or to avoid custom quantization overhead.
    pub fn encode_sixel_static(img: &RgbaImage) -> Vec<u8> {
        let width = img.width();
        let height = img.height();

        let get_color_index = |r: u8, g: u8, b: u8, a: u8| -> Option<usize> {
            if a < 128 {
                return None;
            }
            let dr_g = (r as i32 - g as i32).abs();
            let dg_b = (g as i32 - b as i32).abs();
            let dr_b = (r as i32 - b as i32).abs();
            if dr_g < 8 && dg_b < 8 && dr_b < 8 {
                let gray_val = (r as u32 + g as u32 + b as u32) / 3;
                let g_idx = (gray_val * 23 / 255) as usize;
                return Some(216 + g_idx);
            }
            
            let cr = ((r as u32 * 5) + 127) / 255;
            let cg = ((g as u32 * 5) + 127) / 255;
            let cb = ((b as u32 * 5) + 127) / 255;
            Some((cr * 36 + cg * 6 + cb) as usize)
        };

        let mut output = Vec::new();
        let header = format!("\x1bPq\"1;1;{};{}", width, height);
        output.extend_from_slice(header.as_bytes());

        // Define 6x6x6 color cube
        for r in 0..6 {
            for g in 0..6 {
                for b in 0..6 {
                    let idx = r * 36 + g * 6 + b;
                    let r_pct = r * 20;
                    let g_pct = g * 20;
                    let b_pct = b * 20;
                    let color_def = format!("#{};2;{};{};{}", idx, r_pct, g_pct, b_pct);
                    output.extend_from_slice(color_def.as_bytes());
                }
            }
        }

        // Define grays
        for g_idx in 0..24 {
            let idx = 216 + g_idx;
            let pct = g_idx * 100 / 23;
            let color_def = format!("#{};2;{};{};{}", idx, pct, pct, pct);
            output.extend_from_slice(color_def.as_bytes());
        }

        let num_bands = (height + 5) / 6;
        for band_idx in 0..num_bands {
            let y_start = band_idx * 6;
            let mut band_has_colors = false;
            
            let mut used_colors = std::collections::HashSet::new();
            for x in 0..width {
                for bit_idx in 0..6 {
                    let y = y_start + bit_idx;
                    if y < height {
                        let pixel = img.get_pixel(x, y);
                        if let Some(idx) = get_color_index(pixel[0], pixel[1], pixel[2], pixel[3]) {
                            used_colors.insert(idx);
                        }
                    }
                }
            }
            
            for &color_idx in &used_colors {
                let mut char_row = Vec::with_capacity(width as usize);
                let mut color_present_in_band = false;
                
                for x in 0..width {
                    let mut sixel_val = 0u8;
                    for bit_idx in 0..6 {
                        let y = y_start + bit_idx;
                        if y < height {
                            let pixel = img.get_pixel(x, y);
                            if let Some(idx) = get_color_index(pixel[0], pixel[1], pixel[2], pixel[3]) {
                                if idx == color_idx {
                                    sixel_val |= 1 << bit_idx;
                                    color_present_in_band = true;
                                }
                            }
                        }
                    }
                    char_row.push(63 + sixel_val);
                }
                
                if color_present_in_band {
                    band_has_colors = true;
                    output.extend_from_slice(format!("#{}", color_idx).as_bytes());
                    let compressed = Self::sixel_rle_compress(&char_row);
                    output.extend_from_slice(&compressed);
                    output.push(b'$');
                }
            }
            
            if band_has_colors {
                if output.last() == Some(&b'$') {
                    output.pop();
                }
            }
            output.push(b'-');
        }
        
        output.extend_from_slice(b"\x1b\\");
        output
    }

    /// Compresses a pixel sequence using Sixel Run-Length Encoding.
    ///
    /// Anchored by spec [S6-11]. Replaces repeated occurrences of a character with a
    /// repeat descriptor format `!<count><char>`.
    pub fn sixel_rle_compress(data: &[u8]) -> Vec<u8> {
        let mut compressed = Vec::new();
        let mut i = 0;
        while i < data.len() {
            let ch = data[i];
            let mut count = 1;
            while i + count < data.len() && data[i + count] == ch {
                count += 1;
            }
            if count >= 3 {
                compressed.extend_from_slice(format!("!{}", count).as_bytes());
                compressed.push(ch);
            } else {
                for _ in 0..count {
                    compressed.push(ch);
                }
            }
            i += count;
        }
        compressed
    }

    /// High-level convenience function to encode a raw RGBA buffer into a Sixel stream.
    pub fn encode_image(width: u32, height: u32, rgba_data: &[u8]) -> Vec<u8> {
        if let Some(img) = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(width, height, rgba_data.to_vec()) {
            Self::encode_sixel(&img, 256)
        } else {
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sixel_rle_compress() {
        let input = b"AAAAABBBCC";
        let compressed = SixelCodec::sixel_rle_compress(input);
        assert_eq!(compressed, b"!5A!3BCC");
    }

    #[test]
    fn test_encode_sixel_basic() {
        let mut img = RgbaImage::new(2, 2);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgba([255, 0, 0, 255]);
        }
        let encoded = SixelCodec::encode_sixel(&img, 256);
        
        let s = String::from_utf8_lossy(&encoded);
        assert!(s.starts_with("\x1bPq\"1;1;2;2"));
        assert!(s.ends_with("\x1b\\"));
        assert!(s.contains("#0;2;100;0;0"));
    }
}
