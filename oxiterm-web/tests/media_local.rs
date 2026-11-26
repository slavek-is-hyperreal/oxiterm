//! Stage 7.6: media in local mode (the core in the browser, `native-media` off).
//!
//! Local mode has no file system: SVG/PNG/Lottie are shown by the browser from a DOM overlay (the
//! session emits `0xE0` asset requests and `0x21` coordinates), the Rive toggle is drawn by the
//! core as cells, and a diagram is drawn by the core from a `.mmd` the page fetched and mounted.
#![cfg(feature = "local")]

use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
use oxiterm_web::local::{hash_path, LocalConfig, LocalSession};

const MEDIA: &str = include_str!("../../examples/demos/media.thtml");
const DIAGRAM: &str = include_str!("../../examples/demos/diagram.thtml");
const FLOW: &str = include_str!("../../examples/assets/flow.mmd");

fn session(start: &str) -> LocalSession {
    let pages = vec![("demos/media.thtml".to_string(), MEDIA.to_string()), ("demos/diagram.thtml".to_string(), DIAGRAM.to_string())];
    let cfg = LocalConfig { start_page: start.into(), cols: 98, rows: 28, mobile: false, actions: false, app_token: String::new() };
    LocalSession::new(&cfg, pages, 0.0).unwrap()
}

/// Every message of the side channel, split by tag.
fn messages(s: &mut LocalSession) -> Vec<Vec<u8>> {
    s.take_messages()
}

fn requests(msgs: &[Vec<u8>]) -> Vec<(u32, String)> {
    msgs.iter()
        .filter(|m| m[0] == 0xE0)
        .map(|m| (u32::from_le_bytes(m[1..5].try_into().unwrap()), String::from_utf8(m[5..].to_vec()).unwrap()))
        .collect()
}

fn coords(msgs: &[Vec<u8>]) -> Vec<u32> {
    msgs.iter().filter(|m| m[0] == 0x21 && m.len() >= 13).map(|m| u32::from_le_bytes(m[1..5].try_into().unwrap())).collect()
}

fn grid_text(list: &DisplayList) -> String {
    let mut out = String::new();
    for item in &list.items {
        if let DisplayItem::CellGrid { cells, .. } = item {
            out.extend(cells.iter().map(|c| c.ch));
            out.push('\n');
        }
    }
    out
}

#[test]
fn media_page_asks_for_every_browser_shown_asset_and_positions_it() {
    let mut s = session("demos/media.thtml");
    s.pump(0.0);
    let msgs = messages(&mut s);
    let req = requests(&msgs);
    let want = [("../assets/mascot.svg", "assets/mascot.svg"), ("../assets/logo.jpg", "assets/logo.jpg"), ("../assets/bell.json", "assets/bell.json")];
    for (src, rel) in want {
        assert!(req.contains(&(hash_path(src), rel.to_string())), "no 0xE0 request for {rel}: {req:?}");
    }
    assert!(!req.iter().any(|(_, r)| r.to_ascii_lowercase().ends_with(".riv")), "the Rive widget is drawn by the core, never fetched");
    let c = coords(&msgs);
    for (src, _) in want {
        assert!(c.contains(&hash_path(src)), "no 0x21 position for {src}");
    }
    assert!(!c.contains(&hash_path("../assets/toggle.riv")));
}

#[test]
fn rive_toggle_is_drawn_as_cells_and_toggles_on_click() {
    let mut s = session("demos/media.thtml");
    s.pump(0.0);
    let list = s.last_list().expect("a frame");
    // The toggle track is (35,35,35) half-block cells; nothing else on the page is.
    let track = oxiterm_proto::style::AnsiColor::TrueColor(35, 35, 35);
    let count = |l: &DisplayList, c: oxiterm_proto::style::AnsiColor| {
        l.items
            .iter()
            .map(|i| match i {
                DisplayItem::CellGrid { cells, .. } => cells.iter().filter(|g| matches!(g.ch, '▀' | '▄' | '█') && (g.fg == c || g.bg == c)).count(),
                _ => 0,
            })
            .sum::<usize>()
    };
    assert!(count(&list, track) >= 6, "toggle not drawn in the display list");
    let blue = oxiterm_proto::style::AnsiColor::TrueColor(0, 120, 255);
    assert_eq!(count(&list, blue), 0);

    // Find the widget: the first cell whose press changes the picture to blue.
    let mut t = 100.0;
    'scan: for row in 0..28u16 {
        for col in 60..98u16 {
            let (px, py) = (f32::from(col) + 0.5, f32::from(row) + 0.5);
            for action in [0u8, 1] {
                let mut p = vec![0x03];
                p.extend_from_slice(&px.to_le_bytes());
                p.extend_from_slice(&py.to_le_bytes());
                p.extend_from_slice(&[0, action, 0]);
                s.input(&p, t);
            }
            t += 400.0;
            s.pump(t);
            if let Some(l) = s.last_list() {
                if count(&l, blue) > 0 {
                    break 'scan;
                }
            }
        }
    }
    assert!(count(&s.last_list().unwrap(), blue) > 0, "clicking the toggle must turn the track blue");
}

#[test]
fn diagram_is_requested_mounted_and_drawn_by_the_core() {
    let mut s = session("demos/diagram.thtml");
    s.pump(0.0);
    let msgs = messages(&mut s);
    let req = requests(&msgs);
    assert!(req.contains(&(hash_path("../assets/flow.mmd"), "assets/flow.mmd".to_string())), "{req:?}");
    // Before the file arrives the box is quiet: no placeholder, no diagram.
    let before = grid_text(&s.last_list().unwrap());
    assert!(!before.contains("5 nodes") && !before.contains("brak grafiki"), "{before}");
    // Refused names are not mounted.
    assert!(!s.mount_asset("../x.mmd", b"flowchart TD\nA-->B"));
    assert!(!s.mount_asset("/etc/passwd", b"x"));
    // The page fetched it: mount, pump, drawn.
    assert!(s.mount_asset("assets/flow.mmd", FLOW.as_bytes()));
    s.pump(50.0);
    let after = grid_text(&s.last_list().unwrap());
    assert!(after.contains("Architektura OxiTerm") && after.contains("5 nodes, 4 edges"), "{after}");
}
