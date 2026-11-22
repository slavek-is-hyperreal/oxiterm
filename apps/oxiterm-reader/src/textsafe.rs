//! Sanitising of untrusted text before it goes into THTML.
//!
//! Three concerns: terminal-injection (control chars, escape sequences, bidi
//! overrides, the renderer's private-use width markers), THTML structure
//! (`& < > " '` entities; attribute values end at the first `"`), and width
//! drift (East-Asian-ambiguous glyphs in clickable labels make hit boxes miss).

use unicode_width::UnicodeWidthChar;

/// True for characters that must never reach a terminal or the THTML parser.
fn is_forbidden(c: char) -> bool {
    let u = c as u32;
    c.is_control()
        || matches!(u,
            0x200B..=0x200F      // zero-width + LRM/RLM
            | 0x2028..=0x202E    // line/para separators, bidi embeddings/overrides
            | 0x2060..=0x206F    // word joiner, invisible operators, bidi isolates
            | 0xFE00..=0xFE0F    // variation selectors (width drift)
            | 0xFEFF             // BOM / ZWNBSP
            | 0xFFF9..=0xFFFB    // interlinear annotation
            | 0xE000..=0xF8FF    // private use (BMP)
            | 0xE0000..=0xE007F  // tags
            | 0xF0000..=0x10FFFF // private use planes (renderer VTM markers live here)
        )
}

/// Removes forbidden characters but keeps spaces (code blocks).
pub fn strip_forbidden(s: &str) -> String {
    s.chars().filter(|&c| c == ' ' || !is_forbidden(c)).collect()
}

/// Collapses whitespace and removes forbidden characters. Never returns control chars.
pub fn clean_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_whitespace() || c == '\u{00A0}' {
            pending_space = true;
            continue;
        }
        if is_forbidden(c) {
            continue;
        }
        if pending_space && !out.is_empty() {
            out.push(' ');
        }
        pending_space = false;
        out.push(c);
    }
    out
}

/// Escapes text for a THTML `<text>` node (the parser decodes exactly these five).
pub fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Makes a value safe for a double-quoted THTML attribute that is NOT entity-decoded
/// (`alt`): drops `" < > &` and forbidden characters, collapses whitespace, truncates.
pub fn attr_text(s: &str, max_cols: usize) -> String {
    let stripped: String = s.chars().filter(|c| !matches!(c, '"' | '<' | '>' | '&' | '\'')).collect();
    truncate_cols(&clean_text(&stripped), max_cols)
}

/// Display width of a string in columns (unambiguous-width model).
pub fn display_width(s: &str) -> usize {
    s.chars().map(|c| c.width().unwrap_or(0)).sum()
}

/// Truncates to at most `max_cols` columns, appending `...` when something was cut.
pub fn truncate_cols(s: &str, max_cols: usize) -> String {
    if display_width(s) <= max_cols {
        return s.to_string();
    }
    let budget = max_cols.saturating_sub(3);
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw > budget {
            break;
        }
        w += cw;
        out.push(c);
    }
    out.push_str("...");
    out
}

/// A label for a clickable element: every character must be unambiguous width 1 or 2.
/// Ambiguous or zero-width characters become `?`/are dropped; the result is truncated.
pub fn clickable_label(s: &str, max_cols: usize) -> String {
    let cleaned = clean_text(s);
    let mut out = String::with_capacity(cleaned.len());
    for c in cleaned.chars() {
        match (c.width(), c.width_cjk()) {
            (Some(a), Some(b)) if a == b && (a == 1 || a == 2) => out.push(c),
            (Some(0), _) | (None, _) => {} // combining marks etc.: drop
            _ => out.push('?'),
        }
    }
    let out = if out.trim().is_empty() { "(untitled)".to_string() } else { out };
    truncate_cols(&out, max_cols)
}

/// Shortens whitespace-free tokens longer than `max_cols` (URLs, hashes): the layout
/// engine cannot break them and they would push the page wider than the viewport.
pub fn shorten_long_tokens(s: &str, max_cols: usize) -> String {
    s.split(' ')
        .map(|t| if display_width(t) > max_cols { truncate_cols(t, max_cols) } else { t.to_string() })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_control_and_escape_sequences() {
        let s = clean_text("a\x1b[31mred\x07\u{202E}evil\u{200B}\u{F0001}b\n\t c");
        assert_eq!(s, "a[31mredevilb c");
        assert!(!s.chars().any(|c| c.is_control()));
    }

    #[test]
    fn escape_covers_the_five_entities() {
        assert_eq!(escape_text(r#"<script>"a" & 'b'</script>"#), "&lt;script&gt;&quot;a&quot; &amp; &apos;b&apos;&lt;/script&gt;");
        assert_eq!(escape_text("<!-- x -->"), "&lt;!-- x --&gt;");
        assert_eq!(escape_text("<style>x</style>"), "&lt;style&gt;x&lt;/style&gt;");
    }

    #[test]
    fn attr_text_breaks_no_quotes() {
        let a = attr_text("x\" event-htmx=\"evil <b> &amp; y", 80);
        assert!(!a.contains('"') && !a.contains('<') && !a.contains('>') && !a.contains('&'));
    }

    #[test]
    fn clickable_label_has_no_ambiguous_width() {
        let l = clickable_label("Next \u{2192} \u{00D7} caf\u{00E9} \u{2500}\u{2502} \u{201C}q\u{201D} 日本", 80);
        for c in l.chars() {
            assert_eq!(c.width(), c.width_cjk(), "{c:?} is ambiguous in {l:?}");
        }
        assert!(l.contains("Next"));
        assert!(l.contains("日本"));
        assert_eq!(clickable_label("   ", 10), "(untitled)");
    }

    #[test]
    fn truncation_respects_columns() {
        assert_eq!(truncate_cols("hello world", 8), "hello...");
        assert_eq!(truncate_cols("hi", 8), "hi");
        assert!(display_width(&truncate_cols("日本語日本語日本語", 10)) <= 10);
    }

    #[test]
    fn long_tokens_are_shortened() {
        let s = shorten_long_tokens(&format!("see {} ok", "x".repeat(200)), 40);
        assert!(s.split(' ').all(|t| display_width(t) <= 40));
    }
}
