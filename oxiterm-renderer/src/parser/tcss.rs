//! TCSS stylesheet parser and cascade resolver.
//!
//! Parses TCSS syntax, extracts selectors (tags, classes, IDs) and declarations,
//! and applies computed style rules cascadingly to the DOM document tree.

use anyhow::Result;
use oxiterm_proto::style::{RenderMode, AnsiColor, FlexDirection, AlignItems, JustifyContent};
use nom::{
    bytes::complete::take_while1,
    character::complete::{multispace0, char as nom_char},
    sequence::{delimited, tuple, preceded},
    IResult,
    branch::alt,
    combinator::{map, opt},
    multi::many0,
};

/// Represents a TCSS selector used to match DOM elements.
#[derive(Debug, Clone)]
pub enum Selector {
    /// Selects elements by class name (e.g., `.btn`).
    Class(String),
    /// Selects elements by their unique ID (e.g., `#submit`).
    Id(String),
    /// Selects elements by their tag name (e.g., `button`).
    Tag(String),
}

/// Represents a single TCSS style declaration property.
#[derive(Debug, Clone)]
pub enum Declaration {
    /// Text foreground color.
    Fg(AnsiColor),
    /// Background color.
    Bg(AnsiColor),
    /// Width in columns.
    Width(f32),
    /// Height in rows.
    Height(f32),
    /// Flex direction.
    FlexDirection(FlexDirection),
    /// Cross-axis alignment.
    AlignItems(AlignItems),
    /// Main-axis content justification.
    JustifyContent(JustifyContent),
    /// Padding for all sides as (vertical, horizontal) cells; px/dp convert per axis.
    Padding(f32, f32),
    /// Top padding.
    PaddingTop(f32),
    /// Right padding.
    PaddingRight(f32),
    /// Bottom padding.
    PaddingBottom(f32),
    /// Left padding.
    PaddingLeft(f32),
    /// Margin for all sides as (vertical, horizontal) cells; px/dp convert per axis.
    Margin(f32, f32),
    /// Top margin.
    MarginTop(f32),
    /// Right margin.
    MarginRight(f32),
    /// Bottom margin.
    MarginBottom(f32),
    /// Left margin.
    MarginLeft(f32),
    /// Border color (enables standard single border).
    Border(AnsiColor),
    /// Border style character preset name (e.g., `rounded`, `double`).
    BorderStyle(String),
    /// Border foreground color.
    BorderColor(AnsiColor),
    /// Text wrap mode (word wrapping).
    Wrap(oxiterm_proto::style::WrapMode),
    /// Flex shorthand value (flex-grow factor).
    Flex(f32),
    /// Positioning model.
    Position(oxiterm_proto::style::Position),
    /// Top offset inset in cells.
    Top(f32),
    /// Right offset inset in cells.
    Right(f32),
    /// Bottom offset inset in cells.
    Bottom(f32),
    /// Left offset inset in cells.
    Left(f32),
    /// Stacking order layer priority.
    ZIndex(i32),
    /// Corner radius in cells (GUI only).
    BorderRadius(f32),
    /// Transition specification declarations.
    Transition(Vec<oxiterm_proto::style::TransitionSpec>),
    /// Unrecognized property or invalid property value.
    Unknown(String),
}

/// Axis a length is measured along; selects the `ch`/`lh` meaning and the px/dp conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Widths, left/right insets, horizontal padding and margin.
    Horizontal,
    /// Heights, top/bottom insets, vertical padding and margin.
    Vertical,
}

/// Provisional reference cell width in px (1 cell = 8 px wide).
///
/// PROVISIONAL until phase 6 of docs/oxi-ui.md supplies real font metrics.
const REF_CELL_W_PX: f32 = 8.0;
/// Provisional reference cell height in px (1 cell = 16 px tall). See [`REF_CELL_W_PX`].
const REF_CELL_H_PX: f32 = 16.0;

