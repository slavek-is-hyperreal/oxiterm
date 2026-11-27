//! The memory runtime proper: object allocation, reference counting with
//! sticky counts, a recursion-free deferred destruction queue, and a
//! Bacon-Rajan style cycle collector (trial deletion, as in Nim's ORC), all
//! generic over a [`Heap`].
//!
//! # State machine of an object
//!
//! ```text
//! live  --release (rc 1 -> 0)-->  PENDING  (in the destruction queue; the rc word
//!                                           is the queue link, children not yet released)
//! PENDING --drain--> freed, or ZOMBIE if it is still in the candidate buffer
//! ZOMBIE --candidate buffer visits it--> freed
//! ```
//!
//! PENDING (colour 5) and ZOMBIE (colour 6) are runtime-internal colours that
//! only dead objects carry; no live object ever has them and nobody holds a
//! pointer to a dead object, so the ABI-visible colour set is unchanged.
//!
//! # Cycle collector
//!
//! `release` that leaves a non-green object alive turns it purple and appends
//! it to the candidate buffer (`roots`). A *trial* over a range of candidates
//! runs the three Bacon-Rajan phases iteratively (explicit stacks in guest
//! memory, no recursion):
//!
//! 1. `mark_gray`: subtract the references that come from inside the subgraph
//!    (every traced child edge of every reached node is decremented once).
//! 2. `scan`: a gray node whose count is still positive is referenced from
//!    outside: `scan_black` restores the counts under it. Otherwise it is white.
//! 3. `collect_white`: white nodes are garbage. Their edges to traced children
//!    were already subtracted in phase 1, so they only release their *green or
//!    sticky* children (never traced) and are freed.
//!
//! `mark_gray` carries a work limit: if the limit is exceeded the trial is
//! undone exactly (every decrement is re-added) so incremental steps never
//! leave the heap half-processed.

use crate::heap::Heap;

pub const HEADER: u32 = 8;
pub const STICKY: u32 = 0x8000_0000;
pub const META_TYPE_MASK: u32 = 0x00FF_FFFF;
pub const COLOUR_SHIFT: u32 = 24;
pub const COLOUR_MASK: u32 = 0b111 << COLOUR_SHIFT;
pub const BUFFERED: u32 = 1 << 27;
/// Internal transient flag (reserved bit 28) used only inside a cycle-collector
/// trial to avoid pushing the same node onto `stack` more than once. Always
/// cleared before a trial returns (whether committed or undone).
const IN_STACK: u32 = 1 << 28;

/// Pre-reserved inline workspace (lives in `.bss` on wasm32, zero bytes in `.wasm`)
/// so batched cycle collection over small/medium cycles needs zero heap headroom.
// powers of two: indices are masked, which keeps a bounds check out of the trace loops
const INLINE_STACK: usize = 2048;
const INLINE_LOG: usize = 1024;
const COLLECT_BATCH: u32 = 256;

pub const BLACK: u32 = 0;
pub const GRAY: u32 = 1;
pub const WHITE: u32 = 2;
pub const PURPLE: u32 = 3;
pub const GREEN: u32 = 4;
/// Internal: in the destruction queue.
pub const PENDING: u32 = 5;
/// Internal: destroyed but still referenced by the candidate buffer.
pub const ZOMBIE: u32 = 6;

pub const TYPE_STRING: u32 = 1;
pub const K_STRUCT: u32 = 1;
pub const K_VARIANT: u32 = 2;
pub const K_CLOSURE: u32 = 3;
pub const K_STRING: u32 = 4;
pub const K_ARRAY: u32 = 5;
pub const FLAG_ACYCLIC: u32 = 1;
pub const TD_SIZE: u32 = 16;

pub const ARR_LEN: u32 = 0;
pub const ARR_CAP: u32 = 4;
pub const ARR_DATA: u32 = 8;
pub const ARR_OBJ_SIZE: u32 = 24; // object_alloc_size(12)
pub const ARR_MAX_LEN: u32 = 0x7FFF_FFFF;

// TrapKind codes (oxiscript-abi trap.rs)
pub const T_UNREACHABLE: u32 = 1;
pub const T_BOUNDS: u32 = 4;
pub const T_BAD_CAST: u32 = 5;
pub const T_NULL: u32 = 6;
pub const T_OOM: u32 = 7;
pub const T_BAD_UTF8: u32 = 9;
/// Runtime-extension trap codes (outside `oxiscript_abi::trap::TrapKind`'s
/// current range 1..=10; proposed additions for `trap.rs`).
/// `oxi_rt_init` rejected the type descriptor table.
pub const T_BAD_TYPEDESC: u32 = 64;
/// Debug builds: release/retain of a pointer that is not a live object
/// (stale or double release, wild pointer).
pub const T_STALE_OBJECT: u32 = 65;

// stat keys (`oxi_gc_stat`)
pub const STAT_PENDING: u32 = 0;
pub const STAT_CANDIDATES: u32 = 1;
pub const STAT_COLLECTIONS: u32 = 2;
pub const STAT_CYCLE_FREED: u32 = 3;
pub const STAT_INTERNAL_BYTES: u32 = 4;
pub const STAT_GC_WANTED: u32 = 5;
pub const STAT_ABORTS: u32 = 6;
// config keys (`oxi_gc_config`)
pub const CFG_DRAIN_BUDGET: u32 = 0;
pub const CFG_ROOT_THRESHOLD: u32 = 1;

/// Work units of the first step of an allocation-failure collection, and the most one
/// such attempt may spend in total (see `Rt::oom_collect`).
pub const OOM_STEP_UNITS: u32 = 2048;
pub const OOM_COLLECT_UNITS: u32 = 8192;
/// Bytes an allocation-failure collection tries to return at least.
const OOM_MIN_TARGET: u32 = 64 * 1024;

/// `oxi_gc_step` result bits.
pub const STEP_MORE: u32 = 1;
/// Candidates remain that exceeded the step budget: only a full
/// `oxi_collect_cycles` will examine them.
pub const STEP_DEFERRED: u32 = 2;

#[inline]
fn colour(m: u32) -> u32 {
    (m >> COLOUR_SHIFT) & 7
}
#[inline]
fn with_colour(m: u32, c: u32) -> u32 {
    (m & !COLOUR_MASK) | (c << COLOUR_SHIFT)
}
#[inline]
fn align8(x: u32) -> Option<u32> {
    x.checked_add(7).map(|v| v & !7)
}

#[derive(Clone, Copy, Default)]
struct Td {
    size: u32,
    kind: u32,
    flags: u32,
    pcount: u32,
    pmap: u32,
}

/// Growable u32 vector living in guest memory (runtime-internal blocks).
#[derive(Clone, Copy)]
struct V32 {
    ptr: u32,
    len: u32,
    cap: u32,
}
impl V32 {
    const fn new() -> V32 {
        V32 { ptr: 0, len: 0, cap: 0 }
    }
}

/// Iterator over the managed-reference slots of an object (record pointer map,
/// or the per-element pointer map of every element of an array).
struct Slots {
    base: u32,
    pmap: u32,
    pcount: u32,
    elem: u32,
    elems_left: u32,
    k: u32,
}
impl Slots {
    #[inline]
    fn next<H: Heap>(&mut self, h: &H) -> Option<u32> {
        if self.pcount == 0 || self.elems_left == 0 {
            return None;
        }
        let a = self.base.wrapping_add(h.ld32(self.pmap + 4 * self.k));
        self.k += 1;
        if self.k == self.pcount {
            self.k = 0;
            self.elems_left -= 1;
            self.base = self.base.wrapping_add(self.elem);
        }
        Some(a)
    }
}

/// Tunables.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Work units (slots/objects) of the destruction queue drained by every
    /// `release`/`alloc` call.
    pub drain_budget: u32,
    /// Candidate-buffer length at which `STAT_GC_WANTED` turns on.
    pub root_threshold: u32,
}

pub struct Rt<H: Heap> {
    pub heap: H,
    td_base: u32,
    td_count: u32,
    q_head: u32,
    q_len: u32,
    roots: V32,
    stack: V32,
    log: V32,
    step_cursor: u32,
    step_write: u32,
    live_objects: u32,
    live_bytes: u64,
    zombie_objects: u32,
    zombie_bytes: u64,
    internal_bytes: u64,
    collections: u32,
    aborts: u32,
    cycle_freed: u64,
    work: u32,
    limit: u32,
    in_trial: bool,
    in_drain: bool,
    in_oom_collect: bool,
    oom_abort: bool,
    batch: u32,
    /// Largest batch `gc_step` may grow back to in the current sweep: set when an aborted
    /// trial halved the batch, reset at the end of the sweep (growing straight back made
    /// steps alternate between aborting and succeeding; each abort burns a whole step).
    batch_cap: u32,
    deferred_limit: u32,
    skip_flagged: bool,
    oom_streak: u32,
    step_unflagged: u32,
    livemap: V32,
    tdc_id: u32,
    tdc: Td,
    inline_stack: [u32; INLINE_STACK],
    inline_log: [u32; INLINE_LOG],
    #[cfg(debug_assertions)]
    floor: u32,
    pub cfg: Config,
}

enum Abort {
    Aborted,
}

