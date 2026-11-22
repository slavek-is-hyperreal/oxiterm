//! Action handling: the glue between `/events`, the sources and the page renderer.

use crate::blocks::{html_to_blocks, Opts};
use crate::config::Config;
use crate::extract::{extract_article, MAX_BLOCKS};
use crate::fetch::{Budget, FetchError, RateLimiter, SafeFetcher};
use crate::model::{Block, LinkTable, LinkTarget, Page};
use crate::render::{render_index, render_page, Geometry};
use crate::store::{parse_link_ref, Store};
use crate::textsafe::{clean_text, truncate_cols};
use crate::{hn, rss, wiki};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use url::Url;

/// One `POST /events` payload (only the fields the reader uses).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Event {
    /// Raw `event-htmx` string.
    pub action: String,
    /// Snapshot of the keys bound on the current page.
    #[serde(default)]
    pub state: HashMap<String, String>,
    /// OxiTerm session id.
    #[serde(default)]
    pub session_id: u64,
}

/// The application.
pub struct App {
    /// Settings.
    pub cfg: Config,
    /// Network access.
    pub fetcher: SafeFetcher,
    /// Generated-site storage.
    pub store: Store,
    limiter: RateLimiter,
}

type Res<T> = Result<T, String>;

fn fe(e: FetchError) -> String {
    e.to_string()
}

