use std::collections::{HashMap, HashSet};

pub fn run(n: i32) -> u32 {
    let range = (n * 2) as u32;
    let mut s: u32 = 88172645;
    let mut next = || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s
    };
    let mut m: HashMap<i32, i32> = HashMap::new();
    let mut seen: HashSet<i32> = HashSet::new();
    for i in 0..n {
        let k = (next() % range) as i32;
        m.insert(k, i);
        seen.insert(k);
    }
    let mut h: u32 = 0;
    for _ in 0..n {
        let k = (next() % range) as i32;
        let v = m.get(&k).copied().unwrap_or(-1);
        h = h.wrapping_mul(31).wrapping_add(v as u32);
    }
    for _ in 0..n {
        let k = (next() % range) as i32;
        m.remove(&k);
    }
    h = h.wrapping_mul(31).wrapping_add(m.len() as u32);
    h = h.wrapping_mul(31).wrapping_add(seen.len() as u32);
    h
}
