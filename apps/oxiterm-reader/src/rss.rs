//! Minimal RSS 2.0 / Atom extraction. Deliberately not an HTML parser: html5ever treats
//! `<link>` as void (detaching RSS link text) and mangles `<![CDATA[ ]]>`.

use crate::textsafe::clean_text;

/// One feed item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Title.
    pub title: String,
    /// Link as written in the feed.
    pub link: String,
    /// Plain-text summary (tags removed).
    pub summary: String,
}

/// Parsed feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    /// Feed title.
    pub title: String,
    /// Items (at most `max_items`).
    pub items: Vec<Item>,
}

/// Decodes the five XML entities and numeric references (once; no double decoding).
pub fn xml_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Text content of an element value: CDATA unwrapped, otherwise entity-decoded.
fn text_value(raw: &str) -> String {
    let t = raw.trim();
    if let Some(c) = t.strip_prefix("<![CDATA[") {
        return c.strip_suffix("]]>").unwrap_or(c).to_string();
    }
    xml_unescape(t)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

/// Finds `<name ...>` (not `<name2`) at or after `from` in `lower`; returns
/// (start of tag, index after `>`, self_closing, attrs string).
fn find_open(lower: &str, name: &str, from: usize) -> Option<(usize, usize, bool, String)> {
    let pat = format!("<{name}");
    let mut pos = from;
    while let Some(i) = lower[pos..].find(&pat) {
        let s = pos + i;
        let after = s + pat.len();
        let next = lower.as_bytes().get(after).copied();
        if matches!(next, Some(b'>') | Some(b' ') | Some(b'/') | Some(b'\t') | Some(b'\n') | Some(b'\r')) {
            let gt = lower[after..].find('>')? + after;
            let attrs = lower[after..gt].trim_end_matches('/').to_string();
            return Some((s, gt + 1, lower[..gt].ends_with('/'), attrs));
        }
        pos = after;
    }
    None
}

/// First element `name` inside `block`: (attrs from the original text, inner raw text).
fn element<'a>(block: &'a str, lower: &str, name: &str) -> Option<(String, &'a str)> {
    let (_, inner_start, self_closing, _) = find_open(lower, name, 0)?;
    let open_end = inner_start;
    let attrs_orig = {
        let s = lower.find(&format!("<{name}"))?;
        block[s + name.len() + 1..open_end - 1].trim_end_matches('/').to_string()
    };
    if self_closing {
        return Some((attrs_orig, ""));
    }
    let close = format!("</{name}>");
    let end = lower[open_end..].find(&close)? + open_end;
    Some((attrs_orig, &block[open_end..end]))
}

fn attr(attrs: &str, name: &str) -> Option<String> {
    let lower = attrs.to_ascii_lowercase();
    let pat = format!("{name}=");
    let mut from = 0;
    while let Some(i) = lower[from..].find(&pat) {
        let s = from + i;
        let prev_ok = s == 0 || lower.as_bytes()[s - 1].is_ascii_whitespace();
        if prev_ok {
            let v = &attrs[s + pat.len()..];
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let end = v[1..].find(q)?;
                return Some(xml_unescape(&v[1..1 + end]));
            }
        }
        from = s + pat.len();
    }
    None
}