/// Parses a TCSS length into cell units (f32).
///
/// Accepted forms (units are case-insensitive): a bare number (cells), `ch` (one cell
/// horizontally), `lh` (one cell vertically), and `px` / `dp` (converted with the
/// provisional 8x16 px reference cell, per `axis`). `ch` and `lh` are converted through
/// the same reference cell, so on the "wrong" axis they scale by the cell aspect ratio
/// (`2ch` vertically = 1 cell, `2lh` horizontally = 4 cells). Negative values are allowed
/// here; see [`parse_length_unsigned`]. Returns `None` for anything malformed
/// (including `nan`, `inf` and exponent forms).
pub fn parse_length(s: &str, axis: Axis) -> Option<f32> {
    let lower = s.trim().to_ascii_lowercase();
    let (num, unit) = match lower.find(|c: char| c.is_ascii_alphabetic()) {
        Some(i) => lower.split_at(i),
        None => (lower.as_str(), ""),
    };
    // Plain decimal only: optional sign, digits, at most one dot (rejects nan/inf/1e3).
    let digits = num.strip_prefix(['-', '+']).unwrap_or(num);
    if digits.is_empty()
        || digits.matches('.').count() > 1
        || !digits.chars().all(|c| c.is_ascii_digit() || c == '.')
        || !digits.chars().any(|c| c.is_ascii_digit())
    {
        return None;
    }
    let v: f32 = num.parse().ok()?;
    let (w, h) = (REF_CELL_W_PX, REF_CELL_H_PX);
    let cells = match (unit, axis) {
        ("", _) => v,
        ("ch", Axis::Horizontal) | ("lh", Axis::Vertical) => v,
        ("ch", Axis::Vertical) => v * w / h,
        ("lh", Axis::Horizontal) => v * h / w,
        ("px" | "dp", Axis::Horizontal) => v / w,
        ("px" | "dp", Axis::Vertical) => v / h,
        _ => return None,
    };
    if cells.is_finite() { Some(cells) } else { None }
}

/// Like [`parse_length`] but rejects any value with a leading `-` (including `-0`),
/// matching the former `u16` parsing of width/height/padding/margin.
pub fn parse_length_unsigned(s: &str, axis: Axis) -> Option<f32> {
    if s.trim().starts_with('-') {
        return None;
    }
    // The old `u16` parse rejected out-of-range values; keep that.
    parse_length(s, axis).filter(|v| *v <= f32::from(u16::MAX))
}

/// Like [`parse_length`] but also rejects values outside `i16` (the old inset range).
pub fn parse_length_inset(s: &str, axis: Axis) -> Option<f32> {
    parse_length(s, axis).filter(|v| (f32::from(i16::MIN)..=f32::from(i16::MAX)).contains(v))
}

/// Parses a padding/margin shorthand value into `(vertical, horizontal)` cells.
fn parse_length_pair(s: &str) -> Option<(f32, f32)> {
    Some((
        parse_length_unsigned(s, Axis::Vertical)?,
        parse_length_unsigned(s, Axis::Horizontal)?,
    ))
}

/// Strips block comments (`/* ... */`) from the TCSS input string.
pub fn strip_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c2) = chars.next() {
                if c2 == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// A parsed stylesheet containing rule pairings of selectors and style declarations.
#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    /// Rules defined in the stylesheet, in source order.
    pub rules: Vec<(Selector, Vec<Declaration>)>,
    /// Render condition of each rule in `rules` (same length, same order).
    /// `None` means the rule always applies; `Some(mode)` means it comes from an
    /// `@media (render: <mode>)` block and applies only in that mode.
    pub rule_modes: Vec<Option<RenderMode>>,
}

/// Parses a raw TCSS string into a [`StyleSheet`].
///
/// # Errors
///
/// Returns an error if the input string contains invalid TCSS rule syntax.
pub fn parse_tcss(input: &str) -> Result<StyleSheet> {
    let stripped = strip_comments(input);
    let (_, parsed) = parse_rules_with_media(&stripped).map_err(|e| anyhow::anyhow!("TCSS Parse Error: {:?}", e))?;
    let mut rules = Vec::with_capacity(parsed.len());
    let mut rule_modes = Vec::with_capacity(parsed.len());
    for (selector, decls, mode) in parsed {
        rules.push((selector, decls));
        rule_modes.push(mode);
    }
    Ok(StyleSheet { rules, rule_modes })
}

/// Parses an inline style string (e.g., `fg: red; padding: 1`) into a list of style declarations.
///
/// # Errors
///
/// Returns an error if the inline string contains invalid syntax.
pub fn parse_inline_tcss(input: &str) -> Result<Vec<Declaration>> {
    let stripped = strip_comments(input);
    let (_, decls) = parse_declarations(&stripped).map_err(|e| anyhow::anyhow!("TCSS Inline Parse Error: {:?}", e))?;
    Ok(decls)
}

