//! Stage 6.4: positioned glyph runs in the display list.
//!
//! Needs `text-shaping` (the server and core enable it; `cargo test -p oxiterm-renderer` alone
//! compiles this file to nothing, run it with `--features text-shaping`).
#![cfg(feature = "text-shaping")]

use std::path::{Path, PathBuf};

use oxiterm_proto::style::{RenderMode, TerminalProfile};
use oxiterm_renderer::font::{CELL_H_PX, CELL_W_PX};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::cell_sink::CellSink;
use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::render::wire;
use oxiterm_renderer::CellBuffer;

#[path = "support/dl_dump.rs"]
mod dl_dump;
use dl_dump::{dump, render};

fn runs(list: &DisplayList) -> Vec<&DisplayItem> {
    list.items.iter().filter(|i| matches!(i, DisplayItem::Text { shaped: Some(_), .. })).collect()
}

const PAGE: &str = r#"<style>.p { font-family: sans-serif; width: 30; wrap: word; fg: #ffffff; bg: #102030 } .b { font-family: sans-serif; font-size: 24px; padding-left: 2 } .m { font-size: 24px }</style>
<screen style="width: 40; height: 14; flex-direction: column;">
<text class="p">The quick brown   fox jumps over the lazy dog and keeps running far away</text>
<button class="b" event-htmx="x:y">Go office</button>
<text class="m">mono big</text>
<text>plain mono</text>
<input type="password" style="font-family: sans-serif; width: 8;">secret</input>
</screen>"#;

#[test]
fn builder_emits_the_stored_layout_positions() {
    let (list, engine, _) = render(PAGE, RenderMode::Gui, 40, 14);
    let items = runs(&list);
    assert_eq!(items.len(), 3, "sans text, sans button and the 24 px mono text are shaped; plain mono and the input are not");
    for item in items {
        let DisplayItem::Text { node, origin, glyphs, shaped, .. } = item else { unreachable!() };
        let run = shaped.as_ref().unwrap();
        let layout = engine.shaped_text(*node).expect("stored layout");
        let mut gi = 0;
        for line in &layout.lines {
            for g in &line.glyphs {
                let Some(out) = glyphs.get(gi) else { break };
                if out.x != g.x / CELL_W_PX || out.y != line.top / CELL_H_PX {
                    continue;
                }
                assert_eq!(out.advance, g.advance / CELL_W_PX);
                assert_eq!(out.ch, layout.cluster_text(g).chars().next().unwrap_or(' '));
                gi += 1;
            }
        }
        assert_eq!(gi, glyphs.len(), "every emitted glyph is a stored glyph, in order");
        assert!(origin.x >= 0.0);
        assert_eq!(run.line_height, layout.font.line_height_px / CELL_H_PX);
        assert_eq!(run.size_px, layout.font.size_px);
        // the baseline is the same on every line of the layout (nothing here falls back)
        for l in &layout.lines {
            assert_eq!((l.baseline - l.top) / CELL_H_PX, run.baseline);
        }
    }
}

#[test]
fn whitespace_runs_collapse_like_the_mono_word_path() {
    let (list, _, _) = render(PAGE, RenderMode::Gui, 40, 14);
    let DisplayItem::Text { glyphs, .. } = runs(&list)[0] else { unreachable!() };
    let text: String = glyphs.iter().map(|g| g.ch).collect();
    assert!(text.contains("brown fox"), "{text:?}");
    assert!(!text.contains("  "), "{text:?}");
}

#[test]
fn ligature_and_combining_mark_clusters_are_carried() {
    let src = "<style>.s { font-family: sans-serif; wrap: none }</style><screen style=\"width: 30; height: 4;\"><text class=\"s\">office fl e\u{301}x</text></screen>";
    let (list, _, _) = render(src, RenderMode::Gui, 30, 4);
    let DisplayItem::Text { glyphs, shaped, .. } = runs(&list)[0] else { unreachable!() };
    let run = shaped.as_ref().unwrap();
    assert!(!run.clusters.is_empty(), "ffi/fl ligatures and the accent are multi-char clusters");
    let mut texts: Vec<&str> = run.clusters.iter().map(|c| &*c.1).collect();
    texts.sort();
    assert!(texts.iter().any(|t| t.contains('\u{301}')), "{texts:?}");
    for (i, t) in &run.clusters {
        assert!(t.chars().count() > 1);
        assert_eq!(glyphs[*i as usize].ch, t.chars().next().unwrap());
    }
    // far fewer glyphs than chars: the ligatures merged
    assert!(glyphs.len() < "office fl e\u{301}x".chars().count());
}

#[test]
fn glyphs_outside_the_clip_are_dropped() {
    // A 1-row, 8-cell box clips both a long nowrap line and a second line.
    let src = "<style>.s { font-family: sans-serif; wrap: none; width: 8; height: 1 }</style><screen style=\"width: 30; height: 4;\"><text class=\"s\">abcdefghijklmnopqrstuvwxyz\nsecond</text></screen>";
    let (list, engine, _) = render(src, RenderMode::Gui, 30, 4);
    let DisplayItem::Text { glyphs, clip, node, .. } = runs(&list)[0] else { unreachable!() };
    let layout = engine.shaped_text(*node).unwrap();
    let total: usize = layout.lines.iter().map(|l| l.glyphs.len()).sum();
    assert!(glyphs.len() < total, "{} of {total}", glyphs.len());
    for g in glyphs {
        assert!(g.x < clip.w && g.y < clip.h, "{g:?} inside {clip:?}");
    }
    assert!(glyphs.iter().all(|g| g.y == 0.0), "second line is below the clip");
}

