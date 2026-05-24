use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, CanvasRenderingContext2d};
use oxiterm_proto::style::AnsiColor;
use oxiterm_renderer::render::diff::AnsiCommand;
use oxiterm_renderer::DiffEngine;


#[wasm_bindgen]
pub struct WebTerminal {
    _canvas: HtmlCanvasElement,
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
}

#[wasm_bindgen]
impl WebTerminal {
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
            _canvas: canvas,
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
        })
    }

    #[wasm_bindgen]
    pub fn draw_commands(&mut self, bytes: &[u8]) -> Result<(), String> {
        let commands = DiffEngine::decode_binary(bytes).map_err(|e| e.to_string())?;
        for cmd in commands {
            match cmd {
                AnsiCommand::MoveCursor(x, y) => {
                    self.cx = x;
                    self.cy = y;
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

    #[wasm_bindgen]
    pub fn clear(&mut self, cols: u16, rows: u16) {
        self.cols = cols;
        let bg_str = get_color_str(&AnsiColor::Reset, false);
        self.ctx.set_fill_style(&wasm_bindgen::JsValue::from_str(&bg_str));
        self.ctx.fill_rect(
            0.0,
            0.0,
            cols as f64 * self.char_width,
            rows as f64 * self.char_height,
        );
    }

    #[wasm_bindgen]
    pub fn get_char_dimensions(&self) -> Vec<f64> {
        vec![self.char_width, self.char_height]
    }
}

impl WebTerminal {
    fn draw_cell(&self, col: u16, row: u16, ch: char, width_cells: u16) {
        let x = col as f64 * self.char_width;
        let y = row as f64 * self.char_height;
        let cell_w = self.char_width * width_cells as f64;

        // 1. Draw Background
        let bg_str = get_color_str(&self.bg, false);
        self.ctx.set_fill_style(&wasm_bindgen::JsValue::from_str(&bg_str));
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
        

        self.ctx.set_fill_style(&wasm_bindgen::JsValue::from_str(&fg_str));
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