impl<H: Heap> Rt<H> {
    pub const fn new(heap: H) -> Self {
        Rt {
            heap,
            td_base: 0,
            td_count: 0,
            q_head: 0,
            q_len: 0,
            roots: V32::new(),
            stack: V32::new(),
            log: V32::new(),
            step_cursor: 0,
            step_write: 0,
            live_objects: 0,
            live_bytes: 0,
            zombie_objects: 0,
            zombie_bytes: 0,
            internal_bytes: 0,
            collections: 0,
            aborts: 0,
            cycle_freed: 0,
            work: 0,
            limit: u32::MAX,
            in_trial: false,
            in_drain: false,
            in_oom_collect: false,
            oom_abort: false,
            batch: u32::MAX,
            batch_cap: u32::MAX,
            deferred_limit: 0,
            skip_flagged: false,
            oom_streak: 0,
            step_unflagged: 0,
            livemap: V32::new(),
            tdc_id: 0,
            tdc: Td { size: 0, kind: 0, flags: 0, pcount: 0, pmap: 0 },
            inline_stack: [0; INLINE_STACK],
            inline_log: [0; INLINE_LOG],
            #[cfg(debug_assertions)]
            floor: u32::MAX,
            cfg: Config { drain_budget: 256, root_threshold: 1024 },
        }
    }

    /// Install the type descriptor table (`oxi_typedesc_base/count`).
    ///
    /// The whole table is validated once; any violation traps with
    /// [`T_BAD_TYPEDESC`] and leaves the runtime uninitialised:
    /// `count <= 2^24`, table inside memory, word3 = 0, kinds in `1..=5` (or an
    /// all-zero hole), kind 4 only at id 1, record sizes multiples of 8 and
    /// below 2^31, array element sizes below 2^31, pointer-map offsets 4-aligned,
    /// strictly increasing and `offset + 4 <= size` (record) / `<= elem` (array),
    /// pointer maps inside memory.
    pub fn init(&mut self, td_base: u32, td_count: u32) {
        if !self.table_is_valid(td_base, td_count) {
            self.trap(T_BAD_TYPEDESC);
        }
        self.td_base = td_base;
        self.td_count = td_count;
        self.tdc_id = 0;
    }

    fn table_is_valid(&self, base: u32, count: u32) -> bool {
        let mem = self.heap.mem_size();
        if count > (1 << 24) || !base.is_multiple_of(4) || u64::from(base) + u64::from(count) * 16 > mem {
            return false;
        }
        for id in 0..count {
            let a = base + id * TD_SIZE;
            let (w0, w1, w2, w3) = (self.heap.ld32(a), self.heap.ld32(a + 4), self.heap.ld32(a + 8), self.heap.ld32(a + 12));
            if w3 != 0 {
                return false;
            }
            let (kind, pc) = (w1 & 0xFF, w1 >> 16);
            if kind == 0 {
                if w0 != 0 || w1 != 0 || w2 != 0 {
                    return false;
                }
                continue;
            }
            if kind > K_ARRAY || (kind == K_STRING) != (id == TYPE_STRING) {
                return false;
            }
            if kind == K_STRING {
                if w0 != 0 || pc != 0 {
                    return false;
                }
                continue;
            }
            let unit = w0;
            if kind == K_ARRAY {
                if unit > 0x7FFF_FFFF {
                    return false;
                }
            } else if !unit.is_multiple_of(8) || unit > 0x7FFF_FFF8 {
                return false;
            }
            if pc > 0 && (w2 == 0 || !w2.is_multiple_of(4) || u64::from(w2) + u64::from(pc) * 4 > mem) {
                return false;
            }
            let mut prev: Option<u32> = None;
            for k in 0..pc {
                let off = self.heap.ld32(w2 + 4 * k);
                if !off.is_multiple_of(4) || u64::from(off) + 4 > u64::from(unit) || prev.is_some_and(|p| off <= p) {
                    return false;
                }
                prev = Some(off);
            }
        }
        true
    }

    #[cold]
    #[inline(never)]
    fn trap(&mut self, kind: u32) -> ! {
        self.heap.trap(kind)
    }

    #[cold]
    #[inline(never)]
    fn drain_cold(&mut self) {
        self.drain(self.cfg.drain_budget);
    }

    // ---- low level -------------------------------------------------------

    #[inline]
    pub fn rc_of(&self, p: u32) -> u32 {
        self.heap.ld32(p - 8)
    }
    #[inline]
    pub fn meta_of(&self, p: u32) -> u32 {
        self.heap.ld32(p - 4)
    }

    /// Populate `self.tdc` for `id` in place (no 20-byte struct return / shadow stack frame).
    #[inline(always)]
    fn ensure_td(&mut self, id: u32) {
        if id != self.tdc_id || id == 0 {
            self.load_td_slow(id);
        }
    }

    #[cold]
    #[inline(never)]
    fn load_td_slow(&mut self, id: u32) {
        if id == 0 || id >= self.td_count {
            self.trap(T_UNREACHABLE);
        }
        let a = self.td_base + id * TD_SIZE;
        let w1 = self.heap.ld32(a + 4);
        self.tdc = Td {
            size: self.heap.ld32(a),
            kind: w1 & 0xFF,
            flags: (w1 >> 8) & 0xFF,
            pcount: w1 >> 16,
            pmap: self.heap.ld32(a + 8),
        };
        self.tdc_id = id;
    }

    #[inline(always)]
    fn td(&mut self, id: u32) -> Td {
        self.ensure_td(id);
        self.tdc
    }

    fn vec_get(&self, w: u8) -> V32 {
        match w {
            0 => self.roots,
            1 => self.stack,
            2 => self.log,
            _ => self.livemap,
        }
    }
    fn vec_set(&mut self, w: u8, v: V32) {
        match w {
            0 => self.roots = v,
            1 => self.stack = v,
            2 => self.log = v,
            _ => self.livemap = v,
        }
    }

    #[inline(always)]
    fn stack_cap(&self) -> u32 {
        if self.stack.ptr == 0 {
            INLINE_STACK as u32
        } else {
            self.stack.cap
        }
    }
    #[inline(always)]
    fn log_cap(&self) -> u32 {
        if self.log.ptr == 0 {
            INLINE_LOG as u32
        } else {
            self.log.cap
        }
    }
    #[inline(always)]
    fn stack_get(&self, i: u32) -> u32 {
        if self.stack.ptr == 0 {
            self.inline_stack[i as usize & (INLINE_STACK - 1)]
        } else {
            self.heap.ld32(self.stack.ptr + 4 * i)
        }
    }
    #[inline(always)]
    fn stack_set(&mut self, i: u32, x: u32) {
        if self.stack.ptr == 0 {
            self.inline_stack[i as usize & (INLINE_STACK - 1)] = x;
        } else {
            self.heap.st32(self.stack.ptr + 4 * i, x);
        }
    }
    #[inline(always)]
    fn log_get(&self, i: u32) -> u32 {
        if self.log.ptr == 0 {
            self.inline_log[i as usize & (INLINE_LOG - 1)]
        } else {
            self.heap.ld32(self.log.ptr + 4 * i)
        }
    }
    #[inline(always)]
    fn log_set(&mut self, i: u32, x: u32) {
        if self.log.ptr == 0 {
            self.inline_log[i as usize & (INLINE_LOG - 1)] = x;
        } else {
            self.heap.st32(self.log.ptr + 4 * i, x);
        }
    }

    /// Allocate zeroed bytes from the heap. On failure reclaim everything
    /// already known dead, then run one cycle collection (BL02) and retry once
    /// before trapping `out_of_memory`.
    #[inline]
    fn raw_alloc(&mut self, size: u32) -> u32 {
        let p = self.heap.alloc(size);
        if p != 0 {
            #[cfg(debug_assertions)]
            {
                self.floor = self.floor.min(p);
            }
            return p;
        }
        self.raw_alloc_slow(size, true)
    }

    #[inline]
    fn raw_alloc_uninit(&mut self, size: u32) -> u32 {
        let p = self.heap.alloc_uninit(size);
        if p != 0 {
            #[cfg(debug_assertions)]
            {
                self.floor = self.floor.min(p);
            }
            return p;
        }
        self.raw_alloc_slow(size, false)
    }

    #[cold]
    #[inline(never)]
    fn raw_alloc_slow(&mut self, size: u32, zeroed: bool) -> u32 {
        let do_alloc = |h: &mut H| if zeroed { h.alloc(size) } else { h.alloc_uninit(size) };
        for attempt in 0..2 {
            if attempt == 0 && !self.in_trial && !self.in_drain {
                self.drain(u32::MAX);
            }
            let p = do_alloc(&mut self.heap);
            if p != 0 {
                #[cfg(debug_assertions)]
                {
                    self.floor = self.floor.min(p);
                }
                return p;
            }
        }
        // BL02/BL2-02: bounded cycle collection, then retry, before trapping OOM.
        if !self.in_trial && !self.in_drain && !self.in_oom_collect && self.roots.len > 0 {
            let mut left = OOM_COLLECT_UNITS;
            let mut target = size.max(OOM_MIN_TARGET);
            while left > 0 {
                self.in_oom_collect = true;
                let spent = self.oom_collect(target, false, left);
                self.in_oom_collect = false;
                left -= spent.min(left);
                self.drain(u32::MAX);
                let p = do_alloc(&mut self.heap);
                if p != 0 {
                    #[cfg(debug_assertions)]
                    {
                        self.floor = self.floor.min(p);
                    }
                    return p;
                }
                if spent == 0 {
                    break;
                }
                // Freed memory may be too fragmented for `size`: ask for more.
                target = target.saturating_mul(2);
            }
        }
        self.trap(T_OOM)
    }

