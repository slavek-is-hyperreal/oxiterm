//! Shared by `shaped_display_list.rs` (native, reads files) and `wasm_pipeline.rs` (native and
//! wasm32-wasip1, embeds everything): one producer of the display list and of its text dump, so
//! the golden is compared against the very same code on every target.
#![allow(dead_code)]

use std::fmt::Write as _;

use oxiterm_proto::style::{AnsiColor, RenderMode, TerminalProfile};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::display_list::{DisplayItem, DisplayList};
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::CellBuffer;

/// Renders `src` in `mode` like a session does and returns the list and the engine.
pub fn render(src: &str, mode: RenderMode, w: u16, h: u16) -> (DisplayList, LayoutEngine, oxiterm_renderer::document::THTMLDocument) {
    let mut doc = THTMLParser::parse_with_mode(src, mode).unwrap();
    let mut engine = LayoutEngine::with_mode(mode);
    engine.set_viewport_rows(h);
    let layout = engine.compute(&mut doc, w, 0, None).unwrap();
    let mut buf = CellBuffer::new(w, h);
    let mut profile = TerminalProfile::default();
    profile.is_web = true;
    let list = Renderer::render_frame_mode(&doc, &layout, &mut buf, &profile, None, None, 0, mode);
    (list, engine, doc)
}

pub fn color(c: AnsiColor) -> String {
    match c {
        AnsiColor::TrueColor(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        AnsiColor::Color256(i) => format!("c{i}"),
        AnsiColor::Reset => "reset".to_string(),
    }
}

/// Stable text form of a list: every item with its numbers; shaped runs with face, size, line
/// height, baseline, face overrides, clusters and every glyph (bits-exact f32 via `{:?}`).
pub fn dump(list: &DisplayList) -> String {
    let mut out = String::new();
    for item in &list.items {
        match item {
            DisplayItem::Fill { node, rect, radius, fg, bg } => {
                writeln!(out, "Fill n{} {:?} r={:?} fg={} bg={}", node.0, rect, radius, color(*fg), color(*bg)).unwrap()
            }
            DisplayItem::Border { node, rect, radius, fg, bg, .. } => {
                writeln!(out, "Border n{} {:?} r={:?} fg={} bg={}", node.0, rect, radius, color(*fg), color(*bg)).unwrap()
            }
            DisplayItem::Text { node, origin, clip, glyphs, fg, bg, mods, shaped } => {
                writeln!(out, "Text n{} origin=({:?},{:?}) clip={:?} fg={} bg={} mods={:?} glyphs={}", node.0, origin.x, origin.y, clip, color(*fg), color(*bg), mods, glyphs.len()).unwrap();
                if let Some(r) = shaped {
                    writeln!(out, "  run face={} size={:?} line_height={:?} baseline={:?} faces={:?} clusters={:?}", r.face, r.size_px, r.line_height, r.baseline, r.glyph_faces, r.clusters).unwrap();
                }
                for g in glyphs {
                    writeln!(out, "  {:?} x={:?} y={:?} adv={:?}", g.ch, g.x, g.y, g.advance).unwrap();
                }
            }
            DisplayItem::CellGrid { rect, cells } => writeln!(out, "CellGrid {:?} cells={}", rect, cells.len()).unwrap(),
            other => writeln!(out, "{other:?}").unwrap(),
        }
    }
    out
}

