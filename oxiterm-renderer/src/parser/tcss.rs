use anyhow::Result;
use oxiterm_proto::style::{AnsiColor, FlexDirection, AlignItems, JustifyContent};
use nom::{
    bytes::complete::take_while1,
    character::complete::{multispace0, char as nom_char},
    sequence::{delimited, tuple, preceded},
    IResult,
    branch::alt,
    combinator::{map, opt},
    multi::many0,
};

#[derive(Debug, Clone)]
pub enum Selector {
    Class(String),
    Id(String),
    Tag(String),
}

#[derive(Debug, Clone)]
pub enum Declaration {
    Fg(AnsiColor),
    Bg(AnsiColor),
    Width(u16),
    Height(u16),
    FlexDirection(FlexDirection),
    AlignItems(AlignItems),
    JustifyContent(JustifyContent),
    Padding(u16),
    PaddingTop(u16),
    PaddingRight(u16),
    PaddingBottom(u16),
    PaddingLeft(u16),
    Margin(u16),
    MarginTop(u16),
    MarginRight(u16),
    MarginBottom(u16),
    MarginLeft(u16),
    Border(AnsiColor),
    BorderStyle(String),
    BorderColor(AnsiColor),
}

pub fn strip_comments(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next(); // consume '*'
            while let Some(c2) = chars.next() {
                if c2 == '*' && chars.peek() == Some(&'/') {
                    chars.next(); // consume '/'
                    break;
                }
            }
        } else {
            result.push(c);
        }
    }
    result
}

#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    pub rules: Vec<(Selector, Vec<Declaration>)>,
}

pub fn parse_tcss(input: &str) -> Result<StyleSheet> {
    let stripped = strip_comments(input);
    let (_, rules) = parse_rules(&stripped).map_err(|e| anyhow::anyhow!("TCSS Parse Error: {:?}", e))?;
    Ok(StyleSheet { rules })
}

pub fn parse_inline_tcss(input: &str) -> Result<Vec<Declaration>> {
    let stripped = strip_comments(input);
    let (_, decls) = parse_declarations(&stripped).map_err(|e| anyhow::anyhow!("TCSS Inline Parse Error: {:?}", e))?;
    Ok(decls)
}