    /// Grow internal vector `w` to capacity >= `min_cap` words. Never traps:
    /// `false` means out of memory. The vector is re-read from `self` after
    /// every step that can drain (a drain may push onto the same vector).
    fn grow_vec(&mut self, w: u8, min_cap: u32) -> bool {
        let mut attempt = 0;
        loop {
            let v = self.vec_get(w);
            let cur_cap = match w {
                1 if v.ptr == 0 => INLINE_STACK as u32,
                2 if v.ptr == 0 => INLINE_LOG as u32,
                _ => v.cap,
            };
            if cur_cap >= min_cap {
                return true;
            }
            // Prefer 2x growth on attempts 0..2. On the final attempt fall back
            // to exact 8-byte-aligned `min_cap` for scratch/bitmap reservations
            // (and tiny `roots` buffers) so doubling never causes a false OOM,
            // but refuse +2-word thrashing on large `roots` buffers so
            // `push_root` runs cycle collection (BL02) instead of copying MiBs
            // on every push.
            let exact_cap = min_cap.checked_add(1).map_or(min_cap, |x| x & !1).max(16);
            let ncap = if attempt >= 2 {
                if w == 0 && cur_cap >= 1024 {
                    return false;
                }
                exact_cap
            } else {
                min_cap.max(cur_cap.saturating_mul(2)).max(16)
            };
            let Some(bytes) = ncap.checked_mul(4) else { return false };
            let np = self.heap.alloc(bytes);
            if np == 0 {
                attempt += 1;
                if attempt >= 3 {
                    return false;
                }
                if attempt == 1 && !self.in_trial && !self.in_drain {
                    self.drain(u32::MAX);
                }
                continue;
            }
            #[cfg(debug_assertions)]
            {
                self.floor = self.floor.min(np);
            }
            // nothing ran since `v` was read
            if v.len > 0 {
                if v.ptr != 0 {
                    self.heap.copy(np, v.ptr, v.len * 4);
                } else if w == 1 {
                    for k in 0..v.len {
                        self.heap.st32(np + 4 * k, self.inline_stack[k as usize]);
                    }
                } else if w == 2 {
                    for k in 0..v.len {
                        self.heap.st32(np + 4 * k, self.inline_log[k as usize]);
                    }
                }
            }
            if v.ptr != 0 {
                self.heap.free(v.ptr, v.cap * 4);
                self.internal_bytes -= u64::from(v.cap) * 4;
            }
            self.internal_bytes += u64::from(bytes);
            let len = if w == 3 { ncap } else { v.len };
            self.vec_set(w, V32 { ptr: np, len, cap: ncap });
            return true;
        }
    }

    /// Push a candidate root onto `self.roots`. If `roots` is full and cannot
    /// grow, run one cycle collection (while temporarily pinning `p` with `rc += 1`
    /// so `p` itself stays live) to reclaim dead candidates and retry once.
    fn push_root(&mut self, p: u32) -> bool {
        if self.roots.len < self.roots.cap {
            self.heap.st32(self.roots.ptr + 4 * self.roots.len, p);
            self.roots.len += 1;
            return true;
        }
        if self.grow_vec(0, self.roots.len + 1) {
            self.heap.st32(self.roots.ptr + 4 * self.roots.len, p);
            self.roots.len += 1;
            return true;
        }
        if !self.in_trial && !self.in_drain && !self.in_oom_collect && self.roots.len > 0 {
            let rc = self.heap.ld32(p - 8);
            let pinned = rc < STICKY;
            if pinned {
                self.heap.st32(p - 8, rc + 1);
            }
            self.in_oom_collect = true;
            let _ = self.oom_collect(OOM_MIN_TARGET, true, OOM_COLLECT_UNITS);
            self.in_oom_collect = false;
            if pinned {
                let rc2 = self.heap.ld32(p - 8);
                self.heap.st32(p - 8, rc2 - 1);
            }
            if self.roots.len < self.roots.cap || self.grow_vec(0, self.roots.len + 1) {
                self.heap.st32(self.roots.ptr + 4 * self.roots.len, p);
                self.roots.len += 1;
                return true;
            }
        }
        false
    }

    #[inline]
    fn try_push_stack(&mut self, x: u32) -> bool {
        let cap = self.stack_cap();
        if self.stack.len >= cap && !self.grow_vec(1, self.stack.len + 1) {
            return false;
        }
        let i = self.stack.len;
        self.stack_set(i, x);
        self.stack.len = i + 1;
        true
    }
    /// Push where capacity was reserved (trial phases 2 and 3).
    #[inline]
    fn push_stack(&mut self, x: u32) {
        if !self.try_push_stack(x) {
            self.trap(T_OOM);
        }
    }
    #[inline]
    fn pop_stack(&mut self) -> Option<u32> {
        if self.stack.len == 0 {
            return None;
        }
        self.stack.len -= 1;
        Some(self.stack_get(self.stack.len))
    }
    #[inline]
    fn push_log1(&mut self, x: u32) {
        let cap = self.log_cap();
        if self.log.len >= cap && !self.grow_vec(2, self.log.len + 1) {
            self.trap(T_OOM);
        }
        let i = self.log.len;
        self.log_set(i, x);
        self.log.len = i + 1;
    }
    fn release_vec(&mut self, w: u8) {
        let v = self.vec_get(w);
        self.vec_set(w, V32::new());
        if v.ptr != 0 {
            self.heap.free(v.ptr, v.cap * 4);
            self.internal_bytes -= u64::from(v.cap) * 4;
        }
    }

    // ---- debug-build live-object tracking (F5) ---------------------------

    /// Debug builds keep one bit per 8-byte granule: set while an object is
    /// live. Release builds compile this away.
    #[cfg(debug_assertions)]
    fn live_mark(&mut self, p: u32, on: bool) {
        let word = p / 256; // 32 granules of 8 bytes per word
        while word >= self.livemap.cap {
            if !on {
                return;
            }
            if !self.grow_vec(3, word + 1) {
                self.trap(T_OOM);
            }
        }
        let a = self.livemap.ptr + 4 * word;
        let bit = 1u32 << ((p / 8) % 32);
        let w = self.heap.ld32(a);
        self.heap.st32(a, if on { w | bit } else { w & !bit });
    }
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn live_mark(&mut self, _p: u32, _on: bool) {}

    /// Trap with [`T_STALE_OBJECT`] unless `p` is a live object (or an
    /// immortal static below every heap allocation).
    #[cfg(debug_assertions)]
    fn live_check(&mut self, p: u32) {
        let word = p / 256;
        if p.is_multiple_of(8) && word < self.livemap.cap {
            let w = self.heap.ld32(self.livemap.ptr + 4 * word);
            if w & (1u32 << ((p / 8) % 32)) != 0 {
                return;
            }
        }
        if p < self.floor && p >= 8 && p.is_multiple_of(8) && self.heap.ld32(p - 8) >= STICKY {
            return; // static object (sticky by construction)
        }
        self.trap(T_STALE_OBJECT);
    }
    #[cfg(not(debug_assertions))]
    #[inline(always)]
    fn live_check(&mut self, _p: u32) {}

    // ---- allocation ------------------------------------------------------

    #[inline]
    fn new_object(&mut self, id: u32, total: u32, col: u32) -> u32 {
        let b = self.raw_alloc(total);
        self.heap.st32(b, 1);
        self.heap.st32(b + 4, id | (col << COLOUR_SHIFT));
        self.live_objects += 1;
        self.live_bytes += u64::from(total);
        self.live_mark(b + HEADER, true);
        b + HEADER
    }

    #[inline]
    fn new_object_uninit(&mut self, id: u32, total: u32, col: u32) -> u32 {
        let b = self.raw_alloc_uninit(total);
        self.heap.st32(b, 1);
        self.heap.st32(b + 4, id | (col << COLOUR_SHIFT));
        self.live_objects += 1;
        self.live_bytes += u64::from(total);
        self.live_mark(b + HEADER, true);
        b + HEADER
    }

    #[inline]
    fn alloc_block(&mut self, size: u32) -> u32 {
        if size == 0 {
            return 0;
        }
        let sz = align8(size).unwrap_or_else(|| self.trap(T_OOM));
        let p = self.raw_alloc(sz);
        self.live_bytes += u64::from(sz);
        p
    }
    #[inline]
    fn alloc_block_uninit(&mut self, size: u32) -> u32 {
        if size == 0 {
            return 0;
        }
        let sz = align8(size).unwrap_or_else(|| self.trap(T_OOM));
        let p = self.raw_alloc_uninit(sz);
        self.live_bytes += u64::from(sz);
        p
    }
    #[inline]
    fn free_block(&mut self, p: u32, size: u32) {
        if p == 0 || size == 0 {
            return;
        }
        let sz = align8(size).unwrap_or(size);
        self.live_bytes -= u64::from(sz);
        #[cfg(debug_assertions)]
        self.heap.bytes_mut(p, sz).fill(0xDD);
        self.heap.free(p, sz);
    }

    /// `oxi_alloc(type_id)`: zeroed record/closure/variant, rc 1.
    #[inline]
    pub fn alloc(&mut self, id: u32) -> u32 {
        if self.q_head != 0 {
            self.drain_cold();
        }
        self.ensure_td(id);
        if self.tdc.kind < K_STRUCT || self.tdc.kind > K_CLOSURE {
            self.trap(T_BAD_CAST);
        }
        // `table_is_valid` already verified non-array `td.size` is 8-aligned and <= 0x7FFF_FFF8.
        let total = HEADER + self.tdc.size;
        let col = (self.tdc.flags & FLAG_ACYCLIC) << 2; // 0 => BLACK(0), 1 => GREEN(4)
        self.new_object(id, total, col)
    }

    #[inline]
    pub fn alloc_bytes(&mut self, size: u32) -> u32 {
        if self.q_head != 0 {
            self.drain_cold();
        }
        self.alloc_block(size)
    }

    #[inline]
    pub fn free_bytes(&mut self, p: u32, size: u32) {
        self.free_block(p, size);
    }

    // ---- retain / release ------------------------------------------------

