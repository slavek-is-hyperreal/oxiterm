//! Proportional text (Phase 6). `bundle` defines the GUI reference cell (6.1); `layout` is the
//! shaped text layout that `LayoutEngine` measures with (6.3).
use std::borrow::Cow;

/// The text of a `wrap:word` node as the mono path paints it: every hard line (`str::lines`)
/// is its words (`split_whitespace`) joined by single spaces, so runs of whitespace collapse and
/// leading and trailing whitespace goes; empty and blank lines stay as empty lines. A
/// `wrap:none` node keeps its text verbatim. The shaped path applies this before shaping, so
/// the layout it measures is the text the cell painter would have drawn. Borrows when the text
/// is already in that form (the common case).
pub fn collapse_word_whitespace(text: &str) -> Cow<'_, str> {
    let canonical = if text.is_ascii() {
        // Byte scan: only single ' ' between words, none at a line's ends, no other whitespace.
        let mut prev_space = true; // a leading space is not canonical
        let mut line_len = 0usize;
        let mut ok = !text.ends_with('\n');
        for &c in text.as_bytes() {
            match c {
                b' ' if prev_space => ok = false,
                b' ' => prev_space = true,
                b'\n' => {
                    ok &= line_len == 0 || !prev_space;
                    prev_space = true;
                    line_len = 0;
                    continue;
                }
                b'\t' | b'\r' | 0x0b | 0x0c => ok = false,
                _ => prev_space = false,
            }
            line_len += 1;
            if !ok {
                break;
            }
        }
        ok && (line_len == 0 || !prev_space)
    } else {
        text.lines().all(|l| {
            let mut prev_space = true;
            for c in l.chars() {
                let sp = c.is_whitespace();
                if sp && (prev_space || c != ' ') {
                    return false;
                }
                prev_space = sp;
            }
            l.is_empty() || !prev_space
        }) && !text.contains('\r') && !text.ends_with('\n')
    };
    if canonical {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        for (j, w) in line.split_whitespace().enumerate() {
            if j > 0 {
                out.push(' ');
            }
            out.push_str(w);
        }
    }
    Cow::Owned(out)
}

#[cfg(feature = "text-shaping")]
pub mod bundle;
#[cfg(feature = "text-shaping")]
pub mod layout;
#[cfg(feature = "text-shaping")]
pub mod shaping;

/// Without `text-shaping` there is nothing to shape with: every cache lookup misses and layout
/// keeps measuring in whole cells.
#[cfg(not(feature = "text-shaping"))]
pub mod layout {
    use crate::font::ResolvedFont;
    use std::sync::Arc;

    /// Placeholder; never constructed.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TextLayout;

    impl TextLayout {
        pub fn min_content_cells(&self) -> f32 {
            0.0
        }
        pub fn max_content_cells(&self) -> f32 {
            0.0
        }
        pub fn height_cells(&self) -> f32 {
            0.0
        }
        pub fn line_count(&self) -> usize {
            0
        }
    }

    /// Placeholder cache.
    #[derive(Default)]
    pub struct TextCache;

    impl TextCache {
        pub fn begin_frame(&mut self) {}
        pub fn base(&mut self, _text: &str, _font: &ResolvedFont) -> Option<Arc<TextLayout>> {
            None
        }
        pub fn at_width(&mut self, base: &Arc<TextLayout>, _width_cells: f32) -> Arc<TextLayout> {
            base.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_matches_the_mono_wrap_path() {
        for (src, want) in [
            ("a b", "a b"),
            ("a  b", "a b"),
            ("  lead and   trail  ", "lead and trail"),
            ("tab\tseparated", "tab separated"),
            ("a\n\nb", "a\n\nb"),
            ("a  b\n   \n c", "a b\n\nc"),
            ("trailing\n", "trailing"),
            ("", ""),
            ("   ", ""),
        ] {
            assert_eq!(collapse_word_whitespace(src), want, "{src:?}");
            // what the mono builder emits per line
            let mono: Vec<String> = src.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
            assert_eq!(collapse_word_whitespace(src), mono.join("\n"), "{src:?}");
        }
        assert!(matches!(collapse_word_whitespace("already fine\nok"), Cow::Borrowed(_)));
        // exhaustive over a small alphabet (ASCII fast path and the Unicode path alike)
        let alpha = ['a', ' ', '\n', '\t', '\r', '\u{e9}', '\u{a0}'];
        let mut idx = vec![0usize; 5];
        loop {
            for len in 0..=5 {
                let src: String = idx[..len].iter().map(|&i| alpha[i]).collect();
                let want: Vec<String> = src.lines().map(|l| l.split_whitespace().collect::<Vec<_>>().join(" ")).collect();
                let got = collapse_word_whitespace(&src);
                assert_eq!(got, want.join("\n"), "{src:?}");
                assert_eq!(matches!(got, Cow::Borrowed(_)), got == src, "{src:?}: borrowed iff unchanged");
            }
            let mut k = 0;
            while k < idx.len() {
                idx[k] += 1;
                if idx[k] < alpha.len() {
                    break;
                }
                idx[k] = 0;
                k += 1;
            }
            if k == idx.len() {
                break;
            }
        }
    }
}
