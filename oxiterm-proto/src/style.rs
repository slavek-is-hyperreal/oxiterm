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

/// Positioning mode for an element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Position {
    /// Standard flexbox flow layout (default).
    #[default]
    Static,
    /// Positioned relative to its normal position in flex flow.
    Relative,
    /// Removed from flex flow, positioned relative to nearest positioned ancestor.
    Absolute,
    /// Positioned relative to viewport root.
    Fixed,
}

/// The computed styling of a node after resolving the TCSS cascade.
///
/// Maps flexbox positioning and visual attributes for DOM rendering.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
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
    /// Optional flex shorthand value (flex-grow factor).
    pub flex: Option<f32>,
    /// Wrap mode for text content.
    pub wrap: WrapMode,
    /// Positioning model.
    pub position: Position,
    /// Top offset inset in character cells.
    pub top: Option<i16>,
    /// Right offset inset in character cells.
    pub right: Option<i16>,
    /// Bottom offset inset in character cells.
    pub bottom: Option<i16>,
    /// Left offset inset in character cells.
    pub left: Option<i16>,
    /// Stacking order layer priority.
    pub z_index: Option<i32>,
    /// Active CSS transition declarations.
    pub transitions: Vec<TransitionSpec>,
    /// Optional opacity factor [0.0, 1.0].
    pub opacity: Option<f32>,
}

/// Animatable CSS style property for transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AnimatableProperty {
    All,
    Left,
    Top,
    Right,
    Bottom,
    Width,
    Height,
    MarginLeft,
    MarginTop,
    MarginRight,
    MarginBottom,
    Opacity,
    Fg,
    Bg,
}

/// Transition timing and curve function.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub enum Easing {
    /// Constant rate of change.
    Linear,
    /// Standard CSS ease: cubic-bezier(0.25, 0.1, 0.25, 1.0).
    #[default]
    Ease,
    /// Slow start: cubic-bezier(0.42, 0.0, 1.0, 1.0).
    EaseIn,
    /// Fast start, smooth deceleration: cubic-bezier(0.0, 0.0, 0.58, 1.0).
    EaseOut,
    /// Slow start and slow end: cubic-bezier(0.42, 0.0, 0.58, 1.0).
    EaseInOut,
    /// Custom cubic Bézier curve defined by control points (x1, y1, x2, y2).
    CubicBezier(f32, f32, f32, f32),
    /// Damped harmonic oscillator spring physics (stiffness, damping, mass).
    Spring { stiffness: f32, damping: f32, mass: f32 },
}

/// Specification for animating a property on state change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransitionSpec {
    /// The style property to animate.
    pub property: AnimatableProperty,
    /// Duration of the transition in milliseconds.
    pub duration_ms: u32,
    /// Delay before transition starts in milliseconds.
    pub delay_ms: u32,
    /// Easing curve or physics function.
    pub easing: Easing,
}

/// Wrap mode for text rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum WrapMode {
    /// Do not wrap text.
    #[default]
    None,
    /// Wrap text at word boundaries.
    Word,
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BorderStyle {
    /// Border color.
    pub fg: AnsiColor,
    /// Unicode character set used to draw borders.
    pub chars: BorderChars,
}

/// Unicode characters used to draw container borders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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

    #[test]
    fn test_computed_style_default_flex() {
        // T-7: ComputedStyle::default().flex is None
        assert_eq!(ComputedStyle::default().flex, None);
    }
}
