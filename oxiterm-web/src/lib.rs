//! Web canvas-based rendering terminal for OxiTerm.
//!
//! Provides WASM bindings for HTML Canvas terminal rendering and template playgrounds.

#![allow(clippy::all, clippy::pedantic)]

use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, CanvasRenderingContext2d};
use oxiterm_proto::style::AnsiColor;
use oxiterm_renderer::render::diff::AnsiCommand;
use oxiterm_renderer::DiffEngine;

/// Canvas rendering WebTerminal WASM wrapper.
#[wasm_bindgen]
pub struct WebTerminal {
    canvas: HtmlCanvasElement,
    ctx: CanvasRenderingContext2d,
    char_width: f64,
    char_height: f64,
    cx: u16,
    cy: u16,
    fg: AnsiColor,
    bg: AnsiColor,
    bold: bool,
    underline: bool,
    italic: bool,
    base_font: String,
    cols: u16,
    rows: u16,
}

#[wasm_bindgen]
impl WebTerminal {
    /// Creates a new `WebTerminal` canvas bridge.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement, font: &str, line_height: f64) -> Result<WebTerminal, JsValue> {
        let ctx = canvas
            .get_context("2d")?
            .ok_or_else(|| JsValue::from_str("Failed to get 2d context"))?
            .dyn_into::<CanvasRenderingContext2d>()?;

        ctx.set_font(font);
        let metrics = ctx.measure_text("M")?;
        let char_width = metrics.width();
        let char_height = line_height;

        Ok(WebTerminal {
            canvas,
            ctx,
            char_width,
            char_height,
            cx: 0,
            cy: 0,
            fg: AnsiColor::Reset,
            bg: AnsiColor::Reset,
            bold: false,
            underline: false,
            italic: false,
            base_font: font.to_string(),
            cols: 80,
            rows: 24,
        })
    }

    /// Renders incoming binary ANSI commands directly to the 2D canvas context.
    #[wasm_bindgen]
    pub fn draw_commands(&mut self, bytes: &[u8]) -> Result<(), String> {
        let commands = DiffEngine::decode_binary(bytes).map_err(|e| e.to_string())?;
        for cmd in commands {
            match cmd {
                AnsiCommand::MoveCursor(x, y) => {
                    self.cx = x;
                    self.cy = y;
                    
                    let mut resized = false;
                    if x >= self.cols {
                        self.cols = x + 1;
                        let new_width = self.cols as f64 * self.char_width;
                        self.canvas.set_width(new_width as u32);
                        resized = true;
                    }
                    if y >= self.rows {
                        self.rows = y + 1;
                        let new_height = self.rows as f64 * self.char_height;
                        self.canvas.set_height(new_height as u32);
                        resized = true;
                    }
                    if resized {
                        self.ctx.set_font(&self.base_font);
                    }
                }
                AnsiCommand::SetColor { fg, bg } => {
                    self.fg = fg;
                    self.bg = bg;
                }
                AnsiCommand::SetModifiers { bold, underline, italic } => {
                    self.bold = bold;
                    self.underline = underline;
                    self.italic = italic;
                }
                AnsiCommand::WriteChar(ch) => {
                    let w = oxiterm_renderer::render::unicode::UnicodeWidthCache::get().width(ch);
                    let w = if w == 0 { 1 } else { w } as u16;
                    self.draw_cell(self.cx, self.cy, ch, w);
                    self.cx += w;
                    if self.cols > 0 && self.cx >= self.cols {
                        self.cx = 0;
                        self.cy += 1;
                    }
                }
                AnsiCommand::Reset => {
                    self.fg = AnsiColor::Reset;
                    self.bg = AnsiColor::Reset;
                    self.bold = false;
                    self.underline = false;
                    self.italic = false;
                }
            }
        }
        Ok(())
    }

    /// Resets the canvas sizing and fills background color.
    #[wasm_bindgen]
    pub fn clear(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        self.rows = rows;
        let bg_str = get_color_str(&AnsiColor::Reset, false);
        self.ctx.set_fill_style_str(&bg_str);
        self.ctx.fill_rect(
            0.0,
            0.0,
            cols as f64 * self.char_width,
            rows as f64 * self.char_height,
        );
    }

    /// Returns the active character cell rendering width and height dimensions.
    #[wasm_bindgen]
    pub fn get_char_dimensions(&self) -> Vec<f64> {
        vec![self.char_width, self.char_height]
    }

    /// Updates the CSS-pixel cell dimensions used for coordinate mapping.
    ///
    /// Called after a backing-resize to inform WASM of the new cell size
    /// (which may differ from the initial measurement when DPR changes).
    #[wasm_bindgen]
    pub fn set_char_dimensions(&mut self, w: f64, h: f64) {
        self.char_width = w;
        self.char_height = h;
    }

    /// Re-applies the device-pixel-ratio scale transform and font after a
    /// canvas backing resize.
    ///
    /// `canvas.width` assignment resets ALL canvas 2D context state (transform
    /// AND font). This must be called immediately after every backing resize so
    /// that WASM coordinate math (which uses CSS-pixel units) maps correctly to
    /// the device-pixel backing.
    #[wasm_bindgen]
    pub fn apply_dpr_scale(&mut self, dpr: f64) {
        // Re-apply scale transform: CSS-pixel coordinates * dpr = device pixels.
        let _ = self.ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
        // Re-set font (reset by canvas.width assignment).
        self.ctx.set_font(&self.base_font);
    }
}