/// Mutates the given style with values parsed from a TCSS declaration.
pub fn apply_declaration(style: &mut oxiterm_proto::style::ComputedStyle, decl: &Declaration) {
    match decl {
        Declaration::Fg(color) => style.fg = *color,
        Declaration::Bg(color) => style.bg = *color,
        Declaration::Width(w) => style.width = Some(*w),
        Declaration::Height(h) => style.height = Some(*h),
        Declaration::Wrap(wrap_mode) => style.wrap = *wrap_mode,
        Declaration::FlexDirection(d) => style.flex_direction = *d,
        Declaration::AlignItems(a) => style.align_items = *a,
        Declaration::JustifyContent(j) => style.justify_content = *j,
        Declaration::Padding(v, h) => {
            style.padding.top = *v;
            style.padding.right = *h;
            style.padding.bottom = *v;
            style.padding.left = *h;
        }
        Declaration::PaddingTop(p) => style.padding.top = *p,
        Declaration::PaddingRight(p) => style.padding.right = *p,
        Declaration::PaddingBottom(p) => style.padding.bottom = *p,
        Declaration::PaddingLeft(p) => style.padding.left = *p,
        Declaration::Margin(v, h) => {
            style.margin.top = *v;
            style.margin.right = *h;
            style.margin.bottom = *v;
            style.margin.left = *h;
        }
        Declaration::MarginTop(m) => style.margin.top = *m,
        Declaration::MarginRight(m) => style.margin.right = *m,
        Declaration::MarginBottom(m) => style.margin.bottom = *m,
        Declaration::MarginLeft(m) => style.margin.left = *m,
        Declaration::Border(color) => {
            if let Some(ref mut border) = style.border {
                border.fg = *color;
            } else {
                style.border = Some(oxiterm_proto::style::BorderStyle {
                    fg: *color,
                    chars: oxiterm_proto::style::BorderChars::default(),
                });
            }
        }
        Declaration::BorderColor(color) => {
            if let Some(ref mut border) = style.border {
                border.fg = *color;
            } else {
                style.border = Some(oxiterm_proto::style::BorderStyle {
                    fg: *color,
                    chars: oxiterm_proto::style::BorderChars::default(),
                });
            }
        }
        Declaration::BorderStyle(style_name) => {
            let chars = match style_name.as_str() {
                "double" => oxiterm_proto::style::BorderChars::double(),
                "rounded" => oxiterm_proto::style::BorderChars::rounded(),
                _ => oxiterm_proto::style::BorderChars::single(),
            };
            if let Some(ref mut border) = style.border {
                border.chars = chars;
            } else {
                style.border = Some(oxiterm_proto::style::BorderStyle {
                    fg: oxiterm_proto::style::AnsiColor::Reset,
                    chars,
                });
            }
        }
        Declaration::Flex(val) => style.flex = Some(*val),
        Declaration::Position(pos) => style.position = *pos,
        Declaration::Top(v) => style.top = Some(*v),
        Declaration::Right(v) => style.right = Some(*v),
        Declaration::Bottom(v) => style.bottom = Some(*v),
        Declaration::Left(v) => style.left = Some(*v),
        Declaration::ZIndex(v) => style.z_index = Some(*v),
        Declaration::BorderRadius(v) => style.border_radius = Some(*v),
        Declaration::Transition(v) => style.transitions = v.clone(),
        Declaration::Unknown(_) => {},
    }
}

fn parse_rules(input: &str) -> IResult<&str, Vec<(Selector, Vec<Declaration>)>> {
    many0(tuple((
        multispace0,
        parse_selector,
        multispace0,
        delimited(nom_char('{'), parse_declarations, nom_char('}')),
        multispace0,
    )))(input).map(|(i, v)| (i, v.into_iter().map(|(_, s, _, d, _)| (s, d)).collect()))
}

type ConditionalRule = (Selector, Vec<Declaration>, Option<RenderMode>);

/// Parses top-level rules and `@media (render: ...)` blocks, keeping source order.
///
/// Rules inside a media block carry `Some(mode)`; all others carry `None`. A media block whose
/// query is not exactly `(render: tui)` / `(render: gui)` is skipped as a whole (like CSS does
/// with queries it does not match) and parsing continues after its matching `}`.
fn parse_rules_with_media(input: &str) -> IResult<&str, Vec<ConditionalRule>> {
    let mut out: Vec<ConditionalRule> = Vec::new();
    let mut rest = input;
    loop {
        let (r, _) = multispace0(rest)?;
        rest = r;
        if rest.starts_with('@') {
            let Some((query, body, after)) = split_media_block(rest) else { break };
            if let Some(mode) = parse_render_query(query) {
                // One level only: a nested @media stops the inner parse, dropping the remainder.
                if let Ok((_, inner)) = parse_rules(body) {
                    out.extend(inner.into_iter().map(|(s, d)| (s, d, Some(mode))));
                }
            }
            rest = after;
            continue;
        }
        match tuple((
            parse_selector,
            multispace0,
            delimited(nom_char('{'), parse_declarations, nom_char('}')),
        ))(rest) {
            Ok((r, (s, _, d))) => {
                out.push((s, d, None));
                rest = r;
            }
            Err(_) => break,
        }
    }
    Ok((rest, out))
}

/// Splits `@media <query> { <body> } <after>` into `(query, body, after)`.
/// Returns `None` when the input is not an `@media` rule with a `{`. An unterminated body
/// extends to the end of the input.
fn split_media_block(input: &str) -> Option<(&str, &str, &str)> {
    let head = input.get(..6)?;
    if !head.eq_ignore_ascii_case("@media") {
        return None;
    }
    let after_kw = &input[6..];
    let open = after_kw.find('{')?;
    let query = &after_kw[..open];
    let body_start = &after_kw[open + 1..];
    let mut depth = 1usize;
    let mut quote: Option<char> = None;
    for (i, c) in body_start.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((query, &body_start[..i], &body_start[i + 1..]));
                    }
                }
                _ => {}
            },
        }
    }
    Some((query, body_start, ""))
}

