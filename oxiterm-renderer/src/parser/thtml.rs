use crate::document::THTMLDocument;
use oxiterm_proto::dom::{Node, NodeId, NodeTag, NodeAttributes};
use anyhow::{Result, anyhow};
use nom::{
    bytes::complete::{tag, take_until, take_while1},
    character::complete::{multispace0, multispace1, char as nom_char},
    sequence::{delimited, tuple},
    IResult,
    branch::alt,
    combinator::{map, opt, recognize},
    multi::many0,
    error::{context, VerboseError},
};
use regex::Regex;
use std::sync::OnceLock;
use crate::parser::tcss::{parse_inline_tcss, apply_declaration};

static ANSI_REGEX: OnceLock<Regex> = OnceLock::new();

pub fn sanitize_style_raw(input: &str) -> String {
    let re = ANSI_REGEX.get_or_init(|| {
        Regex::new(r"[\u001b\u009b][\[()#;?]*(?:[0-9]{1,4}(?:;[0-9]{0,4})*)?[0-9A-ORZcf-nqry=><]").unwrap()
    });
    re.replace_all(input, "").to_string()
}

/// BUG-M03: Sanitize `event-htmx` attribute values.
/// Only URL-safe characters are permitted. All control chars and escape sequences are stripped.
pub fn sanitize_htmx_value(input: &str) -> String {
    input
        .chars()
        .filter(|&c| matches!(c,
            'a'..='z' | 'A'..='Z' | '0'..='9'
            | '-' | '_' | ':' | '/' | '.'
        ))
        .collect()
}

pub struct THTMLParser;

type ParseResult<'a, T> = IResult<&'a str, T, VerboseError<&'a str>>;

impl THTMLParser {
    pub fn parse(input: &str) -> Result<THTMLDocument> {
        let mut doc = THTMLDocument::new();
        
        let (_, nodes) = Self::parse_nodes(input)
            .map_err(|e| anyhow!("THTML Parse Error at: {}", match e {
                nom::Err::Error(ve) | nom::Err::Failure(ve) => {
                    let mut err_msg = String::new();
                    for (i, (substring, kind)) in ve.errors.iter().enumerate() {
                        let line = substring.lines().next().unwrap_or("");
                        err_msg.push_str(&format!("  {}: {:?} near \"{}\"\n", i, kind, line.chars().take(20).collect::<String>()));
                    }
                    err_msg
                }
                _ => format!("{:?}", e),
            }))?;
        
        let root_id = doc.root;
        for node in nodes {
            Self::insert_node_recursive(&mut doc, root_id, node)?;
        }
        
        Ok(doc)
    }

    fn insert_node_recursive(doc: &mut THTMLDocument, parent_id: NodeId, parsed: ParsedNode) -> Result<()> {
        let mut node = Node::new(parsed.tag);
        node.attrs = parsed.attrs;
        node.text = parsed.text;
        
        // Apply inline styles if present
        if let Some(ref style_str) = node.attrs.style_raw {
            if let Ok(decls) = parse_inline_tcss(style_str) {
                for decl in decls {
                    apply_declaration(&mut node.style, &decl);
                }
            }
        }
        
        let node_id = doc.arena.alloc(node);
        doc.append_child(parent_id, node_id)?;
        
        for child in parsed.children {
            Self::insert_node_recursive(doc, node_id, child)?;
        }
        
        Ok(())
    }

