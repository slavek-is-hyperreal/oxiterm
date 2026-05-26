//! Unicode display-width caching and virtual terminal modifier sequences.
//!
//! Provides a thread-safe cache to determine cell widths of characters,
//! and handles injection of PUA-B Virtual Terminal Modifiers (VTM) for terminal layout synchronization.

use unicode_width::UnicodeWidthChar;
use std::collections::HashMap;
use parking_lot::RwLock;
use std::sync::OnceLock;

/// Cache keeping track of visual column width of Unicode characters.
pub struct UnicodeWidthCache {
    cache: RwLock<HashMap<char, u8>>,
}

static CACHE: OnceLock<UnicodeWidthCache> = OnceLock::new();

impl UnicodeWidthCache {
    /// Returns the global thread-safe singleton instance of the width cache.
    pub fn get() -> &'static Self {
        CACHE.get_or_init(|| Self {
            cache: RwLock::new(HashMap::new()),
        })
    }

    /// Fetches the visual column width of the given character, computing and caching it if not found.
    pub fn width(&self, ch: char) -> u8 {
        {
            let read = self.cache.read();
            if let Some(&w) = read.get(&ch) {
                return w;
            }
        }

        let w = u8::try_from(ch.width().unwrap_or(0)).unwrap_or(0);
        let mut write = self.cache.write();
        write.insert(ch, w);
        w
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
    fn test_vtm_modifier() {
        let mut s = String::new();
        insert_vtm_modifier(&mut s, 2);
        assert_eq!(s.chars().count(), 1);
        let ch = s.chars().next().unwrap();
        assert_eq!(ch as u32, 0xD0002);
    }
}