impl App {
    /// Builds the app and writes the start pages into the site dir.
    pub fn new(cfg: Config, fetcher: SafeFetcher) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&cfg.site_dir)?;
        let store = Store::open(&cfg.site_dir)?;
        let app = App { limiter: RateLimiter::new(cfg.actions_per_minute), cfg, fetcher, store };
        app.write_index()?;
        app.store.prune(Duration::from_secs(app.cfg.cache_max_age_hours * 3600));
        Ok(app)
    }

    /// (Re)writes `index.thtml` and `index_mobile.thtml`.
    pub fn write_index(&self) -> std::io::Result<()> {
        std::fs::write(self.cfg.site_dir.join("index.thtml"), render_index(&self.cfg, false))?;
        std::fs::write(self.cfg.site_dir.join("index_mobile.thtml"), render_index(&self.cfg, true))
    }

    /// Handles one event. `None` means "nothing to say" (204).
    pub fn handle(&self, ev: &Event) -> Option<Value> {
        let (verb, arg) = match ev.action.split_once(':') {
            Some((v, a)) => (v, a),
            None => (ev.action.as_str(), ""),
        };
        if arg.len() > 4096 {
            return None;
        }
        let known = matches!(verb, "fetch_url" | "hn_top" | "open_story" | "wiki_search" | "wiki_random" | "rss" | "open_link" | "open_original" | "go_home");
        if !known {
            return None; // built-in actions (set:..., inc:...) and anything unknown
        }
        if verb == "go_home" {
            // Pages live in cache/, and the engine resolves link targets relative to the
            // current page, so "Back" is an action that navigates from the site root.
            return Some(json!({"status": "", "navigate": "index.thtml"}));
        }
        if verb == "open_original" {
            return self.open_original(arg);
        }
        if !self.limiter.check(ev.session_id) {
            return Some(json!({"status": "Slow down: too many requests, wait a minute."}));
        }
        let budget = Budget::new(self.cfg.max_fetches_per_action);
        let r = match verb {
            "fetch_url" => self.fetch_url(ev.state.get("url").map(String::as_str).unwrap_or(""), &budget),
            "hn_top" => self.hn_top(&budget),
            "open_story" => arg.parse::<u64>().map_err(|_| "bad story id".to_string()).and_then(|id| self.hn_story(id, &budget)),
            "wiki_search" => wiki::valid_lang(arg).ok_or_else(|| "bad language".to_string()).and_then(|l| self.wiki_search(l, ev.state.get("wiki_q").map(String::as_str).unwrap_or(""), &budget)),
            "wiki_random" => wiki::valid_lang(arg).ok_or_else(|| "bad language".to_string()).and_then(|l| self.wiki_random(l, &budget)),
            "rss" => arg.parse::<usize>().map_err(|_| "bad feed".to_string()).and_then(|i| self.rss(i, &budget)),
            "open_link" => self.open_link(arg, &budget),
            _ => return None,
        };
        Some(match r {
            Ok((nav, _title)) => json!({
                "status": "",
                // Relative to the app base dir (the reader-site root), as the engine requires.
                "navigate": nav,
            }),
            Err(msg) => json!({"status": format!("Error: {}", truncate_cols(&clean_text(&msg), 66))}),
        })
    }

    fn open_original(&self, arg: &str) -> Option<Value> {
        let src = self.store.source(arg)?;
        let u = Url::parse(&src).ok()?;
        matches!(u.scheme(), "http" | "https").then(|| json!({"open_url": u.to_string()}))
    }

    fn open_link(&self, arg: &str, budget: &Budget) -> Res<(String, String)> {
        let (id, n) = parse_link_ref(arg).ok_or("bad link")?;
        let target = self.store.link(id, n).ok_or("this page has expired, go back and reopen it")?;
        match target {
            LinkTarget::Url(u) => self.article_for_url(&u, budget),
            LinkTarget::Wiki { lang, title } => {
                let lang = wiki::valid_lang(&lang).ok_or("bad language")?;
                self.wiki_article(lang, &title, budget)
            }
            LinkTarget::HnStory(id) => self.hn_story(id, budget),
        }
    }

    /// Normalises what the user typed: trims, adds `https://` when no scheme is given.
    pub fn normalize_input_url(raw: &str) -> Option<String> {
        let t = raw.trim();
        if t.is_empty() || t.len() > crate::netguard::MAX_URL_LEN || t.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return None;
        }
        if t.contains("://") {
            Some(t.to_string())
        } else if t.contains(':') && !t.contains('.') {
            None
        } else {
            Some(format!("https://{t}"))
        }
    }

    fn fetch_url(&self, raw: &str, budget: &Budget) -> Res<(String, String)> {
        if !self.cfg.allow_any_url && self.cfg.effective_allowed_hosts().is_none() {
            return Err("arbitrary URLs are disabled".into());
        }
        let u = Self::normalize_input_url(raw).ok_or("type a URL first")?;
        self.article_for_url(&u, budget)
    }

    /// Fetch + extract + render an arbitrary URL.
    pub fn article_for_url(&self, raw: &str, budget: &Budget) -> Res<(String, String)> {
        let url = self.fetcher.parse_url(raw).map_err(fe)?;
        let id = self.store.ids.page_id(&format!("url:{url}"));
        if let Some(nav) = self.cached(&id) {
            return Ok((nav, url.host_str().unwrap_or("page").to_string()));
        }
        let f = self.fetcher.fetch(&url, crate::fetch::Kind::Document, budget).map_err(fe)?;
        let ct = f.content_type.as_deref().unwrap_or("").to_ascii_lowercase();
        if !(ct.is_empty() || ct.contains("html") || ct.contains("xml") || ct.starts_with("text/plain")) {
            return Err(format!("unsupported content type ({})", truncate_cols(&clean_text(&ct), 30)));
        }
        let html = String::from_utf8_lossy(&f.body).into_owned();
        let page = extract_article(&html, &f.final_url);
        self.finish(page, &id, budget)
    }

    fn cached(&self, id: &str) -> Option<String> {
        self.store.fresh_page(id, Duration::from_secs(self.cfg.cache_ttl_secs))
    }

    /// Downloads images, renders both variants, writes files. Returns (navigate path, title).
    pub fn finish(&self, mut page: Page, id: &str, budget: &Budget) -> Res<(String, String)> {
        crate::images::process_images(&mut page.blocks, &self.fetcher, budget, &self.store.cache_dir, &self.store.ids, self.cfg.max_images_per_article, self.cfg.content_cols);
        let desktop = render_page(&page, id, Geometry::desktop(&self.cfg));
        let mobile = render_page(&page, id, Geometry::mobile());
        let nav = self.store.save_page(id, &desktop, page.source_url.as_deref(), &page.links).map_err(|e| format!("cannot write page: {e}"))?;
        self.store.save_mobile(id, &mobile).map_err(|e| format!("cannot write page: {e}"))?;
        Ok((nav, page.title))
    }

    fn hn_top(&self, budget: &Budget) -> Res<(String, String)> {
        let id = self.store.ids.page_id("hn:top");
        if let Some(nav) = self.cached(&id) {
            return Ok((nav, "Hacker News".into()));
        }
        let stories = hn::fetch_top(&self.fetcher, self.cfg.hn_stories, budget).map_err(fe)?;
        if stories.is_empty() {
            return Err("Hacker News returned no stories".into());
        }
        let mut page = Page::new("Hacker News - top stories");
        page.list_mode = true;
        page.links_title = "Stories".into();
        page.source_url = Some("https://news.ycombinator.com/".into());
        for s in &stories {
            page.links.add(LinkTarget::HnStory(s.id), &s.title, Some(format!("{} points by {} - {} comments", s.score, s.by, s.descendants)));
        }
        self.finish(page, &id, budget)
    }

    fn hn_story(&self, sid: u64, budget: &Budget) -> Res<(String, String)> {
        let id = self.store.ids.page_id(&format!("hn:story:{sid}"));
        if let Some(nav) = self.cached(&id) {
            return Ok((nav, "Story".into()));
        }
        let s = hn::fetch_story(&self.fetcher, sid, budget).map_err(fe)?;
        let mut page = Page::new(&s.title);
        page.meta = Some(format!("{} points by {} - {} comments", s.score, s.by, s.descendants));
        page.links_title = "Open".into();
        page.source_url = Some(format!("https://news.ycombinator.com/item?id={sid}"));
        if let Some(u) = s.url.as_deref().and_then(|u| Url::parse(u).ok()).filter(|u| matches!(u.scheme(), "http" | "https")) {
            page.links.add(LinkTarget::Url(u.to_string()), "Read the article", Some(u.host_str().unwrap_or("").to_string()));
        }
        page.links.add(LinkTarget::Url(format!("https://news.ycombinator.com/item?id={sid}")), "Discussion on Hacker News", None);
        if let Some(t) = &s.text {
            let base = Url::parse("https://news.ycombinator.com/").unwrap();
            let mut tmp = LinkTable::default();
            page.blocks = html_to_blocks(t, &base, &Opts { wiki_lang: None, max_blocks: 100 }, &mut tmp);
            for e in tmp.entries {
                page.links.add(e.target, &e.label, None);
            }
        }
        self.finish(page, &id, budget)
    }

    fn wiki_search(&self, lang: &str, q: &str, budget: &Budget) -> Res<(String, String)> {
        let q = truncate_cols(&clean_text(q), 100);
        if q.is_empty() {
            return Err("type a search term first".into());
        }
        let id = self.store.ids.page_id(&format!("wiki:s:{lang}:{q}"));
        if let Some(nav) = self.cached(&id) {
            return Ok((nav, q));
        }
        let url = Url::parse(&wiki::search_url(lang, &q)).map_err(|_| "bad query")?;
        let (_, body) = self.fetcher.fetch_text(&url, budget).map_err(fe)?;
        let results = wiki::parse_search(&body);
        if results.is_empty() {
            return Err("nothing found".into());
        }
        let mut page = Page::new(&format!("Wikipedia ({}): {}", lang.to_uppercase(), q));
        page.list_mode = true;
        page.links_title = "Results".into();
        for (title, desc) in results {
            page.links.add(LinkTarget::Wiki { lang: lang.to_string(), title: title.clone() }, &title, (!desc.is_empty()).then_some(desc));
        }
        self.finish(page, &id, budget)
    }

    fn wiki_random(&self, lang: &str, budget: &Budget) -> Res<(String, String)> {
        let url = Url::parse(&wiki::random_url(lang)).map_err(|_| "bad url")?;
        let (_, body) = self.fetcher.fetch_text(&url, budget).map_err(fe)?;
        let title = wiki::parse_summary_title(&body).ok_or("no random article")?;
        self.wiki_article(lang, &title, budget)
    }

    fn wiki_article(&self, lang: &str, title: &str, budget: &Budget) -> Res<(String, String)> {
        let title = clean_text(title);
        if title.is_empty() || title.len() > 200 {
            return Err("bad title".into());
        }
        let id = self.store.ids.page_id(&format!("wiki:a:{lang}:{title}"));
        if let Some(nav) = self.cached(&id) {
            return Ok((nav, title));
        }
        let api = Url::parse(&wiki::article_url(lang, &title)).map_err(|_| "bad title")?;
        let (final_url, html) = self.fetcher.fetch_text(&api, budget).map_err(fe)?;
        let mut page = Page::new(&title);
        page.meta = Some(format!("Wikipedia ({})", lang.to_uppercase()));
        page.source_url = Some(format!("https://{lang}.wikipedia.org/wiki/{}", wiki::encode_component(&title.replace(' ', "_"))));
        let opts = Opts { wiki_lang: Some(lang.to_string()), max_blocks: MAX_BLOCKS };
        let mut blocks = html_to_blocks(&html, &final_url, &opts, &mut page.links);
        // Parsoid repeats the title as the first heading.
        if matches!(blocks.first(), Some(Block::Heading(_, h)) if h.eq_ignore_ascii_case(&title)) {
            blocks.remove(0);
        }
        page.blocks = blocks;
        self.finish(page, &id, budget)
    }

    fn rss(&self, idx: usize, budget: &Budget) -> Res<(String, String)> {
        let feed_cfg = self.cfg.feeds.get(idx).ok_or("unknown feed")?;
        let id = self.store.ids.page_id(&format!("rss:{}", feed_cfg.url));
        if let Some(nav) = self.cached(&id) {
            return Ok((nav, feed_cfg.name.clone()));
        }
        let url = self.fetcher.parse_url(&feed_cfg.url).map_err(fe)?;
        let (final_url, xml) = self.fetcher.fetch_text(&url, budget).map_err(fe)?;
        let feed = rss::parse_feed(&xml, 30);
        let mut page = Page::new(&if feed.title.is_empty() { feed_cfg.name.clone() } else { feed.title.clone() });
        page.list_mode = true;
        page.links_title = "Entries".into();
        page.source_url = Some(final_url.to_string());
        for it in &feed.items {
            let Ok(link) = final_url.join(it.link.trim()) else { continue };
            if !matches!(link.scheme(), "http" | "https") {
                continue;
            }
            page.links.add(LinkTarget::Url(link.to_string()), &it.title, (!it.summary.is_empty()).then(|| it.summary.clone()));
        }
        if page.links.entries.is_empty() {
            return Err("the feed has no entries".into());
        }
        self.finish(page, &id, budget)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_url_normalisation() {
        assert_eq!(App::normalize_input_url("example.com/a").as_deref(), Some("https://example.com/a"));
        assert_eq!(App::normalize_input_url(" http://x.org ").as_deref(), Some("http://x.org"));
        assert_eq!(App::normalize_input_url("javascript:alert(1)"), None);
        assert_eq!(App::normalize_input_url(""), None);
        assert_eq!(App::normalize_input_url("a b"), None);
        assert!(App::normalize_input_url("file:///etc/passwd").is_some(), "kept as typed; the fetcher refuses the scheme");
    }
}