    #[inline]
    pub fn retain(&mut self, p: u32) {
        if p == 0 {
            return;
        }
        self.live_check(p);
        let rc = self.heap.ld32(p - 8);
        if rc >= STICKY {
            return;
        }
        if rc == 0 {
            self.trap(T_UNREACHABLE);
        }
        self.heap.st32(p - 8, rc + 1); // may become STICKY: deterministic leak
        let m = self.heap.ld32(p - 4);
        let c = colour(m);
        if c != BLACK && c != GREEN {
            self.heap.st32(p - 4, with_colour(m, BLACK));
        }
    }

    #[cold]
    #[inline(never)]
    fn buffer_purple(&mut self, p: u32, m: u32) {
        if m & BUFFERED == 0 {
            if !self.push_root(p) {
                self.trap(T_OOM);
            }
            let m2 = self.heap.ld32(p - 4);
            self.heap.st32(p - 4, with_colour(m2, PURPLE) | BUFFERED);
        } else {
            self.heap.st32(p - 4, with_colour(m, PURPLE));
        }
    }

    /// Decrement without draining: the shared core of `release` and of every
    /// child release.
    #[inline]
    fn dec(&mut self, p: u32) {
        self.live_check(p);
        let rc = self.heap.ld32(p - 8);
        if rc >= STICKY {
            return;
        }
        let m = self.heap.ld32(p - 4);
        if rc == 1 {
            // dies: queue it (the rc word becomes the link)
            self.live_mark(p, false);
            self.heap.st32(p - 4, with_colour(m, PENDING));
            self.heap.st32(p - 8, self.q_head);
            self.q_head = p;
            self.q_len += 1;
            return;
        }
        if rc == 0 {
            self.trap(T_UNREACHABLE);
        }
        self.heap.st32(p - 8, rc - 1);
        let c = colour(m);
        if c != GREEN && !(c == PURPLE && m & BUFFERED != 0) {
            self.buffer_purple(p, m);
        }
    }

    #[inline(always)]
    fn free_object_known(&mut self, p: u32, total: u32) {
        self.live_objects -= 1;
        self.live_bytes -= u64::from(total);
        #[cfg(debug_assertions)]
        self.heap.bytes_mut(p - 8, total).fill(0xDD);
        self.heap.free(p - 8, total);
    }

    /// `oxi_release`. wasmi charges the fuel of every instruction of a function body that
    /// has no `loop`/`if` up front, on every call, whichever branch runs. So the exported
    /// entry is a thin dispatcher (null check, one load, one branch) and every path is a
    /// separate small function: a shared object (rc > 1) pays only for its own few
    /// instructions, not for the record/string/array freeing code it never reaches.
    #[inline]
    pub fn release(&mut self, p: u32) {
        if p == 0 {
            return;
        }
        self.live_check(p);
        let rc = self.heap.ld32(p - 8);
        if rc != 1 {
            self.release_rc_ne_1(p, rc);
            return;
        }
        self.release_unique(p);
    }

    /// `release` of an object whose `rc` is 1 (the overwhelming majority of
    /// temporaries): freed directly; buffered objects, strings and arrays go to
    /// their own functions.
    #[inline(never)]
    fn release_unique(&mut self, p: u32) {
        let m = self.heap.ld32(p - 4);
        if m & BUFFERED != 0 {
            self.release_queue_slow(p, m);
            return;
        }
        let id = m & META_TYPE_MASK;
        if id == TYPE_STRING {
            self.release_string(p);
            return;
        }
        self.ensure_td(id);
        if self.tdc.kind == K_ARRAY {
            self.release_array_fast_or_slow(p, m);
            return;
        }
        self.live_mark(p, false);
        let total = HEADER + self.tdc.size;
        let pcount = self.tdc.pcount;
        if pcount > 0 {
            self.release_record_children(p, pcount, self.tdc.pmap);
        }
        self.free_object_known(p, total);
        if self.q_head != 0 {
            self.drain_cold();
        }
    }

    #[inline(never)]
    fn release_string(&mut self, p: u32) {
        self.live_mark(p, false);
        let len = self.heap.ld32(p);
        let total = (HEADER + 8 + len + 1 + 7) & !7;
        self.free_object_known(p, total);
        if self.q_head != 0 {
            self.drain_cold();
        }
    }

    #[cold]
    #[inline(never)]
    fn release_record_children(&mut self, p: u32, pcount: u32, pmap: u32) {
        let prev = self.in_drain;
        self.in_drain = true;
        for k in 0..pcount {
            let c = self.heap.ld32(p + self.heap.ld32(pmap + 4 * k));
            if c != 0 {
                self.dec(c);
            }
        }
        self.in_drain = prev;
    }

    #[cold]
    #[inline(never)]
    fn release_array_fast_or_slow(&mut self, p: u32, m: u32) {
        let elem = self.tdc.size;
        let pcount = self.tdc.pcount;
        let pmap = self.tdc.pmap;
        if pcount == 0 {
            self.live_mark(p, false);
            let data = self.heap.ld32(p + ARR_DATA);
            if data != 0 {
                let cap = self.heap.ld32(p + ARR_CAP);
                self.free_block(data, cap.wrapping_mul(elem));
            }
            self.free_object_known(p, ARR_OBJ_SIZE);
            if self.q_head != 0 {
                self.drain_cold();
            }
            return;
        }
        // Small reference-containing arrays: release elements iteratively without
        // paying the full `drain` loop overhead when elements are leaf records.
        let len = self.heap.ld32(p + ARR_LEN);
        if len <= 8 {
            self.live_mark(p, false);
            let data = self.heap.ld32(p + ARR_DATA);
            let cap = self.heap.ld32(p + ARR_CAP);
            let prev = self.in_drain;
            self.in_drain = true;
            for i in 0..len {
                let e = data + i * elem;
                for k in 0..pcount {
                    let c = self.heap.ld32(e + self.heap.ld32(pmap + 4 * k));
                    if c != 0 {
                        self.dec_or_free_leaf(c);
                    }
                }
            }
            self.in_drain = prev;
            if data != 0 {
                self.free_block(data, cap.wrapping_mul(elem));
            }
            self.free_object_known(p, ARR_OBJ_SIZE);
            if self.q_head != 0 {
                self.drain_cold();
            }
            return;
        }
        self.release_queue_slow(p, m);
    }

    /// Non-recursive child release: frees an unbuffered pointer-free leaf record
    /// or string immediately (rc 1 -> 0) and delegates everything else to `dec`.
    #[inline]
    fn dec_or_free_leaf(&mut self, c: u32) {
        self.live_check(c);
        let rc = self.heap.ld32(c - 8);
        if rc == 1 {
            let cm = self.heap.ld32(c - 4);
            if cm & BUFFERED == 0 {
                let cid = cm & META_TYPE_MASK;
                if cid == TYPE_STRING {
                    self.live_mark(c, false);
                    let slen = self.heap.ld32(c);
                    let total = (HEADER + 8 + slen + 1 + 7) & !7;
                    self.free_object_known(c, total);
                    return;
                }
                self.ensure_td(cid);
                if self.tdc.kind != K_ARRAY && self.tdc.pcount == 0 {
                    self.live_mark(c, false);
                    let total = HEADER + self.tdc.size;
                    self.free_object_known(c, total);
                    return;
                }
            }
        }
        self.dec(c);
    }

    #[cold]
    #[inline(never)]
    fn release_queue_slow(&mut self, p: u32, m: u32) {
        self.live_mark(p, false);
        self.heap.st32(p - 4, with_colour(m, PENDING));
        self.heap.st32(p - 8, self.q_head);
        self.q_head = p;
        self.q_len += 1;
        self.drain(self.cfg.drain_budget);
    }

    #[cold]
    #[inline(never)]
    fn release_rc_ne_1(&mut self, p: u32, rc: u32) {
        if rc >= STICKY {
            return;
        }
        if rc == 0 {
            self.trap(T_UNREACHABLE);
        }
        self.heap.st32(p - 8, rc - 1);
        let m = self.heap.ld32(p - 4);
        let c = colour(m);
        if c != GREEN && !(c == PURPLE && m & BUFFERED != 0) {
            self.buffer_purple(p, m);
        }
        if self.q_head != 0 {
            self.drain_cold();
        }
    }

    pub fn is_unique(&self, p: u32) -> u32 {
        u32::from(p != 0 && self.heap.ld32(p - 8) == 1)
    }

    // ---- object sizes / slots -------------------------------------------

    fn obj_total(&mut self, p: u32, id: u32) -> u32 {
        if id == TYPE_STRING {
            let len = self.heap.ld32(p);
            return align8(HEADER + 8 + len + 1).unwrap_or(u32::MAX);
        }
        let td = self.td(id);
        if td.kind == K_ARRAY {
            ARR_OBJ_SIZE
        } else {
            HEADER + align8(td.size).unwrap_or(0)
        }
    }

    fn slots(&mut self, p: u32) -> Slots {
        let id = self.heap.ld32(p - 4) & META_TYPE_MASK;
        self.slots_id(p, id)
    }

    fn slots_id(&mut self, p: u32, id: u32) -> Slots {
        if id == TYPE_STRING {
            return Slots { base: 0, pmap: 0, pcount: 0, elem: 0, elems_left: 0, k: 0 };
        }
        let td = self.td(id);
        if td.kind == K_ARRAY {
            Slots {
                base: self.heap.ld32(p + ARR_DATA),
                pmap: td.pmap,
                pcount: td.pcount,
                elem: td.size,
                elems_left: self.heap.ld32(p + ARR_LEN),
                k: 0,
            }
        } else {
            Slots { base: p, pmap: td.pmap, pcount: td.pcount, elem: 0, elems_left: 1, k: 0 }
        }
    }

