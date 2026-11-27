//! Shared terminal-escape and bidi-control sanitiser for diagnostic output,
//! echoed source snippets, runtime log lines, playground messages and editor
//! text paths (`docs/roadmap/oxiscript.md`, stage 8.Sec `[R3-206]`).
//!
//! OxiTerm is a terminal UI, so raw control characters (C0 including ESC/CR/LF,
//! DEL, C1 including 8-bit CSI `U+009B` and OSC `U+009D`) or bidirectional
//! override/isolate controls (`U+202A..=U+202E`, `U+2066..=U+2069`, etc.) in
//! echoed source, identifiers, file paths or guest strings could inject ANSI
//! escape sequences or spoof displayed text.
//!
//! # Two-context rule (identifiers/paths vs guest log text)
//!
//! * **Code, identifiers, file names and diagnostics** ([`is_unsafe_terminal_char`],
//!   [`sanitize_terminal`], [`sanitize_terminal_bounded`]): escape all control
//!   characters, bidi controls and *all* invisible format characters including
//!   `U+200C` (ZWNJ) and `U+200D` (ZWJ), so invisible characters in paths,
//!   snippets or diagnostic text cannot spoof identifiers or file names.
//! * **Guest log / natural-language text** ([`is_unsafe_log_char`],
//!   [`sanitize_log`], [`sanitize_log_bounded`]): escape all control characters
//!   (C0/DEL/C1), all bidi overrides/isolates/marks, line/paragraph separators
//!   and tag/invisible spoofing characters, while **preserving** `U+200C` (ZWNJ)
//!   and `U+200D` (ZWJ). ZWNJ and ZWJ are neither terminal controls nor bidi
//!   overrides; they are required for ordinary emoji ZWJ sequences, Persian/Arabic
//!   and Indic orthography.

/// Returns `true` for any character that must be escaped in **code, identifier,
/// file-name and diagnostic** contexts before printing to a terminal, browser
/// playground or editor panel:
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

/// Returns `true` for any character that must be escaped in **guest log /
/// natural-language text** contexts: everything in [`is_unsafe_terminal_char`]
/// *except* `U+200C` (ZWNJ) and `U+200D` (ZWJ), which are legitimate
/// orthographic joiners (emoji ZWJ sequences, Persian, Indic scripts) and not
/// terminal controls or bidi overrides.
#[must_use]
pub fn is_unsafe_log_char(c: char) -> bool {
    !matches!(c, '\u{200c}' | '\u{200d}') && is_unsafe_terminal_char(c)
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

fn sanitize_with(s: &str, pred: fn(char) -> bool) -> String {
    if !s.chars().any(pred) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if pred(c) {
            push_escaped(&mut out, c);
        } else {
            out.push(c);
        }
    }
    out
}

fn sanitize_bounded_with(s: &str, max_bytes: usize, pred: fn(char) -> bool) -> String {
    let mut out = String::with_capacity(s.len().min(max_bytes));
    let mut buf = String::new();
    for c in s.chars() {
        buf.clear();
        if pred(c) {
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

/// Escape every unsafe terminal/bidi/invisible character in `s` into a visible
/// ASCII sequence (`\xNN` or `\u{xxxx}`). Safe strings are returned unchanged.
/// Use for identifiers, file names, diagnostic messages and echoed source.
#[must_use]
pub fn sanitize_terminal(s: &str) -> String {
    sanitize_with(s, is_unsafe_terminal_char)
}

/// Like [`sanitize_terminal`], stopping before the output exceeds `max_bytes`
/// without splitting a UTF-8 character or an escape sequence.
#[must_use]
pub fn sanitize_terminal_bounded(s: &str, max_bytes: usize) -> String {
    sanitize_bounded_with(s, max_bytes, is_unsafe_terminal_char)
}

/// Escape every terminal control, bidi control and spoofing invisible character
/// in guest log text `s` while preserving `U+200C` (ZWNJ) and `U+200D` (ZWJ).
#[must_use]
pub fn sanitize_log(s: &str) -> String {
    sanitize_with(s, is_unsafe_log_char)
}

/// Like [`sanitize_log`], stopping before the output exceeds `max_bytes`
/// without splitting a UTF-8 character or an escape sequence.
#[must_use]
pub fn sanitize_log_bounded(s: &str, max_bytes: usize) -> String {
    sanitize_bounded_with(s, max_bytes, is_unsafe_log_char)
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

    #[test]
    fn log_sanitiser_preserves_zwj_zwnj_while_terminal_escapes_them() {
        let emoji = "family \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        let persian = "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}";
        let indic = "\u{0915}\u{094D}\u{200D}\u{0937}";
        for s in [emoji, persian, indic] {
            assert_eq!(sanitize_log(s), s);
            assert_eq!(sanitize_log_bounded(s, 512), s);
            assert_ne!(sanitize_terminal(s), s, "terminal/path sanitiser must still escape ZWJ/ZWNJ");
        }
        // Controls, C1 and bidi overrides are still escaped in log text.
        let hostile = "\x1b[2J\x1b]0;t\x07\u{009b}31m\r\n\u{202e}\u{2066}hi\u{2069}\u{200b}";
        let clean = sanitize_log(hostile);
        assert!(!clean.chars().any(is_unsafe_log_char), "{clean:?}");
        assert!(clean.contains("\\x1b[2J"));
        assert!(clean.contains("\\x9b31m"));
        assert!(clean.contains("\\u{202e}"));
        assert!(clean.contains("\\u{200b}"));
    }
}
