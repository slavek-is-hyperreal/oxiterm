//! Styling definitions and terminal capability profiling.
//!
//! This module represents ANSI colors, flexbox layout styles, and the Device
//! Attributes (DA1) response parser used for client terminal capability detection.

use serde::{Deserialize, Serialize};

/// Color representation in ANSI format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AnsiColor {
    /// Full 24-bit RGB color (TrueColor).
    TrueColor(u8, u8, u8),
    /// An index from the 256-color ANSI palette.
    Color256(u8),
    /// Reset the color to the terminal's default color.
    #[default]
    Reset,
}

/// The computed styling of a node after resolving the TCSS cascade.
///
/// Maps flexbox positioning and visual attributes for DOM rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComputedStyle {
    /// Foreground text color.
    pub fg: AnsiColor,
    /// Background color.
    pub bg: AnsiColor,
    /// Element width in terminal cell columns.
    pub width: Option<u16>,
    /// Element height in terminal cell rows.
    pub height: Option<u16>,
    /// Flexbox layout direction (row or column).
    pub flex_direction: FlexDirection,
    /// Align items along the cross axis of the flex container.
    pub align_items: AlignItems,
    /// Distribute space between and around items along the main axis.
    pub justify_content: JustifyContent,
    /// Inner padding spaces.
    pub padding: Rect,
    /// Outer margin spaces.
    pub margin: Rect,
    /// Element border styling (if defined).
    pub border: Option<BorderStyle>,
}

/// The direction of the flex container's layout.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum FlexDirection {
    /// Horizontal layout (default).
    #[default]
    Row,
    /// Vertical layout.
    Column,
}

/// Wyrównanie (alignment) of items along the cross axis.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum AlignItems {
    /// Align items to the start of the cross axis.
    FlexStart,
    /// Align items to the end of the cross axis.
    FlexEnd,
    /// Center items along the cross axis.
    Center,
    /// Stretch items to fill the container (default).
    #[default]
    Stretch,
}

/// Space distribution along the main axis of a flex container.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum JustifyContent {
    /// Align items to the start of the main axis (default).
    #[default]
    FlexStart,
    /// Align items to the end of the main axis.
    FlexEnd,
    /// Center items along the main axis.
    Center,
    /// Distribute items evenly; first item is at the start, last is at the end.
    SpaceBetween,
    /// Distribute items evenly with equal space around them.
    SpaceAround,
}

/// Margin or padding spacing metrics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Rect {
    /// Top margin/padding size.
    pub top: u16,
    /// Right margin/padding size.
    pub right: u16,
    /// Bottom margin/padding size.
    pub bottom: u16,
    /// Left margin/padding size.
    pub left: u16,
}

/// Border styling definition containing colors and characters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderStyle {
    /// Border color.
    pub fg: AnsiColor,
    /// Unicode character set used to draw borders.
    pub chars: BorderChars,
}

/// Unicode characters used to draw container borders.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderChars {
    /// Top-left corner character.
    pub top_left: char,
    /// Top border line character.
    pub top: char,
    /// Top-right corner character.
    pub top_right: char,
    /// Left border line character.
    pub left: char,
    /// Right border line character.
    pub right: char,
    /// Bottom-left corner character.
    pub bot_left: char,
    /// Bottom border line character.
    pub bot: char,
    /// Bottom-right corner character.
    pub bot_right: char,
}

impl BorderChars {
    /// Creates a classic single-line border character set with sharp corners.
    pub fn single() -> Self {
        Self {
            top_left: '┌',
            top: '─',
            top_right: '┐',
            left: '│',
            right: '│',
            bot_left: '└',
            bot: '─',
            bot_right: '┘',
        }
    }

    /// Creates a single-line border character set with rounded corners.
    pub fn rounded() -> Self {
        Self {
            top_left: '╭',
            top: '─',
            top_right: '╮',
            left: '│',
            right: '│',
            bot_left: '╰',
            bot: '─',
            bot_right: '╯',
        }
    }

    /// Creates a double-line border character set with sharp corners.
    pub fn double() -> Self {
        Self {
            top_left: '╔',
            top: '═',
            top_right: '╗',
            left: '║',
            right: '║',
            bot_left: '╚',
            bot: '═',
            bot_right: '╝',
        }
    }
}

impl Default for BorderChars {
    fn default() -> Self {
        Self::single()
    }
}

/// Supported color depth capabilities of the client terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorDepth {
    /// Full 24-bit TrueColor support.
    #[default]
    TrueColor,
    /// 256-color ANSI support (8-bit palette).
    Color256,
    /// Basic 16-color ANSI support.
    Color16,
}

/// Capabilities profile of the connected client terminal.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerminalProfile {
    /// True if the terminal supports the Kitty keyboard protocol for extended keys.
    pub supports_kitty_kbd: bool,
    /// True if the terminal supports direct graphics rendering via Kitty Graphics Protocol.
    pub supports_kitty_gfx: bool,
    /// True if the terminal supports image rendering via Sixel.
    pub supports_sixel: bool,
    /// True if the terminal supports extended SGR mouse tracking (1006).
    pub supports_sgr_mouse: bool,
    /// Detected color depth support.
    pub color_depth: ColorDepth,
    /// True if the client is a web client using DOM media overlays.
    #[serde(default)]
    pub is_web: bool,
}

impl TerminalProfile {
    /// Parses raw device attribute sequences (DA1 responses) from the terminal.
    ///
    /// Used to detect mouse tracking, Sixel graphics, Kitty APC graphic capabilities, etc.
    pub fn parse_da1_response(&mut self, response: &[u8]) {
        let s = String::from_utf8_lossy(response);
        tracing::debug!("Parsing DA1 response: {}", s);
        if s.contains("\x1b_G") && s.contains("OK") {
            self.supports_kitty_gfx = true;
        }
        if s.contains("?64") || s.contains("?62") || s.contains("?63") || s.contains("?65") {
            self.supports_sgr_mouse = true;
            self.color_depth = ColorDepth::TrueColor;
        }
        if s.contains(";4;") || s.contains(";4c") {
            self.supports_sixel = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_da1_response_kitty_and_mouse() {
        let mut profile = TerminalProfile::default();
        let response = b"\x1b_Gi=31,s=1,v=1,a=q,t=d;OK\x1b\\\x1b[?62;c";
        profile.parse_da1_response(response);
        assert!(profile.supports_kitty_gfx);
        assert!(profile.supports_sgr_mouse);
        assert_eq!(profile.color_depth, ColorDepth::TrueColor);
    }
}
