use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnsiColor {
    TrueColor(u8, u8, u8),
    Color256(u8),
    Reset,
}

impl Default for AnsiColor {
    fn default() -> Self {
        Self::Reset
    }
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum FlexDirection {
    #[default]
    Row,
    Column,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum AlignItems {
    #[default]
    FlexStart,
    FlexEnd,
    Center,
    Stretch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
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

impl Default for BorderChars {
    fn default() -> Self {
        // Standard Unicode Box Drawing
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
}
