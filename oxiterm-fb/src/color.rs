//! Colour mapping, identical to the web painter's `get_color_str` / `ansi_256_to_rgb`
//! (`oxiterm-web/src/lib.rs`). The table is duplicated (that crate is a wasm-bindgen cdylib the
//! sink must not depend on); `tests/primitives.rs` checks both agree for every index.

use oxiterm_proto::style::AnsiColor;

/// An opaque RGB colour.
pub type Rgb = [u8; 3];

const PALETTE16: [Rgb; 16] = [
    [15, 23, 42],
    [239, 68, 68],
    [34, 197, 94],
    [234, 179, 8],
    [59, 130, 246],
    [168, 85, 247],
    [6, 182, 212],
    [241, 245, 249],
    [100, 116, 139],
    [248, 113, 113],
    [74, 222, 128],
    [253, 224, 71],
    [96, 165, 250],
    [192, 132, 252],
    [34, 211, 238],
    [255, 255, 255],
];

/// Background of `AnsiColor::Reset` (slate-900).
pub const DEFAULT_BG: Rgb = [0x0f, 0x17, 0x2a];
/// Foreground of `AnsiColor::Reset` (slate-100).
pub const DEFAULT_FG: Rgb = [0xf1, 0xf5, 0xf9];

/// Palette entry of a 256-colour index.
pub fn ansi_256(idx: u8) -> Rgb {
    if idx < 16 {
        PALETTE16[idx as usize]
    } else if idx < 232 {
        let v = idx - 16;
        let steps = [0u8, 95, 135, 175, 223, 255];
        [steps[((v / 36) % 6) as usize], steps[((v / 6) % 6) as usize], steps[(v % 6) as usize]]
    } else {
        let l = 8 + (idx - 232) * 10;
        [l, l, l]
    }
}

/// Colour of `c` used as a foreground (`Reset` = default text colour).
pub fn fg(c: &AnsiColor) -> Rgb {
    match c {
        AnsiColor::Reset => DEFAULT_FG,
        AnsiColor::TrueColor(r, g, b) => [*r, *g, *b],
        AnsiColor::Color256(i) => ansi_256(*i),
    }
}

/// Colour of `c` used as a background (`Reset` = page background).
pub fn bg(c: &AnsiColor) -> Rgb {
    match c {
        AnsiColor::Reset => DEFAULT_BG,
        other => fg(other),
    }
}
