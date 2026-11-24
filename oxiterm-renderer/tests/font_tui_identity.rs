//! Stage 6.2: in TUI mode `font-family`, `font-size` and `line-height` change nothing, while
//! `font-weight`/`font-style` map onto the bold/italic cell flags.

use oxiterm_proto::style::{RenderMode, TerminalProfile};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::CellBuffer;

const BODY: &str = r#"<screen class="root"><box class="card"><text class="t">Hello world, a long enough sentence to wrap in the box</text><button class="b" event="x">Go</button></box><text>tail</text></screen>"#;

fn render(style: &str, mode: RenderMode) -> (Vec<oxiterm_renderer::render::buffer::Cell>, usize) {
    let src = format!("<style>{style}</style>{BODY}");
    let mut doc = THTMLParser::parse_with_mode(&src, mode).unwrap();
    let mut engine = LayoutEngine::with_mode(mode);
    engine.set_viewport_rows(12);
    let layout = engine.compute(&mut doc, 40, 0, None).unwrap();
    let mut buf = CellBuffer::new(40, 12);
    let mut profile = TerminalProfile::default();
    profile.is_web = mode == RenderMode::Gui;
    Renderer::render_node(&doc, &layout, &mut buf, &profile, None, None, 0);
    let nbold = buf.cells.iter().filter(|c| c.bold).count();
    (buf.cells.clone(), nbold)
}

const BASE: &str = ".card { width: 30; border: red; padding: 1; wrap: word } .t { wrap: word }";
const FONTS: &str = ".root { font-family: sans-serif; font-size: 30px; line-height: 3 } \
                     .t { font-size: 2em; line-height: 40px; font-family: \"Noto Sans\" } \
                     @media (render: gui) { .b { font-size: 40px } }";

#[test]
fn tui_output_identical_with_and_without_gui_only_font_properties() {
    let (plain, _) = render(BASE, RenderMode::Tui);
    let (fonts, _) = render(&format!("{BASE} {FONTS}"), RenderMode::Tui);
    assert_eq!(plain, fonts);
}

#[test]
fn weight_and_style_still_apply_in_tui_and_fonts_around_them_change_nothing() {
    let ws = ".t { font-weight: bold; font-style: italic }";
    let (plain, nb0) = render(BASE, RenderMode::Tui);
    let (a, nb1) = render(&format!("{BASE} {ws}"), RenderMode::Tui);
    let (b, nb2) = render(&format!("{BASE} {ws} {FONTS}"), RenderMode::Tui);
    assert_eq!(nb0, 0);
    assert!(nb1 > 0, "bold reaches the cells");
    assert_eq!(nb1, nb2);
    assert!(a.iter().any(|c| c.italic));
    assert_ne!(plain, a);
    assert_eq!(a, b, "font-family/size/line-height add nothing on top of weight/style");
}

#[test]
fn bold_is_inherited_by_text_children() {
    let (_, nb) = render(&format!("{BASE} .card {{ font-weight: bold }}"), RenderMode::Tui);
    assert!(nb > 0);
}
