//! HTML (already reduced by readability, or Wikipedia's clean HTML) -> structured blocks.
//!
//! Nothing from the page survives except text, image URLs (re-fetched and re-encoded
//! later) and link destinations (moved into the server-side [`LinkTable`]).

use crate::model::{Block, LinkTable, LinkTarget};
use crate::textsafe::clean_text;
use scraper::{ElementRef, Html, Node};
use url::Url;

/// Conversion options.
#[derive(Debug, Clone, Default)]
pub struct Opts {
    /// Wikipedia language: enables wiki link resolution and skips tables/refs.
    pub wiki_lang: Option<String>,
    /// Maximum number of blocks.
    pub max_blocks: usize,
}

const SKIP_TAGS: &[&str] = &[
    "script", "style", "noscript", "svg", "math", "template", "iframe", "object", "embed", "form",
    "button", "select", "textarea", "input", "audio", "video", "canvas", "head", "nav", "footer", "aside",
];

struct Ctx<'a> {
    base: &'a Url,
    opts: &'a Opts,
    links: &'a mut LinkTable,
    out: Vec<Block>,
    pending_images: Vec<Block>,
}

/// Converts `html` into blocks, registering links in `links`.
pub fn html_to_blocks(html: &str, base: &Url, opts: &Opts, links: &mut LinkTable) -> Vec<Block> {
    let doc = Html::parse_fragment(html);
    let mut ctx = Ctx { base, opts, links, out: Vec::new(), pending_images: Vec::new() };
    walk_children(doc.root_element(), &mut ctx, 0);
    ctx.out
}

fn full(ctx: &Ctx) -> bool {
    ctx.out.len() >= ctx.opts.max_blocks.max(1)
}

fn is_hidden(el: &ElementRef) -> bool {
    let v = el.value();
    v.attr("hidden").is_some()
        || v.attr("aria-hidden") == Some("true")
        || v.attr("style").map(|s| {
            let s: String = s.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
            s.contains("display:none") || s.contains("visibility:hidden")
        }).unwrap_or(false)
}

fn has_class(el: &ElementRef, names: &[&str]) -> bool {
    el.value().classes().any(|c| names.contains(&c))
}

fn skipped(el: &ElementRef, ctx: &Ctx) -> bool {
    let name = el.value().name();
    if SKIP_TAGS.contains(&name) || is_hidden(el) {
        return true;
    }
    if ctx.opts.wiki_lang.is_some() {
        if name == "table" || name == "sup" && has_class(el, &["mw-ref", "reference"]) {
            return true;
        }
        if has_class(el, &["mw-editsection", "navbox", "noprint", "metadata", "mbox-small", "reflist", "refbegin", "hatnote", "mw-empty-elt", "sistersitebox", "toc"]) {
            return true;
        }
    }
    false
}

fn walk_children(el: ElementRef, ctx: &mut Ctx, depth: usize) {
    if depth > 64 {
        return;
    }
    // Loose inline content between blocks is gathered into an implicit paragraph.
    let mut inline_run = String::new();
    for child in el.children() {
        if full(ctx) {
            break;
        }
        match child.value() {
            Node::Text(t) => inline_run.push_str(t),
            Node::Element(_) => {
                let ce = ElementRef::wrap(child).unwrap();
                if skipped(&ce, ctx) {
                    continue;
                }
                if is_block(ce.value().name()) {
                    flush_para(&mut inline_run, ctx);
                    block(ce, ctx, depth + 1);
                } else {
                    inline(ce, ctx, &mut inline_run, 0);
                }
            }
            _ => {}
        }
    }
    flush_para(&mut inline_run, ctx);
}

fn flush_para(run: &mut String, ctx: &mut Ctx) {
    let t = clean_text(run);
    run.clear();
    if !t.is_empty() && !full(ctx) {
        ctx.out.push(Block::Para(t));
    }
    drain_images(ctx);
}