    fn dealloc_object(&mut self, p: u32) {
        self.live_mark(p, false);
        let id = self.heap.ld32(p - 4) & META_TYPE_MASK;
        let total = self.obj_total(p, id);
        self.live_objects -= 1;
        self.live_bytes -= u64::from(total);
        #[cfg(debug_assertions)]
        self.heap.bytes_mut(p - 8, total).fill(0xDD);
        self.heap.free(p - 8, total);
    }

    /// Free an array's data block and clear the header fields.
    fn free_array_data(&mut self, p: u32, elem: u32) {
        let data = self.heap.ld32(p + ARR_DATA);
        let cap = self.heap.ld32(p + ARR_CAP);
        self.heap.st32(p + ARR_LEN, 0);
        self.heap.st32(p + ARR_CAP, 0);
        self.heap.st32(p + ARR_DATA, 0);
        self.free_block(data, cap.wrapping_mul(elem));
    }

    /// Children are done and the array block (if any) released: free the
    /// object, or keep it as a zombie while the candidate buffer still lists it.
    #[inline]
    fn finish_free_known(&mut self, p: u32, m: u32, total: u32) {
        self.live_mark(p, false);
        if m & BUFFERED != 0 {
            self.zombie_objects += 1;
            self.zombie_bytes += u64::from(total);
            self.heap.st32(p - 4, with_colour(m, ZOMBIE));
            self.heap.st32(p - 8, 0);
        } else {
            self.free_object_known(p, total);
        }
    }

    // ---- destruction queue ----------------------------------------------

    /// Process the destruction queue with at most `budget` work units
    /// (slots, plus one per object). Resumable at slot granularity inside
    /// arrays (the array `len` is the cursor), at object granularity
    /// otherwise. Returns the unused budget.
    pub fn drain(&mut self, mut budget: u32) -> u32 {
        let prev = self.in_drain;
        self.in_drain = true;
        while self.q_head != 0 {
            if budget == 0 {
                self.in_drain = prev;
                return 0;
            }
            // Detach the head first: releasing its children pushes onto the queue.
            let p = self.q_head;
            self.q_head = self.heap.ld32(p - 8);
            self.q_len -= 1;
            let m = self.heap.ld32(p - 4);
            let id = m & META_TYPE_MASK;
            let total = if id == TYPE_STRING {
                let len = self.heap.ld32(p);
                align8(HEADER + 8 + len + 1).unwrap_or(u32::MAX)
            } else {
                let td = self.td(id);
                if td.kind == K_ARRAY {
                    let data = self.heap.ld32(p + ARR_DATA);
                    // pointer-free elements need no walk (also: zero-size elements)
                    let mut i = if td.pcount == 0 { 0 } else { self.heap.ld32(p + ARR_LEN) };
                    while i > 0 {
                        if budget == 0 {
                            // out of budget mid-array: len is the cursor; requeue
                            self.heap.st32(p + ARR_LEN, i);
                            self.heap.st32(p - 8, self.q_head);
                            self.q_head = p;
                            self.q_len += 1;
                            self.in_drain = prev;
                            return 0;
                        }
                        i -= 1;
                        let e = data + i * td.size;
                        for k in 0..td.pcount {
                            let c = self.heap.ld32(e + self.heap.ld32(td.pmap + 4 * k));
                            if c != 0 {
                                self.dec(c);
                            }
                        }
                        budget = budget.saturating_sub(1 + td.pcount);
                    }
                    self.free_array_data(p, td.size);
                    ARR_OBJ_SIZE
                } else {
                    for k in 0..td.pcount {
                        let c = self.heap.ld32(p + self.heap.ld32(td.pmap + 4 * k));
                        if c != 0 {
                            self.dec(c);
                        }
                    }
                    budget = budget.saturating_sub(1 + td.pcount);
                    HEADER + td.size
                }
            };
            budget = budget.saturating_sub(1);
            self.finish_free_known(p, m, total);
        }
        self.in_drain = prev;
        budget
    }

    // ---- stats -----------------------------------------------------------

    /// Number of live managed objects (drains the destruction queue first;
    /// uncollected cycles still count).
    pub fn live_objects(&mut self) -> u32 {
        self.drain(u32::MAX);
        self.live_objects - self.zombie_objects
    }
    pub fn live_bytes(&mut self) -> u32 {
        self.drain(u32::MAX);
        u32::try_from(self.live_bytes - self.zombie_bytes).unwrap_or(u32::MAX)
    }

    pub fn stat(&self, key: u32) -> u32 {
        match key {
            STAT_PENDING => self.q_len,
            STAT_CANDIDATES => self.roots.len - (self.step_cursor - self.step_write),
            STAT_COLLECTIONS => self.collections,
            STAT_CYCLE_FREED => u32::try_from(self.cycle_freed).unwrap_or(u32::MAX),
            STAT_INTERNAL_BYTES => u32::try_from(self.internal_bytes).unwrap_or(u32::MAX),
            STAT_GC_WANTED => u32::from(self.roots.len >= self.cfg.root_threshold),
            STAT_ABORTS => self.aborts,
            _ => 0,
        }
    }

    pub fn config(&mut self, key: u32, value: u32) {
        match key {
            CFG_DRAIN_BUDGET => self.cfg.drain_budget = value.max(1),
            CFG_ROOT_THRESHOLD => self.cfg.root_threshold = value,
            _ => {}
        }
    }

    // ---- cycle collector -------------------------------------------------

    /// A child worth tracing: non-null, not sticky, not green.
    #[inline]
    fn traced(&self, c: u32) -> bool {
        c != 0 && self.heap.ld32(c - 8) < STICKY && colour(self.heap.ld32(c - 4)) != GREEN
    }

    /// Candidate entry `i` if it takes part in a trial: non-zero and not
    /// flagged (flag bits live in the low bits of the 8-aligned address).
    #[inline]
    fn root_active(&self, i: u32) -> u32 {
        let raw = self.heap.ld32(self.roots.ptr + 4 * i);
        if raw & 7 != 0 && self.skip_flagged {
            0
        } else {
            raw & !7
        }
    }

    fn abort_oom(&mut self) -> Result<(), Abort> {
        self.oom_abort = true;
        Err(Abort::Aborted)
    }

    /// Phase 1 from `root`: subtract every traced edge of every reached node.
    /// The work limit is checked per slot; the log records `(node, slots done)`
    /// so that [`Self::undo_gray`] re-adds exactly what was subtracted.
    fn mark_gray(&mut self, root: u32) -> Result<(), Abort> {
        let rm = self.heap.ld32(root - 4);
        if rm & IN_STACK == 0 {
            if !self.try_push_stack(root) {
                return self.abort_oom();
            }
            self.heap.st32(root - 4, rm | IN_STACK);
        }
        while let Some(n) = self.pop_stack() {
            let m = self.heap.ld32(n - 4) & !IN_STACK;
            if colour(m) == GRAY {
                self.heap.st32(n - 4, m);
                continue;
            }
            // log first: a node is only coloured once undo knows about it
            if self.log.len + 2 > self.log_cap() && !self.grow_vec(2, self.log.len + 2) {
                self.heap.st32(n - 4, m);
                return self.abort_oom();
            }
            let entry_idx = self.log.len;
            self.log_set(entry_idx, n);
            self.log_set(entry_idx + 1, 0);
            self.log.len = entry_idx + 2;
            self.heap.st32(n - 4, with_colour(m, GRAY));
            self.work = self.work.wrapping_add(1);
            let mut it = self.slots_id(n, m & META_TYPE_MASK);
            let mut done = 0u32;
            let mut res = Ok(());
            while let Some(sa) = it.next(&self.heap) {
                if self.work > self.limit {
                    res = Err(Abort::Aborted);
                    break;
                }
                done += 1;
                self.work = self.work.wrapping_add(1);
                let c = self.heap.ld32(sa);
                if c == 0 {
                    continue;
                }
                let rc = self.heap.ld32(c - 8);
                let cm = self.heap.ld32(c - 4);
                if rc >= STICKY || colour(cm) == GREEN {
                    continue; // not traced
                }
                if rc == 0 {
                    self.trap(T_UNREACHABLE);
                }
                self.heap.st32(c - 8, rc - 1);
                if colour(cm) != GRAY && (cm & IN_STACK) == 0 {
                    if !self.try_push_stack(c) {
                        self.oom_abort = true;
                        res = Err(Abort::Aborted);
                        break;
                    }
                    self.heap.st32(c - 4, cm | IN_STACK);
                }
            }
            self.log_set(entry_idx + 1, done);
            res?;
        }
        Ok(())
    }

    /// Exactly reverse every `mark_gray` side effect recorded in the log.
    fn undo_gray(&mut self) {
        self.aborts += 1;
        while let Some(n) = self.pop_stack() {
            let m = self.heap.ld32(n - 4);
            if m & IN_STACK != 0 {
                self.heap.st32(n - 4, m & !IN_STACK);
            }
        }
        let mut i = 0;
        while i < self.log.len {
            let n = self.log_get(i);
            let done = self.log_get(i + 1);
            let mut it = self.slots(n);
            for _ in 0..done {
                let Some(sa) = it.next(&self.heap) else { break };
                let c = self.heap.ld32(sa);
                if self.traced(c) {
                    let rc = self.heap.ld32(c - 8);
                    self.heap.st32(c - 8, rc + 1);
                }
            }
            i += 2;
        }
        let mut i = 0;
        while i < self.log.len {
            let n = self.log_get(i);
            let m = self.heap.ld32(n - 4) & !IN_STACK;
            let c = if m & BUFFERED != 0 { PURPLE } else { BLACK };
            self.heap.st32(n - 4, with_colour(m, c));
            i += 2;
        }
        self.log.len = 0;
    }