    fn parse_nodes(input: &str) -> ParseResult<'_, Vec<ParsedNode>> {
        many0(alt((
            map(Self::parse_element, Some),
            map(Self::parse_comment, |_| None),
            map(multispace1, |_| None),
        )))(input).map(|(i, v)| (i, v.into_iter().flatten().collect()))
    }

    fn parse_comment(input: &str) -> ParseResult<'_, ()> {
        let (input, _) = tag("<!--")(input)?;
        let (input, _) = take_until("-->")(input)?;
        let (input, _) = tag("-->")(input)?;
        Ok((input, ()))
    }

    fn parse_element(input: &str) -> ParseResult<'_, ParsedNode> {
        let (input, _) = multispace0(input)?;
        let (input, _) = context("Opening bracket", nom_char('<'))(input)?;
        let (input, tag_name) = context("Tag name", parse_tag_name)(input)?;
        let (input, attrs) = context("Attributes", parse_attributes)(input)?;
        
        let (input, self_closing) = opt(nom_char('/'))(input)?;
        let (input, _) = context("Closing bracket", nom_char('>'))(input)?;
        
        if self_closing.is_some() {
            return Ok((input, ParsedNode {
                tag: tag_name,
                attrs,
                children: Vec::new(),
                text: None,
            }));
        }

        let mut children = Vec::new();
        let mut text_content = String::new();
        let mut current_input = input;

        loop {
            let close_tag_name = tag_name_to_str(tag_name);
            // Efficiently take text until the next tag or closing tag
            if let Ok((rem, text)) = take_until::<&str, &str, VerboseError<&str>>("<")(current_input) {
                if !text.is_empty() {
                    text_content.push_str(text);
                }
                current_input = rem;
            }

            // Check if it's the closing tag
            let close_tag_res: ParseResult<&str> = recognize(tuple((tag("</"), tag(close_tag_name), multispace0, nom_char('>'))))(current_input);
            
            if let Ok((remaining, _)) = close_tag_res {
                current_input = remaining;
                break;
            }

            if let Ok((remaining, child)) = Self::parse_element(current_input) {
                children.push(child);
                current_input = remaining;
            } else {
                if let Some(c) = current_input.chars().next() {
                    text_content.push(c);
                    current_input = &current_input[c.len_utf8()..];
                } else {
                    break;
                }
            }
        }

        let text = if text_content.trim().is_empty() {
            None
        } else {
            Some(text_content.trim().to_string())
        };

        Ok((current_input, ParsedNode {
            tag: tag_name,
            attrs,
            children,
            text,
        }))
    }
}

struct ParsedNode {
    tag: NodeTag,
    attrs: NodeAttributes,
    children: Vec<ParsedNode>,
    text: Option<String>,
}

fn tag_name_to_str(tag: NodeTag) -> &'static str {
    match tag {
        NodeTag::Screen => "screen",
        NodeTag::Box => "box",
        NodeTag::Text => "text",
        NodeTag::Input => "input",
        NodeTag::Button => "button",
        NodeTag::Img => "img",
    }
}

fn parse_tag_name(input: &str) -> ParseResult<'_, NodeTag> {
    alt((
        map(tag("screen"), |_| NodeTag::Screen),
        map(tag("box"), |_| NodeTag::Box),
        map(tag("text"), |_| NodeTag::Text),
        map(tag("input"), |_| NodeTag::Input),
        map(tag("button"), |_| NodeTag::Button),
        map(tag("img"), |_| NodeTag::Img),
    ))(input)
}

fn parse_attributes(mut input: &str) -> ParseResult<'_, NodeAttributes> {
    let mut attrs = NodeAttributes::default();
    
    while let Ok((rem, (key, value))) = parse_attr_kv(input) {
        match key.as_str() {
            "id" => attrs.id = Some(value),
            "class" => attrs.class = value.split_whitespace().map(|s| s.to_string()).collect(),
            "style" => attrs.style_raw = Some(value),
            "src" => attrs.src = Some(value),
            "event-htmx" => attrs.event_htmx = Some(value),
            "bind-state" => attrs.bind_state = Some(value),
            _ => {}
        }
        input = rem;
    }
    
    Ok((input, attrs))
}

