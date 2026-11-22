//! OxiUI phase 3a: `Renderer::render_frame` must return a self-contained display list.
//!
//! Painting that list with `CellSink` onto an EMPTY buffer must reproduce exactly the buffer
//! `render_node` paints, for every example page. (The page walk mirrors `golden_cells.rs`.)

use std::path::{Path, PathBuf};

use oxiterm_proto::style::TerminalProfile;
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::cell_sink::CellSink;
use oxiterm_renderer::render::display_list::DisplayItem;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::render::wire;
use oxiterm_renderer::CellBuffer;

const SIZES: &[(u16, u16)] = &[(80, 24), (120, 40), (30, 10)];

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

#[test]
fn render_frame_list_reproduces_render_node_buffer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let examples = root.join("../examples").canonicalize().unwrap();
    let mut files = Vec::new();
    thtml_files(&examples, &mut files);
    files.sort();
    assert!(!files.is_empty());

    let mut legacy_grids = 0;
    for file in &files {
        let content = std::fs::read_to_string(file).unwrap();
        let Ok(mut doc) = THTMLParser::parse(&content) else { continue };
        for &(w, h) in SIZES {
            let mut engine = LayoutEngine::new();
            engine.set_viewport_rows(h);
            let Ok(layout) = engine.compute(&mut doc, w, 0, None) else { continue };
            let mut profile = TerminalProfile::default();
            profile.is_web = true;

            let mut expected = CellBuffer::new(w, h);
            Renderer::render_node(&doc, &layout, &mut expected, &profile, file.parent(), Some(&examples), 0);

            let mut buf = CellBuffer::new(w, h);
            let list = Renderer::render_frame(&doc, &layout, &mut buf, &profile, file.parent(), Some(&examples), 0);
            assert_eq!(buf.cells, expected.cells, "{file:?}@{w}x{h}: render_frame buffer differs");
            assert!(
                !list.items.iter().any(|i| matches!(i, DisplayItem::Legacy { .. })),
                "{file:?}: Legacy item left in list"
            );
            legacy_grids += list.items.iter().filter(|i| matches!(i, DisplayItem::CellGrid { .. })).count();

            // Over the wire and back, then painted onto an empty buffer.
            let list = wire::decode(&wire::encode(&list)).unwrap();
            let mut replay = CellBuffer::new(w, h);
            CellSink::paint(&list, &mut replay, |_, _, _| panic!("no Legacy expected"));
            assert_eq!(replay.cells, expected.cells, "{file:?}@{w}x{h}: replayed list differs");
        }
    }
    eprintln!("legacy CellGrids replayed: {legacy_grids}");
}
