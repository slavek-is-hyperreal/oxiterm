//! BAD ACTOR 7.6: hostile media content and sizes against the renderer. `#[ignore]`d tests assert
//! the desired behaviour and fail today (`--ignored`); the others pin what held.
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oxiterm_proto::style::{RenderMode, TerminalProfile};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::CellBuffer;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ba76_r_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn render(tag: &str, src: &str, w: u16, h: u16, profile: &TerminalProfile, app: &Path) -> (CellBuffer, Duration) {
    let (cols, rows) = (w + 4, h + 2);
    let page = format!(r#"<screen style="width: {cols}; height: {rows}; flex-direction: column;"><{tag} src="{src}" alt="x" style="width: {w}; height: {h};" /></screen>"#);
    let mut doc = THTMLParser::parse_with_mode(&page, RenderMode::Tui).unwrap();
    let mut engine = LayoutEngine::new();
    engine.set_viewport_rows(rows);
    let layout = engine.compute(&mut doc, cols, 0, None).unwrap();
    let mut buf = CellBuffer::new(cols, rows);
    let t = Instant::now();
    Renderer::render_node(&doc, &layout, &mut buf, profile, Some(app), Some(app), 0);
    (buf, t.elapsed())
}

/// Run `f` on a thread; fail the test (without hanging it) when it takes longer than `limit`.
fn bounded<T: Send + 'static>(limit: Duration, f: impl FnOnce() -> T + Send + 'static) -> Option<(T, Duration)> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let t = Instant::now();
        let r = f();
        let _ = tx.send((r, t.elapsed()));
    });
    rx.recv_timeout(limit).ok()
}

