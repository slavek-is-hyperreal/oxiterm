//! Article extraction: readability (dom_smoothie, a Mozilla Readability port) -> blocks.

use crate::blocks::{html_to_blocks, Opts};
use crate::model::{Block, LinkTable, Page};
use crate::textsafe::clean_text;
use scraper::{Html, Selector};
use url::Url;

/// Maximum blocks per page.
pub const MAX_BLOCKS: usize = 400;

/// Turns a fetched HTML document into a [`Page`].
pub fn extract_article(html: &str, url: &Url) -> Page {
    if markup_stats(html).1 > MAX_NESTING {
        let mut p = Page::new(url.host_str().unwrap_or("Page"));
        p.blocks.push(Block::Para("This page is too deeply nested to render safely.".to_string()));
        p.source_url = Some(url.to_string());
        return p;
    }
    let (title, byline, content) = readability(html, url).unwrap_or_else(|| fallback(html));
    let mut links = LinkTable::default();
    let opts = Opts { wiki_lang: None, max_blocks: MAX_BLOCKS };
    let mut blocks = html_to_blocks(&content, url, &opts, &mut links);
    if blocks.is_empty() {
        // Readability found nothing usable: try the whole body.
        let (_, _, body) = fallback(html);
        links = LinkTable::default();
        blocks = html_to_blocks(&body, url, &opts, &mut links);
    }
    // A leading heading that repeats the title is noise.
    if let Some(Block::Heading(_, h)) = blocks.first() {
        if clean_text(h).eq_ignore_ascii_case(&clean_text(&title)) {
            blocks.remove(0);
        }
    }
    let mut page = Page::new(&clean_text(&title));
    if page.title.is_empty() {
        page.title = url.host_str().unwrap_or("Article").to_string();
    }
    let host = url.host_str().unwrap_or("").to_string();
    page.meta = Some(match byline.map(|b| clean_text(&b)).filter(|b| !b.is_empty()) {
        Some(b) => format!("{b} - {host}"),
        None => host,
    });
    page.source_url = Some(url.to_string());
    page.blocks = blocks;
    page.links = links;
    page
}

/// Markup size statistics from a cheap linear scan: (tag count, maximum nesting depth).
pub fn markup_stats(html: &str) -> (usize, i64) {
    // Byte-level and linear: `>` is searched only in a short window after each `<`, and the
    // str is never sliced (no char-boundary panics, no quadratic rescans of tag-less input).
    let b = html.as_bytes();
    let (mut tags, mut depth, mut max_depth) = (0usize, 0i64, 0i64);
    let mut i = 0;
    while i + 1 < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        tags += 1;
        let c = b[i + 1];
        if c == b'/' {
            depth -= 1;
        } else if c.is_ascii_alphabetic() {
            let name: Vec<u8> = b[i + 1..].iter().take(12).take_while(|x| x.is_ascii_alphanumeric()).map(|x| x.to_ascii_lowercase()).collect();
            let window = &b[i + 1..(i + 1 + 256).min(b.len())];
            let self_closing = window.iter().position(|&x| x == b'>').map(|g| g > 0 && window[g - 1] == b'/').unwrap_or(false);
            let void = matches!(name.as_slice(), b"br" | b"img" | b"hr" | b"meta" | b"link" | b"input" | b"source" | b"wbr" | b"col" | b"area" | b"base" | b"embed" | b"param" | b"track");
            if !void && !self_closing {
                depth += 1;
                max_depth = max_depth.max(depth);
            }
        }
        i += 1;
    }
    (tags, max_depth)
}

/// Readability scoring (and html5ever tree building) is super-linear on deeply nested
/// markup: a 2000-deep `<div>` chain takes ~1 s, 20000 takes ~100 s. Moderately complex
/// documents bypass readability; absurd nesting is refused outright.
pub fn too_complex_for_readability(html: &str) -> bool {
    let (tags, depth) = markup_stats(html);
    tags > 40_000 || depth > 120
}

/// Nesting beyond this is an attack, not a page.
pub const MAX_NESTING: i64 = 1000;

fn readability(html: &str, url: &Url) -> Option<(String, Option<String>, String)> {
    if too_complex_for_readability(html) {
        return None;
    }
    let cfg = dom_smoothie::Config { max_elements_to_parse: 60_000, ..Default::default() };
    let html = html.to_string();
    let u = url.to_string();
    // Hostile markup must never take the server down: contain panics from the parser.
    let r = std::panic::catch_unwind(move || {
        let mut r = dom_smoothie::Readability::new(html, Some(&u), Some(cfg)).ok()?;
        let a = r.parse().ok()?;
        Some((a.title.to_string(), a.byline.map(|b| b.to_string()), a.content.to_string()))
    });
    r.ok().flatten()
}

fn fallback(html: &str) -> (String, Option<String>, String) {
    let doc = Html::parse_document(html);
    let title = Selector::parse("title")
        .ok()
        .and_then(|s| doc.select(&s).next().map(|e| e.text().collect::<String>()))
        .unwrap_or_default();
    for sel in ["article", "main", "body"] {
        if let Ok(s) = Selector::parse(sel) {
            if let Some(e) = doc.select(&s).next() {
                return (title, None, e.inner_html());
            }
        }
    }
    (title, None, String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub const FIXTURE: &str = include_str!("../tests/fixtures/article.html");

    #[test]
    fn extracts_title_body_links_images() {
        let url = Url::parse("https://blog.example.org/posts/rust-reader?utm=1").unwrap();
        let p = extract_article(FIXTURE, &url);
        assert!(p.title.contains("Reading the web"));
        assert!(p.blocks.iter().any(|b| matches!(b, Block::Para(t) if t.contains("readability"))));
        assert!(p.blocks.iter().any(|b| matches!(b, Block::Image { .. })));
        assert!(p.blocks.iter().any(|b| matches!(b, Block::Code(_))));
        assert!(!p.links.entries.is_empty());
        // Navigation / ads / scripts must not leak.
        let all = format!("{:?}", p.blocks);
        assert!(!all.contains("SUBSCRIBE NOW"), "{all}");
        assert!(!all.contains("trackingSecret"));
    }

    #[test]
    fn empty_and_garbage_input_does_not_panic() {
        let url = Url::parse("https://e.org/").unwrap();
        let started = std::time::Instant::now();
        for h in ["<a\u{e9}", &"<a".repeat(1_000_000)] {
            let _ = markup_stats(h);
        }
        assert!(started.elapsed() < std::time::Duration::from_secs(2), "markup_stats must be linear");
        for h in ["", "<", "<<<>>>", "\u{0}\u{1b}[2J", "<html><body></body></html>", &"<div>".repeat(20000)] {
            let p = extract_article(h, &url);
            assert!(!p.title.is_empty());
        }
    }
}