    fn scan_black(&mut self, n: u32, nm: u32) {
        // `scan` shares the stack: only consume what this call pushed.
        let base = self.stack.len;
        self.heap.st32(n - 4, with_colour(nm, BLACK));
        self.push_stack(n);
        while self.stack.len > base {
            let m = self.pop_stack().unwrap_or(0);
            let mut it = self.slots(m);
            while let Some(sa) = it.next(&self.heap) {
                let c = self.heap.ld32(sa);
                if c == 0 {
                    continue;
                }
                let rc = self.heap.ld32(c - 8);
                let cm = self.heap.ld32(c - 4);
                if rc >= STICKY || colour(cm) == GREEN {
                    continue;
                }
                self.heap.st32(c - 8, rc + 1);
                if colour(cm) != BLACK {
                    self.heap.st32(c - 4, with_colour(cm, BLACK));
                    self.push_stack(c);
                }
            }
        }
    }

    fn scan(&mut self, root: u32) {
        let rm = self.heap.ld32(root - 4);
        if colour(rm) != GRAY || (rm & IN_STACK) != 0 {
            return;
        }
        self.heap.st32(root - 4, rm | IN_STACK);
        self.push_stack(root);
        while let Some(n) = self.pop_stack() {
            let nm = self.heap.ld32(n - 4) & !IN_STACK;
            if colour(nm) != GRAY {
                self.heap.st32(n - 4, nm);
                continue;
            }
            if self.heap.ld32(n - 8) > 0 {
                self.scan_black(n, nm);
            } else {
                self.heap.st32(n - 4, with_colour(nm, WHITE));
                let mut it = self.slots_id(n, nm & META_TYPE_MASK);
                while let Some(sa) = it.next(&self.heap) {
                    let c = self.heap.ld32(sa);
                    if c != 0 {
                        let cm = self.heap.ld32(c - 4);
                        if colour(cm) == GRAY && (cm & IN_STACK) == 0 && self.heap.ld32(c - 8) < STICKY {
                            self.heap.st32(c - 4, cm | IN_STACK);
                            self.push_stack(c);
                        }
                    }
                }
            }
        }
    }

    /// Gather the white subgraph under `root` into the log (marking black on
    /// push so each node enters the stack at most once). Never allocates: the
    /// log holds at least one pair per marked node.
    fn gather_white(&mut self, root: u32) {
        let rm = self.heap.ld32(root - 4);
        if colour(rm) != WHITE {
            return;
        }
        self.heap.st32(root - 4, with_colour(rm, BLACK));
        self.push_stack(root);
        while let Some(n) = self.pop_stack() {
            self.push_log1(n);
            let mut it = self.slots(n);
            while let Some(sa) = it.next(&self.heap) {
                let c = self.heap.ld32(sa);
                if c != 0 {
                    let cm = self.heap.ld32(c - 4);
                    if colour(cm) == WHITE {
                        self.heap.st32(c - 4, with_colour(cm, BLACK));
                        self.push_stack(c);
                    }
                }
            }
        }
    }

    /// One trial over `roots[start..end]` (entries that are flagged are
    /// skipped). `Ok((freed, work, marked))`, or `Err(work)` when the work
    /// limit was hit or an internal vector could not grow (`oom_abort`); in
    /// both cases the heap is fully restored.
    fn trial(&mut self, start: u32, end: u32, limit: u32) -> Result<(u32, u32, u32), u32> {
        self.in_trial = true;
        let r = self.trial_inner(start, end, limit);
        self.in_trial = false;
        r
    }

    fn trial_inner(&mut self, start: u32, end: u32, limit: u32) -> Result<(u32, u32, u32), u32> {
        self.work = 0;
        self.limit = limit;
        self.oom_abort = false;
        self.log.len = 0;
        self.stack.len = 0;
        // phase 1: mark roots
        for i in start..end {
            self.work = self.work.wrapping_add(1);
            let s = self.root_active(i);
            if s == 0 {
                continue;
            }
            if self.work > self.limit {
                self.undo_gray();
                return Err(self.work);
            }
            let m = self.heap.ld32(s - 4);
            let col = colour(m);
            if col == PURPLE {
                if self.mark_gray(s).is_err() {
                    self.undo_gray();
                    return Err(self.work);
                }
            } else {
                self.drop_candidate(i, s, m);
            }
        }
        let used = self.work;
        let marked = self.log.len / 2;
        // Phases 2 and 3 must not allocate. Because `scan` uses `IN_STACK` and
        // `scan_black`/`gather_white` recolour on push, `stack.len` never exceeds
        // `2 * nodes_marked + 2` = `self.log.len + 2`.
        let need = self.log.len.checked_add(4);
        if need.is_none_or(|n| self.stack_cap() < n && !self.grow_vec(1, n)) {
            self.oom_abort = true;
            self.undo_gray();
            return Err(used);
        }
        // phase 2: scan
        for i in start..end {
            let s = self.root_active(i);
            if s != 0 {
                self.scan(s);
            }
        }
        // phase 3: collect
        self.log.len = 0;
        for i in start..end {
            let s = self.root_active(i);
            if s == 0 {
                continue;
            }
            self.heap.st32(self.roots.ptr + 4 * i, 0);
            let m = self.heap.ld32(s - 4);
            self.heap.st32(s - 4, m & !BUFFERED);
            self.gather_white(s);
        }
        let dead = self.log.len;
        // pass A: release the untraced (green / sticky) children of the dead
        for i in 0..dead {
            let n = self.log_get(i);
            let mut it = self.slots(n);
            while let Some(sa) = it.next(&self.heap) {
                let c = self.heap.ld32(sa);
                if c != 0 && !self.traced(c) {
                    self.dec(c);
                }
            }
        }
        // pass B: free
        for i in 0..dead {
            let n = self.log_get(i);
            let m = self.heap.ld32(n - 4);
            let id = m & META_TYPE_MASK;
            let total = if id == TYPE_STRING {
                let len = self.heap.ld32(n);
                align8(HEADER + 8 + len + 1).unwrap_or(u32::MAX)
            } else {
                let td = self.td(id);
                if td.kind == K_ARRAY {
                    self.free_array_data(n, td.size);
                    ARR_OBJ_SIZE
                } else {
                    HEADER + td.size
                }
            };
            self.finish_free_known(n, m, total);
        }
        self.log.len = 0;
        Ok((dead, used, marked))
    }

    /// A candidate entry whose object is no longer purple: unlist it; a dead
    /// object that was only kept for the listing (ZOMBIE) is freed now.
    fn drop_candidate(&mut self, i: u32, s: u32, m: u32) {
        self.heap.st32(self.roots.ptr + 4 * i, 0);
        if colour(m) == ZOMBIE {
            let total = self.obj_total(s, m & META_TYPE_MASK);
            self.zombie_objects -= 1;
            self.zombie_bytes -= u64::from(total);
            self.dealloc_object(s);
        } else {
            self.heap.st32(s - 4, m & !BUFFERED);
        }
    }

    fn compact_roots(&mut self) {
        let mut w = 0;
        for r in 0..self.roots.len {
            let s = self.heap.ld32(self.roots.ptr + 4 * r);
            if s != 0 {
                self.heap.st32(self.roots.ptr + 4 * w, s);
                w += 1;
            }
        }
        self.roots.len = w;
        self.step_cursor = 0;
        self.step_write = 0;
        self.step_unflagged = 0;
    }

    /// Move candidate entry `k` (if any) down to the sweep's write position.
    fn keep_root(&mut self, k: u32) {
        let s = self.heap.ld32(self.roots.ptr + 4 * k);
        if s != 0 {
            let w = self.step_write;
            self.heap.st32(self.roots.ptr + 4 * w, s);
            if w != k {
                self.heap.st32(self.roots.ptr + 4 * k, 0);
            }
            self.step_write += 1;
        }
    }

    /// `oxi_collect_cycles`: drain the destruction queue and examine every
    /// candidate (also those a bounded step deferred) in small batches so
    /// collecting garbage that fits in memory needs almost no headroom.
    /// Returns the number of objects freed by cycle collection.
    pub fn collect_cycles(&mut self) -> u32 {
        self.collect_cycles_inner(true)
    }

    fn collect_cycles_inner(&mut self, trap_on_oom: bool) -> u32 {
        self.collections += 1;
        let mut total = 0u32;
        for i in 0..self.roots.len {
            let raw = self.heap.ld32(self.roots.ptr + 4 * i);
            self.heap.st32(self.roots.ptr + 4 * i, raw & !7);
        }
        self.deferred_limit = 0;
        self.skip_flagged = false;
        self.oom_streak = 0;
        self.batch = u32::MAX;
        loop {
            self.drain(u32::MAX);
            if self.roots.len == 0 {
                break;
            }
            let len = self.roots.len;
            let mut i = 0u32;
            let mut batch = COLLECT_BATCH;
            let mut max_batch = u32::MAX;
            let mut sweep_freed = 0u32;
            let mut sweep_ooms = 0u32;
            while i < len {
                let end = i.saturating_add(batch).min(len);
                let mut r = self.trial(i, end, u32::MAX);
                if r.is_err() {
                    // Reclaim anything dead outside the trial.
                    self.drain(u32::MAX);
                    if end - i > 1 {
                        batch = ((end - i) / 2).max(1);
                        max_batch = batch;
                        continue;
                    }
                    // Single root hit OOM: retry once after drain.
                    r = self.trial(i, end, u32::MAX);
                }
                if let Ok((freed, _, marked)) = r {
                    let span = end - i;
                    total += freed;
                    sweep_freed += freed;
                    i = end;
                    if marked > span.saturating_mul(2)
                        || (batch > COLLECT_BATCH && marked > span.saturating_add(64))
                    {
                        // Roots reach into a shared subgraph larger than this batch
                        // (e.g. a deep live linked list): jump batch up by at least
                        // `marked` (and 4x) so total work stays O(N) instead of
                        // repeatedly re-scanning the shared tail.
                        batch = batch.saturating_mul(4).max(marked).min(max_batch);
                    } else if batch < COLLECT_BATCH {
                        batch = (batch * 2).min(COLLECT_BATCH);
                    }
                } else {
                    sweep_ooms += 1;
                    i = end;
                    if sweep_ooms >= 4 && sweep_freed == 0 {
                        break;
                    }
                }
            }
            self.compact_roots();
            if sweep_ooms > 0 {
                if sweep_freed > 0 {
                    continue;
                }
                if trap_on_oom {
                    self.trap(T_OOM);
                }
                break;
            }
        }
        self.cycle_freed += u64::from(total);
        // Do not keep large scratch vectors alive between collections.
        if self.stack.cap > 4096 {
            self.release_vec(1);
        }
        if self.log.cap > 4096 {
            self.release_vec(2);
        }
        if self.roots.len == 0 && self.roots.cap > 4096 {
            self.release_vec(0);
        }
        total
    }

