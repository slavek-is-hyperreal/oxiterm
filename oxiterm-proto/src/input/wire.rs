//! Decoders of the web client's binary input messages (`0x01` key, `0x02` cell mouse, `0x03`
//! fractional pointer). The WebSocket server and the in-browser local mode (OxiUI 7.1) both
//! decode the same bytes with these functions, so the two modes cannot drift apart.

use super::{KeyEvent, KeyKind, KeyModifiers, MouseAction, MouseButton, MouseInput, PointerInput};

/// Largest grid a client may ask for, in cells. A terminal or canvas of 500x200 cells covers a 5K
/// display at a 10x20 px cell; anything bigger is a client bug or an attack (an unclamped
/// 65535x65535 resize allocates gigabytes per buffer and aborts the process).
pub const MAX_COLS: u16 = 500;
/// See [`MAX_COLS`].
pub const MAX_ROWS: u16 = 200;

/// Clamps a client-supplied grid size to `1..=MAX_COLS` x `1..=MAX_ROWS`.
pub fn clamp_dims(cols: u16, rows: u16) -> (u16, u16) {
    (cols.clamp(1, MAX_COLS), rows.clamp(1, MAX_ROWS))
}

/// Same clamp for sizes that arrive as `u32` (SSH `pty-req` / `window-change`).
pub fn clamp_dims_u32(cols: u32, rows: u32) -> (u16, u16) {
    clamp_dims(u16::try_from(cols).unwrap_or(u16::MAX), u16::try_from(rows).unwrap_or(u16::MAX))
}

/// Longest navigation target (`0x31`) the decoder passes on.
pub const MAX_NAV_BYTES: usize = 1024;

fn modifiers(flags: u8) -> KeyModifiers {
    KeyModifiers { shift: flags & 1 != 0, ctrl: flags & 2 != 0, alt: flags & 4 != 0, ..Default::default() }
}

fn button(b: u8) -> MouseButton {
    match b {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        3 => MouseButton::WheelUp,
        4 => MouseButton::WheelDown,
        _ => MouseButton::None,
    }
}

fn action(a: u8) -> MouseAction {
    match a {
        0 => MouseAction::Press,
        1 => MouseAction::Release,
        _ => MouseAction::Move,
    }
}

/// `[0x01, codepoint:u32le, flags]` (6 bytes). `None` when short or not a Unicode scalar.
pub fn decode_key(bytes: &[u8]) -> Option<KeyEvent> {
    if bytes.len() < 6 || bytes[0] != 0x01 {
        return None;
    }
    let val = u32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    Some(KeyEvent { codepoint: char::from_u32(val)?, modifiers: modifiers(bytes[5]), kind: KeyKind::Press })
}

/// `[0x02, col:u16le, row:u16le, button, action, flags]` (8 bytes), 1-based cell.
pub fn decode_mouse(bytes: &[u8]) -> Option<MouseInput> {
    if bytes.len() < 8 || bytes[0] != 0x02 {
        return None;
    }
    Some(MouseInput {
        col: u16::from_le_bytes([bytes[1], bytes[2]]),
        row: u16::from_le_bytes([bytes[3], bytes[4]]),
        button: button(bytes[5]),
        action: action(bytes[6]),
        modifiers: modifiers(bytes[7]),
    })
}

/// `[0x10, cols:u16le, rows:u16le]` (5 bytes), clamped to the grid limits (see [`MAX_COLS`]).
pub fn decode_resize(bytes: &[u8]) -> Option<(u16, u16)> {
    if bytes.len() < 5 || bytes[0] != 0x10 {
        return None;
    }
    Some(clamp_dims(u16::from_le_bytes([bytes[1], bytes[2]]), u16::from_le_bytes([bytes[3], bytes[4]])))
}

/// `[0x11, mobile]`: the viewport flag.
pub fn decode_viewport(bytes: &[u8]) -> Option<bool> {
    if bytes.len() < 2 || bytes[0] != 0x11 {
        return None;
    }
    Some(bytes[1] != 0)
}

