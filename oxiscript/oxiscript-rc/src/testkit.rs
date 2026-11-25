//! Test support (native only): the `Guest` abstraction that lets the same
//! tests drive the runtime natively (`SimGuest`) and as real wasm (wasmi, in
//! `tests/`), a standard type table, and the reference-model fuzzer.

use crate::rt::Rt;
use crate::sim::SimHeap;
use std::collections::HashMap;

/// What a test needs from a runtime instance. All addresses are guest u32.
pub trait Guest {
    fn init(&mut self, base: u32, count: u32);
    fn alloc(&mut self, id: u32) -> u32;
    fn retain(&mut self, p: u32);
    fn release(&mut self, p: u32);
    fn clone_obj(&mut self, p: u32) -> u32;
    fn is_unique(&mut self, p: u32) -> bool;
    fn collect(&mut self) -> u32;
    fn gc_step(&mut self, budget: u32) -> u32;
    fn stat(&mut self, key: u32) -> u32;
    fn config(&mut self, key: u32, v: u32);
    fn live_objects(&mut self) -> u32;
    fn live_bytes(&mut self) -> u32;
    fn rd(&mut self, a: u32) -> u32;
    fn wr(&mut self, a: u32, v: u32);
    fn alloc_bytes(&mut self, n: u32) -> u32;
    fn free_bytes(&mut self, p: u32, n: u32);
    fn array_new(&mut self, id: u32, len: u32) -> u32;
    fn array_grow(&mut self, a: u32, cap: u32);
    fn str_new(&mut self, s: &str) -> u32;
    fn str_read(&mut self, p: u32) -> String;
    fn rd_bytes(&mut self, a: u32, n: u32) -> Vec<u8>;
    fn wr_bytes(&mut self, a: u32, b: &[u8]);
}

/// Native guest: `Rt` over the checked simulated heap.
pub struct SimGuest {
    pub rt: Rt<SimHeap>,
}

impl SimGuest {
    pub fn new(arena: usize, checks: bool) -> SimGuest {
        SimGuest { rt: Rt::new(SimHeap::new(arena, checks)) }
    }
    /// Standard guest with the standard type table installed.
    pub fn standard() -> SimGuest {
        let mut g = SimGuest::new(256 << 20, true);
        install_types(&mut g, &standard_types());
        g
    }
}

impl Guest for SimGuest {
    fn init(&mut self, base: u32, count: u32) {
        self.rt.init(base, count);
    }
    fn alloc(&mut self, id: u32) -> u32 {
        self.rt.alloc(id)
    }
    fn retain(&mut self, p: u32) {
        self.rt.retain(p);
    }
    fn release(&mut self, p: u32) {
        self.rt.release(p);
    }
    fn clone_obj(&mut self, p: u32) -> u32 {
        self.rt.clone_obj(p)
    }
    fn is_unique(&mut self, p: u32) -> bool {
        self.rt.is_unique(p) == 1
    }
    fn collect(&mut self) -> u32 {
        self.rt.collect_cycles()
    }
    fn gc_step(&mut self, budget: u32) -> u32 {
        self.rt.gc_step(budget)
    }
    fn stat(&mut self, key: u32) -> u32 {
        self.rt.stat(key)
    }
    fn config(&mut self, key: u32, v: u32) {
        self.rt.config(key, v);
    }
    fn live_objects(&mut self) -> u32 {
        self.rt.live_objects()
    }
    fn live_bytes(&mut self) -> u32 {
        self.rt.live_bytes()
    }
    fn rd(&mut self, a: u32) -> u32 {
        use crate::heap::Heap;
        self.rt.heap.ld32(a)
    }
    fn wr(&mut self, a: u32, v: u32) {
        use crate::heap::Heap;
        self.rt.heap.st32(a, v);
    }
    fn alloc_bytes(&mut self, n: u32) -> u32 {
        self.rt.alloc_bytes(n)
    }
    fn free_bytes(&mut self, p: u32, n: u32) {
        self.rt.free_bytes(p, n);
    }
    fn array_new(&mut self, id: u32, len: u32) -> u32 {
        self.rt.array_new(id, len)
    }
    fn array_grow(&mut self, a: u32, cap: u32) {
        self.rt.array_grow(a, cap);
    }
    fn str_new(&mut self, s: &str) -> u32 {
        use crate::heap::Heap;
        let tmp = self.rt.alloc_bytes(s.len().max(1) as u32);
        self.rt.heap.bytes_mut(tmp, s.len() as u32).copy_from_slice(s.as_bytes());
        let r = self.rt.str_new(tmp, s.len() as u32);
        self.rt.free_bytes(tmp, s.len().max(1) as u32);
        r
    }
    fn str_read(&mut self, p: u32) -> String {
        use crate::heap::Heap;
        let len = self.rt.heap.ld32(p);
        String::from_utf8(self.rt.heap.bytes(p + 8, len).to_vec()).unwrap()
    }
    fn rd_bytes(&mut self, a: u32, n: u32) -> Vec<u8> {
        use crate::heap::Heap;
        self.rt.heap.bytes(a, n).to_vec()
    }
    fn wr_bytes(&mut self, a: u32, b: &[u8]) {
        use crate::heap::Heap;
        self.rt.heap.bytes_mut(a, b.len() as u32).copy_from_slice(b);
    }
}

