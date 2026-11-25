//! Native test heap: a fixed arena managed by the *real* dlmalloc (so the
//! allocator is exercised natively too), with optional access checking.
//!
//! With `checks` on, every load/store must lie inside a live allocation and
//! every `free` must name a live block of the right size: a use-after-free,
//! double free or out-of-bounds access in the runtime panics immediately.

use crate::heap::Heap;
use dlmalloc::{Allocator, Dlmalloc};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Panic payload of a runtime trap (`oxi_trap`) in the simulated heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Trap(pub u32);

const PAGE: usize = 64 * 1024;
/// Low bytes that are never handed out (ABI: `[0, 1024)` is reserved).
const RESERVED: usize = 4096;

pub struct Arena {
    base: usize,
    next: AtomicUsize,
    limit: usize,
}

// SAFETY: the arena only hands out disjoint ranges of memory owned by the
// SimHeap that created it; `alloc`'s bump pointer is atomic.
unsafe impl Allocator for Arena {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let size = size.div_ceil(PAGE) * PAGE;
        let off = self.next.fetch_add(size, Ordering::Relaxed);
        if off + size > self.limit {
            self.next.fetch_sub(size, Ordering::Relaxed);
            return (core::ptr::null_mut(), 0, 0);
        }
        ((self.base + off) as *mut u8, size, 0)
    }
    fn remap(&self, _: *mut u8, _: usize, _: usize, _: bool) -> *mut u8 {
        core::ptr::null_mut()
    }
    fn free_part(&self, _: *mut u8, _: usize, _: usize) -> bool {
        false
    }
    fn free(&self, _: *mut u8, _: usize) -> bool {
        false
    }
    fn can_release_part(&self, _: u32) -> bool {
        false
    }
    fn allocates_zeros(&self) -> bool {
        true
    }
    fn page_size(&self) -> usize {
        PAGE
    }
}

pub struct SimHeap {
    mem: Box<[u8]>,
    dl: Dlmalloc<Arena>,
    live: BTreeMap<u32, u32>,
    pub checks: bool,
    /// Last trap kind (also carried by the panic payload).
    pub last_trap: u32,
    /// Fault injection: `(xorshift state, percent)`; an allocation fails with that
    /// probability, but never twice in a row (so one retry always succeeds).
    inject: Option<(u64, u64)>,
    just_failed: bool,
    fail_next: u32,
    pub injected_failures: u64,
}

impl SimHeap {
    /// `size` bytes of address space (zero-filled lazily by the OS).
    pub fn new(size: usize, checks: bool) -> SimHeap {
        let mut mem = vec![0u8; size].into_boxed_slice();
        let base = mem.as_mut_ptr() as usize;
        let dl = Dlmalloc::new_with_allocator(Arena { base, next: AtomicUsize::new(RESERVED), limit: size });
        SimHeap { mem, dl, live: BTreeMap::new(), checks, last_trap: 0, inject: None, just_failed: false, fail_next: 0, injected_failures: 0 }
    }

    /// Make `alloc` fail with probability `pct` percent (see the field docs).
    pub fn inject_failures(&mut self, seed: u64, pct: u64) {
        self.inject = Some((seed | 1, pct));
    }

    /// Make the next `n` allocations fail unconditionally (persistent OOM).
    pub fn fail_next(&mut self, n: u32) {
        self.fail_next = n;
    }

    pub fn live_blocks(&self) -> usize {
        self.live.len()
    }
    /// Total bytes of live allocations (objects, blocks and runtime-internal).
    pub fn live_alloc_bytes(&self) -> u64 {
        self.live.values().map(|&s| u64::from(s)).sum()
    }
    pub fn mem_mut(&mut self) -> &mut [u8] {
        &mut self.mem
    }
    /// Bytes of arena claimed from the "system" (heap footprint).
    pub fn footprint(&self) -> usize {
        self.dl.allocator().next.load(Ordering::Relaxed)
    }

    #[inline]
    fn check(&self, a: u32, n: u32) {
        if !self.checks {
            return;
        }
        let end = u64::from(a) + u64::from(n);
        match self.live.range(..=a).next_back() {
            Some((&s, &sz)) if end <= u64::from(s) + u64::from(sz) => {}
            other => panic!("access of {n} bytes at {a:#x} outside any live block (nearest {other:?})"),
        }
    }
}

impl Heap for SimHeap {
    #[inline]
    fn ld32(&self, a: u32) -> u32 {
        self.check(a, 4);
        let i = a as usize;
        u32::from_le_bytes(self.mem[i..i + 4].try_into().unwrap())
    }
    #[inline]
    fn st32(&mut self, a: u32, v: u32) {
        self.check(a, 4);
        let i = a as usize;
        self.mem[i..i + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn bytes(&self, a: u32, len: u32) -> &[u8] {
        if len > 0 {
            self.check(a, len);
        }
        &self.mem[a as usize..a as usize + len as usize]
    }
    fn bytes_mut(&mut self, a: u32, len: u32) -> &mut [u8] {
        if len > 0 {
            self.check(a, len);
        }
        &mut self.mem[a as usize..a as usize + len as usize]
    }
    fn copy(&mut self, dst: u32, src: u32, len: u32) {
        if len > 0 {
            self.check(dst, len);
            self.check(src, len);
        }
        self.mem.copy_within(src as usize..(src + len) as usize, dst as usize);
    }
    fn alloc(&mut self, size: u32) -> u32 {
        assert!(size > 0);
        if self.fail_next > 0 {
            self.fail_next -= 1;
            self.injected_failures += 1;
            return 0;
        }
        if self.just_failed {
            self.just_failed = false;
        } else if let Some((st, pct)) = self.inject.as_mut() {
            *st ^= *st >> 12;
            *st ^= *st << 25;
            *st ^= *st >> 27;
            if st.wrapping_mul(0x2545_F491_4F6C_DD1D) % 100 < *pct {
                self.just_failed = true;
                self.injected_failures += 1;
                return 0;
            }
        }
        // SAFETY: plain dlmalloc use; size/align are valid.
        let p = unsafe { self.dl.calloc(size as usize, 8) };
        if p.is_null() {
            return 0;
        }
        let off = p as usize - self.mem.as_ptr() as usize;
        let off = u32::try_from(off).expect("arena larger than 4 GiB");
        assert!(self.live.insert(off, size).is_none());
        off
    }
    fn free(&mut self, a: u32, size: u32) {
        let got = self.live.remove(&a);
        assert_eq!(got, Some(size), "free of {a:#x}: not a live block of that size (double free?)");
        // SAFETY: `a` came from `alloc` above.
        unsafe { self.dl.free(self.mem.as_mut_ptr().add(a as usize), size as usize, 8) };
    }
    fn mem_size(&self) -> u64 {
        self.mem.len() as u64
    }
    fn trap(&mut self, kind: u32) -> ! {
        self.last_trap = kind;
        std::panic::panic_any(Trap(kind))
    }
}
