use crate::input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseInput, MouseButton, MouseAction};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Escaped,
    Csi,
}

#[derive(Debug)]
pub struct OverflowError;

/// S6-15: BoundedSubnegBuffer - Fixed capacity to prevent memory DoS
pub struct BoundedSubnegBuffer {
    buf: Vec<u8>,
    capacity: usize,
}

impl BoundedSubnegBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
            capacity,
        }
    }
    
    /// S6-16: push that rejects on limit
    pub fn push(&mut self, byte: u8) -> Result<(), OverflowError> {
        if self.buf.len() >= self.capacity {
            Err(OverflowError)
        } else {
            self.buf.push(byte);
            Ok(())
        }
    }
    
    pub fn clear(&mut self) {
        self.buf.clear();
    }
    
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

/// S6-17: InputStateMachine for Defensive Parsing
pub struct InputStateMachine {
    state: State,
    buffer: BoundedSubnegBuffer,
    last_activity: Instant,
}

impl Default for InputStateMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl InputStateMachine {
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            // S6-15: Max 256 bytes buffer
            buffer: BoundedSubnegBuffer::new(256),
            last_activity: Instant::now(),
        }
    }

    /// S6-19: Reset to Idle state on error/overflow
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.buffer.clear();
    }

    pub fn feed_slice(&mut self, data: &[u8]) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for &b in data {
            if let Some(ev) = self.feed(b) {
                events.push(ev);
            }
        }
        events
    }

    /// S6-18: feed step, panic-free
    pub fn feed(&mut self, byte: u8) -> Option<InputEvent> {
        self.last_activity = Instant::now();
        
        match self.state {
            State::Idle => {
                if byte == 0x1b {
                    self.state = State::Escaped;
                    let _ = self.buffer.push(byte);
                    None
                } else if byte == 0x11 || byte == 0x13 {
                    // XON/XOFF handled directly in ReactorThread
                    None
                } else {
                    Some(InputEvent::KeyPress(KeyEvent {
                        codepoint: byte as char,
                        modifiers: KeyModifiers::default(),
                        kind: KeyKind::Press,
                    }))
                }
            }
            State::Escaped => {
                if self.buffer.push(byte).is_err() {
                    self.reset();
                    return None;
                }
                if byte == b'[' {
                    self.state = State::Csi;
                    None
                } else {
                    let data = self.buffer.as_slice().to_vec();
                    self.reset();
                    Some(InputEvent::Unknown(data))
                }
            }
            State::Csi => {
                if self.buffer.push(byte).is_err() {
                    self.reset();
                    return None;
                }
                
                if (0x40..=0x7e).contains(&byte) {
                    let data = self.buffer.as_slice().to_vec();
                    self.reset();
                    
                    let term = byte;
                    let seq_len = data.len();
                    if seq_len > 3 {
                        let seq = &data[2..seq_len - 1]; // Skip ESC [ and Terminator
                        match term {
                            b'u' => Self::parse_kitty(seq).map(InputEvent::KeyPress).or_else(|| Some(InputEvent::Unknown(data.clone()))),
                            b'M' | b'm' => {
                                if data[2] == b'<' {
                                    Self::parse_sgr(&data[3..seq_len - 1], term == b'M')
                                        .map(InputEvent::MouseEvent)
                                        .or_else(|| Some(InputEvent::Unknown(data.clone())))
                                } else {
                                    Some(InputEvent::Unknown(data))
                                }
                            }
                            b'c' => {
                                if data[2] == b'?' {
                                    Some(InputEvent::CapabilityResponse(data))
                                } else {
                                    Some(InputEvent::Unknown(data))
                                }
                            }
                            _ => Some(InputEvent::Unknown(data)),
                        }
                    } else {
                        Some(InputEvent::Unknown(data))
                    }
                } else {
                    None
                }
            }
        }
    }

    fn parse_kitty(seq: &[u8]) -> Option<KeyEvent> {
        let s = std::str::from_utf8(seq).ok()?;
        let parts: Vec<&str> = s.split(';').collect();
        if parts.is_empty() { return None; }

        let codepoint = parts[0].parse::<u32>().ok().and_then(std::char::from_u32)?;
        
        let (modifiers, kind) = if parts.len() > 1 {
            let mod_part = parts[1];
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
