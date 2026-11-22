//! Generated-site storage: keyed ids, page files, link-table sidecars, pruning.
//!
//! Layout under the site dir (the app base directory of `oxiterm-cli serve`):
//! `index.thtml`, `cache/<id>.thtml`, `cache/<id>.json` (link table, server-side only),
//! `cache/<hash>.png`, `cache/.salt`.
//!
//! Ids are 128-bit keyed hashes (SipHash over `salt || data`, salt is random and persisted),
//! so collisions cannot be engineered by a page author.

use crate::model::{LinkTable, LinkTarget};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Keyed id generator.
#[derive(Clone)]
pub struct Ids {
    salt: [u8; 16],
}

impl Ids {
    /// Loads (or creates) the persisted salt in `cache_dir/.salt`.
    pub fn load_or_create(cache_dir: &Path) -> std::io::Result<Self> {
        let p = cache_dir.join(".salt");
        if let Ok(s) = std::fs::read_to_string(&p) {
            if let Some(salt) = parse_hex16(s.trim()) {
                return Ok(Ids { salt });
            }
        }
        let ids = Ids::random();
        std::fs::write(&p, ids.salt.iter().map(|b| format!("{b:02x}")).collect::<String>())?;
        Ok(ids)
    }

    /// Fresh random salt (per-process entropy from the std hasher keys + clock).
    pub fn random() -> Self {
        use std::hash::BuildHasher;
        let mut salt = [0u8; 16];
        let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        for (i, chunk) in salt.chunks_mut(8).enumerate() {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            (nanos, i, std::process::id()).hash(&mut h);
            chunk.copy_from_slice(&h.finish().to_le_bytes());
        }
        Ids { salt }
    }

    /// Fixed salt (tests).
    pub fn fixed(seed: u8) -> Self {
        Ids { salt: [seed; 16] }
    }

    /// 32 hex chars, lowercase.
    pub fn hash_bytes(&self, data: &[u8]) -> String {
        let one = |tag: u8| {
            let mut h = DefaultHasher::new();
            self.salt.hash(&mut h);
            tag.hash(&mut h);
            data.len().hash(&mut h);
            h.write(data);
            h.finish()
        };
        format!("{:016x}{:016x}", one(1), one(2))
    }

    /// Page id for a source key (`url:...`, `hn:top`, ...).
    pub fn page_id(&self, key: &str) -> String {
        self.hash_bytes(key.as_bytes())
    }
}

