use base64::{Engine as _, engine::general_purpose::STANDARD};

pub struct KittyImageManager;

impl KittyImageManager {
    /// OxiTerm S6-04: Transmit PNG compressed image via Kitty Graphics Protocol with Base64 Chunking and Cell Sizing
    pub fn transmit_image(
        pixel_w: u32,
        pixel_h: u32,
        cols: u32,
        rows: u32,
        rgba_data: &[u8],
    ) -> Vec<u8> {
        // Compress RGBA to PNG
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
                    // a=T -> Transmit and display, f=100 -> PNG, c=cols, r=rows, m=more
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

        // Fallback to uncompressed transmission if PNG encoding fails
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
}