// ---- type table ----------------------------------------------------------

/// One descriptor to install. Index in the slice = type id.
#[derive(Clone, Debug, Default)]
pub struct TypeSpec {
    pub kind: u32,
    /// Payload size (records) or element size (arrays).
    pub size: u32,
    pub acyclic: bool,
    pub ptrs: Vec<u32>,
}

/// Ids of the standard test types.
pub mod ty {
    pub const STRING: u32 = 1;
    /// struct { a: ref, b: ref, val: u32 } (payload 16)
    pub const NODE: u32 = 16;
    /// acyclic struct { val: u32 } (payload 8)
    pub const LEAF: u32 = 17;
    /// acyclic struct { x: ref to acyclic } (payload 8)
    pub const GREENBOX: u32 = 18;
    /// closure { fn, res, cap0: ref, cap1: ref } (payload 16)
    pub const CLOSURE: u32 = 19;
    /// union variant { tag, v: ref } (payload 8)
    pub const VARIANT: u32 = 20;
    /// array of refs (elem 4)
    pub const REFARRAY: u32 = 21;
    /// array of inline {u32, ref} (elem 8)
    pub const INLINEARRAY: u32 = 22;
    /// acyclic array of u32
    pub const SCALARARRAY: u32 = 23;
    /// struct with 8 ref slots (payload 32)
    pub const WIDE: u32 = 24;
}

pub fn standard_types() -> Vec<TypeSpec> {
    let mut v = vec![TypeSpec::default(); 16];
    v[1] = TypeSpec { kind: 4, size: 0, acyclic: true, ptrs: vec![] };
    let rec = |kind, size, acyclic, ptrs: &[u32]| TypeSpec { kind, size, acyclic, ptrs: ptrs.to_vec() };
    v.push(rec(1, 16, false, &[0, 4])); // 16 NODE
    v.push(rec(1, 8, true, &[])); // 17 LEAF
    v.push(rec(1, 8, true, &[0])); // 18 GREENBOX
    v.push(rec(3, 16, false, &[8, 12])); // 19 CLOSURE
    v.push(rec(2, 8, false, &[4])); // 20 VARIANT
    v.push(rec(5, 4, false, &[0])); // 21 REFARRAY
    v.push(rec(5, 8, false, &[4])); // 22 INLINEARRAY
    v.push(rec(5, 4, true, &[])); // 23 SCALARARRAY
    v.push(rec(1, 32, false, &[0, 4, 8, 12, 16, 20, 24, 28])); // 24 WIDE
    v
}

/// Write the table into guest memory (via `alloc_bytes`) and `init` the runtime.
pub fn install_types<G: Guest>(g: &mut G, specs: &[TypeSpec]) {
    let base = g.alloc_bytes(specs.len() as u32 * 16);
    for (id, s) in specs.iter().enumerate() {
        let a = base + id as u32 * 16;
        let pmap = if s.ptrs.is_empty() { 0 } else { g.alloc_bytes(s.ptrs.len() as u32 * 4) };
        for (i, off) in s.ptrs.iter().enumerate() {
            g.wr(pmap + 4 * i as u32, *off);
        }
        if s.kind != 0 {
            g.wr(a, s.size);
            g.wr(a + 4, s.kind | (u32::from(s.acyclic) << 8) | ((s.ptrs.len() as u32) << 16));
            g.wr(a + 8, pmap);
        }
    }
    g.init(base, specs.len() as u32);
}

// ---- small helpers used by many tests -----------------------------------

