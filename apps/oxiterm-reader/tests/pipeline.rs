//! End-to-end pipeline tests over a scripted fake network (no real network).

use oxiterm_proto::style::TerminalProfile;
use oxiterm_reader::app::{App, Event};
use oxiterm_reader::config::{Config, FeedConfig};
use oxiterm_reader::fetch::{Backend, FetchError, RawResponse, SafeFetcher};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::CellBuffer;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use url::Url;

const ARTICLE: &str = include_str!("fixtures/article.html");
const HOSTILE: &str = include_str!("fixtures/hostile.html");
const WIKI: &str = include_str!("fixtures/wiki.html");
const RSS: &str = include_str!("fixtures/feed.rss");

struct Fake {
    routes: HashMap<String, RawResponse>,
    seen: Mutex<Vec<String>>,
}
struct Shared(Arc<Fake>);

impl Backend for Shared {
    fn get(&self, url: &Url, _a: &str, max: usize) -> Result<RawResponse, FetchError> {
        self.0.seen.lock().unwrap().push(url.to_string());
        let r = self.0.routes.get(url.as_str()).cloned().ok_or(FetchError::Network("no route".into()))?;
        if r.body.len() > max {
            return Err(FetchError::TooLarge);
        }
        Ok(r)
    }
}

fn resp(ct: &str, body: &[u8]) -> RawResponse {
    RawResponse { status: 200, location: None, content_type: Some(ct.into()), body: body.to_vec() }
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255]));
    let mut c = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(img).write_to(&mut c, image::ImageFormat::Png).unwrap();
    c.into_inner()
}