#[test]
fn inputs_and_mono_text_stay_on_the_cell_path() {
    let (list, _, _) = render(PAGE, RenderMode::Gui, 40, 14);
    let plain: Vec<String> = list
        .items
        .iter()
        .filter_map(|i| match i {
            DisplayItem::Text { glyphs, shaped: None, .. } => Some(glyphs.iter().map(|g| g.ch).collect()),
            _ => None,
        })
        .collect();
    assert!(plain.iter().any(|t| t == "plain mono"), "{plain:?}");
    // the password input is masked, whatever font it inherits
    assert!(plain.iter().any(|t| t == "******__"), "{plain:?}");
    assert!(!plain.iter().any(|t| t.contains("secret")));
}

#[test]
fn tui_never_has_shaped_runs() {
    let (list, _, _) = render(PAGE, RenderMode::Tui, 40, 14);
    assert!(runs(&list).is_empty());
}

fn thtml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            thtml_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("thtml") {
            out.push(path);
        }
    }
}

/// Pages with only default-font text: no shaped run, and every client version gets the very same
/// bytes (v3 clients too: the version stays 2 unless a shaped run exists).
#[test]
fn default_pages_encode_identically_for_every_client() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let examples = root.join("../examples").canonicalize().unwrap();
    let mut files = Vec::new();
    thtml_files(&examples, &mut files);
    files.sort();
    let mut checked = 0;
    for file in &files {
        let src = std::fs::read_to_string(file).unwrap();
        for mode in [RenderMode::Tui, RenderMode::Gui] {
            let Ok(mut doc) = THTMLParser::parse_with_mode(&src, mode) else { continue };
            let mut engine = LayoutEngine::with_mode(mode);
            engine.set_viewport_rows(24);
            let Ok(layout) = engine.compute(&mut doc, 80, 0, None) else { continue };
            let mut buf = CellBuffer::new(80, 24);
            let mut profile = TerminalProfile::default();
            profile.is_web = true;
            let list = Renderer::render_frame_mode(&doc, &layout, &mut buf, &profile, file.parent(), Some(&examples), 0, mode);
            assert!(runs(&list).is_empty(), "{file:?} {mode:?}");
            let v2 = wire::encode(&list);
            assert_eq!(v2[0], 2);
            assert_eq!(wire::encode_for(&list, 2), v2);
            assert_eq!(wire::encode_for(&list, 3), v2, "{file:?} {mode:?}");
            assert_eq!(wire::decode(&v2).unwrap(), list);
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}

#[test]
fn shaped_page_round_trips_v3_and_downgrades_for_v2() {
    let (list, _, _) = render(PAGE, RenderMode::Gui, 40, 14);
    assert!(!runs(&list).is_empty());
    let v3 = wire::encode_for(&list, 3);
    assert_eq!(v3[0], 3);
    assert_eq!(wire::decode(&v3).unwrap(), list);
    let v2 = wire::encode_for(&list, 2);
    assert_eq!(v2[0], 2);
    assert_eq!(v2, wire::encode(&list));
    assert_eq!(wire::decode(&v2).unwrap(), wire::downgrade_v2(&list));
    assert!(runs(&wire::downgrade_v2(&list)).is_empty());
    // the cell sink accepts both without panicking, and the replay of the decoded v3 list is the
    // same as the replay of the original
    let paint = |l: &DisplayList| {
        let mut b = CellBuffer::new(40, 14);
        CellSink::paint(l, &mut b, |_, _, _| {});
        b.cells
    };
    assert_eq!(paint(&wire::decode(&v3).unwrap()), paint(&list));
    let _ = paint(&wire::decode(&v2).unwrap());
}

// ---------------------------------------------------------------------------------------------
// Golden: the display list of the proportional benchmark page (GUI, 80x24).

/// `OXI_DL_GOLDEN_DIR` overrides the directory (used to check determinism without touching the
/// committed files); `OXI_BLESS_DL=1` writes the file instead of comparing.
#[test]
fn sans_page_display_list_golden() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let page = root.join("../oxiterm-server/benches/pages/sans.thtml");
    let src = std::fs::read_to_string(&page).unwrap();
    let dir = std::env::var_os("OXI_DL_GOLDEN_DIR").map(PathBuf::from).unwrap_or_else(|| root.join("tests/golden_dl"));
    let (list, _, _) = render(&src, RenderMode::Gui, 80, 24);
    assert!(runs(&list).len() > 5, "the fixture must produce shaped runs (is text-shaping on?)");
    let got = dump(&list);
    let file = dir.join("sans@80x24.txt");
    if std::env::var_os("OXI_BLESS_DL").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&file, &got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&file).unwrap_or_else(|_| {
        panic!("missing {file:?}; bless it with: OXI_BLESS_DL=1 cargo test -p oxiterm-renderer --features text-shaping --test shaped_display_list sans_page_display_list_golden")
    });
    assert!(want == got, "display list of the sans page changed; review and re-bless with OXI_BLESS_DL=1 (see the header of this file)");
}