/// Largest viewport the wire accepts is 500x200 cells. The web Rive widget rasterises 8x16 px per
/// cell (4000x3200 = 12.8 Mpx) with a per-pixel f32 loop, then resizes. One-off per toggle state
/// (the asset cache keys on the state bits), so a few hundred ms is the worst case.
#[test]
fn rive_at_the_largest_viewport_is_bounded() {
    let d = tmp("rive");
    std::fs::write(d.join("t.riv"), [0u8; 4]).unwrap();
    let web = TerminalProfile { is_web: true, ..Default::default() };
    let ssh = TerminalProfile { supports_kitty_gfx: true, ..Default::default() };
    let (_, tw) = render("img", "t.riv", 496, 198, &web, &d);
    let (_, ts) = render("img", "t.riv", 496, 198, &ssh, &d);
    // second identical render: is it cached? (a web profile never reads the asset cache)
    let (_, tw2) = render("img", "t.riv", 496, 198, &web, &d);
    let (_, ts2) = render("img", "t.riv", 496, 198, &ssh, &d);
    eprintln!("RIVE 496x198 cells: web {tw:?} then {tw2:?}, ssh/kitty {ts:?} then {ts2:?}");
    // The bound is for release builds (what ships). Unoptimised debug builds of the image
    // encoders are ~30x slower; CI runs the workspace tests in debug, so allow more there.
    let bound = Duration::from_secs(if cfg!(debug_assertions) { 120 } else { 10 });
    assert!(tw < bound && ts < bound, "web {tw:?}, ssh {ts:?}, bound {bound:?}");
    // The cache makes every later identical frame cheap in any build.
    assert!(tw2 < Duration::from_secs(2) && ts2 < Duration::from_secs(2), "cached: web {tw2:?}, ssh {ts2:?}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Every refused `src` form must draw nothing for every tag (hostile spellings beyond the
/// parity test's list).
#[test]
fn hostile_src_spellings_draw_nothing() {
    let d = tmp("src");
    std::fs::create_dir_all(d.join("app")).unwrap();
    std::fs::write(d.join("secret.mmd"), "graph TD\nTOPSECRETLABEL-->B\n").unwrap();
    let app = d.join("app");
    let web = TerminalProfile { is_web: true, ..Default::default() };
    for src in ["../secret.mmd", "%2e%2e/secret.mmd", "..%2fsecret.mmd", "%252e%252e/secret.mmd", "\u{ff0e}\u{ff0e}/secret.mmd", "..\\secret.mmd", "/etc/passwd", "file:///etc/passwd"] {
        let (b, _) = render("diagram", src, 30, 10, &web, &app);
        // (a literal, missing file may draw the "missing" placeholder; it must never show the secret)
        let t: String = b.cells.iter().map(|c| c.ch).collect();
        assert!(!t.contains("TOPSECRET"), "{src:?} leaked the file");
    }
    let _ = std::fs::remove_dir_all(&d);
}

fn diagram_time(name: &str, mmd: String) -> (Option<Duration>, usize) {
    let d = tmp(name);
    std::fs::write(d.join("g.mmd"), &mmd).unwrap();
    let dd = d.clone();
    let r = bounded(Duration::from_secs(60), move || {
        let web = TerminalProfile { is_web: true, ..Default::default() };
        let (b, t) = render("diagram", "g.mmd", 80, 24, &web, &dd);
        let drawn = b.cells.iter().filter(|c| c.ch != ' ').count();
        eprintln!("   drawn cells: {drawn}");
        t
    });
    let _ = std::fs::remove_dir_all(&d);
    (r.map(|x| x.1), mmd.len())
}

/// Hostile Mermaid sources. Timings are printed (`--nocapture`); the server has no size cap on
/// the `.mmd` it reads (the browser mount caps at 256 KiB).
#[test]
fn mermaid_hostile_shapes_complete_in_time() {
    let mut rows = Vec::new();
    let chain = |n: usize| (0..n).map(|i| format!("N{i} --> N{}\n", i + 1)).collect::<String>();
    let fan = |n: usize| (0..n).map(|i| format!("R --> L{i}\n")).collect::<String>();
    let full = |n: usize| (0..n).flat_map(|i| (0..n).map(move |j| format!("A{i} --> A{j}\n"))).collect::<String>();
    let cases: Vec<(&str, String)> = vec![
        ("chain500", format!("graph TD\n{}", chain(500))),
        ("chain5000", format!("graph TD\n{}", chain(5000))),
        ("fan2000", format!("graph LR\n{}", fan(2000))),
        ("complete40", format!("graph TD\n{}", full(40))),
        ("cycle1000", format!("graph TD\n{}N1000 --> N0\n", chain(1000))),
        ("longlabel", format!("graph TD\nA[{}] --> B\n", "x".repeat(1_000_000))),
        ("nest2000", format!("graph TD\n{}A-->B\n{}", "subgraph s\n".repeat(2000), "end\n".repeat(2000))),
        // within the parser's caps (64 KiB, 256 nodes, 512 edges): the worst the layout can see
        ("chain255", format!("graph TD\n{}", chain(255))),
        ("fan255", format!("graph LR\n{}", fan(255))),
        ("dense256x512", format!("graph TD\n{}", (0..512).map(|i| format!("N{} --> N{}\n", i % 256, (i * 37 + 11) % 256)).collect::<String>())),
        ("label60k", format!("graph TD\nA[{}] --> B\n", "w".repeat(60_000))),
        ("wide_labels", format!("graph LR\n{}", (0..255).map(|i| format!("N{i}[{}] --> N{}\n", "x".repeat(200), i + 1)).collect::<String>())),
        ("junk", "\u{0}\u{ffff}[[[[(((({{{{ -->-->--> |||| ;;;;\n".repeat(20000)),
    ];
    for (n, src) in cases {
        let (t, len) = diagram_time(n, src);
        eprintln!("MERMAID {n:12} src {len:>9} B -> {}", t.map_or("TIMEOUT >60s".to_string(), |t| format!("{t:?}")));
        rows.push((n, t));
    }
    for (n, t) in &rows {
        assert!(t.is_some_and(|t| t < Duration::from_secs(20)), "{n}: {t:?}");
    }
}

/// Hostile Lottie JSON through rlottie (C++): a crash would kill this process, so each case runs in
/// a child (`BA76_LOTTIE=<case>`) with a timeout.
#[cfg(feature = "native-media")]
#[test]
fn lottie_hostile_documents_in_a_child() {
    if let Ok(case) = std::env::var("BA76_LOTTIE") {
        let json = lottie_case(&case);
        // Test correction (fixes.md F-06): the child used to call `rlottie::Animation::from_data` itself,
        // bypassing every guard. It now goes through the product loader; `REJECTED` is the safe outcome.
        let parsed = oxiterm_renderer::render::cache::parse_lottie_guarded(json);
        if let Err(e) = &parsed {
            println!("REJECTED {e:?}");
        }
        if let Ok(mut a) = parsed {
            let n = a.totalframe();
            let mut s = rlottie::Surface::new(rlottie::Size { width: 64, height: 64 });
            a.render(if n > 0 { n / 2 } else { 0 }, &mut s);
            println!("LOADED frames={n} size={:?}", a.size());
        }
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let mut bad = Vec::new();
    for case in ["huge_dims", "frames_1e9", "deep_precomp", "many_layers", "nan_inf", "bracket_bomb", "truncated", "huge_shape_points", "layer_ref_cycle", "dag_blowup", "points_near_cap", "unreachable_cycle", "duplicate_id"] {
        let t = Instant::now();
        let mut ch = std::process::Command::new(&exe)
            .args(["lottie_hostile_documents_in_a_child", "--exact", "--nocapture", "--test-threads=1"])
            .env("BA76_LOTTIE", case)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let mut res = "TIMEOUT";
        while t.elapsed() < Duration::from_secs(30) {
            if let Some(st) = ch.try_wait().unwrap() {
                {
                    use std::os::unix::process::ExitStatusExt;
                    eprintln!("   {case}: status {st:?} signal {:?}", st.signal());
                }
                res = if st.success() { "ok" } else { "CRASH" };
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        if res == "TIMEOUT" {
            let _ = ch.kill();
        }
        let out = ch.wait_with_output().map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(|l| l.find("LOADED").or_else(|| l.find("REJECTED")).map(|i| l[i..].to_string())).collect::<String>()).unwrap_or_default();
        eprintln!("LOTTIE {case:18} {res:8} {:?} {out}", t.elapsed());
        if res != "ok" {
            bad.push((case, res));
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
}

fn lottie_case(c: &str) -> String {
    let layer = |i: usize| format!(r#"{{"ty":4,"ind":{i},"ip":0,"op":60,"st":0,"ks":{{}},"shapes":[{{"ty":"rc","p":{{"a":0,"k":[0,0]}},"s":{{"a":0,"k":[50,50]}},"r":{{"a":0,"k":0}}}},{{"ty":"fl","c":{{"a":0,"k":[1,0,0,1]}},"o":{{"a":0,"k":100}}}}]}}"#);
    match c {
        "huge_dims" => r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":2000000000,"h":2000000000,"layers":[]}"#.into(),
        "frames_1e9" => format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":1000000000,"w":100,"h":100,"layers":[{}]}}"#, layer(1)),
        "deep_precomp" => {
            let n = 3000;
            let assets: Vec<String> = (0..n).map(|i| format!(r#"{{"id":"c{i}","layers":[{{"ty":0,"refId":"c{}","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{{}}}}]}}"#, i + 1)).collect();
            format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{}],"layers":[{{"ty":0,"refId":"c0","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{{}}}}]}}"#, assets.join(","))
        }
        "layer_ref_cycle" => r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{"id":"a","layers":[{"ty":0,"refId":"a","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{}}]}],"layers":[{"ty":0,"refId":"a","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{}}]}"#.into(),
        "many_layers" => format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{}]}}"#, (0..100_000).map(layer).collect::<Vec<_>>().join(",")),
        "nan_inf" => r#"{"v":"5.5.2","fr":1e999,"ip":-1e999,"op":1e999,"w":1e999,"h":-1e999,"layers":[{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{"p":{"a":0,"k":[1e999,-1e999]},"s":{"a":0,"k":[1e308,1e308]}},"shapes":[]}]}"#.into(),
        "bracket_bomb" => "[".repeat(5_000_000),
        "truncated" => r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{"ty":4,"#.into(),
        "huge_shape_points" => {
            let pts = (0..500_000).map(|i| format!("[{},{}]", i % 100, (i * 7) % 100)).collect::<Vec<_>>().join(",");
            format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{{}},"shapes":[{{"ty":"sh","ks":{{"a":0,"k":{{"c":true,"v":[{pts}],"i":[{pts}],"o":[{pts}]}}}}}},{{"ty":"fl","c":{{"a":0,"k":[1,0,0,1]}},"o":{{"a":0,"k":100}}}}]}}]}}"#)
        }
        // 40 levels, each instantiating the next one twice: 2^40 layer instances if expanded.
        "dag_blowup" => {
            let assets: Vec<String> = (0..40)
                .map(|i| format!(r#"{{"id":"c{i}","layers":[{{"ty":0,"refId":"c{n}","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{{}}}},{{"ty":0,"refId":"c{n}","ind":2,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{{}}}}]}}"#, n = i + 1))
                .chain(std::iter::once(r#"{"id":"c40","layers":[]}"#.to_string()))
                .collect();
            format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{}],"layers":[{{"ty":0,"refId":"c0","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{{}}}}]}}"#, assets.join(","))
        }
        // 19k points: the document is small, rlottie would not finish (cliff measured between 10k and 20k).
        "points_near_cap" => {
            let pts = (0..19_000).map(|i| format!("[{},{}]", i % 100, (i * 7) % 100)).collect::<Vec<_>>().join(",");
            format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{{}},"shapes":[{{"ty":"sh","ks":{{"a":0,"k":{{"c":true,"v":[{pts}],"i":[{pts}],"o":[{pts}]}}}}}},{{"ty":"fl","c":{{"a":0,"k":[1,0,0,1]}},"o":{{"a":0,"k":100}}}}]}}]}}"#)
        }
        // a precomp that refers to itself but that no layer instantiates
        "unreachable_cycle" => r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{"id":"z","layers":[{"ty":0,"refId":"z","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{}}]}],"layers":[]}"#.into(),
        // two assets with one id: the first refers to itself, the second is empty
        "duplicate_id" => r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"assets":[{"id":"a","layers":[{"ty":0,"refId":"a","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{}}]},{"id":"a","layers":[]}],"layers":[{"ty":0,"refId":"a","ind":1,"ip":0,"op":60,"st":0,"w":100,"h":100,"ks":{}}]}"#.into(),
        c if c.starts_with("points_") => {
            let n: usize = c["points_".len()..].parse().unwrap();
            let pts = (0..n).map(|i| format!("[{},{}]", i % 100, (i * 7) % 100)).collect::<Vec<_>>().join(",");
            format!(r#"{{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[{{"ty":4,"ind":1,"ip":0,"op":60,"st":0,"ks":{{}},"shapes":[{{"ty":"sh","ks":{{"a":0,"k":{{"c":true,"v":[{pts}],"i":[{pts}],"o":[{pts}]}}}}}},{{"ty":"fl","c":{{"a":0,"k":[1,0,0,1]}},"o":{{"a":0,"k":100}}}}]}}]}}"#)
        }
        _ => String::new(),
    }
}

/// F-12: `PlaybackRegistry` used to be a process-global keyed by file path only. One visitor's click
/// flipped the widget for every session showing the same file (and hover revealed that another visitor
/// is present). The state is scoped by session now (`TerminalProfile::playback_scope`).
///
/// Test adaptation (fixes.md F-12): the setters take the scope; visitor A clicks in scope 1 and visitor
/// B renders in scope 2.
#[test]
fn rive_state_is_per_session() {
    use oxiterm_renderer::render::cache::PlaybackRegistry;
    let d = tmp("riveglobal");
    std::fs::write(d.join("t.riv"), [0u8; 4]).unwrap();
    let a = TerminalProfile { is_web: true, playback_scope: 1, ..Default::default() };
    let b = TerminalProfile { is_web: true, playback_scope: 2, ..Default::default() };
    let (a_before, _) = render("img", "t.riv", 20, 6, &a, &d);
    let (b_before, _) = render("img", "t.riv", 20, 6, &b, &d);
    // visitor A (session 1) hovers and clicks the widget
    let key = oxiterm_proto::pathsafe::resolve_media(Some(&d), Some(&d), "t.riv").unwrap();
    PlaybackRegistry::get().set_hover(1, &key, true);
    PlaybackRegistry::get().set_click(1, &key, true, Some((1, 1)));
    let (a_after, _) = render("img", "t.riv", 20, 6, &a, &d);
    let (b_after, _) = render("img", "t.riv", 20, 6, &b, &d);
    let colours = |buf: &CellBuffer| buf.cells.iter().map(|c| (c.ch, format!("{:?}{:?}", c.fg, c.bg))).collect::<Vec<_>>();
    assert_ne!(colours(&a_before), colours(&a_after), "visitor A sees their own click");
    assert_eq!(colours(&b_before), colours(&b_after), "visitor B saw visitor A's click");
    PlaybackRegistry::get().drop_scope(1);
    PlaybackRegistry::get().drop_scope(2);
    let _ = std::fs::remove_dir_all(&d);
}
