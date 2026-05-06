use crate::input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseInput, MouseButton, MouseAction};

pub struct InputDecoder {
    buffer: Vec<u8>,
    last_activity: std::time::Instant,
}

impl InputDecoder {
    pub fn new() -> Self {
        Self { 
            buffer: Vec::new(),
            last_activity: std::time::Instant::now(),
        }
    }
}

impl Default for InputDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl InputDecoder {

    pub fn feed(&mut self, data: &[u8]) -> Vec<InputEvent> {
        self.buffer.extend_from_slice(data);
        let mut events = Vec::new();
        
        while !self.buffer.is_empty() {
            if let Some((event, consumed)) = self.try_decode() {
                events.push(event);
                self.buffer.drain(..consumed);
                self.last_activity = std::time::Instant::now();
            } else {
                // If we can't decode yet, but it starts with ESC, wait for more data
                // UNLESS it has timed out (S5-33)
                if self.buffer[0] == 0x1b && self.buffer.len() < 16 && self.last_activity.elapsed() < std::time::Duration::from_millis(100) {
                    break;
                }
                
                // Otherwise, it might be a single char or a garbage sequence
                let ch = self.buffer[0] as char;
                events.push(InputEvent::KeyPress(KeyEvent {
                    codepoint: ch,
                    modifiers: KeyModifiers::default(),
                    kind: KeyKind::Press,
                }));
                self.buffer.remove(0);
                self.last_activity = std::time::Instant::now();
            }
        }
        
        events
    }

    fn try_decode(&self) -> Option<(InputEvent, usize)> {
        if self.buffer.is_empty() {
            return None;
        }

        if self.buffer[0] != 0x1b {
            return None;
        }

        if self.buffer.len() < 2 {
            return None;
        }

        match self.buffer[1] {
            b'[' => self.decode_csi(),
            _ => None, // Handle other ESC sequences if needed
        }
    }

    fn decode_csi(&self) -> Option<(InputEvent, usize)> {
        // Find terminator (u for Kitty, M/m for SGR)
        let mut end = None;
        for (i, &b) in self.buffer.iter().enumerate().skip(2) {
            if (0x40..=0x7e).contains(&b) {
                end = Some((i, b));
                break;
            }
        }

        let (idx, term) = end?;
        let seq = &self.buffer[2..idx];
        let full_len = idx + 1;

        match term {
            b'u' => Self::parse_kitty(seq).map(|ev| (InputEvent::KeyPress(ev), full_len)),
            b'M' | b'm' => {
                if self.buffer.len() > 2 && self.buffer[2] == b'<' {
                    Self::parse_sgr(&self.buffer[3..idx], term == b'M').map(|ev| (InputEvent::MouseEvent(ev), full_len))
                } else {
                    None
                }
            }
            _ => Some((InputEvent::Unknown(self.buffer[..full_len].to_vec()), full_len)),
        }
    }

    fn parse_kitty(seq: &[u8]) -> Option<KeyEvent> {
        let s = std::str::from_utf8(seq).ok()?;
        let parts: Vec<&str> = s.split(';').collect();
        if parts.is_empty() { return None; }

        let codepoint = parts[0].parse::<u32>().ok().and_then(std::char::from_u32)?;
        
        let (modifiers, kind) = if parts.len() > 1 {
            let mod_part = parts[1];
            // Format can be "mods:kind"
            let sub_parts: Vec<&str> = mod_part.split(':').collect();
            let mod_val = sub_parts[0].parse::<u32>().unwrap_or(1).saturating_sub(1);
            
            let modifiers = KeyModifiers {
                shift: (mod_val & 0x01) != 0,
                alt: (mod_val & 0x02) != 0,
                ctrl: (mod_val & 0x04) != 0,
                meta: (mod_val & 0x08) != 0,
                ..KeyModifiers::default()
            };

            let kind = if sub_parts.len() > 1 {
                match sub_parts[1] {
                    "2" => KeyKind::Repeat,
                    "3" => KeyKind::Release,
                    _ => KeyKind::Press,
                }
            } else {
                KeyKind::Press
            };
            (modifiers, kind)
        } else {
            (KeyModifiers::default(), KeyKind::Press)
        };

        Some(KeyEvent { codepoint, modifiers, kind })
    }

    fn parse_sgr(seq: &[u8], pressed: bool) -> Option<MouseInput> {
        let s = std::str::from_utf8(seq).ok()?;
        let parts: Vec<&str> = s.split(';').collect();
        if parts.len() != 3 { return None; }

        let b_val = parts[0].parse::<u32>().ok()?;
        let col = parts[1].parse::<u16>().ok()?;
        let row = parts[2].parse::<u16>().ok()?;

        let modifiers = KeyModifiers {
            shift: (b_val & 0x04) != 0,
            alt: (b_val & 0x08) != 0,
            ctrl: (b_val & 0x10) != 0,
            ..KeyModifiers::default()
        };

        let button = match b_val & 0x43 {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            64 => MouseButton::WheelUp,
            65 => MouseButton::WheelDown,
            _ => MouseButton::None,
        };

        let action = if (b_val & 0x20) != 0 {
            MouseAction::Move
        } else if pressed {
            MouseAction::Press
        } else {
            MouseAction::Release
        };

        Some(MouseInput {
            col,
            row,
            button,
            action,
            modifiers,
        })
    }
}