pub fn ptr_slot_addrs<G: Guest>(g: &mut G, p: u32) -> Vec<u32> {
    let id = g.rd(p - 4) & 0x00FF_FFFF;
    match id {
        ty::NODE | ty::WIDE | ty::CLOSURE | ty::GREENBOX | ty::VARIANT => {
            let offs: &[u32] = match id {
                ty::NODE => &[0, 4],
                ty::WIDE => &[0, 4, 8, 12, 16, 20, 24, 28],
                ty::CLOSURE => &[8, 12],
                ty::GREENBOX => &[0],
                _ => &[4],
            };
            offs.iter().map(|o| p + o).collect()
        }
        ty::REFARRAY | ty::INLINEARRAY => {
            let len = g.rd(p);
            let data = g.rd(p + 8);
            let (esz, off) = if id == ty::REFARRAY { (4, 0) } else { (8, 4) };
            (0..len).map(|i| data + i * esz + off).collect()
        }
        _ => vec![],
    }
}

/// Tiny deterministic PRNG (xorshift64*).
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    pub fn chance(&mut self, pct: u64) -> bool {
        self.next() % 100 < pct
    }
}

// ---- reference-model fuzzer ----------------------------------------------

struct MObj {
    rt: u32,
    ty: u32,
    slots: Vec<Option<usize>>,
    immortal: bool,
    str_len: u32,
}

#[derive(Default, Debug, Clone)]
pub struct FuzzStats {
    pub ops: u64,
    pub collects: u64,
    pub steps: u64,
    pub max_live: u64,
    pub cycle_freed: u64,
    pub aborts: u64,
}

struct Model {
    objs: Vec<MObj>,
    roots: Vec<usize>,
}

fn is_green_ty(t: u32) -> bool {
    matches!(t, ty::STRING | ty::LEAF | ty::SCALARARRAY)
}

impl Model {
    fn reachable(&self) -> Vec<usize> {
        let mut seen = vec![false; self.objs.len()];
        let mut stack: Vec<usize> = self.roots.clone();
        for (i, o) in self.objs.iter().enumerate() {
            if o.immortal {
                stack.push(i);
            }
        }
        let mut out = vec![];
        while let Some(n) = stack.pop() {
            if seen[n] {
                continue;
            }
            seen[n] = true;
            out.push(n);
            for c in self.objs[n].slots.iter().flatten() {
                stack.push(*c);
            }
        }
        out
    }
    fn walk(&self, rng: &mut Rng) -> Option<usize> {
        if self.roots.is_empty() {
            return None;
        }
        let start = self.roots[rng.below(self.roots.len())];
        Some(self.walk_from(start, rng))
    }
    fn walk_from(&self, start: usize, rng: &mut Rng) -> usize {
        let mut cur = start;
        for _ in 0..rng.below(4) {
            let kids: Vec<usize> = self.objs[cur].slots.iter().flatten().copied().collect();
            if kids.is_empty() {
                break;
            }
            cur = kids[rng.below(kids.len())];
        }
        cur
    }
}

fn slot_addr<G: Guest>(g: &mut G, o: &MObj, i: usize) -> u32 {
    match o.ty {
        ty::NODE | ty::WIDE => o.rt + 4 * i as u32,
        ty::CLOSURE => o.rt + 8 + 4 * i as u32,
        ty::GREENBOX => o.rt,
        ty::VARIANT => o.rt + 4,
        ty::REFARRAY => g.rd(o.rt + 8) + 4 * i as u32,
        ty::INLINEARRAY => g.rd(o.rt + 8) + 8 * i as u32 + 4,
        _ => unreachable!(),
    }
}

fn expected_bytes<G: Guest>(g: &mut G, o: &MObj) -> u64 {
    let al = |x: u64| (x + 7) & !7;
    match o.ty {
        ty::STRING => al(8 + 8 + u64::from(o.str_len) + 1),
        ty::NODE | ty::CLOSURE | ty::WIDE => 8 + match o.ty { ty::WIDE => 32, _ => 16 },
        ty::LEAF | ty::GREENBOX | ty::VARIANT => 16,
        ty::REFARRAY | ty::INLINEARRAY | ty::SCALARARRAY => {
            let cap = u64::from(g.rd(o.rt + 4));
            let esz = match o.ty { ty::INLINEARRAY => 8, _ => 4 };
            24 + al(cap * esz)
        }
        _ => unreachable!(),
    }
}