    /// Bounded, non-trapping cycle collection for the allocation-failure paths
    /// (BL2-02). Runs [`Self::gc_step`] with growing budgets until at least
    /// `target` bytes were returned to the heap (or, with `need_root_slot`, the
    /// candidate buffer has room again), the collector is idle, or `limit` work
    /// units were spent. Returns the units spent (0 = nothing was examined).
    ///
    /// The bound is what keeps a recovery attempt from exceeding the fuel of
    /// the call it runs in: one step costs at most [`OOM_STEP_UNITS`] units, a
    /// whole attempt at most `limit` (at most [`OOM_COLLECT_UNITS`]). A garbage
    /// subgraph larger than that is *not* reclaimed here and the allocation
    /// traps `out_of_memory`; the host's incremental collection between calls
    /// (or `oxi_collect_cycles`) handles such graphs.
    fn oom_collect(&mut self, target: u32, need_root_slot: bool, limit: u32) -> u32 {
        let start = self.live_bytes;
        let mut spent = 0u32;
        let mut step = OOM_STEP_UNITS.min(limit);
        loop {
            let r = self.gc_step(step);
            spent = spent.saturating_add(step);
            let freed = start.saturating_sub(self.live_bytes);
            if freed >= u64::from(target) || (need_root_slot && self.roots.len < self.roots.cap) {
                break;
            }
            if r == 0 || spent >= limit {
                break;
            }
            if r & STEP_MORE == 0 {
                // only candidates that exceeded this step's budget remain: raise it
                step = step.saturating_mul(4).min(OOM_COLLECT_UNITS);
            }
            step = step.min(limit - spent);
        }
        self.collections += 1;
        if self.q_head != 0 || spent > 0 {
            self.drain(u32::MAX);
        }
        if need_root_slot {
            self.free_one_root_slot();
        }
        spent
    }

    /// A partial sweep leaves a gap of zeros in the candidate buffer
    /// (`[step_write, step_cursor)`). Free one slot at its end in O(1): with nothing left
    /// unexamined the gap is simply cut off; otherwise the last unexamined entry moves into
    /// the gap (it counts as swept for this round and is examined in the next one).
    fn free_one_root_slot(&mut self) {
        let gap = self.step_cursor - self.step_write;
        if gap == 0 {
            return;
        }
        if self.roots.len == self.step_cursor {
            self.roots.len -= gap;
            self.step_cursor = self.step_write;
            return;
        }
        let last = self.heap.ld32(self.roots.ptr + 4 * (self.roots.len - 1));
        self.heap.st32(self.roots.ptr + 4 * self.step_write, last);
        self.step_write += 1;
        self.roots.len -= 1;
    }

    /// Bounded collection step (`oxi_gc_step`).
    ///
    /// Work bound: a call performs O(`budget`) slot/node visits (drain: up to
    /// `budget` units plus at most one record's pointer count, since a record is
    /// destroyed atomically; mark: at most `budget` units; scan, collect and the
    /// undo of an aborted trial revisit only what the mark phase touched). It
    /// examines candidates in batches: the batch starts as the whole remaining
    /// buffer (so candidates that share a big live subgraph are marked once) and
    /// halves after every aborted trial. A single candidate whose mark exceeds
    /// the budget is *deferred*: flagged and skipped (1 unit) by later steps until
    /// a step with a larger budget (or `oxi_collect_cycles`) comes along.
    ///
    /// Returns [`STEP_MORE`] if another call has work to do, [`STEP_DEFERRED`]
    /// if deferred candidates exist, 0 when idle. `budget == 0` does nothing.
    pub fn gc_step(&mut self, budget: u32) -> u32 {
        if budget == 0 {
            return 0; // no work requested, none reported pending: a host loop terminates
        }
        let mut left = self.drain(budget);
        if self.q_head != 0 {
            return STEP_MORE;
        }
        self.skip_flagged = budget <= self.deferred_limit;
        if self.step_cursor < self.roots.len && left < budget - budget / 2 {
            // too little budget left to tell "candidate too big" from "budget spent"
            return STEP_MORE;
        }
        // In-place incremental compaction: `[0, write)` are survivors of this
        // sweep, `[write, cursor)` is a gap of zeros, `[cursor, len)` is unexamined.
        while left > 0 && self.step_cursor < self.roots.len {
            let i = self.step_cursor;
            let raw = self.heap.ld32(self.roots.ptr + 4 * i);
            if raw == 0 {
                self.step_cursor += 1;
                left = left.saturating_sub(1);
                continue;
            }
            if raw & 7 != 0 && self.skip_flagged {
                // deferred at a budget at least as big as this one: stay out of the way,
                // unless it is no longer a candidate (died or was retained): reap it
                let s = raw & !7;
                let m = self.heap.ld32(s - 4);
                if colour(m) == PURPLE {
                    self.keep_root(i);
                } else {
                    self.drop_candidate(i, s, m);
                }
                self.step_cursor += 1;
                left = left.saturating_sub(1);
                continue;
            }
            let lim = left;
            let span = lim.min(self.batch).max(1);
            let end = u32::try_from((u64::from(i) + u64::from(span)).min(u64::from(self.roots.len))).unwrap_or(self.roots.len);
            if let Ok((freed, used, _)) = self.trial(i, end, lim) {
                self.cycle_freed += u64::from(freed);
                left = left.saturating_sub(used.max(1));
                if used < lim / 2 {
                    // cheap: try more roots next time, but not past a size that just aborted
                    self.batch = self.batch.saturating_mul(2).min(self.batch_cap);
                }
                for k in i..end {
                    self.keep_root(k);
                }
                self.step_cursor = end;
                self.oom_streak = 0;
            } else {
                // limit hit or an internal vector could not grow: the trial was undone
                left = 0;
                if self.oom_abort {
                    self.oom_streak += 1;
                } else if end - i > 1 {
                    self.batch = ((end - i) / 2).max(1);
                    self.batch_cap = self.batch;
                } else {
                    if lim >= budget - budget / 2 {
                        self.heap.st32(self.roots.ptr + 4 * i, (raw & !7) | 1);
                        self.deferred_limit = self.deferred_limit.max(lim);
                    } else {
                        self.step_unflagged += 1;
                    }
                    self.keep_root(i);
                    self.step_cursor = i + 1;
                }
            }
        }
        if self.step_cursor >= self.roots.len {
            self.batch_cap = u32::MAX;
            self.roots.len = self.step_write;
            self.step_cursor = 0;
            self.step_write = 0;
            let unflagged = core::mem::take(&mut self.step_unflagged);
            let mut r = 0;
            if unflagged > 0 || self.q_head != 0 {
                r |= STEP_MORE;
            }
            if self.roots.len > unflagged {
                r |= STEP_DEFERRED;
            }
            return r;
        }
        if self.oom_streak >= 2 && self.q_head == 0 {
            // repeated allocation failure inside trials: do not invite the host to spin;
            // `oxi_collect_cycles` will drain, retry and trap OutOfMemory cleanly
            return STEP_DEFERRED;
        }
        STEP_MORE
    }

    // ---- clone / arrays / strings ---------------------------------------

    fn null_check(&mut self, p: u32) {
        if p == 0 {
            self.trap(T_NULL);
        }
    }

    pub fn clone_obj(&mut self, p: u32) -> u32 {
        self.null_check(p);
        let id = self.heap.ld32(p - 4) & META_TYPE_MASK;
        if id == TYPE_STRING {
            return self.clone_str_slow(p);
        }
        self.ensure_td(id);
        let col = (self.tdc.flags & FLAG_ACYCLIC) << 2;
        if self.tdc.kind == K_ARRAY {
            return self.clone_array_slow(p, id, col);
        }
        let payload = self.tdc.size;
        let pcount = self.tdc.pcount;
        let pmap = self.tdc.pmap;
        let n = self.new_object_uninit(id, HEADER + payload, col);
        self.heap.copy(n, p, payload);
        if pcount > 0 {
            self.retain_record_children(n, pcount, pmap);
        }
        n
    }

    #[cold]
    #[inline(never)]
    fn retain_record_children(&mut self, n: u32, pcount: u32, pmap: u32) {
        for k in 0..pcount {
            let c = self.heap.ld32(n + self.heap.ld32(pmap + 4 * k));
            self.retain(c);
        }
    }

    #[cold]
    #[inline(never)]
    fn clone_str_slow(&mut self, p: u32) -> u32 {
        let len = self.heap.ld32(p);
        let n = self.new_string(len);
        self.heap.copy(n + 8, p + 8, len);
        n
    }