/// Parses RSS 2.0 or Atom text.
pub fn parse_feed(xml: &str, max_items: usize) -> Feed {
    let lower = xml.to_ascii_lowercase(); // ASCII lowercasing keeps byte offsets identical
    let (item_tag, atom) = if find_open(&lower, "item", 0).is_some() { ("item", false) } else { ("entry", true) };

    let first_item = find_open(&lower, item_tag, 0).map(|t| t.0).unwrap_or(xml.len());
    let head = &xml[..first_item];
    let feed_title = element(head, &lower[..first_item], "title").map(|(_, t)| clean_text(&text_value(t))).unwrap_or_default();

    let mut items = Vec::new();
    let mut pos = 0;
    while items.len() < max_items {
        let Some((s, inner, selfc, _)) = find_open(&lower, item_tag, pos) else { break };
        if selfc {
            pos = inner;
            continue;
        }
        let close = format!("</{item_tag}>");
        let end = lower[inner..].find(&close).map(|e| e + inner).unwrap_or(xml.len());
        let block = &xml[inner..end];
        let lblock = &lower[inner..end];
        pos = end.max(s + 1);

        let title = element(block, lblock, "title").map(|(_, t)| clean_text(&text_value(t))).unwrap_or_default();
        let link = if atom { atom_link(block, lblock) } else { element(block, lblock, "link").map(|(_, t)| text_value(t).trim().to_string()).unwrap_or_default() };
        let summary_raw = ["description", "summary", "content"].iter().find_map(|n| element(block, lblock, n).map(|(_, t)| t));
        let summary = summary_raw.map(|t| clean_text(&strip_tags(&xml_unescape_if_escaped(&text_value(t))))).unwrap_or_default();
        if !title.is_empty() || !link.is_empty() {
            items.push(Item { title: if title.is_empty() { "(untitled)".into() } else { title }, link, summary: crate::textsafe::truncate_cols(&summary, 200) });
        }
    }
    Feed { title: feed_title, items }
}

/// Description fields are often entity-escaped HTML; decode once more so tags can be stripped.
fn xml_unescape_if_escaped(s: &str) -> String {
    if s.contains("&lt;") {
        xml_unescape(s)
    } else {
        s.to_string()
    }
}

fn atom_link(block: &str, lower: &str) -> String {
    let mut pos = 0;
    let mut fallback = String::new();
    while let Some((s, gt, _, _)) = find_open(lower, "link", pos) {
        let attrs = &block[s + 5..gt - 1];
        let rel = attr(attrs, "rel");
        if let Some(h) = attr(attrs, "href") {
            if rel.as_deref().map(|r| r == "alternate").unwrap_or(true) {
                return h;
            }
            if fallback.is_empty() {
                fallback = h;
            }
        }
        pos = gt;
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss2_with_cdata_and_link_text() {
        let f = parse_feed(include_str!("../tests/fixtures/feed.rss"), 10);
        assert_eq!(f.title, "Example Dev Blog");
        assert_eq!(f.items.len(), 3);
        assert_eq!(f.items[0].title, "Rust 2.0 <released> & more");
        assert_eq!(f.items[0].link, "https://blog.example.org/rust-2");
        assert!(f.items[0].summary.starts_with("The big release"), "{}", f.items[0].summary);
        assert!(!f.items[0].summary.contains('<'));
        assert_eq!(f.items[1].title, "Plain \"title\"");
        assert_eq!(f.items[2].link, "https://blog.example.org/3");
    }

    #[test]
    fn atom_links_prefer_alternate() {
        let f = parse_feed(include_str!("../tests/fixtures/feed.atom"), 10);
        assert_eq!(f.title, "Atom Example");
        assert_eq!(f.items.len(), 2);
        assert_eq!(f.items[0].link, "https://atom.example.org/a1");
        assert_eq!(f.items[1].link, "https://atom.example.org/a2");
        assert_eq!(f.items[0].title, "First <entry>");
    }

    #[test]
    fn caps_garbage_and_entities() {
        assert_eq!(parse_feed(include_str!("../tests/fixtures/feed.rss"), 1).items.len(), 1);
        for g in ["", "<item>", "<item><title>", "<entry/><entry><title>x</title>", "&&&;;<item><title>&#x41;&#66;&bogus;</title></item>"] {
            let _ = parse_feed(g, 5);
        }
        assert_eq!(parse_feed("<item><title>&#x41;&#66;&bogus;&amp;lt;</title></item>", 5).items[0].title, "AB&bogus;&lt;");
        assert_eq!(xml_unescape("a &amp; b &#8364; &lt;"), "a & b \u{20ac} <");
    }
}
