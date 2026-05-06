use base64::{Engine as _, engine::general_purpose::STANDARD};

pub struct KittyImageManager;

impl KittyImageManager {
    /// OxiTerm S6-04: Transmit 32-bit RGBA image via Kitty Graphics Protocol with Base64 Chunking
    pub fn transmit_image(width: u32, height: u32, rgba_data: &[u8]) -> Vec<u8> {
        let b64 = STANDARD.encode(rgba_data);
        let chunks = b64.as_bytes().chunks(4096);
        let count = chunks.len();

        let mut output = Vec::new();
        for (i, chunk) in chunks.enumerate() {
            let m = if i == count - 1 { 0 } else { 1 };
            if i == 0 {
                // f=32 -> 32-bit RGBA, s=width, v=height, m=more
                let header = format!("\x1b_Gf=32,s={},v={},m={};", width, height, m);
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
