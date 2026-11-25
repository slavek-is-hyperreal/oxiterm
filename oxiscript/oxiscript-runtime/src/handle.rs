//! Generational handle table: opaque `u32` ids a guest can hold but not forge
//! into access to anything it was not given.
//!
//! Layout of a handle: bits 0..16 = slot index + 1 (so 0 is never valid),
//! bits 16..31 = generation (bit 31 is always 0 so a handle is a positive
//! `i32`). Freeing a slot bumps its generation, so a stale handle fails.
//! The generation wraps after 32768 reuses of one slot; every slot starts at
//! a caller-supplied `seed` so handles of different instances differ.

const GEN_MASK: u32 = 0x7FFF;
/// Largest number of simultaneously live entries in one table.
pub const MAX_HANDLES: usize = 0xFFFF;

struct Entry<T> {
    generation: u32,
    value: Option<T>,
}

pub struct HandleTable<T> {
    entries: Vec<Entry<T>>,
    free: Vec<u32>,
    live: usize,
    cap: usize,
    seed: u32,
}

impl<T> HandleTable<T> {
    /// `cap` is the maximum number of live entries (clamped to [`MAX_HANDLES`]).
    pub fn new(cap: usize, seed: u32) -> Self {
        HandleTable {
            entries: Vec::new(),
            free: Vec::new(),
            live: 0,
            cap: cap.min(MAX_HANDLES),
            seed: seed & GEN_MASK,
        }
    }

    pub fn len(&self) -> usize {
        self.live
    }

    pub fn is_empty(&self) -> bool {
        self.live == 0
    }

    fn encode(index: usize, generation: u32) -> u32 {
        ((generation & GEN_MASK) << 16) | (index as u32 + 1)
    }

    fn decode(h: u32) -> Option<(usize, u32)> {
        if h >> 31 != 0 {
            return None;
        }
        let idx = h & 0xFFFF;
        if idx == 0 {
            return None;
        }
        Some((idx as usize - 1, (h >> 16) & GEN_MASK))
    }

    /// Insert; `None` when the table is full.
    pub fn insert(&mut self, value: T) -> Option<u32> {
        if self.live >= self.cap {
            return None;
        }
        let index = if let Some(i) = self.free.pop() {
            i as usize
        } else {
            self.entries.push(Entry {
                generation: self.seed,
                value: None,
            });
            self.entries.len() - 1
        };
        let e = &mut self.entries[index];
        e.value = Some(value);
        self.live += 1;
        Some(Self::encode(index, e.generation))
    }

    pub fn get(&self, h: u32) -> Option<&T> {
        let (i, g) = Self::decode(h)?;
        let e = self.entries.get(i)?;
        if e.generation != g {
            return None;
        }
        e.value.as_ref()
    }

    pub fn get_mut(&mut self, h: u32) -> Option<&mut T> {
        let (i, g) = Self::decode(h)?;
        let e = self.entries.get_mut(i)?;
        if e.generation != g {
            return None;
        }
        e.value.as_mut()
    }

    pub fn remove(&mut self, h: u32) -> Option<T> {
        let (i, g) = Self::decode(h)?;
        let e = self.entries.get_mut(i)?;
        if e.generation != g {
            return None;
        }
        let v = e.value.take()?;
        e.generation = (e.generation + 1) & GEN_MASK;
        self.free.push(i as u32);
        self.live -= 1;
        Some(v)
    }

    pub fn iter(&self) -> impl Iterator<Item = (u32, &T)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| e.value.as_ref().map(|v| (Self::encode(i, e.generation), v)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_and_forged() {
        let mut t = HandleTable::new(4, 7);
        let a = t.insert("a").unwrap();
        assert_eq!(t.get(a), Some(&"a"));
        assert_eq!(t.remove(a), Some("a"));
        assert_eq!(t.get(a), None);
        let b = t.insert("b").unwrap();
        assert_ne!(a, b);
        assert_eq!(t.get(a), None);
        assert_eq!(t.get(0), None);
        assert_eq!(t.get(u32::MAX), None);
        assert_eq!(t.get(0x8000_0001), None);
        assert_eq!(t.get(b ^ 0x10000), None);
    }

    #[test]
    fn capacity() {
        let mut t = HandleTable::new(2, 0);
        assert!(t.insert(1).is_some());
        assert!(t.insert(2).is_some());
        assert!(t.insert(3).is_none());
    }
}
