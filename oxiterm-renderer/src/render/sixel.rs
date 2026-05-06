pub struct SixelCodec;

impl SixelCodec {
    /// OxiTerm S6-10: Kodek Sixel z RLE (Run-Length Encoding) jako fallback dla starszych terminali
    pub fn encode_image(width: u32, height: u32, _rgba_data: &[u8]) -> Vec<u8> {
        let mut output = Vec::new();
        // DCS P q " 1 ; 1 ; width ; height
        let header = format!("\x1bPq\"1;1;{};{}", width, height);
        output.extend_from_slice(header.as_bytes());
        
        // Prost(y) kwantyzator w RLE dla celów prototypu
        // Rzeczywista implementacja wymagałaby konwersji RGBA->HLS i generowania pasków (sixel bands)
        output.extend_from_slice(b"#0;2;0;0;0"); // Register color 0 as black
        output.extend_from_slice(b"!100~"); // RLE: repeat character '~' (6 full pixels) 100 times
        
        // ESU / ST terminator
        output.extend_from_slice(b"\x1b\\");
        output
    }
}