impl WebTerminal {
    fn draw_cell(&self, col: u16, row: u16, ch: char, width_cells: u16) {
        let x = col as f64 * self.char_width;
        let y = row as f64 * self.char_height;
        let cell_w = self.char_width * width_cells as f64;

        // 1. Draw Background
        let bg_str = get_color_str(&self.bg, false);
        self.ctx.set_fill_style_str(&bg_str);
        self.ctx.fill_rect(x, y, cell_w, self.char_height);

        if ch == ' ' || ch == '\0' {
            return;
        }

        // 2. Set Font styling (bold, italic) using base_font
        let font_style = format!(
            "{}{}{}",
            if self.bold { "bold " } else { "" },
            if self.italic { "italic " } else { "" },
            self.base_font
        );
        self.ctx.set_font(&font_style);

        // 3. Draw Foreground Text
        let fg_str = get_color_str(&self.fg, true);
        
        self.ctx.set_fill_style_str(&fg_str);
        self.ctx.set_text_baseline("top");
        let ch_str = ch.to_string();
        let _ = self.ctx.fill_text(&ch_str, x, y);

        // 4. Draw Underline
        if self.underline {
            self.ctx.fill_rect(x, y + self.char_height - 2.0, cell_w, 2.0);
        }
    }
}

fn get_color_str(color: &AnsiColor, is_fg: bool) -> String {
    match color {
        AnsiColor::Reset => {
            if is_fg {
                "#f1f5f9".to_string()
            } else {
                "#0f172a".to_string()
            }
        }
        AnsiColor::TrueColor(r, g, b) => {
            format!("rgb({},{},{})", r, g, b)
        }
        AnsiColor::Color256(idx) => {
            let (r, g, b) = ansi_256_to_rgb(*idx);
            format!("rgb({},{},{})", r, g, b)
        }
    }
}

fn ansi_256_to_rgb(idx: u8) -> (u8, u8, u8) {
    if idx < 16 {
        let palette = [
            (15, 23, 42),     // 0: Black (slate-900 override)
            (239, 68, 68),    // 1: Red (red-500)
            (34, 197, 94),    // 2: Green (green-500)
            (234, 179, 8),    // 3: Yellow (yellow-500)
            (59, 130, 246),   // 4: Blue (blue-500)
            (168, 85, 247),   // 5: Magenta (purple-500)
            (6, 182, 212),    // 6: Cyan (cyan-500)
            (241, 245, 249),  // 7: White (slate-100)
            (100, 116, 139),  // 8: Bright Black (slate-500)
            (248, 113, 113),  // 9: Bright Red (red-400)
            (74, 222, 128),   // 10: Bright Green (green-400)
            (253, 224, 71),   // 11: Bright Yellow (yellow-300)
            (96, 165, 250),   // 12: Bright Blue (blue-400)
            (192, 132, 252),  // 13: Bright Magenta (purple-400)
            (34, 211, 238),   // 14: Bright Cyan (cyan-400)
            (255, 255, 255),  // 15: Bright White
        ];
        palette[idx as usize]
    } else if idx < 232 {
        let val = idx - 16;
        let r = (val / 36) % 6;
        let g = (val / 6) % 6;
        let b = val % 6;
        let steps = [0, 95, 135, 175, 223, 255];
        (steps[r as usize], steps[g as usize], steps[b as usize])
    } else {
        let level = 8 + (idx - 232) * 10;
        (level, level, level)
    }
}

/// Static template playground WASM endpoint.
#[wasm_bindgen]
pub struct Playground;

#[wasm_bindgen]
impl Playground {
    /// Mounts raw file contents to the static playground virtual filesystem cache.
    #[wasm_bindgen]
    pub fn mount_asset(path: &str, data: &[u8]) {
        let path_buf = std::path::PathBuf::from(path);
        oxiterm_renderer::render::renderer::VIRTUAL_FS.with(|fs| {
            fs.borrow_mut().insert(path_buf, data.to_vec());
        });
    }

