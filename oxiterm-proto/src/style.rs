use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AnsiColor {
    TrueColor(u8, u8, u8),
    Color256(u8),
    #[default]
    Reset,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ComputedStyle {
    pub fg: AnsiColor,
    pub bg: AnsiColor,
    pub width: Option<u16>,
    pub height: Option<u16>,
    pub flex_direction: FlexDirection,
    pub align_items: AlignItems,
    pub justify_content: JustifyContent,
    pub padding: Rect,
    pub margin: Rect,
    pub border: Option<BorderStyle>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum AlignItems {
    FlexStart,
    FlexEnd,
    Center,
    #[default]
    Stretch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum JustifyContent {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    SpaceBetween,
    SpaceAround,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct Rect {
    pub top: u16,
    pub right: u16,
    pub bottom: u16,
    pub left: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderStyle {
    pub fg: AnsiColor,
    pub chars: BorderChars,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BorderChars {
    pub top_left: char,
    pub top: char,
    pub top_right: char,
    pub left: char,
    pub right: char,
    pub bot_left: char,
    pub bot: char,
    pub bot_right: char,
}

impl BorderChars {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ColorDepth {
    #[default]
    TrueColor,
    Color256,
    Color16,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerminalProfile {
    pub supports_kitty_kbd: bool,
    pub supports_kitty_gfx: bool,
    pub supports_sixel: bool,
    pub supports_sgr_mouse: bool,
    pub color_depth: ColorDepth,
}

impl TerminalProfile {
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
        // Combined Kitty APC response and standard DA1 response indicating SGR mouse support (?62)
        let response = b"\x1b_Gi=31,s=1,v=1,a=q,t=d;OK\x1b\\\x1b[?62;c";
        profile.parse_da1_response(response);
        assert!(profile.supports_kitty_gfx);
        assert!(profile.supports_sgr_mouse);
        assert_eq!(profile.color_depth, ColorDepth::TrueColor);
    }
}