/// Run one randomized program of `ops` operations against `g` (which must
/// have the standard types installed and an empty heap) and check it against a
/// reference model: nothing reachable is ever lost, after `collect` the live
/// set and every reference count equal the model's exactly, and at the end
/// (all roots dropped) nothing but immortal data remains.
pub fn fuzz<G: Guest>(g: &mut G, seed: u64, ops: usize) -> FuzzStats {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut m = Model { objs: vec![], roots: vec![] };
    let mut st = FuzzStats::default();
    let base_bytes = g.live_bytes();
    let base_objs = g.live_objects();
    let all = [
        ty::NODE, ty::NODE, ty::NODE, ty::WIDE, ty::LEAF, ty::GREENBOX, ty::CLOSURE, ty::VARIANT,
        ty::REFARRAY, ty::INLINEARRAY, ty::SCALARARRAY, ty::STRING,
    ];
    for step in 0..ops {
        st.ops += 1;
        let r = rng.below(100);
        match r {
            0..=29 => {
                let t = all[rng.below(all.len())];
                let (p, slots, sl) = match t {
                    ty::STRING => {
                        let s: String = (0..rng.below(12)).map(|_| if rng.chance(10) { 'é' } else { 'a' }).collect();
                        (g.str_new(&s), 0, s.len() as u32)
                    }
                    ty::REFARRAY | ty::INLINEARRAY | ty::SCALARARRAY => {
                        let n = rng.below(5) as u32;
                        let p = g.array_new(t, n);
                        (p, if t == ty::SCALARARRAY { 0 } else { n as usize }, 0)
                    }
                    ty::NODE => (g.alloc(t), 2, 0),
                    ty::WIDE => (g.alloc(t), 8, 0),
                    ty::CLOSURE => (g.alloc(t), 2, 0),
                    ty::GREENBOX | ty::VARIANT => (g.alloc(t), 1, 0),
                    _ => (g.alloc(t), 0, 0),
                };
                m.objs.push(MObj { rt: p, ty: t, slots: vec![None; slots], immortal: false, str_len: sl });
                m.roots.push(m.objs.len() - 1);
            }
            30..=40 => {
                if let Some(&o) = m.roots.get(rng.below(m.roots.len().max(1))) {
                    g.retain(m.objs[o].rt);
                    m.roots.push(o);
                }
            }
            41..=58 => {
                if !m.roots.is_empty() {
                    let i = rng.below(m.roots.len());
                    let o = m.roots.swap_remove(i);
                    g.release(m.objs[o].rt);
                }
            }
            59..=83 => {
                let Some(src) = m.walk(&mut rng) else { continue };
                // a third of the links point back into the source's own subgraph: cycles
                let dst = if rng.chance(10) {
                    None
                } else if rng.chance(35) {
                    Some(m.walk_from(src, &mut rng))
                } else {
                    m.walk(&mut rng)
                };
                if m.objs[src].slots.is_empty() {
                    continue;
                }
                if let Some(d) = dst {
                    if m.objs[src].ty == ty::GREENBOX && !is_green_ty(m.objs[d].ty) {
                        continue;
                    }
                }
                let i = rng.below(m.objs[src].slots.len());
                let a = slot_addr(g, &m.objs[src], i);
                let old = g.rd(a);
                if let Some(d) = dst {
                    g.retain(m.objs[d].rt);
                }
                g.wr(a, dst.map_or(0, |d| m.objs[d].rt));
                g.release(old);
                m.objs[src].slots[i] = dst;
            }
            84..=86 => {
                // clone a root
                if let Some(&o) = m.roots.get(rng.below(m.roots.len().max(1))) {
                    let src = &m.objs[o];
                    let p = g.clone_obj(src.rt);
                    let slots = src.slots.clone();
                    let (t, sl) = (src.ty, src.str_len);
                    m.objs.push(MObj { rt: p, ty: t, slots, immortal: false, str_len: sl });
                    m.roots.push(m.objs.len() - 1);
                }
            }
            87..=88 => {
                // push onto a ref array
                let Some(a) = m.walk(&mut rng) else { continue };
                let t = m.objs[a].ty;
                if t != ty::REFARRAY && t != ty::INLINEARRAY || m.objs[a].slots.len() >= 10 {
                    continue;
                }
                let dst = if rng.chance(15) { None } else { m.walk(&mut rng) };
                let p = m.objs[a].rt;
                let len = g.rd(p);
                assert_eq!(len, m.objs[a].slots.len() as u32, "array len mismatch");
                g.array_grow(p, len + 1);
                let esz = if t == ty::REFARRAY { 4 } else { 8 };
                let data = g.rd(p + 8);
                let off = if t == ty::REFARRAY { 0 } else { 4 };
                if let Some(d) = dst {
                    g.retain(m.objs[d].rt);
                }
                g.wr(data + len * esz + off, dst.map_or(0, |d| m.objs[d].rt));
                if t == ty::INLINEARRAY {
                    g.wr(data + len * esz, 7);
                }
                g.wr(p, len + 1);
                m.objs[a].slots.push(dst);
            }
            89..=90 => {
                // saturate a root object's count -> sticky -> immortal
                if let Some(&o) = m.roots.get(rng.below(m.roots.len().max(1))) {
                    if !m.objs[o].immortal && rng.chance(30) {
                        g.wr(m.objs[o].rt - 8, 0x7FFF_FFFF);
                        g.retain(m.objs[o].rt);
                        assert!(g.rd(m.objs[o].rt - 8) >= 0x8000_0000, "retain at 2^31-1 did not saturate");
                        m.objs[o].immortal = true;
                    }
                }
            }
            91..=94 => {
                let mut left = (rng.below(40) + 1) as u32;
                let mut guard = 0;
                loop {
                    let s = g.gc_step(left);
                    st.steps += 1;
                    guard += 1;
                    if s & 1 == 0 || guard > 8 {
                        break;
                    }
                    left = (rng.below(400) + 1) as u32;
                }
            }
            95..=97 => {
                let d = rng.below(4) as u32;
                g.config(0, [1, 3, 64, 256][d as usize]);
            }
            _ => {
                st.collects += 1;
                check_full(g, &m, base_bytes, base_objs, &mut st);
            }
        }
        if step % 8 == 0 {
            check_light(g, &m);
        }
    }
    // tear down
    while let Some(o) = m.roots.pop() {
        g.release(m.objs[o].rt);
    }
    check_full(g, &m, base_bytes, base_objs, &mut st);
    st.aborts = u64::from(g.stat(6));
    st
}