/// Parses a media query of the exact form `(render: tui|gui)` (case- and whitespace-insensitive).
fn parse_render_query(query: &str) -> Option<RenderMode> {
    let q = query.trim().strip_prefix('(')?.strip_suffix(')')?;
    let (feature, value) = q.split_once(':')?;
    if !feature.trim().eq_ignore_ascii_case("render") {
        return None;
    }
    match value.trim().to_ascii_lowercase().as_str() {
        "tui" => Some(RenderMode::Tui),
        "gui" => Some(RenderMode::Gui),
        _ => None,
    }
}

/// Finds the end of a declaration value: the first `;` or `}` outside quotes and parentheses.
fn declaration_end(input: &str) -> usize {
    let mut quote: Option<char> = None;
    let mut parens = 0usize;
    for (i, c) in input.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '(' => parens += 1,
                ')' => parens = parens.saturating_sub(1),
                ';' | '}' if parens == 0 => return i,
                _ => {}
            },
        }
    }
    // Unbalanced quote/paren: fall back to the plain splitter so one malformed declaration
    // cannot swallow the rest of the rule and the rules after it.
    input.find(|c| c == ';' || c == '}').unwrap_or(input.len())
}

fn parse_selector(input: &str) -> IResult<&str, Selector> {
    alt((
        map(preceded(nom_char('.'), take_while1(|c: char| c.is_alphanumeric() || c == '-' || c == '_')), |s: &str| Selector::Class(s.to_string())),
        map(preceded(nom_char('#'), take_while1(|c: char| c.is_alphanumeric() || c == '-' || c == '_')), |s: &str| Selector::Id(s.to_string())),
        map(take_while1(|c: char| c.is_alphanumeric()), |s: &str| Selector::Tag(s.to_string())),
    ))(input)
}

fn parse_declarations(input: &str) -> IResult<&str, Vec<Declaration>> {
    many0(tuple((
        multispace0,
        parse_declaration,
        opt(nom_char(';')),
        multispace0,
    )))(input).map(|(i, v)| (i, v.into_iter().filter_map(|(_, d, _, _)| d).collect()))
}