fn tmpdir(tag: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("reader-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn routes() -> HashMap<String, RawResponse> {
    let mut r = HashMap::new();
    let mut add = |u: &str, x: RawResponse| {
        r.insert(u.to_string(), x);
    };
    add("https://blog.example.org/posts/rust-reader", resp("text/html; charset=utf-8", ARTICLE.as_bytes()));
    add("https://blog.example.org/img/diagram.png", resp("image/png", &png(640, 320)));
    add("https://hostile.example.com/page", resp("text/html", HOSTILE.as_bytes()));
    add("https://img.example.com/a.png", resp("image/png", &png(300, 200)));
    add("https://ok.example.com/a?b=1&c=2", resp("text/html", ARTICLE.as_bytes()));
    add("https://hacker-news.firebaseio.com/v0/topstories.json", resp("application/json", b"[101,102,103]"));
    add("https://hacker-news.firebaseio.com/v0/item/101.json", resp("application/json", br#"{"id":101,"title":"Show HN: <b>Reader</b> \u2192 mode","url":"https://blog.example.org/posts/rust-reader","score":321,"by":"ada","descendants":45,"type":"story"}"#));
    add("https://hacker-news.firebaseio.com/v0/item/102.json", resp("application/json", br#"{"id":102,"title":"Ask HN: anything?","text":"Body with <a href=\"https://ok.example.com/a?b=1&amp;c=2\">a link</a> and <p>second para","score":5,"by":"bob","descendants":1}"#));
    add("https://hacker-news.firebaseio.com/v0/item/103.json", resp("application/json", br#"{"id":103,"deleted":true}"#));
    add("https://pl.wikipedia.org/w/rest.php/v1/search/title?q=Zielona%20G%C3%B3ra&limit=12", resp("application/json", br#"{"pages":[{"title":"Zielona G\u00f3ra","description":"miasto w Polsce"},{"title":"Zielona G\u00f3ra (powiat)"}]}"#));
    add("https://pl.wikipedia.org/api/rest_v1/page/html/Zielona_G%C3%B3ra", resp("text/html", WIKI.as_bytes()));
    add("https://en.wikipedia.org/api/rest_v1/page/random/summary", resp("application/json", br#"{"title":"Zielona G\u00f3ra"}"#));
    add("https://en.wikipedia.org/api/rest_v1/page/html/Zielona_G%C3%B3ra", resp("text/html", WIKI.as_bytes()));
    add("https://upload.wikimedia.org/thumb/zg.png", resp("image/png", &png(220, 147)));
    add("https://blog.example.org/feed.xml", resp("application/rss+xml", RSS.as_bytes()));
    r
}

fn app(tag: &str, tweak: impl FnOnce(&mut Config)) -> (Arc<App>, Arc<Fake>, PathBuf) {
    let dir = tmpdir(tag);
    let mut cfg = Config { site_dir: dir.clone(), feeds: vec![FeedConfig { name: "Blog".into(), url: "https://blog.example.org/feed.xml".into() }], ..Config::default() };
    tweak(&mut cfg);
    let fake = Arc::new(Fake { routes: routes(), seen: Mutex::new(vec![]) });
    let fetcher = SafeFetcher::with_backend(&cfg, Box::new(Shared(fake.clone())));
    (Arc::new(App::new(cfg, fetcher).unwrap()), fake, dir)
}

fn ev(action: &str, state: &[(&str, &str)], sid: u64) -> Event {
    Event { action: action.into(), state: state.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(), session_id: sid }
}

fn navigate(patch: &serde_json::Value) -> String {
    patch.get("navigate").and_then(|v| v.as_str()).unwrap_or_else(|| panic!("no navigate in {patch}")).to_string()
}

/// Walks the parsed document: every action and src must match the allow-lists.
fn assert_safe_doc(path: &Path) {
    let src = std::fs::read_to_string(path).unwrap();
    let doc = THTMLParser::parse(&src).unwrap_or_else(|e| panic!("{path:?} does not parse: {e}\n{src}"));
    let mut stack = vec![doc.root];
    let mut n_actions = 0;
    while let Some(id) = stack.pop() {
        let node = doc.arena.get(id).unwrap();
        if let Some(a) = &node.attrs.event_htmx {
            n_actions += 1;
            let ok = a == "go_home"
                || a.starts_with("open_link:") && a[10..].split_once('.').map(|(p, n)| p.len() == 32 && p.bytes().all(|b| b.is_ascii_hexdigit()) && n.bytes().all(|b| b.is_ascii_digit())).unwrap_or(false)
                || a.starts_with("open_original:") && a[14..].len() == 32;
            assert!(ok, "unexpected action {a:?} in {path:?}");
        }
        if let Some(s) = &node.attrs.src {
            assert!(s.len() > 4 && s.ends_with(".png") && s[..s.len() - 4].bytes().all(|b| b.is_ascii_hexdigit()), "unexpected src {s:?}");
        }
        for (_, text, symbols, _) in doc.clickable_ambiguous_width() {
            assert!(symbols.is_empty(), "ambiguous clickable {text:?}");
        }
        stack.extend(node.children.iter().copied());
    }
    assert!(n_actions >= 1);
}

/// Parse + layout + render without panics, at desktop and mobile sizes.
fn assert_renders(path: &Path, site: &Path, cols: u16, rows: u16) {
    let src = std::fs::read_to_string(path).unwrap();
    let mut doc = THTMLParser::parse(&src).unwrap();
    let mut engine = LayoutEngine::new();
    engine.set_viewport_rows(rows);
    let layout = engine.compute(&mut doc, cols, 0, None).unwrap();
    let mut buf = CellBuffer::new(cols, rows);
    let mut profile = TerminalProfile::default();
    profile.is_web = false;
    Renderer::render_node(&doc, &layout, &mut buf, &profile, path.parent(), Some(site), 0);
    profile.is_web = true;
    Renderer::render_node(&doc, &layout, &mut buf, &profile, path.parent(), Some(site), 0);
}

fn export_for_lint(site: &Path, name: &str) {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("reader-lint");
    std::fs::create_dir_all(&out).unwrap();
    for e in std::fs::read_dir(site.join("cache")).unwrap().flatten() {
        if e.path().extension().and_then(|x| x.to_str()) == Some("thtml") {
            std::fs::copy(e.path(), out.join(format!("{name}-{}", e.file_name().to_string_lossy()))).unwrap();
        }
    }
    for f in ["index.thtml", "index_mobile.thtml"] {
        std::fs::copy(site.join(f), out.join(format!("{name}-{f}"))).unwrap();
    }
}

#[test]
fn article_end_to_end_with_images_and_navigate() {
    let (app, fake, site) = app("article", |_| {});
    let p = app.handle(&ev("fetch_url", &[("url", "blog.example.org/posts/rust-reader")], 1)).unwrap();
    let nav = navigate(&p);
    assert!(nav.starts_with("cache/") && nav.ends_with(".thtml") && !nav.contains("_mobile") && !nav.contains(".."), "{nav}");
    let page = site.join(&nav);
    assert!(page.exists());
    let id = nav.trim_start_matches("cache/").trim_end_matches(".thtml");
    assert!(site.join(format!("cache/{id}_mobile.thtml")).exists());
    let src = std::fs::read_to_string(&page).unwrap();
    assert!(src.contains("Reading the web"));
    assert!(src.contains("<img src=\""), "image emitted");
    assert!(!src.contains("SUBSCRIBE NOW"));
    // The downloaded image was re-encoded and stored as PNG under cache/.
    let pngs: Vec<_> = std::fs::read_dir(site.join("cache")).unwrap().flatten().filter(|e| e.path().extension().map(|x| x == "png").unwrap_or(false)).collect();
    assert_eq!(pngs.len(), 1);
    assert_eq!(&std::fs::read(pngs[0].path()).unwrap()[..4], b"\x89PNG");
    assert_safe_doc(&page);
    assert_safe_doc(&site.join(format!("cache/{id}_mobile.thtml")));
    assert_renders(&page, &site, 80, 24);
    assert_renders(&site.join(format!("cache/{id}_mobile.thtml")), &site, 40, 24);
    // Images really resolve from the page's directory (web overlay path and Kitty path).
    {
        let mut doc = THTMLParser::parse(&src).unwrap();
        let mut engine = LayoutEngine::new();
        engine.set_viewport_rows(24);
        let layout = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rows = layout.total_height.max(24);
        let mut profile = TerminalProfile::default();
        profile.is_web = true;
        let mut buf = CellBuffer::new(80, rows);
        Renderer::render_node(&doc, &layout, &mut buf, &profile, page.parent(), Some(&site), 0);
        let glyphs: String = buf.cells.iter().map(|c| c.ch).collect();
        assert!(!glyphs.contains("brak grafiki") && !glyphs.contains("missing"), "image placeholder drawn");
        let mut profile = TerminalProfile::default();
        profile.is_web = false;
        profile.supports_kitty_gfx = true;
        let mut buf = CellBuffer::new(80, rows);
        Renderer::render_node(&doc, &layout, &mut buf, &profile, page.parent(), Some(&site), 0);
        assert!(!buf.graphics.is_empty(), "Kitty graphics payload expected for the stored PNG");
    }
    // Second request within the TTL is served from cache: no new network traffic.
    let before = fake.seen.lock().unwrap().len();
    let p2 = app.handle(&ev("fetch_url", &[("url", "blog.example.org/posts/rust-reader")], 1)).unwrap();
    assert_eq!(navigate(&p2), nav);
    assert_eq!(fake.seen.lock().unwrap().len(), before);
    export_for_lint(&site, "article");
}

#[test]
fn hostile_page_cannot_inject_markup_or_actions() {
    let (app, _fake, site) = app("hostile", |_| {});
    let p = app.handle(&ev("fetch_url", &[("url", "https://hostile.example.com/page")], 2)).unwrap();
    let page = site.join(navigate(&p));
    let src = std::fs::read_to_string(&page).unwrap();
    // Nothing from the page is an element or attribute.
    assert!(!src.contains("<script"), "{src}");
    assert!(!src.contains("onclick"));
    // The host may be shown as link-list text, but never inside an attribute value.
    for chunk in src.split('"').skip(1).step_by(2) {
        assert!(!chunk.contains("evil.example.com") && !chunk.contains("http"), "attribute value {chunk:?} carries page data");
    }
    assert!(!src.contains("https://evil"), "full URLs never appear");
    assert!(!src.contains("event-htmx=\"evil") && !src.contains("event-htmx=\"steal") && !src.contains("event-htmx=\"x\""));
    assert!(!src.contains('\u{1b}') && !src.contains('\u{202e}') && !src.contains('\u{7}'));
    assert!(src.contains("&lt;script&gt;alert(1)&lt;/script&gt;") || src.contains("alert(1)"), "literal text is shown, escaped");
    assert!(src.contains("and &amp;lt; double entity"), "double entities stay literal: {src}");
    assert_safe_doc(&page);
    assert_renders(&page, &site, 80, 24);
    // Long unbreakable token was shortened to the column.
    assert!(!src.contains(&"A".repeat(80)));
    // XML-wellformed enough for the lint script.
    export_for_lint(&site, "hostile");
}

#[test]
fn hn_list_story_and_open_link_flow() {
    let (app, fake, site) = app("hn", |_| {});
    let p = app.handle(&ev("hn_top", &[], 3)).unwrap();
    let list = site.join(navigate(&p));
    let src = std::fs::read_to_string(&list).unwrap();
    assert!(src.contains("Show HN") && src.contains("321 points by ada"));
    assert!(src.contains("Ask HN: anything"), "story 102 listed; deleted 103 skipped");
    assert!(!src.contains('\u{2192}'));
    assert_safe_doc(&list);
    assert_renders(&list, &site, 80, 24);
    let id = navigate(&p).trim_start_matches("cache/").trim_end_matches(".thtml").to_string();
    // Clicking story 1 (id 101) opens the story page via an opaque link id.
    let p2 = app.handle(&ev(&format!("open_link:{id}.1"), &[], 3)).unwrap();
    let story = site.join(navigate(&p2));
    assert_safe_doc(&story);
    assert!(std::fs::read_to_string(&story).unwrap().contains("Read the article"));
    let sid = navigate(&p2).trim_start_matches("cache/").trim_end_matches(".thtml").to_string();
    // "Read the article" -> link 1 on the story page -> the article URL goes through the pipeline.
    let before = fake.seen.lock().unwrap().len();
    let p3 = app.handle(&ev(&format!("open_link:{sid}.1"), &[], 3)).unwrap();
    assert!(navigate(&p3).starts_with("cache/"));
    assert!(fake.seen.lock().unwrap()[before..].iter().any(|u| u == "https://blog.example.org/posts/rust-reader"));
    // Ask-HN story renders its text and links.
    let p4 = app.handle(&ev("open_story:102", &[], 3)).unwrap();
    let ask = site.join(navigate(&p4));
    assert_safe_doc(&ask);
    assert_renders(&ask, &site, 80, 24);
    // open_original yields a one-shot open_url for web clients.
    let p5 = app.handle(&ev(&format!("open_original:{sid}"), &[], 3)).unwrap();
    assert_eq!(p5["open_url"], "https://news.ycombinator.com/item?id=101");
    export_for_lint(&site, "hn");
}

#[test]
fn forged_or_stale_link_ids_fetch_nothing() {
    let (app, fake, _site) = app("forge", |_| {});
    let fake_id = "0123456789abcdef0123456789abcdef";
    for a in [
        "open_link:../../etc/passwd.1".to_string(),
        format!("open_link:{fake_id}.1"),
        format!("open_link:{fake_id}.99"),
        "open_link:https://evil.example.com/".to_string(),
        "open_link:".to_string(),
        "open_story:notanumber".to_string(),
        "wiki_search:../x".to_string(),
        "rss:99".to_string(),
        "open_original:../../x".to_string(),
    ] {
        let p = app.handle(&ev(&a, &[], 9));
        if let Some(p) = p {
            assert!(p.get("navigate").is_none(), "{a} -> {p}");
        }
    }
    assert!(fake.seen.lock().unwrap().is_empty(), "no request may be made: {:?}", fake.seen.lock().unwrap());
    assert_eq!(app.handle(&ev("go_home", &[], 9)).unwrap()["navigate"], "index.thtml");
    assert!(app.handle(&ev("set:tab=player", &[], 9)).is_none());
    assert!(app.handle(&ev("inc:counter", &[], 9)).is_none());
}

#[test]
fn wikipedia_search_random_and_article() {
    let (app, _f, site) = app("wiki", |_| {});
    let p = app.handle(&ev("wiki_search:pl", &[("wiki_q", "Zielona Góra")], 4)).unwrap();
    let list = site.join(navigate(&p));
    assert_safe_doc(&list);
    let id = navigate(&p).trim_start_matches("cache/").trim_end_matches(".thtml").to_string();
    let p2 = app.handle(&ev(&format!("open_link:{id}.1"), &[], 4)).unwrap();
    let art = site.join(navigate(&p2));
    let src = std::fs::read_to_string(&art).unwrap();
    assert!(src.contains("founded long ago") && src.contains("Old town"));
    assert!(!src.contains("Population"), "tables skipped");
    assert!(src.contains("<img src="), "wikimedia image downloaded");
    assert_safe_doc(&art);
    assert_renders(&art, &site, 80, 24);
    // Wiki-internal links resolve to Wiki targets (not URLs): link 1 is "Poland".
    let aid = navigate(&p2).trim_start_matches("cache/").trim_end_matches(".thtml").to_string();
    assert!(app.store.link(&aid, 1).is_some());
    let p3 = app.handle(&ev("wiki_random:en", &[], 4)).unwrap();
    assert!(navigate(&p3).starts_with("cache/"));
    assert_eq!(app.handle(&ev("wiki_search:pl", &[("wiki_q", "  ")], 4)).unwrap()["status"], "Error: type a search term first");
    export_for_lint(&site, "wiki");
}

#[test]
fn rss_feed_page() {
    let (app, _f, site) = app("rss", |_| {});
    let p = app.handle(&ev("rss:0", &[], 5)).unwrap();
    let page = site.join(navigate(&p));
    let src = std::fs::read_to_string(&page).unwrap();
    assert!(src.contains("Rust 2.0 &lt;released&gt; &amp; more"));
    assert_safe_doc(&page);
    assert_renders(&page, &site, 80, 24);
    export_for_lint(&site, "rss");
}

#[test]
fn policy_ssrf_rate_limit_and_errors() {
    let (app, fake, _s) = app("policy", |c| {
        c.allow_any_url = false;
        c.actions_per_minute = 3;
    });
    let status = |p: Option<serde_json::Value>| p.unwrap()["status"].as_str().unwrap().to_string();
    assert!(status(app.handle(&ev("fetch_url", &[("url", "https://example.com/")], 6))).contains("refused"), "allow list");
    assert!(status(app.handle(&ev("fetch_url", &[("url", "http://169.254.169.254/latest")], 6))).starts_with("Error:"));
    assert!(status(app.handle(&ev("fetch_url", &[("url", "http://localhost:22/")], 6))).starts_with("Error:"));
    assert!(fake.seen.lock().unwrap().is_empty());
    assert!(status(app.handle(&ev("fetch_url", &[("url", "http://127.0.0.1/")], 6))).contains("Slow down"), "4th action in a minute");
    // Another session is unaffected.
    assert!(status(app.handle(&ev("fetch_url", &[("url", "")], 7))).contains("type a URL"));
}

#[test]
fn per_action_fetch_budget_limits_images() {
    let (app, fake, _s) = app("budget", |c| c.max_fetches_per_action = 1);
    let p = app.handle(&ev("fetch_url", &[("url", "https://blog.example.org/posts/rust-reader")], 8)).unwrap();
    assert!(p.get("navigate").is_some(), "page itself loads; image download is skipped");
    assert_eq!(fake.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn events_endpoint_auth_and_flow() {
    use hyper::{Method, StatusCode};
    let (app, _f, _s) = app("server", |_| {});
    let body = br#"{"action":"hn_top","state":{},"session_id":1,"username":"u","auth_method":"Guest"}"#;
    let r = |tok: &str, auth: Option<&str>, m: Method, path: &str, b: &[u8]| {
        let app = app.clone();
        let (tok, auth, path, b) = (tok.to_string(), auth.map(str::to_string), path.to_string(), b.to_vec());
        async move { oxiterm_reader::server::route(app, &tok, &m, &path, auth.as_deref(), &b).await }
    };
    assert_eq!(r("tok", None, Method::POST, "/events", body).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(r("tok", Some("Bearer nope"), Method::POST, "/events", body).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(r("", Some("Bearer "), Method::POST, "/events", body).await.0, StatusCode::UNAUTHORIZED, "fail closed");
    assert_eq!(r("tok", Some("Bearer tok"), Method::GET, "/events", body).await.0, StatusCode::NOT_FOUND);
    assert_eq!(r("tok", Some("Bearer tok"), Method::POST, "/events", b"{bad").await.0, StatusCode::BAD_REQUEST);
    let (code, json) = r("tok", Some("Bearer tok"), Method::POST, "/events", body).await;
    assert_eq!(code, StatusCode::OK);
    assert!(json.unwrap().contains("\"navigate\":\"cache/"));
    let (code, _) = r("tok", Some("Bearer tok"), Method::POST, "/events", br#"{"action":"set:tab=x","state":{},"session_id":1}"#).await;
    assert_eq!(code, StatusCode::NO_CONTENT);
}

#[test]
fn start_pages_parse_and_render() {
    let (_app, _f, site) = app("index", |_| {});
    for (f, cols, rows) in [("index.thtml", 80, 24), ("index_mobile.thtml", 40, 24)] {
        let p = site.join(f);
        let doc = THTMLParser::parse(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert!(doc.clickable_ambiguous_width().iter().all(|(_, _, s, _)| s.is_empty()));
        assert_renders(&p, &site, cols, rows);
    }
    export_for_lint(&site, "start");
}