fn drain_images(ctx: &mut Ctx) {
    let imgs: Vec<Block> = ctx.pending_images.drain(..).collect();
    for i in imgs {
        if !full(ctx) {
            ctx.out.push(i);
        }
    }
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div" | "section" | "article" | "main" | "header" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6"
            | "ul" | "ol" | "li" | "blockquote" | "pre" | "figure" | "figcaption" | "hr" | "table" | "tr"
            | "dl" | "dt" | "dd" | "details" | "summary" | "address" | "center" | "body" | "html" | "fieldset"
    )
}

fn block(el: ElementRef, ctx: &mut Ctx, depth: usize) {
    let name = el.value().name();
    match name {
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            let level = (name.as_bytes()[1] - b'0').clamp(1, 3);
            let t = inline_text(el, ctx);
            if !t.is_empty() {
                ctx.out.push(Block::Heading(level, t));
            }
            drain_images(ctx);
        }
        "p" | "dt" | "dd" | "summary" | "address" | "figcaption" => {
            let t = inline_text(el, ctx);
            if !t.is_empty() {
                ctx.out.push(Block::Para(t));
            }
            drain_images(ctx);
        }
        "ul" | "ol" => {
            let ordered = name == "ol";
            let mut items = Vec::new();
            for c in el.children() {
                if let Some(li) = ElementRef::wrap(c) {
                    if li.value().name() == "li" && !skipped(&li, ctx) {
                        let t = inline_text(li, ctx);
                        if !t.is_empty() {
                            items.push(t);
                        }
                    }
                }
                if items.len() >= 100 {
                    break;
                }
            }
            if !items.is_empty() {
                ctx.out.push(Block::List { ordered, items });
            }
            drain_images(ctx);
        }
        "blockquote" => {
            let t = inline_text(el, ctx);
            if !t.is_empty() {
                ctx.out.push(Block::Quote(t));
            }
        }
        "pre" => {
            let raw: String = el.text().collect();
            let lines: Vec<String> = raw
                .lines()
                .take(80)
                .map(|l| {
                    crate::textsafe::strip_forbidden(&l.replace('\t', "    ")).trim_end().to_string()
                })
                .collect();
            if lines.iter().any(|l| !l.trim().is_empty()) {
                ctx.out.push(Block::Code(lines));
            }
        }
        "hr" => ctx.out.push(Block::Rule),
        "table" | "tr" => {
            // Flatten rows to one paragraph each; wiki tables are skipped earlier.
            if name == "table" {
                for c in el.descendants().filter_map(ElementRef::wrap) {
                    if c.value().name() == "tr" && !skipped(&c, ctx) {
                        let cells: Vec<String> = c
                            .children()
                            .filter_map(ElementRef::wrap)
                            .filter(|x| matches!(x.value().name(), "td" | "th"))
                            .map(|x| clean_text(&x.text().collect::<String>()))
                            .filter(|s| !s.is_empty())
                            .collect();
                        if !cells.is_empty() && !full(ctx) {
                            ctx.out.push(Block::Para(cells.join(" | ")));
                        }
                    }
                    if ctx.out.len() >= ctx.opts.max_blocks {
                        break;
                    }
                }
            }
        }
        "figure" => {
            // Images first, then caption text, via the normal child walk.
            walk_children(el, ctx, depth);
        }
        _ => walk_children(el, ctx, depth),
    }
}

/// Inline text of an element; links become `text [n]`, images are queued.
fn inline_text(el: ElementRef, ctx: &mut Ctx) -> String {
    let mut s = String::new();
    for child in el.children() {
        match child.value() {
            Node::Text(t) => s.push_str(t),
            Node::Element(_) => {
                let ce = ElementRef::wrap(child).unwrap();
                if skipped(&ce, ctx) {
                    continue;
                }
                inline(ce, ctx, &mut s, 0);
            }
            _ => {}
        }
    }
    clean_text(&s)
}