fn parse_attr_kv(input: &str) -> ParseResult<'_, (String, String)> {
    let (input, _) = multispace1(input)?;
    let (input, key) = take_while1(|c: char| c.is_alphanumeric() || c == '-' || c == '_')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, _) = nom_char('=')(input)?;
    let (input, _) = multispace0(input)?;
    let (input, value) = delimited(nom_char('"'), take_until("\""), nom_char('"'))(input)?;
    
    let value = match key {
        "style" => sanitize_style_raw(value),
        // BUG-M03: sanitize event-htmx — allow only URL-safe chars, reject ANSI/escape sequences
        "event-htmx" => sanitize_htmx_value(value),
        "bind-state" => sanitize_htmx_value(value), // Same safety rules for state keys
        _ => value.to_string(),
    };
    
    Ok((input, (key.to_string(), value)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let input = r#"<box id="main"><text>Hello</text></box>"#;
        let doc = THTMLParser::parse(input).unwrap();
        let root = doc.get_root();
        assert_eq!(root.tag, NodeTag::Screen);
        assert_eq!(root.children.len(), 1);
        
        let box_id = root.children[0];
        let box_node = doc.get_node(box_id).unwrap();
        assert_eq!(box_node.tag, NodeTag::Box);
        assert_eq!(box_node.attrs.id, Some("main".to_string()));
        assert_eq!(box_node.children.len(), 1);
        
        let text_id = box_node.children[0];
        let text_node = doc.get_node(text_id).unwrap();
        assert_eq!(text_node.text, Some("Hello".to_string()));
    }

    #[test]
    fn test_sanitize_style() {
        let input = "\x1b[31mred text\x1b[0m";
        let sanitized = sanitize_style_raw(input);
        assert_eq!(sanitized, "red text");
    }

    #[test]
    fn test_sanitize_htmx() {
        let input = "alert('xss');/path/to/target";
        let sanitized = sanitize_htmx_value(input);
        // Only URL-safe chars allowed
        assert!(!sanitized.contains('('));
        assert!(!sanitized.contains('\''));
        assert!(sanitized.contains("/path/to/target"));
    }

    #[test]
    fn test_parse_nested() {
        let input = r#"<box><box><text>Deep</text></box></box>"#;
        let doc = THTMLParser::parse(input).unwrap();
        let root = doc.get_root();
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn test_parse_attributes_extra() {
        let input = r#"<button id="btn" bind-state="count" event-htmx="inc:count">Go</button>"#;
        let doc = THTMLParser::parse(input).unwrap();
        let root = doc.get_root();
        let btn_id = root.children[0];
        let btn = doc.get_node(btn_id).unwrap();
        assert_eq!(btn.attrs.id, Some("btn".to_string()));
        assert_eq!(btn.attrs.bind_state, Some("count".to_string()));
        assert_eq!(btn.attrs.event_htmx, Some("inc:count".to_string()));
    }

    #[test]
    fn test_parse_inline_style() {
        let input = r#"<box style="fg: red; bg: #0000ff; width: 50; flex-direction: column;">Styled</box>"#;
        let doc = THTMLParser::parse(input).unwrap();
        let root = doc.get_root();
        let box_node = doc.get_node(root.children[0]).unwrap();
        
        assert_eq!(box_node.style.width, Some(50));
        assert_eq!(box_node.style.flex_direction, oxiterm_proto::style::FlexDirection::Column);
        // Fg color should be red (TrueColor(255, 0, 0))
        assert_eq!(box_node.style.fg, oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0));
        // Bg color should be blue (TrueColor(0, 0, 255))
        assert_eq!(box_node.style.bg, oxiterm_proto::style::AnsiColor::TrueColor(0, 0, 255));
    }

    #[test]
    fn test_parse_style_missing_semicolon() {
        // Test inline style without trailing semicolon
        let input = r#"<box style="fg: red">Styled</box>"#;
        let doc = THTMLParser::parse(input).unwrap();
        let root = doc.get_root();
        let box_node = doc.get_node(root.children[0]).unwrap();
        assert_eq!(box_node.style.fg, oxiterm_proto::style::AnsiColor::TrueColor(255, 0, 0));

        // Test stylesheet parsing block without trailing semicolon
        let css = ".myclass { fg: green }";
        let stylesheet = crate::parser::tcss::parse_tcss(css).unwrap();
        assert_eq!(stylesheet.rules.len(), 1);
        let (_selector, decls) = &stylesheet.rules[0];
        assert_eq!(decls.len(), 1);
    }
}
