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
            let mut x = 0;
            while x < next.width {
                let idx = y as usize * next.width as usize + x as usize;
                let next_cell = &next.cells[idx];
                
                let char_w = crate::render::unicode::UnicodeWidthCache::get().width(next_cell.ch) as u16;
                let char_w = if char_w == 0 { 1 } else { char_w };

                if next_cell.skip {
                    cur_x = None;
                    cur_y = None;
                    x += char_w;
                    continue;
                }

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
                    cur_x = Some(x + char_w);
                    if cur_x.unwrap() >= next.width {
                        cur_x = Some(0);
                        cur_y = Some(y + 1);
                    }
                }
                x += char_w;
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

    pub fn encode_binary(commands: &[AnsiCommand]) -> Vec<u8> {
        let mut buf = Vec::new();
        for cmd in commands {
            match cmd {
                AnsiCommand::MoveCursor(x, y) => {
                    buf.push(0x01);
                    buf.extend_from_slice(&x.to_le_bytes());
                    buf.extend_from_slice(&y.to_le_bytes());
                }
                AnsiCommand::SetColor { fg, bg } => {
                    buf.push(0x02);
                    // Encode fg
                    match fg {
                        AnsiColor::Reset => {
                            buf.push(0);
                        }
                        AnsiColor::TrueColor(r, g, b) => {
                            buf.push(1);
                            buf.push(*r);
                            buf.push(*g);
                            buf.push(*b);
                        }
                        AnsiColor::Color256(idx) => {
                            buf.push(2);
                            buf.push(*idx);
                        }
                    }
                    // Encode bg
                    match bg {
                        AnsiColor::Reset => {
                            buf.push(0);
                        }
                        AnsiColor::TrueColor(r, g, b) => {
                            buf.push(1);
                            buf.push(*r);
                            buf.push(*g);
                            buf.push(*b);
                        }
                        AnsiColor::Color256(idx) => {
                            buf.push(2);
                            buf.push(*idx);
                        }
                    }
                }
                AnsiCommand::WriteChar(ch) => {
                    buf.push(0x03);
                    buf.extend_from_slice(&(*ch as u32).to_le_bytes());
                }
                AnsiCommand::SetModifiers { bold, underline, italic } => {
                    buf.push(0x04);
                    let mut flags = 0u8;
                    if *bold { flags |= 1; }
                    if *underline { flags |= 2; }
                    if *italic { flags |= 4; }
                    buf.push(flags);
                }
                AnsiCommand::Reset => {
                    buf.push(0x05);
                }
            }
        }
        buf
    }

    pub fn decode_binary(bytes: &[u8]) -> Result<Vec<AnsiCommand>, &'static str> {
        let mut commands = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let tag = bytes[i];
            i += 1;
            match tag {
                0x01 => {
                    if i + 4 > bytes.len() { return Err("Truncated MoveCursor"); }
                    let x = u16::from_le_bytes([bytes[i], bytes[i+1]]);
                    let y = u16::from_le_bytes([bytes[i+2], bytes[i+3]]);
                    i += 4;
                    commands.push(AnsiCommand::MoveCursor(x, y));
                }
                0x02 => {
                    if i >= bytes.len() { return Err("Truncated SetColor fg type"); }
                    let fg_type = bytes[i];
                    i += 1;
                    let fg = match fg_type {
                        0 => AnsiColor::Reset,
                        1 => {
                            if i + 3 > bytes.len() { return Err("Truncated TrueColor fg"); }
                            let r = bytes[i];
                            let g = bytes[i+1];
                            let b = bytes[i+2];
                            i += 3;
                            AnsiColor::TrueColor(r, g, b)
                        }
                        2 => {
                            if i >= bytes.len() { return Err("Truncated Color256 fg"); }
                            let idx = bytes[i];
                            i += 1;
                            AnsiColor::Color256(idx)
                        }
                        _ => return Err("Invalid fg type"),
                    };

                    if i >= bytes.len() { return Err("Truncated SetColor bg type"); }
                    let bg_type = bytes[i];
                    i += 1;
                    let bg = match bg_type {
                        0 => AnsiColor::Reset,
                        1 => {
                            if i + 3 > bytes.len() { return Err("Truncated TrueColor bg"); }
                            let r = bytes[i];
                            let g = bytes[i+1];
                            let b = bytes[i+2];
                            i += 3;
                            AnsiColor::TrueColor(r, g, b)
                        }
                        2 => {
                            if i >= bytes.len() { return Err("Truncated Color256 bg"); }
                            let idx = bytes[i];
                            i += 1;
                            AnsiColor::Color256(idx)
                        }
                        _ => return Err("Invalid bg type"),
                    };

                    commands.push(AnsiCommand::SetColor { fg, bg });
                }
                0x03 => {
                    if i + 4 > bytes.len() { return Err("Truncated WriteChar"); }
                    let val = u32::from_le_bytes([bytes[i], bytes[i+1], bytes[i+2], bytes[i+3]]);
                    i += 4;
                    let ch = char::from_u32(val).ok_or("Invalid char value")?;
                    commands.push(AnsiCommand::WriteChar(ch));
                }
                0x04 => {
                    if i >= bytes.len() { return Err("Truncated SetModifiers"); }
                    let flags = bytes[i];
                    i += 1;
                    let bold = (flags & 1) != 0;
                    let underline = (flags & 2) != 0;
                    let italic = (flags & 4) != 0;
                    commands.push(AnsiCommand::SetModifiers { bold, underline, italic });
                }
                0x05 => {
                    commands.push(AnsiCommand::Reset);
                }
                _ => return Err("Invalid tag"),
            }
        }
        Ok(commands)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::buffer::CellBuffer;
    use oxiterm_proto::style::AnsiColor;

    #[test]
    fn test_diff_empty() {
        let prev = CellBuffer::new(10, 10);
        let next = CellBuffer::new(10, 10);
        let cmds = DiffEngine::diff(&prev, &next);
        assert!(cmds.is_empty());
    }

    #[test]
    fn test_diff_simple_write() {
        let prev = CellBuffer::new(10, 1);
        let mut next = CellBuffer::new(10, 1);
        next.cells[0].ch = 'H';
        let cmds = DiffEngine::diff(&prev, &next);
        assert_eq!(cmds.len(), 2); // MoveCursor(0,0) + WriteChar('H')
        assert!(matches!(cmds[0], AnsiCommand::MoveCursor(0, 0)));
        assert!(matches!(cmds[1], AnsiCommand::WriteChar('H')));
    }

    #[test]
    fn test_diff_style_change() {
        let prev = CellBuffer::new(10, 1);
        let mut next = CellBuffer::new(10, 1);
        next.cells[0].ch = 'X';
        next.cells[0].fg = AnsiColor::Color256(1);
        let cmds = DiffEngine::diff(&prev, &next);
        // MoveCursor + SetColor + WriteChar
        assert_eq!(cmds.len(), 3);
        assert!(matches!(cmds[1], AnsiCommand::SetColor { .. }));
    }

    #[test]
    fn test_diff_wide_character() {
        let prev = CellBuffer::new(10, 1);
        let mut next = CellBuffer::new(10, 1);
        next.cells[0].ch = '🚀'; // Width 2
        next.cells[2].ch = 'A'; // Width 1
        
        let cmds = DiffEngine::diff(&prev, &next);
        
        // 1. MoveCursor(0, 0)
        // 2. WriteChar('🚀')
        // 3. WriteChar('A') (Should NOT have MoveCursor because cursor naturally advanced to x=2)
        assert_eq!(cmds.len(), 3);
        assert!(matches!(cmds[0], AnsiCommand::MoveCursor(0, 0)));
        assert!(matches!(cmds[1], AnsiCommand::WriteChar('🚀')));
        assert!(matches!(cmds[2], AnsiCommand::WriteChar('A')));
    }

    #[test]
    fn test_binary_serialization() {
        let original = vec![
            AnsiCommand::MoveCursor(10, 20),
            AnsiCommand::SetColor {
                fg: AnsiColor::TrueColor(1, 2, 3),
                bg: AnsiColor::Color256(42),
            },
            AnsiCommand::WriteChar('X'),
            AnsiCommand::SetModifiers {
                bold: true,
                underline: false,
                italic: true,
            },
            AnsiCommand::Reset,
        ];

        let encoded = DiffEngine::encode_binary(&original);
        let decoded = DiffEngine::decode_binary(&encoded).expect("Decoding failed");
        assert_eq!(original, decoded);
    }
}
