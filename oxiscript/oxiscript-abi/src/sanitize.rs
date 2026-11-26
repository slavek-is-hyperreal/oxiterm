//! Shared terminal-escape and bidi-control sanitiser for diagnostic output,
//! echoed source snippets, runtime log lines, playground messages and editor
//! text paths (`docs/roadmap/oxiscript.md`, stage 8.Sec `[R3-206]`).
//!
//! OxiTerm is a terminal UI, so raw control characters (C0 including ESC/CR/LF,
//! DEL, C1 including 8-bit CSI `U+009B` and OSC `U+009D`) or bidirectional
//! override/isolate controls (`U+202A..=U+202E`, `U+2066..=U+2069`, etc.) in
//! echoed source, identifiers, file paths or guest strings could inject ANSI
//! escape sequences or spoof displayed text.

/// Returns `true` for any character that must be escaped before printing to a
/// terminal, log sink, browser playground or editor diagnostic panel:
/// * all Unicode control characters (C0 `U+0000..=U+001F` including ESC, CR,
///   LF, BEL, BS, NUL, TAB; DEL `U+007F`; C1 `U+0080..=U+009F` including 8-bit
///   CSI `U+009B` and 8-bit OSC `U+009D`);
/// * bidirectional controls (Trojan Source / CVE-2021-42574: `U+061C`,
///   `U+200E..=U+200F`, `U+202A..=U+202E`, `U+2066..=U+2069`);
/// * invisible format characters and Unicode line/paragraph separators
///   (`U+00AD`, `U+180E`, `U+200B..=U+200D`, `U+2028..=U+2029`,
///   `U+2060..=U+2064`, `U+206A..=U+206F`, `U+FEFF`, `U+FFF9..=U+FFFB`,
///   `U+E0000..=U+E007F`).
#[must_use]
pub fn is_unsafe_terminal_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00ad}'
                | '\u{061c}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206f}'
                | '\u{feff}'
                | '\u{fff9}'..='\u{fffb}'
                | '\u{e0000}'..='\u{e007f}'
        )
}

/// Format a single unsafe character as a visible ASCII escape (`\xNN` for
/// code points `< 0x100`, `\u{xxxx}` otherwise).
fn push_escaped(out: &mut String, c: char) {
    use std::fmt::Write;
    let code = c as u32;
    if code < 0x100 {
        let _ = write!(out, "\\x{code:02x}");
    } else {
        let _ = write!(out, "\\u{{{code:x}}}");
    }
}

/// Escape every unsafe terminal/bidi character in `s` into a visible ASCII
/// sequence (`\xNN` or `\u{xxxx}`). Safe strings are returned unchanged.
#[must_use]
pub fn sanitize_terminal(s: &str) -> String {
    if !s.chars().any(is_unsafe_terminal_char) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if is_unsafe_terminal_char(c) {
            push_escaped(&mut out, c);
        } else {
            out.push(c);
        }
    }
    out
}

/// Like [`sanitize_terminal`], stopping before the output exceeds `max_bytes`
/// without splitting a UTF-8 character or an escape sequence.
#[must_use]
pub fn sanitize_terminal_bounded(s: &str, max_bytes: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max_bytes));
    let mut buf = String::new();
    for c in s.chars() {
        buf.clear();
        if is_unsafe_terminal_char(c) {
            push_escaped(&mut buf, c);
        } else {
            buf.push(c);
        }
        if out.len() + buf.len() > max_bytes {
            break;
        }
        out.push_str(&buf);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_text_passes_through_unchanged() {
        let s = "hello world: zażółć gęślą jaźń 123 + (a === b)";
        assert_eq!(sanitize_terminal(s), s);
    }

    #[test]
    fn neutralises_esc_csi_osc_c1_cr_and_bidi() {
        // ESC + CSI clear screen
        assert_eq!(sanitize_terminal("\x1b[2Jhi"), "\\x1b[2Jhi");
        // OSC title rewrite with BEL terminator
        assert_eq!(sanitize_terminal("\x1b]0;pwn\x07ok"), "\\x1b]0;pwn\\x07ok");
        // OSC 8 hyperlink with ST terminator
        assert_eq!(
            sanitize_terminal("\x1b]8;;https://x\x1b\\link\x1b]8;;\x1b\\"),
            "\\x1b]8;;https://x\\x1b\\link\\x1b]8;;\\x1b\\"
        );
        // 8-bit C1 CSI (U+009B) and OSC (U+009D) + ST (U+009C)
        assert_eq!(
            sanitize_terminal("\u{009b}31mred\u{009d}0;title\u{009c}"),
            "\\x9b31mred\\x9d0;title\\x9c"
        );
        // CR overwrite and LF / NUL / DEL
        assert_eq!(sanitize_terminal("fake\rreal\n\0\x7f"), "fake\\x0dreal\\x0a\\x00\\x7f");
        // Bidi controls (Trojan Source)
        assert_eq!(
            sanitize_terminal("a\u{202e}b\u{2066}c\u{2069}d\u{200f}e\u{061c}f"),
            "a\\u{202e}b\\u{2066}c\\u{2069}d\\u{200f}e\\u{61c}f"
        );
        // Output never retains any unsafe character
        let hostile = "\x1b[31m\x1b]0;t\x07\u{009b}2J\r\n\u{202e}\u{2066}\u{2069}\u{2028}\u{feff}";
        let clean = sanitize_terminal(hostile);
        assert!(!clean.chars().any(is_unsafe_terminal_char), "{clean:?}");
    }

    #[test]
    fn bounded_never_splits_an_escape() {
        // "\x1b" is 4 bytes; cap at 3 must yield "" rather than "\x1"
        assert_eq!(sanitize_terminal_bounded("\x1b", 3), "");
        assert_eq!(sanitize_terminal_bounded("\x1b", 4), "\\x1b");
        // "\u{202e}" is 8 bytes
        assert_eq!(sanitize_terminal_bounded("a\u{202e}b", 8), "a");
        assert_eq!(sanitize_terminal_bounded("a\u{202e}b", 9), "a\\u{202e}");
    }
}