fn parse_declaration(input: &str) -> IResult<&str, Option<Declaration>> {
    let (input, key) = take_while1(|c: char| c.is_alphanumeric() || c == '-')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = nom_char(':')(input)?;
    let (input, _) = multispace0(input)?;
    
    let end_idx = declaration_end(input);
    let (value, input) = input.split_at(end_idx);

    let val_str = value.trim();
    let decl = match key {
        "fg" | "color" => Some(Declaration::Fg(parse_color(val_str))),
        "bg" | "background-color" => Some(Declaration::Bg(parse_color(val_str))),
        "width" => match parse_length_unsigned(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::Width(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "height" => match parse_length_unsigned(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::Height(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "flex" => match val_str.parse::<f32>() {
            Ok(v) if v > 0.0 => Some(Declaration::Flex(v)),
            _ => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "wrap" => match val_str {
            "word" => Some(Declaration::Wrap(oxiterm_proto::style::WrapMode::Word)),
            _ => Some(Declaration::Wrap(oxiterm_proto::style::WrapMode::None)),
        },
        "flex-direction" => match val_str {
            "column" => Some(Declaration::FlexDirection(FlexDirection::Column)),
            _ => Some(Declaration::FlexDirection(FlexDirection::Row)),
        },
        "align-items" => match val_str {
            "flex-end" => Some(Declaration::AlignItems(AlignItems::FlexEnd)),
            "center" => Some(Declaration::AlignItems(AlignItems::Center)),
            "stretch" => Some(Declaration::AlignItems(AlignItems::Stretch)),
            _ => Some(Declaration::AlignItems(AlignItems::FlexStart)),
        },
        "justify-content" => match val_str {
            "flex-end" => Some(Declaration::JustifyContent(JustifyContent::FlexEnd)),
            "center" => Some(Declaration::JustifyContent(JustifyContent::Center)),
            "space-between" => Some(Declaration::JustifyContent(JustifyContent::SpaceBetween)),
            "space-around" => Some(Declaration::JustifyContent(JustifyContent::SpaceAround)),
            _ => Some(Declaration::JustifyContent(JustifyContent::FlexStart)),
        },
        "padding" => match parse_length_pair(val_str) {
            Some((v, h)) => Some(Declaration::Padding(v, h)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "padding-top" => match parse_length_unsigned(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::PaddingTop(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "padding-right" => match parse_length_unsigned(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::PaddingRight(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "padding-bottom" => match parse_length_unsigned(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::PaddingBottom(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "padding-left" => match parse_length_unsigned(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::PaddingLeft(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "margin" => match parse_length_pair(val_str) {
            Some((v, h)) => Some(Declaration::Margin(v, h)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "margin-top" => match parse_length_unsigned(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::MarginTop(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "margin-right" => match parse_length_unsigned(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::MarginRight(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "margin-bottom" => match parse_length_unsigned(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::MarginBottom(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "margin-left" => match parse_length_unsigned(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::MarginLeft(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "border" => Some(Declaration::Border(parse_color(val_str))),
        "border-style" => Some(Declaration::BorderStyle(val_str.to_string())),
        "border-color" => Some(Declaration::BorderColor(parse_color(val_str))),
        "position" => match val_str {
            "relative" => Some(Declaration::Position(oxiterm_proto::style::Position::Relative)),
            "absolute" => Some(Declaration::Position(oxiterm_proto::style::Position::Absolute)),
            "fixed" => Some(Declaration::Position(oxiterm_proto::style::Position::Fixed)),
            "static" => Some(Declaration::Position(oxiterm_proto::style::Position::Static)),
            _ => {
                tracing::warn!("Unknown TCSS position value: {}", val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "top" => match parse_length_inset(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::Top(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "right" => match parse_length_inset(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::Right(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "bottom" => match parse_length_inset(val_str, Axis::Vertical) {
            Some(v) => Some(Declaration::Bottom(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "left" => match parse_length_inset(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::Left(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "z-index" => match val_str.parse::<i32>() {
            Ok(v) => Some(Declaration::ZIndex(v)),
            Err(_) => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "transition" => {
            let specs = parse_transitions(val_str);
            Some(Declaration::Transition(specs))
        },
        // GUI-only properties: accepted and ignored without a warning (TUI has no equivalent).
        "border-radius" => match parse_length_unsigned(val_str, Axis::Horizontal) {
            Some(v) => Some(Declaration::BorderRadius(v)),
            None => {
                tracing::warn!("Unknown TCSS property value for {}: {}", key, val_str);
                Some(Declaration::Unknown(format!("{key}: {val_str}")))
            }
        },
        "box-shadow" | "font-family" | "font-size" => {
            Some(Declaration::Unknown(format!("{key}: {val_str}")))
        }
        _ => {
            tracing::warn!("Unknown TCSS property: {}", key);
            Some(Declaration::Unknown(format!("{key}: {val_str}")))
        }
    };
    
    Ok((input, decl))
}

fn parse_color(value: &str) -> AnsiColor {
    let value = value.to_lowercase();
    let value = value.trim();
    
    if value == "reset" || value == "transparent" {
        return AnsiColor::Reset;
    }
    
    match value {
        "black" => return AnsiColor::TrueColor(0, 0, 0),
        "red" => return AnsiColor::TrueColor(255, 0, 0),
        "green" => return AnsiColor::TrueColor(0, 255, 0),
        "yellow" => return AnsiColor::TrueColor(255, 255, 0),
        "blue" => return AnsiColor::TrueColor(0, 0, 255),
        "magenta" => return AnsiColor::TrueColor(255, 0, 255),
        "cyan" => return AnsiColor::TrueColor(0, 255, 255),
        "white" => return AnsiColor::TrueColor(255, 255, 255),
        _ => {}
    }

    if value.starts_with('#') && value.len() == 7 {
        let r = u8::from_str_radix(&value[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&value[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&value[5..7], 16).unwrap_or(0);
        return AnsiColor::TrueColor(r, g, b);
    }
    if let Ok(n) = value.parse::<u8>() {
        return AnsiColor::Color256(n);
    }
    AnsiColor::Reset
}

fn parse_transitions(val_str: &str) -> Vec<oxiterm_proto::style::TransitionSpec> {
    use oxiterm_proto::style::{AnimatableProperty, Easing, TransitionSpec};
    let mut specs = Vec::new();
    
    for item in val_str.split(',') {
        let parts: Vec<&str> = item.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }
        
        let props = match parts[0].to_lowercase().as_str() {
            "left" => vec![AnimatableProperty::Left],
            "top" => vec![AnimatableProperty::Top],
            "right" => vec![AnimatableProperty::Right],
            "bottom" => vec![AnimatableProperty::Bottom],
            "width" => vec![AnimatableProperty::Width],
            "height" => vec![AnimatableProperty::Height],
            "margin-left" => vec![AnimatableProperty::MarginLeft],
            "margin-top" => vec![AnimatableProperty::MarginTop],
            "margin-right" => vec![AnimatableProperty::MarginRight],
            "margin-bottom" => vec![AnimatableProperty::MarginBottom],
            "margin" => vec![
                AnimatableProperty::MarginLeft,
                AnimatableProperty::MarginTop,
                AnimatableProperty::MarginRight,
                AnimatableProperty::MarginBottom,
            ],
            "fg" | "color" => vec![AnimatableProperty::Fg],
            "bg" | "background-color" => vec![AnimatableProperty::Bg],
            "opacity" => vec![AnimatableProperty::Opacity],
            "all" => vec![
                AnimatableProperty::Left,
                AnimatableProperty::Top,
                AnimatableProperty::Right,
                AnimatableProperty::Bottom,
                AnimatableProperty::Width,
                AnimatableProperty::Height,
                AnimatableProperty::MarginLeft,
                AnimatableProperty::MarginTop,
                AnimatableProperty::Opacity,
                AnimatableProperty::Fg,
                AnimatableProperty::Bg,
            ],
            _ => continue,
        };
        
        let duration_ms = if parts.len() > 1 {
            parse_duration_ms(parts[1])
        } else {
            300
        };
        
        let easing = if parts.len() > 2 {
            parse_easing(parts[2])
        } else {
            Easing::Ease
        };
        
        let delay_ms = if parts.len() > 3 {
            parse_duration_ms(parts[3])
        } else {
            0
        };
        
        for prop in props {
            specs.push(TransitionSpec {
                property: prop,
                duration_ms,
                delay_ms,
                easing,
            });
        }
    }
    
    specs
}

fn parse_duration_ms(s: &str) -> u32 {
    let s = s.trim().to_lowercase();
    if let Some(num) = s.strip_suffix("ms") {
        num.parse::<u32>().unwrap_or(0)
    } else if let Some(num) = s.strip_suffix('s') {
        (num.parse::<f32>().unwrap_or(0.0) * 1000.0) as u32
    } else {
        s.parse::<u32>().unwrap_or(0)
    }
}

fn parse_easing(s: &str) -> oxiterm_proto::style::Easing {
    use oxiterm_proto::style::Easing;
    let s = s.trim().to_lowercase();
    match s.as_str() {
        "linear" => Easing::Linear,
        "ease" => Easing::Ease,
        "ease-in" => Easing::EaseIn,
        "ease-out" => Easing::EaseOut,
        "ease-in-out" => Easing::EaseInOut,
        _ if s.starts_with("spring") => {
            let mut stiffness = 100.0;
            let mut damping = 10.0;
            let mut mass = 1.0;
            if let Some(inner) = s.strip_prefix("spring(").and_then(|x| x.strip_suffix(')')) {
                let nums: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                if !nums.is_empty() { stiffness = nums[0]; }
                if nums.len() >= 2 { damping = nums[1]; }
                if nums.len() >= 3 { mass = nums[2]; }
            }
            Easing::Spring { stiffness, damping, mass }
        }
        _ if s.starts_with("cubic-bezier(") => {
            if let Some(inner) = s.strip_prefix("cubic-bezier(").and_then(|x| x.strip_suffix(')')) {
                let nums: Vec<f32> = inner.split(',').filter_map(|p| p.trim().parse().ok()).collect();
                if nums.len() == 4 {
                    return Easing::CubicBezier(nums[0], nums[1], nums[2], nums[3]);
                }
            }
            Easing::Ease
        }
        _ => Easing::Ease,
    }
}


/// Applies stylesheet rules to all DOM nodes inside the document cascadingly.
///
/// Cascade order resolution:
/// 1. Tag type selector rules (lowest priority).
/// 2. Class name selector rules.
/// 3. ID selector rules.
/// 4. Inline element styles (highest priority).
pub fn apply_styles(doc: &mut crate::document::THTMLDocument, stylesheet: &StyleSheet) {
    apply_styles_for_mode(doc, stylesheet, RenderMode::Tui);
}

/// Like [`apply_styles`], but only applies rules that are unconditional or whose
/// `@media (render: ...)` condition equals `mode`.
pub fn apply_styles_for_mode(doc: &mut crate::document::THTMLDocument, stylesheet: &StyleSheet, mode: RenderMode) {
    let active: Vec<&(Selector, Vec<Declaration>)> = stylesheet
        .rules
        .iter()
        .enumerate()
        .filter(|(i, _)| match stylesheet.rule_modes.get(*i).copied().flatten() {
            None => true,
            Some(m) => m == mode,
        })
        .map(|(_, r)| r)
        .collect();
    for (_id, node) in doc.arena.iter_mut() {
        let tag_name = match node.tag {
            oxiterm_proto::dom::NodeTag::Screen => "screen",
            oxiterm_proto::dom::NodeTag::Box => "box",
            oxiterm_proto::dom::NodeTag::Text => "text",
            oxiterm_proto::dom::NodeTag::Input => "input",
            oxiterm_proto::dom::NodeTag::Button => "button",
            oxiterm_proto::dom::NodeTag::Img => "img",
            oxiterm_proto::dom::NodeTag::Video => "video",
            oxiterm_proto::dom::NodeTag::For => "for",
            oxiterm_proto::dom::NodeTag::Diagram => "diagram",
        };

        let mut new_style = oxiterm_proto::style::ComputedStyle::default();

        // 1. Tag rules
        for (selector, decls) in active.iter().map(|r| (&r.0, &r.1)) {
            if let Selector::Tag(ref t) = selector {
                if t.to_lowercase() == tag_name {
                    for decl in decls {
                        apply_declaration(&mut new_style, decl);
                    }
                }
            }
        }

        // 2. Class rules
        for (selector, decls) in active.iter().map(|r| (&r.0, &r.1)) {
            if let Selector::Class(ref c) = selector {
                if node.attrs.class.contains(c) {
                    for decl in decls {
                        apply_declaration(&mut new_style, decl);
                    }
                }
            }
        }

        // 3. Id rules
        for (selector, decls) in active.iter().map(|r| (&r.0, &r.1)) {
            if let Selector::Id(ref id) = selector {
                if node.attrs.id.as_deref() == Some(id) {
                    for decl in decls {
                        apply_declaration(&mut new_style, decl);
                    }
                }
            }
        }

        // 4. Inline styles
        if let Some(ref style_str) = node.attrs.style_raw {
            if let Ok(decls) = parse_inline_tcss(style_str) {
                for decl in decls {
                    apply_declaration(&mut new_style, &decl);
                }
            }
        }

        node.style = new_style;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radius_of(v: &str) -> Option<f32> {
        let decls = parse_inline_tcss(&format!("border-radius: {v};")).unwrap();
        let mut style = oxiterm_proto::style::ComputedStyle::default();
        for d in &decls {
            apply_declaration(&mut style, d);
        }
        style.border_radius
    }

    #[test]
    fn test_border_radius_parsing() {
        assert_eq!(radius_of("2"), Some(2.0));
        assert_eq!(radius_of("1.5"), Some(1.5));
        assert_eq!(radius_of("16px"), Some(2.0));
        assert_eq!(radius_of("4dp"), Some(0.5));
        assert_eq!(radius_of("-1"), None);
        assert_eq!(radius_of("-4px"), None);
        assert_eq!(radius_of("4px 8px"), None);
        assert_eq!(radius_of("abc"), None);
    }

    #[test]
    fn test_border_parsing() {
        let input = "
            border-style: rounded;
            border-color: #7aa2f7;
        ";
        let decls = parse_inline_tcss(input).unwrap();
        assert_eq!(decls.len(), 2);
        
        let mut style = oxiterm_proto::style::ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        
        let border = style.border.unwrap();
        assert_eq!(border.chars.top_left, '╭');
        assert_eq!(border.fg, AnsiColor::TrueColor(122, 162, 247));
    }

    #[test]
    fn test_individual_padding_margin_parsing() {
        let input = "
            padding-left: 2;
            padding-top: 1;
            margin-bottom: 3;
            margin-right: 4;
        ";
        let decls = parse_inline_tcss(input).unwrap();
        assert_eq!(decls.len(), 4);
        
        let mut style = oxiterm_proto::style::ComputedStyle::default();
        for decl in &decls {
            apply_declaration(&mut style, decl);
        }
        
        assert_eq!(style.padding.left, 2.0);
        assert_eq!(style.padding.top, 1.0);
        assert_eq!(style.padding.right, 0.0);
        assert_eq!(style.padding.bottom, 0.0);
        assert_eq!(style.margin.bottom, 3.0);
        assert_eq!(style.margin.right, 4.0);
        assert_eq!(style.margin.left, 0.0);
        assert_eq!(style.margin.top, 0.0);
    }

    #[test]
    fn test_apply_styles_cascading() {
        let mut doc = crate::document::THTMLDocument::new();
        let mut btn = oxiterm_proto::dom::Node::new(oxiterm_proto::dom::NodeTag::Button);
        btn.attrs.id = Some("submit".to_string());
        btn.attrs.class = vec!["btn".to_string(), "primary".to_string()];
        btn.attrs.style_raw = Some("fg: red".to_string());
        
        let root = doc.root;
        let btn_id = doc.arena.alloc(btn);
        doc.append_child(root, btn_id).unwrap();

        let tcss = "
            button { width: 10; }
            .btn { width: 20; fg: yellow; bg: blue; }
            #submit { bg: magenta; }
        ";
        let stylesheet = parse_tcss(tcss).unwrap();
        apply_styles(&mut doc, &stylesheet);

        let final_btn = doc.arena.get(btn_id).unwrap();
        assert_eq!(final_btn.style.width, Some(20.0));
        assert_eq!(final_btn.style.bg, AnsiColor::TrueColor(255, 0, 255));
        assert_eq!(final_btn.style.fg, AnsiColor::TrueColor(255, 0, 0));
    }

    #[test]
    fn test_comments_stripping() {
        let tcss = "
            /* Comment at start */
            button {
                /* Inner comment */
                width: 10;
            }
            /* Comment between rules */
            .btn {
                width: 20;
            }
        ";
        let stylesheet = parse_tcss(tcss).unwrap();
        assert_eq!(stylesheet.rules.len(), 2);
    }

    #[test]
    fn test_tcss_flex_and_unknown_declarations() {
        // T-1: flex: 1 -> Declaration::Flex(1.0)
        let decls = parse_inline_tcss("flex: 1").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Flex(v)) if (*v - 1.0).abs() < f32::EPSILON));

        // T-2: flex: 2.5 -> Declaration::Flex(2.5)
        let decls = parse_inline_tcss("flex: 2.5").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Flex(v)) if (*v - 2.5).abs() < f32::EPSILON));

        // T-3: flex: abc -> Declaration::Unknown("flex: abc")
        let decls = parse_inline_tcss("flex: abc").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Unknown(s)) if s == "flex: abc"));

        // T-4: nonexistent-prop: 5 -> Declaration::Unknown("nonexistent-prop: 5")
        let decls = parse_inline_tcss("nonexistent-prop: 5").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Unknown(s)) if s == "nonexistent-prop: 5"));

        // T-5: font-weight: bold -> Declaration::Unknown("font-weight: bold")
        let decls = parse_inline_tcss("font-weight: bold").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Unknown(s)) if s == "font-weight: bold"));

        // T-6: height: auto -> Declaration::Unknown("height: auto")
        let decls = parse_inline_tcss("height: auto").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Unknown(s)) if s == "height: auto"));

        // T-6b: ComputedStyle po zastosowaniu height: auto ma height == None
        let mut style = oxiterm_proto::style::ComputedStyle::default();
        if let Some(decl) = decls.first() {
            apply_declaration(&mut style, decl);
        }
        assert_eq!(style.height, None);

        // T-21: flex: 0 -> Declaration::Unknown("flex: 0")
        let decls = parse_inline_tcss("flex: 0").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Unknown(s)) if s == "flex: 0"));

        // T-22: flex: -1 -> Declaration::Unknown("flex: -1")
        let decls = parse_inline_tcss("flex: -1").unwrap();
        assert!(matches!(decls.first(), Some(Declaration::Unknown(s)) if s == "flex: -1"));
    }

    #[test]
    fn test_parse_length_units() {
        use Axis::{Horizontal as H, Vertical as V};
        // Bare numbers are cells.
        assert_eq!(parse_length("10", H), Some(10.0));
        assert_eq!(parse_length("-2", H), Some(-2.0));
        assert_eq!(parse_length("0.5", V), Some(0.5));
        assert_eq!(parse_length(".5", V), Some(0.5));
        // ch / lh on their own axes are cells; case-insensitive.
        assert_eq!(parse_length("4ch", H), Some(4.0));
        assert_eq!(parse_length("3LH", V), Some(3.0));
        // Provisional 8x16 px reference cell.
        assert_eq!(parse_length("16px", H), Some(2.0));
        assert_eq!(parse_length("16px", V), Some(1.0));
        assert_eq!(parse_length("24dp", H), Some(3.0));
        assert_eq!(parse_length("24DP", V), Some(1.5));
        // Cross-axis ch/lh go through the reference cell.
        assert_eq!(parse_length("2ch", V), Some(1.0));
        assert_eq!(parse_length("2lh", H), Some(4.0));
        // Garbage.
        for bad in ["", "abc", "10em", "nan", "inf", "1e3", "1.2.3", "--1", "px", "10 px", "%"] {
            assert_eq!(parse_length(bad, H), None, "{bad:?} must be rejected");
        }
    }

    #[test]
    fn test_length_declarations() {
        let decls = parse_inline_tcss("width: 10ch; height: 32px; padding: 16px; top: -2; margin-left: 1.5").unwrap();
        let mut style = oxiterm_proto::style::ComputedStyle::default();
        for d in &decls { apply_declaration(&mut style, d); }
        assert_eq!(style.width, Some(10.0));
        assert_eq!(style.height, Some(2.0));
        assert_eq!((style.padding.top, style.padding.left), (1.0, 2.0));
        assert_eq!((style.padding.bottom, style.padding.right), (1.0, 2.0));
        assert_eq!(style.top, Some(-2.0));
        assert_eq!(style.margin.left, 1.5);

        // Negative width/padding/margin, and unparseable values, stay ignored (Unknown).
        for src in ["width: -1", "height: -0", "padding: -3", "margin-top: -1", "width: abc", "left: nope", "width: 70000", "padding: 65536", "top: 40000", "left: -40000"] {
            let d = parse_inline_tcss(src).unwrap();
            assert!(matches!(d.first(), Some(Declaration::Unknown(_))), "{src}");
        }
    }
}