fn inline(el: ElementRef, ctx: &mut Ctx, out: &mut String, depth: usize) {
    if depth > 48 {
        return;
    }
    let name = el.value().name();
    match name {
        "br" => out.push(' '),
        "img" => {
            if let Some(b) = image_block(&el, ctx.base) {
                ctx.pending_images.push(b);
            }
        }
        "a" => {
            let mut label = String::new();
            for c in el.children() {
                match c.value() {
                    Node::Text(t) => label.push_str(t),
                    Node::Element(_) => {
                        let ce = ElementRef::wrap(c).unwrap();
                        if !skipped(&ce, ctx) {
                            inline(ce, ctx, &mut label, depth + 1);
                        }
                    }
                    _ => {}
                }
            }
            let label = clean_text(&label);
            out.push_str(&label);
            if let Some(target) = el.value().attr("href").and_then(|h| resolve_link(h, ctx)) {
                let l = if label.is_empty() { "link".to_string() } else { label.clone() };
                if let Some(n) = ctx.links.add(target, &l, None) {
                    if !label.is_empty() {
                        out.push_str(&format!(" [{n}]"));
                    }
                }
            }
        }
        _ => {
            for c in el.children() {
                match c.value() {
                    Node::Text(t) => out.push_str(t),
                    Node::Element(_) => {
                        let ce = ElementRef::wrap(c).unwrap();
                        if !skipped(&ce, ctx) {
                            if is_block(ce.value().name()) {
                                out.push(' ');
                            }
                            inline(ce, ctx, out, depth + 1);
                            if is_block(ce.value().name()) {
                                out.push(' ');
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn image_block(el: &ElementRef, base: &Url) -> Option<Block> {
    let v = el.value();
    let raw = v.attr("src").filter(|s| !s.trim().is_empty() && !s.trim_start().starts_with("data:")).or_else(|| v.attr("data-src"))?;
    // Tracking pixels / icons.
    let small = |a: &str| v.attr(a).and_then(|x| x.trim().parse::<u32>().ok()).map(|n| n <= 16).unwrap_or(false);
    if small("width") || small("height") {
        return None;
    }
    let url = base.join(raw.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    // SVG from the web is never fetched (decoder DoS surface): alt text only.
    let path = url.path().to_ascii_lowercase();
    if path.ends_with(".svg") {
        let alt = crate::textsafe::attr_text(v.attr("alt").unwrap_or(""), 120);
        return Some(Block::Image { src: String::new(), alt, local: None });
    }
    let alt = crate::textsafe::attr_text(v.attr("alt").unwrap_or(""), 120);
    Some(Block::Image { src: url.to_string(), alt, local: None })
}

fn resolve_link(href: &str, ctx: &Ctx) -> Option<LinkTarget> {
    let href = href.trim();
    if href.is_empty() || href.starts_with('#') {
        return None;
    }
    if let Some(lang) = &ctx.opts.wiki_lang {
        let rest = href.strip_prefix("./").or_else(|| href.strip_prefix("/wiki/"));
        if let Some(rest) = rest {
            let title = rest.split(['#', '?']).next().unwrap_or("");
            let title = percent_decode(title).replace('_', " ");
            if title.is_empty() || title.contains(':') || title.len() > 200 {
                return None;
            }
            return Some(LinkTarget::Wiki { lang: lang.clone(), title });
        }
    }
    let url = ctx.base.join(href).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.as_str().len() > crate::netguard::MAX_URL_LEN {
        return None;
    }
    let mut url = url;
    url.set_fragment(None);
    Some(LinkTarget::Url(url.to_string()))
}

/// Minimal percent-decoder (invalid sequences are kept literally).
pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = ((b[i + 1] as char).to_digit(16), (b[i + 2] as char).to_digit(16)) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conv(html: &str) -> (Vec<Block>, LinkTable) {
        let mut links = LinkTable::default();
        let base = Url::parse("https://example.com/a/b").unwrap();
        let blocks = html_to_blocks(html, &base, &Opts { wiki_lang: None, max_blocks: 500 }, &mut links);
        (blocks, links)
    }

    #[test]
    fn structure_is_extracted() {
        let (b, l) = conv(r#"<h1>Title</h1><p>Hello <a href="/x">world</a> and <b>bold</b>.</p>
            <ul><li>one</li><li>two <a href="https://other.org/p#frag">t</a></li></ul>
            <blockquote>quoted</blockquote><pre>fn main() {
    println!();
}</pre><hr>"#);
        assert_eq!(b[0], Block::Heading(1, "Title".into()));
        assert_eq!(b[1], Block::Para("Hello world [1] and bold.".into()));
        assert_eq!(b[2], Block::List { ordered: false, items: vec!["one".into(), "two t [2]".into()] });
        assert_eq!(b[3], Block::Quote("quoted".into()));
        assert!(matches!(&b[4], Block::Code(lines) if lines.len() == 3 && lines[1].starts_with("    println")));
        assert_eq!(b[5], Block::Rule);
        assert_eq!(l.entries[0].target, LinkTarget::Url("https://example.com/x".into()));
        assert_eq!(l.entries[1].target, LinkTarget::Url("https://other.org/p".into()), "fragment dropped");
    }

    #[test]
    fn duplicate_links_share_a_number() {
        let (b, l) = conv(r#"<p><a href="/x">a</a> <a href="/x">b</a></p>"#);
        assert_eq!(b[0], Block::Para("a [1] b [1]".into()));
        assert_eq!(l.entries.len(), 1);
    }

    #[test]
    fn dangerous_links_are_dropped_but_text_kept() {
        let (b, l) = conv(r#"<p><a href="javascript:alert(1)">js</a> <a href="data:text/html,x">d</a> <a href="file:///etc/passwd">f</a> <a href="mailto:a@b.c">m</a></p>"#);
        assert_eq!(b[0], Block::Para("js d f m".into()));
        assert!(l.entries.is_empty());
    }

    #[test]
    fn scripts_styles_hidden_and_forms_are_removed() {
        let (b, _) = conv(r#"<p>keep</p><script>alert(1)</script><style>p{}</style><p hidden>no</p>
            <p style="display: none">no2</p><form><p>no3</p></form><noscript>no4</noscript><svg><text>no5</text></svg>"#);
        assert_eq!(b, vec![Block::Para("keep".into())]);
    }

    #[test]
    fn images_resolve_and_svg_is_alt_only() {
        let (b, _) = conv(r#"<p><img src="/i/a.png" alt="A &quot;pic&quot;"></p><img src="x.svg" alt="logo"><img src="data:image/png;base64,AAAA"><img src="/p.gif" width="1" height="1"><img src="ftp://h/a.png">"#);
        assert_eq!(b.len(), 2);
        assert!(matches!(&b[0], Block::Image { src, alt, .. } if src == "https://example.com/i/a.png" && !alt.contains('"')));
        assert!(matches!(&b[1], Block::Image { src, alt, .. } if src.is_empty() && alt == "logo"));
    }

    #[test]
    fn wiki_links_resolve_to_titles() {
        let mut links = LinkTable::default();
        let base = Url::parse("https://pl.wikipedia.org/api/rest_v1/page/html/X").unwrap();
        let o = Opts { wiki_lang: Some("pl".into()), max_blocks: 100 };
        let b = html_to_blocks(r##"<p><a href="./Zielona_G%C3%B3ra#a">ZG</a> <a href="./Plik:X.jpg">f</a> <sup class="mw-ref"><a href="#cite">[1]</a></sup> <a href="https://x.org/">x</a></p><table><tr><td>t</td></tr></table>"##, &base, &o, &mut links);
        assert_eq!(b, vec![Block::Para("ZG [1] f x [2]".into())]);
        assert_eq!(links.entries[0].target, LinkTarget::Wiki { lang: "pl".into(), title: "Zielona Góra".into() });
    }

    #[test]
    fn block_cap_is_honoured() {
        let html: String = (0..100).map(|i| format!("<p>p{i}</p>")).collect();
        let mut links = LinkTable::default();
        let b = html_to_blocks(&html, &Url::parse("https://e.com/").unwrap(), &Opts { wiki_lang: None, max_blocks: 10 }, &mut links);
        assert_eq!(b.len(), 10);
    }

    #[test]
    fn percent_decode_edge_cases() {
        assert_eq!(percent_decode("a%20b%"), "a b%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%41"), "A");
    }
}
