//! THTML source to a GUI-mode [`DisplayList`], the way a display-list session produces one
//! (the same steps as `oxiterm-renderer/tests/support/dl_dump.rs` and `render_example`). Used by
//! the CLI, the benches and the PNG goldens.

use std::path::Path;

use oxiterm_proto::style::{RenderMode, TerminalProfile};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::display_list::DisplayList;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::CellBuffer;

/// Largest viewport (cells) the page helper builds: a million cells is far beyond any display.
pub const MAX_CELLS: u32 = 1 << 20;

/// Parses, lays out (GUI mode, `cols` x `rows` cells) and builds the display list of `src`.
/// `base_dir` resolves relative assets, `app_base_dir` the application root (both optional).
pub fn display_list(
    src: &str,
    base_dir: Option<&Path>,
    app_base_dir: Option<&Path>,
    cols: u16,
    rows: u16,
) -> Result<DisplayList, String> {
    if cols == 0 || rows == 0 || u32::from(cols) * u32::from(rows) > MAX_CELLS {
        return Err(format!("viewport {cols}x{rows} cells is outside 1..={MAX_CELLS} cells"));
    }
    let mode = RenderMode::Gui;
    let mut doc = THTMLParser::parse_with_mode(src, mode).map_err(|e| format!("parse: {e}"))?;
    let mut engine = LayoutEngine::with_mode(mode);
    engine.set_viewport_rows(rows);
    let layout = engine.compute(&mut doc, cols, 0, None).map_err(|e| format!("layout: {e}"))?;
    let mut buf = CellBuffer::new(cols, rows);
    let profile = TerminalProfile { is_web: true, ..TerminalProfile::default() };
    Ok(Renderer::render_frame_mode(&doc, &layout, &mut buf, &profile, base_dir, app_base_dir, 0, mode))
}

/// [`display_list`] of a file; the file's directory is the asset base.
pub fn display_list_of_file(path: &Path, app_base_dir: Option<&Path>, cols: u16, rows: u16) -> Result<DisplayList, String> {
    let src = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    display_list(&src, path.parent(), app_base_dir, cols, rows)
}
