use crate::input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseInput, MouseButton, MouseAction};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Idle,
    Escaped,
    Csi,
    Apc,
    ApcEscaped,
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
    utf8_buf: Vec<u8>,
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
            utf8_buf: Vec::with_capacity(4),
            last_activity: Instant::now(),
        }
    }

    /// S6-19: Reset to Idle state on error/overflow
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.buffer.clear();
        self.utf8_buf.clear();
    }

    pub fn sgr_timeout_guard(&mut self) {
        if self.state != State::Idle && self.last_activity.elapsed() > std::time::Duration::from_millis(200) {
            self.reset();
        }
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
        self.sgr_timeout_guard();
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
                } else if byte >= 0x80 {
                    // Check if this is the start of a sequence
                    if self.utf8_buf.is_empty() {
                        let expected_len = if (0xc2..=0xdf).contains(&byte) {
                            2
                        } else if (0xe0..=0xef).contains(&byte) {
                            3
                        } else if (0xf0..=0xf4).contains(&byte) {
                            4
                        } else {
                            0
                        };
                        if expected_len == 0 {
                            // Invalid UTF-8 start byte: discard to prevent state leak
                            return None;
                        }
                    }

                    self.utf8_buf.push(byte);
                    match std::str::from_utf8(&self.utf8_buf) {
                        Ok(s) => {
                            let cp = s.chars().next().unwrap();
                            self.utf8_buf.clear();
                            Some(InputEvent::KeyPress(KeyEvent {
                                codepoint: cp,
                                modifiers: KeyModifiers::default(),
                                kind: KeyKind::Press,
                            }))
                        }
                        Err(e) => {
                            if e.error_len().is_some() || self.utf8_buf.len() >= 4 {
                                self.utf8_buf.clear();
                            }
                            None
                        }
                    }
                } else {
                    self.utf8_buf.clear();
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
                } else if byte == b'_' {
                    self.state = State::Apc;
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
            State::Apc => {
                if self.buffer.push(byte).is_err() {
                    self.reset();
                    return None;
                }
                if byte == 0x1b {
                    self.state = State::ApcEscaped;
                }
                None
            }
            State::ApcEscaped => {
                if self.buffer.push(byte).is_err() {
                    self.reset();
                    return None;
                }
                if byte == b'\\' {
                    let data = self.buffer.as_slice().to_vec();
                    self.reset();
                    if data.starts_with(&[0x1b, b'_', b'G']) {
                        Some(InputEvent::CapabilityResponse(data))
                    } else {
                        Some(InputEvent::Unknown(data))
                    }
                } else if byte == 0x1b {
                    // Stay in ApcEscaped
                    None
                } else {
                    self.state = State::Apc;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_decoding_and_state_leaks() {
        let mut sm = InputStateMachine::new();

        // 1. Decodes valid UTF-8 sequence (e.g. 'ł' which is 0xc5, 0x82)
        let ev1 = sm.feed(0xc5);
        assert!(ev1.is_none());
        let ev2 = sm.feed(0x82);
        if let Some(InputEvent::KeyPress(ke)) = ev2 {
            assert_eq!(ke.codepoint, 'ł');
        } else {
            panic!("Expected KeyEvent");
        }

        // 2. Reject invalid UTF-8 start byte immediately to prevent leak
        let ev_invalid = sm.feed(0x80); // Invalid start byte
        assert!(ev_invalid.is_none());
        assert!(sm.utf8_buf.is_empty()); // Should not leak / store this byte

        // 3. Incomplete followed by ASCII char boundary reset
        let ev3 = sm.feed(0xc5); // Incomplete ł
        assert!(ev3.is_none());
        assert_eq!(sm.utf8_buf.len(), 1);
        let ev4 = sm.feed(0x61); // ASCII 'a'
        assert_eq!(sm.utf8_buf.len(), 0); // Should be cleared!
        if let Some(InputEvent::KeyPress(ke)) = ev4 {
            assert_eq!(ke.codepoint, 'a');
        } else {
            panic!("Expected KeyEvent 'a'");
        }

        // 4. Incomplete followed by invalid continuation byte clears buffer
        let ev5 = sm.feed(0xc5); // Incomplete ł
        assert!(ev5.is_none());
        let ev6 = sm.feed(0xc5); // Another start byte instead of continuation -> invalid
        assert!(ev6.is_none());
        assert!(sm.utf8_buf.is_empty()); // Should clear immediately
    }

    #[test]
    fn test_sgr_timeout_guard() {
        let mut sm = InputStateMachine::new();
        
        // Feed partial escape sequence
        sm.feed(0x1b); // ESC
        sm.feed(b'['); // [
        assert_eq!(sm.state, State::Csi);
        
        // Wait for timeout (e.g. 250ms)
        std::thread::sleep(std::time::Duration::from_millis(250));
        
        // Feed next character - should trigger sgr_timeout_guard, resetting state to Idle
        // and treating the new character 'a' as normal input
        let ev = sm.feed(b'a');
        assert_eq!(sm.state, State::Idle);
        if let Some(InputEvent::KeyPress(ke)) = ev {
            assert_eq!(ke.codepoint, 'a');
        } else {
            panic!("Expected KeyEvent 'a' after timeout reset");
        }
    }
}