pub fn apply_declaration(style: &mut oxiterm_proto::style::ComputedStyle, decl: &Declaration) {
    match decl {
        Declaration::Fg(color) => style.fg = *color,
        Declaration::Bg(color) => style.bg = *color,
        Declaration::Width(w) => style.width = Some(*w),
        Declaration::Height(h) => style.height = Some(*h),
        Declaration::FlexDirection(d) => style.flex_direction = *d,
        Declaration::AlignItems(a) => style.align_items = *a,
        Declaration::JustifyContent(j) => style.justify_content = *j,
        Declaration::Padding(p) => {
            style.padding.top = *p;
            style.padding.right = *p;
            style.padding.bottom = *p;
            style.padding.left = *p;
        }
        Declaration::PaddingTop(p) => style.padding.top = *p,
        Declaration::PaddingRight(p) => style.padding.right = *p,
        Declaration::PaddingBottom(p) => style.padding.bottom = *p,
        Declaration::PaddingLeft(p) => style.padding.left = *p,
        Declaration::Margin(m) => {
            style.margin.top = *m;
            style.margin.right = *m;
            style.margin.bottom = *m;
            style.margin.left = *m;
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
    
    // Take value until ; or } or end of input
    let end_idx = input.find(|c| c == ';' || c == '}').unwrap_or(input.len());
    let (value, input) = input.split_at(end_idx);

    let decl = match key {
        "fg" | "color" => Some(Declaration::Fg(parse_color(value.trim()))),
        "bg" | "background-color" => Some(Declaration::Bg(parse_color(value.trim()))),
        "width" => Some(Declaration::Width(value.trim().parse().unwrap_or(0))),
        "height" => Some(Declaration::Height(value.trim().parse().unwrap_or(0))),
        "flex-direction" => match value.trim() {
            "column" => Some(Declaration::FlexDirection(FlexDirection::Column)),
            _ => Some(Declaration::FlexDirection(FlexDirection::Row)),
        },
        "align-items" => match value.trim() {
            "flex-end" => Some(Declaration::AlignItems(AlignItems::FlexEnd)),
            "center" => Some(Declaration::AlignItems(AlignItems::Center)),
            "stretch" => Some(Declaration::AlignItems(AlignItems::Stretch)),
            _ => Some(Declaration::AlignItems(AlignItems::FlexStart)),
        },
        "justify-content" => match value.trim() {
            "flex-end" => Some(Declaration::JustifyContent(JustifyContent::FlexEnd)),
            "center" => Some(Declaration::JustifyContent(JustifyContent::Center)),
            "space-between" => Some(Declaration::JustifyContent(JustifyContent::SpaceBetween)),
            "space-around" => Some(Declaration::JustifyContent(JustifyContent::SpaceAround)),
            _ => Some(Declaration::JustifyContent(JustifyContent::FlexStart)),
        },
        "padding" => Some(Declaration::Padding(value.trim().parse().unwrap_or(0))),
        "padding-top" => Some(Declaration::PaddingTop(value.trim().parse().unwrap_or(0))),
        "padding-right" => Some(Declaration::PaddingRight(value.trim().parse().unwrap_or(0))),
        "padding-bottom" => Some(Declaration::PaddingBottom(value.trim().parse().unwrap_or(0))),
        "padding-left" => Some(Declaration::PaddingLeft(value.trim().parse().unwrap_or(0))),
        "margin" => Some(Declaration::Margin(value.trim().parse().unwrap_or(0))),
        "margin-top" => Some(Declaration::MarginTop(value.trim().parse().unwrap_or(0))),
        "margin-right" => Some(Declaration::MarginRight(value.trim().parse().unwrap_or(0))),
        "margin-bottom" => Some(Declaration::MarginBottom(value.trim().parse().unwrap_or(0))),
        "margin-left" => Some(Declaration::MarginLeft(value.trim().parse().unwrap_or(0))),
        "border" => Some(Declaration::Border(parse_color(value.trim()))),
        "border-style" => Some(Declaration::BorderStyle(value.trim().to_string())),
        "border-color" => Some(Declaration::BorderColor(parse_color(value.trim()))),
        _ => None,
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

pub fn apply_styles(doc: &mut crate::document::THTMLDocument, stylesheet: &StyleSheet) {
    for (_id, node) in doc.arena.iter_mut() {
        let tag_name = match node.tag {
            oxiterm_proto::dom::NodeTag::Screen => "screen",
            oxiterm_proto::dom::NodeTag::Box => "box",
            oxiterm_proto::dom::NodeTag::Text => "text",
            oxiterm_proto::dom::NodeTag::Input => "input",
            oxiterm_proto::dom::NodeTag::Button => "button",
            oxiterm_proto::dom::NodeTag::Img => "img",
            oxiterm_proto::dom::NodeTag::Video => "video",
        };

        let mut new_style = oxiterm_proto::style::ComputedStyle::default();

        // 1. Tag rules
        for (selector, decls) in &stylesheet.rules {
            if let Selector::Tag(ref t) = selector {
                if t.to_lowercase() == tag_name {
                    for decl in decls {
                        apply_declaration(&mut new_style, decl);
                    }
                }
            }
        }

        // 2. Class rules
        for (selector, decls) in &stylesheet.rules {
            if let Selector::Class(ref c) = selector {
                if node.attrs.class.contains(c) {
                    for decl in decls {
                        apply_declaration(&mut new_style, decl);
                    }
                }
            }
        }

        // 3. Id rules
        for (selector, decls) in &stylesheet.rules {
            if let Selector::Id(ref id) = selector {
                if node.attrs.id.as_deref() == Some(id) {
                    for decl in decls {
                        apply_declaration(&mut new_style, decl);
                    }
                }
            }
        }

        // 4. Inline styles (overwriting stylesheet styles)
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
        
        assert_eq!(style.padding.left, 2);
        assert_eq!(style.padding.top, 1);
        assert_eq!(style.padding.right, 0);
        assert_eq!(style.padding.bottom, 0);
        assert_eq!(style.margin.bottom, 3);
        assert_eq!(style.margin.right, 4);
        assert_eq!(style.margin.left, 0);
        assert_eq!(style.margin.top, 0);
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
        assert_eq!(final_btn.style.width, Some(20));
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
}
