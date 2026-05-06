use crate::document::THTMLDocument;
use oxiterm_proto::dom::{NodeId, NodeTag};
use anyhow::Result;
use nom::{
    bytes::complete::{tag, take_until, take_while1},
    character::complete::{multispace0, char as nom_char},
    sequence::delimited,
    IResult,
    branch::alt,
    combinator::map,
};
use regex::Regex;
use std::sync::OnceLock;

static ANSI_REGEX: OnceLock<Regex> = OnceLock::new();

pub fn sanitize_style_raw(input: &str) -> String {
    let re = ANSI_REGEX.get_or_init(|| {
        Regex::new(r"[\u001b\u009b][\[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]").unwrap()
    });
    // Remove ANSI escape sequences
    re.replace_all(input, "").to_string()
}

pub struct THTMLParser<'a> {
    _input: &'a str,
}

impl<'a> THTMLParser<'a> {
    pub fn parse(_input: &'a str) -> Result<THTMLDocument> {
        let mut doc = THTMLDocument::new();
        let root_id = doc.root;
        Self::parse_into(&mut doc, root_id);
        Ok(doc)
    }

    fn parse_into(_doc: &mut THTMLDocument, _parent: NodeId) {
        // Implementation placeholder
    }
}

/// A very basic nom parser for THTML tags
#[allow(dead_code)]
fn parse_tag_name(input: &str) -> IResult<&str, NodeTag> {
    alt((
        map(tag("screen"), |_| NodeTag::Screen),
        map(tag("box"), |_| NodeTag::Box),
        map(tag("text"), |_| NodeTag::Text),
        map(tag("input"), |_| NodeTag::Input),
        map(tag("button"), |_| NodeTag::Button),
        map(tag("img"), |_| NodeTag::Img),
    ))(input)
}

#[allow(dead_code)]
fn parse_attr_kv(input: &str) -> IResult<&str, (String, String)> {
    let (input, key) = take_while1(|c: char| c.is_alphanumeric() || c == '-' || c == '_')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = nom_char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, value) = delimited(nom_char('"'), take_until("\""), nom_char('"'))(input)?;
    
    let value = if key == "style" {
        sanitize_style_raw(value)
    } else {
        value.to_string()
    };
    
    Ok((input, (key.to_string(), value)))
}
