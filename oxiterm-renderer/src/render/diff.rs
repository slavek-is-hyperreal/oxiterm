use crate::render::buffer::CellBuffer;
use oxiterm_proto::style::AnsiColor;

#[derive(Debug, PartialEq, Clone)]
pub enum AnsiCommand {
    MoveCursor(u16, u16),
    SetColor { fg: AnsiColor, bg: AnsiColor },
    WriteChar(char),
    SetModifiers { bold: bool, underline: bool, italic: bool },
    Reset,
}

pub struct DiffEngine;

impl DiffEngine {
    pub fn diff(prev: &CellBuffer, next: &CellBuffer) -> Vec<AnsiCommand> {
        let mut commands = Vec::new();
        
        let mut cur_fg = AnsiColor::Reset;
        let mut cur_bg = AnsiColor::Reset;
        let mut cur_bold = false;
        let mut cur_underline = false;
        let mut cur_italic = false;
        let mut cur_x: Option<u16> = None;
        let mut cur_y: Option<u16> = None;

        for y in 0..next.height {
            for x in 0..next.width {
                let idx = y as usize * next.width as usize + x as usize;
                let next_cell = &next.cells[idx];
                let prev_cell = prev.cells.get(idx);
                
                if Some(next_cell) != prev_cell {
                    // 1. Move Cursor
                    if cur_x != Some(x) || cur_y != Some(y) {
                        commands.push(AnsiCommand::MoveCursor(x, y));
                        cur_y = Some(y);
                    }

                    // 2. Update Style
                    if next_cell.fg != cur_fg || next_cell.bg != cur_bg {
                        commands.push(AnsiCommand::SetColor { fg: next_cell.fg, bg: next_cell.bg });
                        cur_fg = next_cell.fg;
                        cur_bg = next_cell.bg;
                    }

                    if next_cell.bold != cur_bold || next_cell.underline != cur_underline || next_cell.italic != cur_italic {
                        commands.push(AnsiCommand::SetModifiers {
                            bold: next_cell.bold,
                            underline: next_cell.underline,
                            italic: next_cell.italic,
                        });
                        cur_bold = next_cell.bold;
                        cur_underline = next_cell.underline;
                        cur_italic = next_cell.italic;
                    }

                    // 3. Write
                    commands.push(AnsiCommand::WriteChar(next_cell.ch));
                    
                    // Update tracked position
                    cur_x = Some(x + 1);
                    if cur_x.unwrap() >= next.width {
                        cur_x = Some(0);
                        cur_y = Some(y + 1);
                    }
                }
            }
        }
        
        commands
    }

    pub fn encode_ansi(commands: &[AnsiCommand]) -> Vec<u8> {
        let mut buf = Vec::new();
        let mut last_fg = AnsiColor::Reset;
        let mut last_bg = AnsiColor::Reset;

        for cmd in commands {
            match cmd {
                AnsiCommand::MoveCursor(x, y) => {
                    buf.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
                }
                AnsiCommand::SetColor { fg, bg } => {
                    if *fg != last_fg {
                        buf.extend_from_slice(Self::encode_color(*fg, true).as_bytes());
                        last_fg = *fg;
                    }
                    if *bg != last_bg {
                        buf.extend_from_slice(Self::encode_color(*bg, false).as_bytes());
                        last_bg = *bg;
                    }
                }
                AnsiCommand::WriteChar(ch) => {
                    let mut b = [0; 4];
                    buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                AnsiCommand::SetModifiers { bold, underline, italic } => {
                    buf.extend_from_slice(b"\x1b[0m"); // Reset first to clear previous
                    // Re-apply colors after reset
                    buf.extend_from_slice(Self::encode_color(last_fg, true).as_bytes());
                    buf.extend_from_slice(Self::encode_color(last_bg, false).as_bytes());
                    
                    if *bold { buf.extend_from_slice(b"\x1b[1m"); }
                    if *underline { buf.extend_from_slice(b"\x1b[4m"); }
                    if *italic { buf.extend_from_slice(b"\x1b[3m"); }
                }
                AnsiCommand::Reset => {
                    buf.extend_from_slice(b"\x1b[0m");
                    last_fg = AnsiColor::Reset;
                    last_bg = AnsiColor::Reset;
                }
            }
        }
        
        buf
    }

    fn encode_color(color: AnsiColor, is_fg: bool) -> String {
        let prefix = if is_fg { "38" } else { "48" };
        match color {
            AnsiColor::TrueColor(r, g, b) => format!("\x1b[{prefix};2;{r};{g};{b}m"),
            AnsiColor::Color256(n) => format!("\x1b[{prefix};5;{n}m"),
            AnsiColor::Reset => if is_fg { "\x1b[39m".to_string() } else { "\x1b[49m".to_string() },
        }
    }
}