/// `[0x31, utf8 path]`: a navigation target, at most [`MAX_NAV_BYTES`] bytes of valid UTF-8.
pub fn decode_navigate(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 2 || bytes[0] != 0x31 || bytes.len() - 1 > MAX_NAV_BYTES {
        return None;
    }
    String::from_utf8(bytes[1..].to_vec()).ok()
}

/// Fractional pointer `[0x03, x:f32le, y:f32le, button, action, flags]` (OxiUI Phase 6.5, 12
/// bytes). `x`/`y` are 0-based canvas cells (`px / on-screen cell size`), fractional. Anything
/// shorter than 12 bytes or with a non-finite coordinate is dropped; trailing bytes are ignored.
pub fn decode_pointer(bytes: &[u8]) -> Option<PointerInput> {
    if bytes.len() < 12 || bytes[0] != 0x03 {
        return None;
    }
    let x = f32::from_le_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]);
    let y = f32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(PointerInput { x, y, button: button(bytes[9]), action: action(bytes[10]), modifiers: modifiers(bytes[11]) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_decodes_and_rejects_bad_scalars() {
        let k = decode_key(&[0x01, b'a', 0, 0, 0, 3]).unwrap();
        assert_eq!(k.codepoint, 'a');
        assert!(k.modifiers.shift && k.modifiers.ctrl && !k.modifiers.alt);
        assert!(decode_key(&[0x01, 0x00, 0xD8, 0, 0, 0]).is_none(), "surrogate");
        assert!(decode_key(&[0x01, b'a', 0, 0, 0]).is_none(), "short");
    }

    #[test]
    fn resize_is_clamped_to_the_grid_limits() {
        assert_eq!(decode_resize(&[0x10, 0xff, 0xff, 0xff, 0xff]), Some((MAX_COLS, MAX_ROWS)));
        assert_eq!(decode_resize(&[0x10, 0, 0, 0, 0]), Some((1, 1)));
        assert_eq!(decode_resize(&[0x10, 80, 0, 24, 0]), Some((80, 24)));
        assert_eq!(decode_resize(&[0x10, 80, 0]), None);
        assert_eq!(clamp_dims_u32(u32::MAX, 70000), (MAX_COLS, MAX_ROWS));
        assert_eq!(clamp_dims_u32(0, 3), (1, 3));
    }

    #[test]
    fn viewport_and_navigate_decode() {
        assert_eq!(decode_viewport(&[0x11, 1]), Some(true));
        assert_eq!(decode_viewport(&[0x11, 0]), Some(false));
        assert_eq!(decode_viewport(&[0x11]), None);
        assert_eq!(decode_navigate(b"\x31a.thtml").as_deref(), Some("a.thtml"));
        assert_eq!(decode_navigate(&[0x31, 0xff]), None);
        assert_eq!(decode_navigate(&[0x31]), None);
        let mut long = vec![0x31];
        long.extend(std::iter::repeat(b'a').take(MAX_NAV_BYTES + 1));
        assert_eq!(decode_navigate(&long), None);
    }

    #[test]
    fn mouse_and_pointer_decode() {
        let m = decode_mouse(&[0x02, 5, 0, 7, 0, 3, 1, 4]).unwrap();
        assert_eq!((m.col, m.row, m.button, m.action), (5, 7, MouseButton::WheelUp, MouseAction::Release));
        assert!(m.modifiers.alt);
        let mut p = vec![0x03];
        p.extend_from_slice(&1.5f32.to_le_bytes());
        p.extend_from_slice(&2.25f32.to_le_bytes());
        p.extend_from_slice(&[0, 2, 0]);
        let d = decode_pointer(&p).unwrap();
        assert_eq!((d.x, d.y, d.action), (1.5, 2.25, MouseAction::Move));
        p[1..5].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(decode_pointer(&p).is_none());
    }
}
