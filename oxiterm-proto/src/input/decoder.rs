//! State machine decoder for raw terminal input streams.
//!
//! Parses incoming byte sequences into high-level [`InputEvent`] representations.
//! Handles ANSI escape sequences, UTF-8 multi-byte characters, Kitty keyboard protocol,
//! and SGR mouse tracking. Designed defensively to prevent state leaks or unbounded memory growth.

use crate::input::{InputEvent, KeyEvent, KeyKind, KeyModifiers, MouseInput, MouseButton, MouseAction};
use std::time::Instant;

/// The parser states for decoding ANSI escape sequences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Idle state, parsing normal characters or waiting for escape start.
    Idle,
    /// ESC character received; awaiting next escape command sequence.
    Escaped,
    /// Parsing a Control Sequence Introducer (CSI) parameter string.
    Csi,
    /// Reading Application Program Command (APC) blocks.
    Apc,
    /// Encountered an ESC while reading APC blocks, awaiting string terminator backslash.
    ApcEscaped,
}

/// An error indicating that the internal input buffer has reached its capacity limit.
#[derive(Debug)]
pub struct OverflowError;

/// Bounded input buffer with a strict capacity limit.
///
/// Anchored by spec [S6-15]. Prevents memory exhaustion attacks by refusing
/// to grow beyond its predefined allocation limit.
pub struct BoundedSubnegBuffer {
    buf: Vec<u8>,
    capacity: usize,
}

impl BoundedSubnegBuffer {
    /// Creates a new buffer with the specified capacity limit.
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: Vec::with_capacity(capacity),
            capacity,
        }
    }
    
    /// Appends a byte to the end of the buffer.
    ///
    /// # Errors
    ///
    /// Returns [`OverflowError`] if the buffer is full, as defined in spec [S6-16].
    pub fn push(&mut self, byte: u8) -> Result<(), OverflowError> {
        if self.buf.len() >= self.capacity {
            Err(OverflowError)
        } else {
            self.buf.push(byte);
            Ok(())
        }
    }
    
    /// Clears the contents of the buffer.
    pub fn clear(&mut self) {
        self.buf.clear();
    }
    
    /// Returns a read-only slice of the buffer.
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }
}

/// A stateful input decoder using defensive parsing.
///
/// Anchored by spec [S6-17]. Processes streams byte-by-byte and resets its
/// internal state automatically upon encountering malformed sequences or timeouts.
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
    /// Creates a new state machine.
    ///
    /// Allocates an internal buffer with a strict limit of 256 bytes [S6-15]
    /// to avoid memory exhaustion from unclosed escape sequences.
    pub fn new() -> Self {
        Self {
            state: State::Idle,
            buffer: BoundedSubnegBuffer::new(256),
            utf8_buf: Vec::with_capacity(4),
            last_activity: Instant::now(),
        }
    }

    /// Resets the state machine back to its idle state, clearing all temporary buffers.
    ///
    /// Anchored by spec [S6-19]. Used to recover from overflow errors and stream out-of-sync events.
    pub fn reset(&mut self) {
        self.state = State::Idle;
        self.buffer.clear();
        self.utf8_buf.clear();
    }

    /// Resets the parser state if too much time has passed since the last received byte.
    ///
    /// This prevents the state machine from being permanently stuck in an incomplete
    /// escape parsing state (e.g. CSI or APC) if a client terminates or drops a packet.
    pub fn sgr_timeout_guard(&mut self) {
        if self.state != State::Idle && self.last_activity.elapsed() > std::time::Duration::from_millis(200) {
            self.reset();
        }
    }

    /// Feeds a slice of raw bytes into the state machine, returning all decoded events.
    pub fn feed_slice(&mut self, data: &[u8]) -> Vec<InputEvent> {
        let mut events = Vec::new();
        for &b in data {
            if let Some(ev) = self.feed(b) {
                events.push(ev);
            }
        }
        events
    }

    /// Feeds a single byte into the parser state machine.
    ///
    /// Processes the byte according to the current state, updating timeouts
    /// and resetting buffers on overflow or state transitions [S6-18].
    pub fn feed(&mut self, byte: u8) -> Option<InputEvent> {
        self.sgr_timeout_guard();
        self.last_activity = Instant::now();
        
        match self.state {
            State::Idle => {
                if byte == 0x1b {
                    self.state = State::Escaped;
                    let _ = self.buffer.push(byte);
                    None
                } else if byte == 0x11 {
                    Some(InputEvent::Xon)
                } else if byte == 0x13 {
                    Some(InputEvent::Xoff)
                } else if byte >= 0x80 {
                    // Handle multi-byte UTF-8 sequences.
                    // Validate start byte range to prevent stale state memory leaks.
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
                        let seq = &data[2..seq_len - 1];
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

        let ev1 = sm.feed(0xc5);
        assert!(ev1.is_none());
        let ev2 = sm.feed(0x82);
        if let Some(InputEvent::KeyPress(ke)) = ev2 {
            assert_eq!(ke.codepoint, 'ł');
        } else {
            panic!("Expected KeyEvent");
        }

        let ev_invalid = sm.feed(0x80);
        assert!(ev_invalid.is_none());
        assert!(sm.utf8_buf.is_empty());

        let ev3 = sm.feed(0xc5);
        assert!(ev3.is_none());
        assert_eq!(sm.utf8_buf.len(), 1);
        let ev4 = sm.feed(0x61);
        assert_eq!(sm.utf8_buf.len(), 0);
        if let Some(InputEvent::KeyPress(ke)) = ev4 {
            assert_eq!(ke.codepoint, 'a');
        } else {
            panic!("Expected KeyEvent 'a'");
        }

        let ev5 = sm.feed(0xc5);
        assert!(ev5.is_none());
        let ev6 = sm.feed(0xc5);
        assert!(ev6.is_none());
        assert!(sm.utf8_buf.is_empty());
    }

    #[test]
    fn test_sgr_timeout_guard() {
        let mut sm = InputStateMachine::new();
        
        sm.feed(0x1b);
        sm.feed(b'[');
        assert_eq!(sm.state, State::Csi);
        
        std::thread::sleep(std::time::Duration::from_millis(250));
        
        let ev = sm.feed(b'a');
        assert_eq!(sm.state, State::Idle);
        if let Some(InputEvent::KeyPress(ke)) = ev {
            assert_eq!(ke.codepoint, 'a');
        } else {
            panic!("Expected KeyEvent 'a' after timeout reset");
        }
    }

    #[test]
    fn test_xon_xoff_handling() {
        let mut sm = InputStateMachine::new();
        
        let ev1 = sm.feed(0x11);
        assert_eq!(ev1, Some(InputEvent::Xon));
        
        let ev2 = sm.feed(0x13);
        assert_eq!(ev2, Some(InputEvent::Xoff));
    }
}