    #[cold]
    #[inline(never)]
    fn clone_array_slow(&mut self, p: u32, id: u32, col: u32) -> u32 {
        let elem = self.tdc.size;
        let pcount = self.tdc.pcount;
        let len = self.heap.ld32(p + ARR_LEN);
        let data = self.heap.ld32(p + ARR_DATA);
        let bytes = len.checked_mul(elem).unwrap_or_else(|| self.trap(T_OOM));
        let n = self.new_object_uninit(id, ARR_OBJ_SIZE, col);
        self.heap.st32(n + ARR_LEN, 0);
        self.heap.st32(n + ARR_CAP, 0);
        self.heap.st32(n + ARR_DATA, 0);
        let blk = self.alloc_block_uninit(bytes);
        if bytes > 0 {
            self.heap.copy(blk, data, bytes);
        }
        self.heap.st32(n + ARR_LEN, len);
        self.heap.st32(n + ARR_CAP, len);
        self.heap.st32(n + ARR_DATA, blk);
        if pcount > 0 {
            let mut it = self.slots(n);
            while let Some(sa) = it.next(&self.heap) {
                let c = self.heap.ld32(sa);
                self.retain(c);
            }
        }
        n
    }

    #[inline]
    pub fn array_new(&mut self, id: u32, len: u32) -> u32 {
        if self.q_head != 0 {
            self.drain_cold();
        }
        if len > ARR_MAX_LEN {
            self.trap(T_BOUNDS);
        }
        self.ensure_td(id);
        if self.tdc.kind != K_ARRAY {
            self.trap(T_BAD_CAST);
        }
        let elem = self.tdc.size;
        let col = (self.tdc.flags & FLAG_ACYCLIC) << 2;
        let bytes = len.checked_mul(elem).unwrap_or_else(|| self.trap(T_OOM));
        let n = self.new_object_uninit(id, ARR_OBJ_SIZE, col);
        let blk = if bytes == 0 {
            0
        } else {
            self.heap.st32(n + ARR_LEN, 0);
            self.heap.st32(n + ARR_CAP, 0);
            self.heap.st32(n + ARR_DATA, 0);
            self.alloc_block(bytes)
        };
        self.heap.st32(n + ARR_LEN, len);
        self.heap.st32(n + ARR_CAP, len);
        self.heap.st32(n + ARR_DATA, blk);
        n
    }

    pub fn array_grow(&mut self, a: u32, min_cap: u32) {
        self.null_check(a);
        if min_cap > ARR_MAX_LEN {
            self.trap(T_BOUNDS);
        }
        let id = self.heap.ld32(a - 4) & META_TYPE_MASK;
        self.ensure_td(id);
        if self.tdc.kind != K_ARRAY {
            self.trap(T_BAD_CAST);
        }
        let elem = self.tdc.size;
        let cap = self.heap.ld32(a + ARR_CAP);
        if min_cap <= cap {
            return;
        }
        let mut ncap = if cap < 4 { 4 } else { cap.saturating_mul(2) };
        if ncap < min_cap {
            ncap = min_cap;
        }
        if ncap > ARR_MAX_LEN {
            ncap = ARR_MAX_LEN;
        }
        let bytes = ncap.checked_mul(elem).unwrap_or_else(|| self.trap(T_OOM));
        let blk = self.alloc_block(bytes);
        let len = self.heap.ld32(a + ARR_LEN);
        let old = self.heap.ld32(a + ARR_DATA);
        if len > 0 {
            self.heap.copy(blk, old, len * elem);
        }
        self.heap.st32(a + ARR_CAP, ncap);
        self.heap.st32(a + ARR_DATA, blk);
        self.free_block(old, cap * elem);
    }

    #[inline]
    fn new_string(&mut self, len: u32) -> u32 {
        if self.q_head != 0 {
            self.drain_cold();
        }
        let total = len
            .checked_add(HEADER + 8 + 1)
            .and_then(align8)
            .unwrap_or_else(|| self.trap(T_OOM));
        let n = self.new_object_uninit(TYPE_STRING, total, GREEN);
        self.heap.st32(n, len);
        self.heap.st32(n + 4, 0);
        self.heap.st8(n + 8 + len, 0);
        n
    }

    pub fn str_new(&mut self, ptr: u32, len: u32) -> u32 {
        if core::str::from_utf8(self.heap.bytes(ptr, len)).is_err() {
            self.trap(T_BAD_UTF8);
        }
        let n = self.new_string(len);
        self.heap.copy(n + 8, ptr, len);
        n
    }

    fn str_len(&mut self, s: u32) -> u32 {
        self.null_check(s);
        self.heap.ld32(s)
    }

    pub fn str_concat(&mut self, a: u32, b: u32) -> u32 {
        let (la, lb) = (self.str_len(a), self.str_len(b));
        let len = la.checked_add(lb).unwrap_or_else(|| self.trap(T_OOM));
        let n = self.new_string(len);
        self.heap.copy(n + 8, a + 8, la);
        self.heap.copy(n + 8 + la, b + 8, lb);
        n
    }

    pub fn str_eq(&mut self, a: u32, b: u32) -> u32 {
        let (la, lb) = (self.str_len(a), self.str_len(b));
        u32::from(la == lb && self.heap.bytes(a + 8, la) == self.heap.bytes(b + 8, lb))
    }

    pub fn str_cmp(&mut self, a: u32, b: u32) -> u32 {
        let (la, lb) = (self.str_len(a), self.str_len(b));
        let r = self.heap.bytes(a + 8, la).cmp(self.heap.bytes(b + 8, lb));
        match r {
            core::cmp::Ordering::Less => u32::MAX, // -1
            core::cmp::Ordering::Equal => 0,
            core::cmp::Ordering::Greater => 1,
        }
    }

    pub fn str_slice(&mut self, s: u32, start: u32, end: u32) -> u32 {
        let len = self.str_len(s);
        // start/end are i32 on the wire: negative values show up as >= 2^31.
        if start > end || end > len {
            self.trap(T_BOUNDS);
        }
        let ok = |h: &H, i: u32| i == len || (h.bytes(s + 8 + i, 1)[0] & 0xC0) != 0x80;
        if !ok(&self.heap, start) || !ok(&self.heap, end) {
            self.trap(T_BAD_UTF8);
        }
        let n = self.new_string(end - start);
        self.heap.copy(n + 8, s + 8 + start, end - start);
        n
    }

    fn string_from_bytes(&mut self, b: &[u8]) -> u32 {
        let n = self.new_string(b.len() as u32);
        self.heap.bytes_mut(n + 8, b.len() as u32).copy_from_slice(b);
        n
    }

    pub fn str_from_i64(&mut self, v: i64) -> u32 {
        let mut buf = [0u8; 20];
        let mut i = buf.len();
        let mut u = v.unsigned_abs();
        loop {
            i -= 1;
            buf[i] = b'0' + (u % 10) as u8;
            u /= 10;
            if u == 0 {
                break;
            }
        }
        if v < 0 {
            // 19 digits + sign fits into 20 bytes except for i64::MIN (19 digits): ok
            i -= 1;
            buf[i] = b'-';
        }
        self.string_from_bytes(&buf[i..])
    }

    #[cfg(not(feature = "float-fmt"))]
    pub fn str_from_f64(&mut self, _v: f64) -> u32 {
        self.trap(T_UNREACHABLE)
    }

    #[cfg(feature = "float-fmt")]
    pub fn str_from_f64(&mut self, v: f64) -> u32 {
        use core::fmt::Write;
        struct Buf([u8; 40], usize);
        impl Write for Buf {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                let b = s.as_bytes();
                if self.1 + b.len() > self.0.len() {
                    return Err(core::fmt::Error);
                }
                self.0[self.1..self.1 + b.len()].copy_from_slice(b);
                self.1 += b.len();
                Ok(())
            }
        }
        let mut b = Buf([0; 40], 0);
        if v.is_nan() {
            let _ = b.write_str("NaN");
        } else if v.is_infinite() {
            let _ = b.write_str(if v > 0.0 { "Infinity" } else { "-Infinity" });
        } else {
            // Rust's shortest round-trip form; integral values lose the ".0".
            let _ = write!(b, "{v:?}");
            if b.1 >= 2 && b.0[b.1 - 2] == b'.' && b.0[b.1 - 1] == b'0' {
                b.1 -= 2;
            }
        }
        let n = b.1;
        let tmp = b.0;
        self.string_from_bytes(&tmp[..n])
    }

    #[cfg(not(feature = "float-fmt"))]
    pub fn str_from_f32(&mut self, _v: f32) -> u32 {
        self.trap(T_UNREACHABLE)
    }

    #[cfg(feature = "float-fmt")]
    pub fn str_from_f32(&mut self, v: f32) -> u32 {
        use core::fmt::Write;
        struct Buf([u8; 40], usize);
        impl Write for Buf {
            fn write_str(&mut self, s: &str) -> core::fmt::Result {
                let b = s.as_bytes();
                if self.1 + b.len() > self.0.len() {
                    return Err(core::fmt::Error);
                }
                self.0[self.1..self.1 + b.len()].copy_from_slice(b);
                self.1 += b.len();
                Ok(())
            }
        }
        let mut b = Buf([0; 40], 0);
        if v.is_nan() {
            let _ = b.write_str("NaN");
        } else if v.is_infinite() {
            let _ = b.write_str(if v > 0.0 { "Infinity" } else { "-Infinity" });
        } else {
            // Rust's shortest round-trip form for f32; integral values lose the ".0".
            let _ = write!(b, "{v:?}");
            if b.1 >= 2 && b.0[b.1 - 2] == b'.' && b.0[b.1 - 1] == b'0' {
                b.1 -= 2;
            }
        }
        let n = b.1;
        let tmp = b.0;
        self.string_from_bytes(&tmp[..n])
    }
}
