//! Data model shared by the extraction, rendering and action layers.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Where a numbered link leads. Stored server-side only; the page carries opaque ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v")]
pub enum LinkTarget {
    /// An absolute http(s) URL, opened through the article pipeline.
    Url(String),
    /// A Wikipedia article.
    Wiki {
        /// `pl` or `en`.
        lang: String,
        /// Article key (decoded title).
        title: String,
    },
    /// A Hacker News story id.
    HnStory(u64),
}

impl LinkTarget {
    /// Stable key used to de-duplicate links on one page.
    pub fn key(&self) -> String {
        match self {
            LinkTarget::Url(u) => format!("u:{u}"),
            LinkTarget::Wiki { lang, title } => format!("w:{lang}:{title}"),
            LinkTarget::HnStory(id) => format!("h:{id}"),
        }
    }
}

/// One entry of a page's link table (1-based numbering in the UI).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEntry {
    /// Destination.
    pub target: LinkTarget,
    /// Visible label (sanitised at render time).
    pub label: String,
    /// Optional second line (score, date, summary).
    pub note: Option<String>,
}

/// Ordered, de-duplicated, capped link table.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LinkTable {
    /// Entries; `n` in the UI is `index + 1`.
    pub entries: Vec<LinkEntry>,
    #[serde(skip)]
    index: HashMap<String, usize>,
}

/// Hard cap on links per page.
pub const MAX_LINKS: usize = 80;

impl LinkTable {
    /// Adds (or finds) a link and returns its 1-based number, or `None` when the table is full.
    pub fn add(&mut self, target: LinkTarget, label: &str, note: Option<String>) -> Option<usize> {
        let key = target.key();
        if let Some(&i) = self.index.get(&key) {
            return Some(i + 1);
        }
        if self.entries.len() >= MAX_LINKS {
            return None;
        }
        self.entries.push(LinkEntry { target, label: label.to_string(), note });
        self.index.insert(key, self.entries.len() - 1);
        Some(self.entries.len())
    }

    /// Resolves a 1-based number.
    pub fn get(&self, n: usize) -> Option<&LinkEntry> {
        n.checked_sub(1).and_then(|i| self.entries.get(i))
    }

    /// Rebuilds the dedup index after deserialisation.
    pub fn reindex(&mut self) {
        self.index = self.entries.iter().enumerate().map(|(i, e)| (e.target.key(), i)).collect();
    }
}

/// An image after download, validation and re-encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalImage {
    /// File name inside `cache/` (`<hash>.png`).
    pub file: String,
    /// Width in cells.
    pub cols: u16,
    /// Height in cells.
    pub rows: u16,
}

/// Structured content block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Block {
    /// Heading, level 1..=3 (deeper levels are folded into 3).
    Heading(u8, String),
    /// Paragraph with `[n]` link markers already inline.
    Para(String),
    /// List items.
    List {
        /// Numbered rather than bulleted.
        ordered: bool,
        /// Item texts.
        items: Vec<String>,
    },
    /// Block quote.
    Quote(String),
    /// Preformatted lines.
    Code(Vec<String>),
    /// Image; `local` is filled in by the image pipeline.
    Image {
        /// Absolute source URL.
        src: String,
        /// Alt text.
        alt: String,
        /// Downloaded copy.
        local: Option<LocalImage>,
    },
    /// Horizontal rule.
    Rule,
}

/// A page ready to render.
#[derive(Debug, Clone)]
pub struct Page {
    /// Title.
    pub title: String,
    /// Byline / site / meta line.
    pub meta: Option<String>,
    /// Source URL kept server-side for "open in browser".
    pub source_url: Option<String>,
    /// Body.
    pub blocks: Vec<Block>,
    /// Link table (numbered references).
    pub links: LinkTable,
    /// Heading above the link list ("Links", "Stories", ...).
    pub links_title: String,
    /// When true the link list is the primary content (HN list, search results, feed).
    pub list_mode: bool,
}

impl Page {
    /// Empty page with a title.
    pub fn new(title: &str) -> Self {
        Page {
            title: title.to_string(),
            meta: None,
            source_url: None,
            blocks: Vec::new(),
            links: LinkTable::default(),
            links_title: "Links".to_string(),
            list_mode: false,
        }
    }
}