fn check_light<G: Guest>(g: &mut G, m: &Model) {
    for n in m.reachable() {
        let o = &m.objs[n];
        for (i, c) in o.slots.iter().enumerate() {
            let a = slot_addr(g, o, i);
            let got = g.rd(a);
            let want = c.map_or(0, |c| m.objs[c].rt);
            assert_eq!(got, want, "slot {i} of reachable object {:#x} (ty {})", o.rt, o.ty);
        }
        let meta = g.rd(o.rt - 4);
        assert_eq!(meta & 0x00FF_FFFF, o.ty, "type id of live object clobbered");
        let col = (meta >> 24) & 7;
        assert!(col <= 4, "live object has internal colour {col}");
        assert!(g.rd(o.rt - 8) >= 1, "reachable object has rc 0");
    }
}

fn check_full<G: Guest>(g: &mut G, m: &Model, base_bytes: u32, base_objs: u32, st: &mut FuzzStats) {
    let freed = g.collect();
    st.cycle_freed += u64::from(freed);
    assert_eq!(g.stat(0), 0, "destruction queue not empty after collect");
    assert_eq!(g.stat(1), 0, "candidates left after collect");
    check_light(g, m);
    let reach = m.reachable();
    st.max_live = st.max_live.max(reach.len() as u64);
    // exact rc
    let mut want: HashMap<usize, u32> = HashMap::new();
    for &r in &m.roots {
        *want.entry(r).or_default() += 1;
    }
    for &n in &reach {
        for c in m.objs[n].slots.iter().flatten() {
            *want.entry(*c).or_default() += 1;
        }
    }
    let mut bytes = 0u64;
    for &n in &reach {
        let o = &m.objs[n];
        let rc = g.rd(o.rt - 8);
        if o.immortal {
            assert!(rc >= 0x8000_0000);
            assert!(!g.is_unique(o.rt));
        } else {
            let w = want.get(&n).copied().unwrap_or(0);
            assert_eq!(rc, w, "rc of object {:#x} (ty {}) after collect", o.rt, o.ty);
            assert_eq!(g.is_unique(o.rt), w == 1);
        }
        bytes += expected_bytes(g, o);
    }
    assert_eq!(u64::from(g.live_objects() - base_objs), reach.len() as u64, "live set != reachable set (leak or premature free)");
    assert_eq!(u64::from(g.live_bytes() - base_bytes), bytes, "live_bytes mismatch");
}
