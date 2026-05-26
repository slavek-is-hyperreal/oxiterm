//! Kitty Graphics Protocol encoder.
//!
//! Provides utilities to transmit compressed PNG and raw RGBA image/frame buffers
//! using the Kitty Graphics Protocol with base64 chunking and cell alignment.

use base64::{Engine as _, engine::general_purpose::STANDARD};

/// Manager handling image formatting and command serialization for Kitty Graphics Protocol.
pub struct KittyImageManager;

impl KittyImageManager {
    /// Encodes and transmits an RGBA byte buffer to the client terminal using PNG compression.
    ///
    /// Anchored by spec [S6-04]. Performs base64 encoding and splits the payload into
    /// 4096-byte chunks to fit within standard terminal input write limits.
    pub fn transmit_image(
        pixel_w: u32,
        pixel_h: u32,
        cols: u32,
        rows: u32,
        rgba_data: &[u8],
    ) -> Vec<u8> {
        let mut png_bytes = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        use image::ImageEncoder;
        if encoder.write_image(
            rgba_data,
            pixel_w,
            pixel_h,
            image::ExtendedColorType::Rgba8,
        ).is_ok() {
            let b64 = STANDARD.encode(&png_bytes);
            let chunks = b64.as_bytes().chunks(4096);
            let count = chunks.len();

            let mut output = Vec::new();
            for (i, chunk) in chunks.enumerate() {
                let m = if i == count - 1 { 0 } else { 1 };
                if i == 0 {
                    let header = format!("\x1b_Ga=T,f=100,c={},r={},m={};", cols, rows, m);
                    output.extend_from_slice(header.as_bytes());
                } else {
                    let header = format!("\x1b_Gm={};", m);
                    output.extend_from_slice(header.as_bytes());
                }
                output.extend_from_slice(chunk);
                output.extend_from_slice(b"\x1b\\");
            }
            return output;
        }

        Self::transmit_image_rgba(pixel_w, pixel_h, cols, rows, rgba_data)
    }

    fn transmit_image_rgba(
        pixel_w: u32,
        pixel_h: u32,
        cols: u32,
        rows: u32,
        rgba_data: &[u8],
    ) -> Vec<u8> {
        let b64 = STANDARD.encode(rgba_data);
        let chunks = b64.as_bytes().chunks(4096);
        let count = chunks.len();

        let mut output = Vec::new();
        for (i, chunk) in chunks.enumerate() {
            let m = if i == count - 1 { 0 } else { 1 };
            if i == 0 {
                let header = format!("\x1b_Ga=T,f=32,s={},v={},c={},r={},m={};", pixel_w, pixel_h, cols, rows, m);
                output.extend_from_slice(header.as_bytes());
            } else {
                let header = format!("\x1b_Gm={};", m);
                output.extend_from_slice(header.as_bytes());
            }
            output.extend_from_slice(chunk);
            output.extend_from_slice(b"\x1b\\");
        }
        output
    }

    /// Generates a command sequence to clear all active Kitty image placements from screen.
    pub fn delete_all_placements() -> Vec<u8> {
        b"\x1b_Ga=d,d=A\x1b\\".to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delete_all_placements() {
        let bytes = KittyImageManager::delete_all_placements();
        assert!(bytes.starts_with(&[0x1b, 0x5f, 0x47]));
        assert!(bytes.ends_with(&[0x1b, 0x5c]));
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(s.contains("a=d"));
        assert!(s.contains("d=A"));
    }
}
