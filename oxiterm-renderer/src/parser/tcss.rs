use anyhow::Result;
use oxiterm_proto::style::{AnsiColor, FlexDirection, AlignItems, JustifyContent};
use nom::{
    bytes::complete::{take_until, take_while1},
    character::complete::{multispace0, multispace1, char as nom_char},
    sequence::{delimited, tuple, preceded},
    IResult,
    branch::alt,
    combinator::{map, opt},
    multi::many0,
    error::Error,
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
    Margin(u16),
}

#[derive(Debug, Clone, Default)]
pub struct StyleSheet {
    pub rules: Vec<(Selector, Vec<Declaration>)>,
}

pub fn parse_tcss(input: &str) -> Result<StyleSheet> {
    let (_, rules) = parse_rules(input).map_err(|e| anyhow::anyhow!("TCSS Parse Error: {:?}", e))?;
    Ok(StyleSheet { rules })
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
    )))(input).map(|(i, v)| (i, v.into_iter().map(|(_, d, _, _)| d).collect()))
}

fn parse_declaration(input: &str) -> IResult<&str, Declaration> {
    let (input, key) = take_while1(|c: char| c.is_alphanumeric() || c == '-')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = nom_char(':')(input)?;
    let (input, _) = multispace0(input)?;
    
    // Explicitly handle error type for or_else
    let (input, value) = if let Ok(res) = take_until::<&str, &str, Error<&str>>(";")(input) {
        res
    } else {
        take_until::<&str, &str, Error<&str>>("}")(input)?
    };
    
    let decl = match key {
        "color" => Declaration::Fg(parse_color(value.trim())),
        "background-color" => Declaration::Bg(parse_color(value.trim())),
        "width" => Declaration::Width(value.trim().parse().unwrap_or(0)),
        "height" => Declaration::Height(value.trim().parse().unwrap_or(0)),
        "flex-direction" => match value.trim() {
            "column" => Declaration::FlexDirection(FlexDirection::Column),
            _ => Declaration::FlexDirection(FlexDirection::Row),
        },
        "align-items" => match value.trim() {
            "flex-end" => Declaration::AlignItems(AlignItems::FlexEnd),
            "center" => Declaration::AlignItems(AlignItems::Center),
            "stretch" => Declaration::AlignItems(AlignItems::Stretch),
            _ => Declaration::AlignItems(AlignItems::FlexStart),
        },
        "justify-content" => match value.trim() {
            "flex-end" => Declaration::JustifyContent(JustifyContent::FlexEnd),
            "center" => Declaration::JustifyContent(JustifyContent::Center),
            "space-between" => Declaration::JustifyContent(JustifyContent::SpaceBetween),
            "space-around" => Declaration::JustifyContent(JustifyContent::SpaceAround),
            _ => Declaration::JustifyContent(JustifyContent::FlexStart),
        },
        "padding" => Declaration::Padding(value.trim().parse().unwrap_or(0)),
        "margin" => Declaration::Margin(value.trim().parse().unwrap_or(0)),
        _ => return Err(nom::Err::Error(Error::new(input, nom::error::ErrorKind::Tag))),
    };
    
    Ok((input, decl))
}

fn parse_color(value: &str) -> AnsiColor {
    if value == "reset" {
        return AnsiColor::Reset;
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
