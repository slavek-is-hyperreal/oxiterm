//! Stage 7.6: what an SSH terminal actually receives for the media demo pages. The real demo
//! pages are rendered with the cell sink for each terminal profile and pushed through the ANSI
//! frame sink (the one every SSH session uses); the bytes on the wire are searched for the image
//! escape sequences, half blocks and diagram text. A sink or emitter that dropped media would
//! fail here even if the cell buffer was right.
use std::path::{Path, PathBuf};

use oxiterm_proto::style::{RenderMode, TerminalProfile};
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::{CellBuffer, FrameSink, LayoutEngine};
use oxiterm_server::backpressure::BoundedFrameChannel;
use oxiterm_server::session::AnsiFrameSink;

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples").canonicalize().unwrap()
}

/// Renders `page` (relative to `examples/`) for `profile` and returns the bytes of the first frame.
fn wire_bytes(page: &str, profile: &TerminalProfile) -> Vec<u8> {
    let ex = examples();
    let path = ex.join(page);
    let mut doc = oxiterm_server::loader::load_thtml_file_mode(&path, RenderMode::Tui).unwrap();
    let (cols, rows) = (100u16, 30u16);
    let mut engine = LayoutEngine::new();
    engine.set_viewport_rows(rows);
    let layout = engine.compute(&mut doc, cols, 0, None).unwrap();
    let mut back = CellBuffer::new(cols, rows);
    Renderer::render_node(&doc, &layout, &mut back, profile, path.parent(), Some(&ex), 0);
    let (tx, mut rx) = BoundedFrameChannel::new(4);
    let mut sink = AnsiFrameSink::new(tx);
    assert!(sink.send_frame(&CellBuffer::new(cols, rows), &back).unwrap());
    rx.try_recv().expect("a frame")
}

fn has(bytes: &[u8], needle: &[u8]) -> bool {
    bytes.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn kitty_terminal_receives_graphics_commands_for_every_media_node() {
    let b = wire_bytes("demos/media.thtml", &TerminalProfile { supports_kitty_gfx: true, ..Default::default() });
    // svg + jpg + lottie + the Rive widget: four transmissions.
    let n = b.windows(3).filter(|w| w == b"\x1b_G").count();
    assert!(n >= 4, "expected 4 kitty graphics commands on the wire, got {n}");
}

#[test]
fn sixel_terminal_receives_sixel_data() {
    let b = wire_bytes("demos/media.thtml", &TerminalProfile { supports_sixel: true, ..Default::default() });
    let n = b.windows(2).filter(|w| w == b"\x1bP").count();
    assert!(n >= 4, "expected 4 sixel strings on the wire, got {n}");
}

#[test]
fn plain_terminal_receives_half_blocks() {
    let b = wire_bytes("demos/media.thtml", &TerminalProfile::default());
    assert!(!has(&b, b"\x1b_G") && !has(&b, b"\x1bP"));
    let blocks = b.windows(3).filter(|w| *w == "\u{2580}".as_bytes() || *w == "\u{2584}".as_bytes() || *w == "\u{2588}".as_bytes()).count();
    assert!(blocks > 100, "expected many half-block cells on the wire, got {blocks}");
}

#[test]
fn every_terminal_receives_the_diagram_text() {
    for profile in [TerminalProfile::default(), TerminalProfile { supports_kitty_gfx: true, ..Default::default() }] {
        let b = wire_bytes("demos/diagram.thtml", &profile);
        assert!(has(&b, "Architektura OxiTerm".as_bytes()), "diagram header missing on the wire");
    }
}
