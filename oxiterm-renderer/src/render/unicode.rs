//! Unicode display-width caching and virtual terminal modifier sequences.
//!
//! Provides a thread-safe cache to determine cell widths of characters,
//! and handles injection of PUA-B Virtual Terminal Modifiers (VTM) for terminal layout synchronization.

use std::sync::atomic::{AtomicU8, Ordering};
use unicode_width::UnicodeWidthChar;

/// The one definition of a character's column width (control chars and
/// zero-width marks are 0). Every fast path below is derived from it.
fn compute_width(ch: char) -> u8 {
    u8::try_from(ch.width().unwrap_or(0)).unwrap_or(0)
}

/// Number of Basic Multilingual Plane code points held in the lazy table.
const BMP: usize = 0x1_0000;

/// Lock-free lazy table for the BMP: `0` means "not computed yet", otherwise the
/// stored value is `width + 1`. Zero-initialised, so it lives in .bss. Writes are
/// idempotent, so `Relaxed` ordering is enough.
static BMP_WIDTHS: [AtomicU8; BMP] = [const { AtomicU8::new(0) }; BMP];

/// Lock-free cache of the visual column width of Unicode characters.
pub struct UnicodeWidthCache {
    _private: (),
}

static CACHE: UnicodeWidthCache = UnicodeWidthCache { _private: () };

impl UnicodeWidthCache {
    /// Returns the global thread-safe singleton instance of the width cache.
    pub fn get() -> &'static Self {
        &CACHE
    }

    /// Fetches the visual column width of the given character, computing and caching it if not found.
    #[inline]
    pub fn width(&self, ch: char) -> u8 {
        let cp = ch as usize;
        if let Some(slot) = BMP_WIDTHS.get(cp) {
            let v = slot.load(Ordering::Relaxed);
            if v != 0 {
                return v - 1;
            }
            let w = compute_width(ch);
            slot.store(w + 1, Ordering::Relaxed);
            w
        } else {
            compute_width(ch)
        }
    }
}

/// Returns `true` for East-Asian *Ambiguous*-width characters — glyphs that render
/// as 1 cell on most terminals but 2 cells on CJK/ambiguous-wide terminals (e.g. the
/// arrows `←`/`→`, many symbols). Detected by `unicode-width` disagreeing with itself
/// between its default and CJK width tables.
///
/// These are unsafe inside a clickable label: the visible text drifts out from under
/// its hit box on terminals that render them wide. Unconditionally wide glyphs (like
/// emoji) are NOT ambiguous — both tables agree — so they are safe.
pub fn is_ambiguous_width(ch: char) -> bool {
    use unicode_width::UnicodeWidthChar;
    match (ch.width(), ch.width_cjk()) {
        (Some(a), Some(b)) => a != b,
        _ => false,
    }
}

/// Appends a Virtual Terminal Modifier (VTM) character sequence to the buffer.
///
/// Modifiers utilize the Supplementary Private Use Area-B range (U+D0000 - U+D08F6)
/// to explicitly declare display widths of subsequent character runs.
pub fn insert_vtm_modifier(buf: &mut String, cluster_width: u8) {
    let modifier = std::char::from_u32(0xD0000 + u32::from(cluster_width)).unwrap_or('\u{D0000}');
    buf.push(modifier);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unicode_width_cache() {
        let cache = UnicodeWidthCache::get();
        assert_eq!(cache.width('A'), 1);
        assert_eq!(cache.width('🚀'), 2);
        assert_eq!(cache.width('\n'), 0);
    }

    #[test]
    fn width_matches_reference_for_every_codepoint() {
        let cache = UnicodeWidthCache::get();
        // Two passes: the first fills the lazy table, the second reads it back.
        for pass in 0..2 {
            for ch in (0..=0x10FFFFu32).filter_map(char::from_u32) {
                assert_eq!(cache.width(ch), compute_width(ch), "U+{:04X} pass {pass}", ch as u32);
            }
        }
    }

    #[test]
    fn width_known_classes() {
        let cache = UnicodeWidthCache::get();
        for c in (0u8..0x20).map(char::from).chain(['\u{7f}']).chain((0x80u32..0xA0).filter_map(char::from_u32)) {
            assert_eq!(cache.width(c), 0, "control U+{:04X}", c as u32);
        }
        for c in (0x20u8..0x7f).map(char::from) {
            assert_eq!(cache.width(c), 1);
        }
        assert_eq!(cache.width('\u{301}'), 0); // combining
        assert_eq!(cache.width('\u{200d}'), 0); // ZWJ
        assert_eq!(cache.width('\u{2500}'), 1); // box drawing
        assert_eq!(cache.width('\u{4e00}'), 2); // CJK
        assert_eq!(cache.width('\u{1f680}'), 2); // emoji (non-BMP)
        assert_eq!(cache.width('\u{2190}'), 1); // ambiguous
        assert_eq!(cache.width('\u{d7}'), 1); // ambiguous
    }

    #[test]
    fn test_is_ambiguous_width() {
        // Ambiguous: narrow by default, wide in CJK context.
        assert!(is_ambiguous_width('←')); // U+2190
        assert!(is_ambiguous_width('→')); // U+2192
        assert!(is_ambiguous_width('×')); // U+00D7
        // Unambiguous: plain ASCII and unconditionally-wide emoji.
        assert!(!is_ambiguous_width('<'));
        assert!(!is_ambiguous_width('x'));
        assert!(!is_ambiguous_width('B'));
        assert!(!is_ambiguous_width('🚀')); // always width 2
    }

    #[test]
    fn test_vtm_modifier() {
        let mut s = String::new();
        insert_vtm_modifier(&mut s, 2);
        assert_eq!(s.chars().count(), 1);
        let ch = s.chars().next().unwrap();
        assert_eq!(ch as u32, 0xD0002);
    }
}
