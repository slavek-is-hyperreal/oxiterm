use crate::render::buffer::CellBuffer;
use oxiterm_proto::style::AnsiColor;

#[derive(Debug)]
pub enum AnsiCommand {
    MoveCursor(u16, u16),
    SetColor(AnsiColor, AnsiColor),
    WriteChar(char),
    SetModifiers { bold: bool, underline: bool, italic: bool },
}

pub struct DiffEngine;

impl DiffEngine {
    pub fn diff(prev: &CellBuffer, next: &CellBuffer) -> Vec<AnsiCommand> {
        let mut commands = Vec::new();
        
        // Wrap in BSU/ESU (Begin/End Synchronized Update)
        // Handled during encoding for now.
        
        for y in 0..next.height {
            for x in 0..next.width {
                let idx = y as usize * next.width as usize + x as usize;
                let prev_cell = prev.cells.get(idx);
                let next_cell = &next.cells[idx];
                
                if Some(next_cell) != prev_cell {
                    commands.push(AnsiCommand::MoveCursor(x, y));
                    commands.push(AnsiCommand::SetColor(next_cell.fg, next_cell.bg));
                    commands.push(AnsiCommand::SetModifiers {
                        bold: next_cell.bold,
                        underline: next_cell.underline,
                        italic: next_cell.italic,
                    });
                    commands.push(AnsiCommand::WriteChar(next_cell.ch));
                }
            }
        }
        
        commands
    }

    pub fn encode_ansi(commands: &[AnsiCommand]) -> Vec<u8> {
        let mut buf = Vec::new();
        
        // BSU: CSI ? 2026 h
        buf.extend_from_slice(b"\x1b[?2026h");
        
        for cmd in commands {
            match cmd {
                AnsiCommand::MoveCursor(x, y) => {
                    buf.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
                }
                AnsiCommand::SetColor(fg, bg) => {
                    // Simplified color encoding
                    buf.extend_from_slice(Self::encode_color(*fg, true).as_bytes());
                    buf.extend_from_slice(Self::encode_color(*bg, false).as_bytes());
                }
                AnsiCommand::WriteChar(ch) => {
                    let mut b = [0; 4];
                    buf.extend_from_slice(ch.encode_utf8(&mut b).as_bytes());
                }
                AnsiCommand::SetModifiers { bold, underline, italic } => {
                    if *bold { buf.extend_from_slice(b"\x1b[1m"); }
                    if *underline { buf.extend_from_slice(b"\x1b[4m"); }
                    if *italic { buf.extend_from_slice(b"\x1b[3m"); }
                    if !bold && !underline && !italic { buf.extend_from_slice(b"\x1b[0m"); }
                }
            }
        }
        
        // ESU: CSI ? 2026 l
        buf.extend_from_slice(b"\x1b[?2026l");
        
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
