use unicode_width::UnicodeWidthChar;
use std::collections::HashMap;
use parking_lot::RwLock;
use std::sync::OnceLock;

pub struct UnicodeWidthCache {
    cache: RwLock<HashMap<char, u8>>,
}

static CACHE: OnceLock<UnicodeWidthCache> = OnceLock::new();

impl UnicodeWidthCache {
    pub fn get() -> &'static Self {
        CACHE.get_or_init(|| Self {
            cache: RwLock::new(HashMap::new()),
        })
    }

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

pub fn insert_vtm_modifier(buf: &mut String, cluster_width: u8) {
    // VTM (Virtual Terminal Modifier) PUA range.
    // The auditor requested U+E000–U+F8FF.
    let modifier = std::char::from_u32(0xE000 + u32::from(cluster_width)).unwrap_or('\u{E000}');
    buf.push(modifier);
}