fn parse_hex16(s: &str) -> Option<[u8; 16]> {
    if s.len() != 32 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; 16];
    for i in 0..16 {
        out[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// True for a 32-char lowercase-hex page id.
pub fn is_page_id(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Parses `<pageid>.<n>` (the payload of `open_link:`); `n` is 1-based.
pub fn parse_link_ref(s: &str) -> Option<(&str, usize)> {
    let (id, n) = s.split_once('.')?;
    if !is_page_id(id) || n.is_empty() || n.len() > 3 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: usize = n.parse().ok()?;
    (n >= 1).then_some((id, n))
}

/// Sidecar content.
#[derive(Debug, Serialize, Deserialize)]
struct Sidecar {
    source: Option<String>,
    links: LinkTable,
}

/// Cache directory handle.
pub struct Store {
    /// `<site>/cache`.
    pub cache_dir: PathBuf,
    /// Keyed ids.
    pub ids: Ids,
    mem: Mutex<std::collections::HashMap<String, (Option<String>, LinkTable)>>,
}

impl Store {
    /// Opens/creates `<site_dir>/cache`.
    pub fn open(site_dir: &Path) -> std::io::Result<Self> {
        let cache_dir = site_dir.join("cache");
        std::fs::create_dir_all(&cache_dir)?;
        let ids = Ids::load_or_create(&cache_dir)?;
        Ok(Store { cache_dir, ids, mem: Mutex::new(Default::default()) })
    }

    /// Writes page + sidecar; returns the navigate path relative to the site root.
    pub fn save_page(&self, id: &str, thtml: &str, source: Option<&str>, links: &LinkTable) -> std::io::Result<String> {
        assert!(is_page_id(id));
        let sc = Sidecar { source: source.map(str::to_string), links: links.clone() };
        write_atomic(&self.cache_dir.join(format!("{id}.json")), serde_json::to_string(&sc).unwrap_or_default().as_bytes())?;
        write_atomic(&self.cache_dir.join(format!("{id}.thtml")), thtml.as_bytes())?;
        let mut mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
        if mem.len() > 256 {
            mem.clear();
        }
        mem.insert(id.to_string(), (source.map(str::to_string), links.clone()));
        Ok(nav_path(id))
    }

    /// Writes the mobile variant (`<id>_mobile.thtml`, picked up by the engine's variant resolver).
    pub fn save_mobile(&self, id: &str, thtml: &str) -> std::io::Result<()> {
        assert!(is_page_id(id));
        write_atomic(&self.cache_dir.join(format!("{id}_mobile.thtml")), thtml.as_bytes())
    }

    /// Looks up a link of a page (memory first, then sidecar).
    pub fn link(&self, id: &str, n: usize) -> Option<LinkTarget> {
        self.with_page(id, |_, l| l.get(n).map(|e| e.target.clone())).flatten()
    }

    /// Source URL of a page.
    pub fn source(&self, id: &str) -> Option<String> {
        self.with_page(id, |s, _| s.clone()).flatten()
    }

    fn with_page<R>(&self, id: &str, f: impl FnOnce(&Option<String>, &LinkTable) -> R) -> Option<R> {
        if !is_page_id(id) {
            return None;
        }
        {
            let mem = self.mem.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((s, l)) = mem.get(id) {
                return Some(f(s, l));
            }
        }
        let raw = std::fs::read_to_string(self.cache_dir.join(format!("{id}.json"))).ok()?;
        let mut sc: Sidecar = serde_json::from_str(&raw).ok()?;
        sc.links.reindex();
        let r = f(&sc.source, &sc.links);
        self.mem.lock().unwrap_or_else(|e| e.into_inner()).insert(id.to_string(), (sc.source, sc.links));
        Some(r)
    }

    /// Existing page younger than `ttl`: returns its navigate path.
    pub fn fresh_page(&self, id: &str, ttl: Duration) -> Option<String> {
        let md = std::fs::metadata(self.cache_dir.join(format!("{id}.thtml"))).ok()?;
        let age = SystemTime::now().duration_since(md.modified().ok()?).ok()?;
        (age < ttl && self.cache_dir.join(format!("{id}.json")).exists()).then(|| nav_path(id))
    }

    /// Deletes cache files (except the salt) older than `max_age`. Returns the count.
    pub fn prune(&self, max_age: Duration) -> usize {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(&self.cache_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.file_name().and_then(|f| f.to_str()) == Some(".salt") || !p.is_file() {
                    continue;
                }
                let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|m| SystemTime::now().duration_since(m).ok()).map(|a| a > max_age).unwrap_or(false);
                if old && std::fs::remove_file(&p).is_ok() {
                    n += 1;
                }
            }
        }
        n
    }
}

/// Navigate path relative to the site root (the app base directory).
pub fn nav_path(id: &str) -> String {
    format!("cache/{id}.thtml")
}

/// Writes via a uniquely named temp file + rename, so readers never see partial files and
/// concurrent writers of the same target never share a temp file.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("f");
    let tmp = path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("oxireader-{tag}-{}-{}", std::process::id(), SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn ids_are_keyed_and_stable() {
        let a = Ids::fixed(1);
        let b = Ids::fixed(2);
        assert_eq!(a.page_id("url:x"), a.page_id("url:x"));
        assert_ne!(a.page_id("url:x"), b.page_id("url:x"));
        assert_ne!(a.page_id("url:x"), a.page_id("url:y"));
        assert!(is_page_id(&a.page_id("k")));
        assert!(!a.page_id("k").contains("_mobile"));
    }

    #[test]
    fn link_ref_parsing_is_strict() {
        let id = Ids::fixed(1).page_id("k");
        assert_eq!(parse_link_ref(&format!("{id}.12")), Some((id.as_str(), 12)));
        for bad in ["", ".1", "../../etc.1", &format!("{id}.0"), &format!("{id}.1234"), &format!("{id}.-1"), &format!("{}.1", id.to_uppercase()), &format!("{id}"), "zz.1", &format!("{id}.1.thtml")] {
            assert!(parse_link_ref(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn sidecar_roundtrip_survives_restart_and_prunes() {
        let dir = tmpdir("store");
        let s = Store::open(&dir).unwrap();
        let id = s.ids.page_id("k");
        let mut t = LinkTable::default();
        t.add(LinkTarget::Url("https://e.com/a".into()), "a", None);
        t.add(LinkTarget::HnStory(7), "b", None);
        let nav = s.save_page(&id, "<box></box>", Some("https://e.com/"), &t).unwrap();
        assert_eq!(nav, format!("cache/{id}.thtml"));
        // New Store (no memory cache) reads the sidecar and keeps the same salt.
        let s2 = Store::open(&dir).unwrap();
        assert_eq!(s2.ids.page_id("k"), id);
        assert_eq!(s2.link(&id, 2), Some(LinkTarget::HnStory(7)));
        assert_eq!(s2.link(&id, 3), None);
        assert_eq!(s2.source(&id).as_deref(), Some("https://e.com/"));
        assert_eq!(s2.link("../../x", 1), None);
        assert!(s2.fresh_page(&id, Duration::from_secs(60)).is_some());
        assert!(s2.fresh_page(&id, Duration::from_secs(0)).is_none());
        assert_eq!(s2.prune(Duration::from_secs(0)), 2 + 0, "page + sidecar pruned, salt kept");
        assert!(dir.join("cache/.salt").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