    /// Renders the given THTML document to either ANSI sequence or HTML.
    /// 
    /// NOTE: Reactive dynamic states are not simulated or evaluated in this static
    /// rendering playground mode. Consequently, nodes containing `bind-show` conditional show
    /// attributes will default to being visible rather than being hidden.
    #[wasm_bindgen]
    pub fn render(html_content: &str, width: u16, height: u16, format: Option<String>) -> Result<String, String> {
        let mut doc = oxiterm_renderer::parser::THTMLParser::parse(html_content).map_err(|e| e.to_string())?;
        let mut engine = oxiterm_renderer::layout::engine::LayoutEngine::new();
        let layout = engine.compute(&mut doc, width, 0, None).map_err(|e| e.to_string())?;
        
        let mut back = oxiterm_renderer::CellBuffer::new(width, height);
        let mut profile = oxiterm_proto::style::TerminalProfile::default();
        profile.supports_kitty_gfx = true;
        profile.supports_sixel = true;
        
        oxiterm_renderer::render::renderer::Renderer::render_node(&doc, &layout, &mut back, &profile, None, 0);
        
        let fmt = format.unwrap_or_else(|| "ansi".to_string());
        if fmt.eq_ignore_ascii_case("html") {
            Ok(Self::render_to_html(&back))
        } else {
            let front = oxiterm_renderer::CellBuffer::new(width, height);
            let commands = oxiterm_renderer::DiffEngine::diff(&front, &back);
            let ansi_bytes = oxiterm_renderer::DiffEngine::encode_ansi(&commands);
            Ok(String::from_utf8_lossy(&ansi_bytes).into_owned())
        }
    }

    fn render_to_html(buffer: &oxiterm_renderer::CellBuffer) -> String {
        let mut html = String::new();
        html.push_str("<pre style=\"font-family: monospace; background: #0f172a; color: #f1f5f9; padding: 10px; margin: 0; line-height: 1.2;\">");
        for y in 0..buffer.height {
            for x in 0..buffer.width {
                if let Some(idx) = buffer.flat_idx(x, y) {
                    let cell = &buffer.cells[idx];
                    if cell.skip {
                        continue;
                    }
                    let mut style = String::new();
                    // Foreground color
                    match &cell.fg {
                        AnsiColor::TrueColor(r, g, b) => {
                            style.push_str(&format!("color: rgb({},{},{});", r, g, b));
                        }
                        AnsiColor::Color256(idx) => {
                            let (r, g, b) = ansi_256_to_rgb(*idx);
                            style.push_str(&format!("color: rgb({},{},{});", r, g, b));
                        }
                        AnsiColor::Reset => {}
                    }
                    // Background color
                    match &cell.bg {
                        AnsiColor::TrueColor(r, g, b) => {
                            style.push_str(&format!("background-color: rgb({},{},{});", r, g, b));
                        }
                        AnsiColor::Color256(idx) => {
                            let (r, g, b) = ansi_256_to_rgb(*idx);
                            style.push_str(&format!("background-color: rgb({},{},{});", r, g, b));
                        }
                        AnsiColor::Reset => {}
                    }
                    if cell.bold {
                        style.push_str("font-weight: bold;");
                    }
                    if cell.italic {
                        style.push_str("font-style: italic;");
                    }
                    if cell.underline {
                        style.push_str("text-decoration: underline;");
                    }
                    
                    if !style.is_empty() {
                        html.push_str(&format!("<span style=\"{}\">", style));
                    }
                    // Escape HTML characters
                    match cell.ch {
                        '&' => html.push_str("&amp;"),
                        '<' => html.push_str("&lt;"),
                        '>' => html.push_str("&gt;"),
                        '"' => html.push_str("&quot;"),
                        '\'' => html.push_str("&#x27;"),
                        '\0' | ' ' => html.push_str("&nbsp;"),
                        _ => html.push(cell.ch),
                    }
                    if !style.is_empty() {
                        html.push_str("</span>");
                    }
                }
            }
            html.push_str("<br/>");
        }
        html.push_str("</pre>");
        html
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_playground_render_ansi() {
        let html = "<text style=\"color: #ff0000; background-color: #00ff00;\">Hello World</text>";
        let rendered = Playground::render(html, 80, 24, Some("ansi".to_string())).unwrap();
        assert!(rendered.contains("Hello World"));
    }

    #[test]
    fn test_playground_render_html() {
        let html = "<text style=\"color: #ff0000; background-color: #00ff00;\">Hello World</text>";
        let rendered = Playground::render(html, 80, 24, Some("html".to_string())).unwrap();
        assert!(rendered.contains("color: rgb(255,0,0)"));
        assert!(rendered.contains("background-color: rgb(0,255,0)"));
        assert!(rendered.contains(">H</span>"));
        assert!(rendered.contains(">e</span>"));
        assert!(rendered.contains(">&nbsp;</span>"));
    }

    #[test]
    fn test_playground_mount_asset() {
        let path = "test_asset.txt";
        let data = b"hello asset";
        Playground::mount_asset(path, data);
        
        let path_buf = std::path::PathBuf::from(path);
        let read_data = oxiterm_renderer::render::renderer::VIRTUAL_FS.with(|fs| {
            fs.borrow().get(&path_buf).cloned()
        });
        assert_eq!(read_data, Some(data.to_vec()));
    }
}
