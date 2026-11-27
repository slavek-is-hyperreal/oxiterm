//! The wasm32 guest: the exports of the memory runtime over raw linear memory
//! and dlmalloc. Single instance = single thread, so one global `Rt`.

use crate::heap::Heap;
use crate::rt::{Rt, T_BAD_TYPEDESC};
use core::cell::Cell;
use core::ptr::{addr_of, addr_of_mut};
use dlmalloc::{Allocator, Dlmalloc};

const PAGE_SIZE: usize = 65_536;
/// Granularity at which dlmalloc requests fresh chunks from linear memory.
/// 4 KiB lets small programs allocate out of the remainder of the initial
/// linear memory pages after `heap_base` without immediately calling `memory.grow`.
const CHUNK_ALIGN: usize = 4096;

extern "C" {
    static __heap_base: u8;
}

#[link(wasm_import_module = "oxi:rt")]
extern "C" {
    /// Provided by the compiler/linker: writes `kind` into the single exported
    /// mutable global `oxi_trap_kind`.
    fn oxi_set_trap(kind: u32);
}

struct WasmSys {
    /// Next linear-memory address to hand to dlmalloc (0 = not yet seeded).
    cur: Cell<usize>,
}

// SAFETY: one instance is one thread; `cur` advances monotonically over
// zeroed linear memory (remainder of initial pages after `heap_base` and
// freshly grown Wasm pages).
unsafe impl Allocator for WasmSys {
    fn alloc(&self, size: usize) -> (*mut u8, usize, u32) {
        let Some(size) = size.checked_add(CHUNK_ALIGN - 1).map(|v| v & !(CHUNK_ALIGN - 1)) else {
            return (core::ptr::null_mut(), 0, 0);
        };
        if size == 0 {
            return (core::ptr::null_mut(), 0, 0);
        }
        // SAFETY: single-threaded.
        unsafe { *addr_of_mut!(HEAP_USED) = true };
        let mut cur = self.cur.get();
        if cur == 0 {
            let rt_end = (&raw const __heap_base as usize).saturating_add(7) & !7;
            // BL2-06: a linked module's static data starts at `__heap_base`; the linker
            // patches `oxi_rt_floor` to its end, so an allocation before `oxi_rt_init`
            // cannot land in it (0 = standalone runtime).
            // SAFETY: single-threaded; a plain word of linear memory.
            let floor = unsafe { core::ptr::read_volatile(addr_of!(oxi_rt_floor)) } as usize;
            cur = rt_end.max(floor);
        }
        let Some(end) = cur.checked_add(size) else {
            return (core::ptr::null_mut(), 0, 0);
        };
        let mem_bytes = core::arch::wasm32::memory_size(0).saturating_mul(PAGE_SIZE);
        if end > mem_bytes {
            let need = end - mem_bytes;
            let pages = need.div_ceil(PAGE_SIZE);
            if core::arch::wasm32::memory_grow(0, pages) == usize::MAX {
                return (core::ptr::null_mut(), 0, 0);
            }
        }
        self.cur.set(end);
        (cur as *mut u8, size, 0)
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
        CHUNK_ALIGN
    }
}

pub struct WasmHeap;

/// Debug runtime only: depth of the freed-block quarantine.
#[cfg(debug_assertions)]
const QUARANTINE: usize = 64;
#[cfg(debug_assertions)]
static mut QUAR: [(u32, u32); QUARANTINE] = [(0, 0); QUARANTINE];
#[cfg(debug_assertions)]
static mut QUAR_AT: usize = 0;

/// Push `(a, size)` into the quarantine ring; returns the evicted oldest entry.
#[cfg(debug_assertions)]
unsafe fn quarantine(a: u32, size: u32) -> (u32, u32) {
    // SAFETY: single-threaded (caller contract).
    unsafe {
        let i = *addr_of!(QUAR_AT);
        let slot = &mut (*addr_of_mut!(QUAR))[i];
        let old = *slot;
        *slot = (a, size);
        *addr_of_mut!(QUAR_AT) = (i + 1) % QUARANTINE;
        old
    }
}

/// Size-class slab allocator in front of `dlmalloc` for small 8-aligned
/// allocations (`8..=256` bytes, 32 classes of 8 bytes).
///
/// A slab is one 4096-byte, 4096-aligned block obtained from `dlmalloc`
/// (`SLAB_REQ` makes its chunk exactly 4096 bytes, so consecutive slabs cut
/// from fresh or coalesced memory are aligned back to back with no waste;
/// when the phase is off, `memalign` fixes it). Its 24-byte header sits at the
/// slab start, so a block finds its slab with `addr & !4095`:
///
/// ```text
///  +0 free_head  first free block of this slab (0 = slab is full)
///  +4 used       blocks handed out
///  +8 next, +12 prev   links of the class's list of slabs that have a free block
/// +16 class      size class index
/// ```
///
/// The hot path stays at one list pop per allocation and one list push per
/// free. A slab that becomes completely free goes back to `dlmalloc`
/// (BL2-01), so memory freed in one size class is reusable by every other
/// size and by large blocks; one empty slab per class is kept as a
/// reserve to avoid thrashing at a slab boundary.
const SMALL_MAX: u32 = 256;
const SMALL_CLASSES: usize = (SMALL_MAX as usize) / 8;
const SLAB_SIZE: u32 = 4096;
const SLAB_MASK: u32 = !(SLAB_SIZE - 1);
/// `dlmalloc` request size whose chunk is exactly `SLAB_SIZE` bytes
/// (4 bytes overhead, 8-byte granularity).
const SLAB_REQ: u32 = SLAB_SIZE - 4;
const SLAB_HDR: u32 = 24;
const S_FREE: u32 = 0;
const S_USED: u32 = 4;
const S_NEXT: u32 = 8;
const S_PREV: u32 = 12;
const S_CLASS: u32 = 16;

#[inline(always)]
fn ld(a: u32) -> u32 {
    // SAFETY: slab headers and free blocks are valid, aligned linear memory.
    unsafe { core::ptr::read(a as usize as *const u32) }
}
#[inline(always)]
fn st(a: u32, v: u32) {
    // SAFETY: see `ld`.
    unsafe { core::ptr::write(a as usize as *mut u32, v) }
}

struct SmallPool {
    /// Per class: first slab of the list of slabs with a free block (0 = none).
    heads: [u32; SMALL_CLASSES],
    /// Per class: the slab kept as the empty reserve (0 = none yet). It carries a phantom
    /// `used` count; the most recently emptied slab replaces a reserve that got busy.
    cache: [u32; SMALL_CLASSES],
}

impl SmallPool {
    const fn new() -> Self {
        SmallPool {
            heads: [0; SMALL_CLASSES],
            cache: [0; SMALL_CLASSES],
        }
    }

    #[inline(always)]
    fn alloc_small(&mut self, sz: u32) -> u32 {
        let idx = ((sz - 1) >> 3) as usize;
        // SAFETY: callers only call `alloc_small` with `8 <= sz <= SMALL_MAX` (8-aligned),
        // so `idx < SMALL_CLASSES` (32) always holds; unchecked avoids a panic block in hot path.
        let s = unsafe { *self.heads.get_unchecked(idx) };
        if s != 0 {
            // A slab in the list always has a free block.
            let h = ld(s + S_FREE);
            let nx = ld(h);
            st(s + S_FREE, nx);
            st(s + S_USED, ld(s + S_USED) + 1);
            if nx == 0 {
                self.unlink_full(idx, s);
            }
            return h;
        }
        self.alloc_small_slow(sz, idx)
    }

    /// The head slab `s` just handed out its last free block.
    #[cold]
    #[inline(never)]
    fn unlink_full(&mut self, idx: usize, s: u32) {
        let nx = ld(s + S_NEXT);
        // SAFETY: `idx < SMALL_CLASSES`.
        unsafe { *self.heads.get_unchecked_mut(idx) = nx };
        if nx != 0 {
            st(nx + S_PREV, 0);
        }
        st(s + S_NEXT, 0);
    }

    #[cold]
    #[inline(never)]
    fn alloc_small_slow(&mut self, sz: u32, idx: usize) -> u32 {
        let s = new_slab();
        if s == 0 {
            return 0;
        }
        let n = (SLAB_SIZE - SLAB_HDR) / sz;
        let first = s + SLAB_HDR;
        // Thread blocks 1..n; block 0 is returned.
        let mut b = first + sz;
        for _ in 1..n - 1 {
            st(b, b + sz);
            b += sz;
        }
        st(b, 0);
        st(s + S_FREE, first + sz);
        st(s + S_USED, 1);
        st(s + S_NEXT, 0);
        st(s + S_PREV, 0);
        st(s + S_CLASS, idx as u32);
        // SAFETY: `idx < SMALL_CLASSES`.
        unsafe { *self.heads.get_unchecked_mut(idx) = s };
        first
    }

    #[inline(always)]
    fn free_small(&mut self, a: u32) {
        let s = a & SLAB_MASK;
        let fh = ld(s + S_FREE);
        st(a, fh);
        st(s + S_FREE, a);
        let used = ld(s + S_USED) - 1;
        st(s + S_USED, used);
        if (fh == 0) | (used == 0) {
            self.free_edge(s, fh == 0, used);
        }
    }

    /// Slab `s` was full (`was_full`: relink it into its class list) and/or is
    /// now empty (`used == 0`: keep as the class reserve or give it back).
    #[cold]
    #[inline(never)]
    fn free_edge(&mut self, s: u32, was_full: bool, used: u32) {
        let idx = ld(s + S_CLASS) as usize;
        // SAFETY: the class index was stored from a valid `idx < SMALL_CLASSES`.
        let (head, cache) = unsafe { (self.heads.get_unchecked_mut(idx), self.cache.get_unchecked_mut(idx)) };
        if was_full {
            let nx = *head;
            st(s + S_NEXT, nx);
            st(s + S_PREV, 0);
            if nx != 0 {
                st(nx + S_PREV, s);
            }
            *head = s;
        }
        if used == 0 {
            let c = *cache;
            // The reserve carries one phantom count: `used == 1` means it is empty.
            let reserve_used = if c == 0 { 0 } else { ld(c + S_USED) };
            if c == 0 || reserve_used > 1 {
                // `s` becomes the reserve (an old reserve that is busy gives its phantom
                // count back and is an ordinary slab again). The phantom `used = 1` keeps
                // `s` from reaching 0 again, so the hot path never re-enters this function
                // for it (a one-object alloc/free loop).
                if c != 0 {
                    st(c + S_USED, reserve_used - 1);
                }
                *cache = s;
                st(s + S_USED, 1);
            } else {
                // an empty reserve exists already: give `s` back
                let (p, n) = (ld(s + S_PREV), ld(s + S_NEXT));
                if p != 0 {
                    st(p + S_NEXT, n);
                } else {
                    *head = n;
                }
                if n != 0 {
                    st(n + S_PREV, p);
                }
                dl_free(s, SLAB_REQ);
            }
        }
    }
}

/// A fresh 4096-aligned slab chunk from `dlmalloc` (0 on out of memory).
#[cold]
#[inline(never)]
fn new_slab() -> u32 {
    let p = dl_malloc(SLAB_REQ);
    if p & (SLAB_SIZE - 1) == 0 {
        return p;
    }
    if p != 0 {
        dl_free(p, SLAB_REQ);
    }
    // SAFETY: single-threaded; power-of-two alignment.
    unsafe { (*addr_of_mut!(DL)).malloc(SLAB_REQ as usize, SLAB_SIZE as usize) as usize as u32 }
}

#[cold]
#[inline(never)]
fn dl_malloc(sz: u32) -> u32 {
    // SAFETY: single-threaded; valid size/align.
    unsafe { (*addr_of_mut!(DL)).malloc(sz as usize, 8) as usize as u32 }
}

#[cold]
#[inline(never)]
fn dl_calloc(sz: u32) -> u32 {
    // SAFETY: single-threaded; valid size/align.
    unsafe { (*addr_of_mut!(DL)).calloc(sz as usize, 8) as usize as u32 }
}

#[cold]
#[inline(never)]
fn dl_free(a: u32, sz: u32) {
    // SAFETY: single-threaded; `a` was returned by `dl_malloc`/`dl_calloc`.
    unsafe { (*addr_of_mut!(DL)).free(a as usize as *mut u8, sz as usize, 8) }
}

static mut DL: Dlmalloc<WasmSys> = Dlmalloc::new_with_allocator(WasmSys { cur: Cell::new(0) });
static mut POOL: SmallPool = SmallPool::new();
static mut RT: Rt<WasmHeap> = Rt::new(WasmHeap);
static mut RT_INITED: bool = false;
/// Set by the first request to the system allocator (see `oxi_rt_init`).
static mut HEAP_USED: bool = false;

/// First allocatable address when the runtime is linked behind program static data
/// (patched by the linker, see `WasmSys::alloc`); exported as a global holding its
/// address, like `oxi_scratch_ptr`. 0 in a standalone runtime.
#[no_mangle]
pub static mut oxi_rt_floor: u32 = 0;

#[repr(C, align(8))]
pub struct Scratch(pub [u8; 4096]);

/// The scratch region. Exporting a static exports a Wasm global whose value
/// is the *address*, which is exactly what `oxi_scratch_ptr` must be.
#[no_mangle]
pub static mut oxi_scratch_ptr: Scratch = Scratch([0; 4096]);

fn rt() -> &'static mut Rt<WasmHeap> {
    // SAFETY: one instance is one thread; exports are never re-entered.
    unsafe { &mut *addr_of_mut!(RT) }
}

impl Heap for WasmHeap {
    #[inline]
    fn ld32(&self, a: u32) -> u32 {
        // SAFETY: wasm bounds-checks the access; addresses come from the runtime.
        unsafe { core::ptr::read(a as usize as *const u32) }
    }
    #[inline]
    fn st32(&mut self, a: u32, v: u32) {
        // SAFETY: see ld32.
        unsafe { core::ptr::write(a as usize as *mut u32, v) }
    }
    fn bytes(&self, a: u32, len: u32) -> &[u8] {
        // SAFETY: see ld32.
        unsafe { core::slice::from_raw_parts(a as usize as *const u8, len as usize) }
    }
    fn bytes_mut(&mut self, a: u32, len: u32) -> &mut [u8] {
        // SAFETY: see ld32.
        unsafe { core::slice::from_raw_parts_mut(a as usize as *mut u8, len as usize) }
    }
    fn copy(&mut self, dst: u32, src: u32, len: u32) {
        // SAFETY: see ld32; ptr::copy is memmove.
        unsafe { core::ptr::copy(src as usize as *const u8, dst as usize as *mut u8, len as usize) }
    }
    #[inline(always)]
    fn alloc(&mut self, size: u32) -> u32 {
        let sz = (size + 7) & !7;
        if sz <= SMALL_MAX {
            // SAFETY: single-threaded.
            let p = unsafe { (*addr_of_mut!(POOL)).alloc_small(sz) };
            if p != 0 {
                // SAFETY: `p` is an 8-aligned block of `sz` bytes.
                unsafe { core::ptr::write_bytes(p as usize as *mut u8, 0, sz as usize) };
            }
            return p;
        }
        dl_calloc(sz)
    }
    #[inline(always)]
    fn alloc_uninit(&mut self, size: u32) -> u32 {
        let sz = (size + 7) & !7;
        if sz <= SMALL_MAX {
            // SAFETY: single-threaded.
            return unsafe { (*addr_of_mut!(POOL)).alloc_small(sz) };
        }
        dl_malloc(sz)
    }
    #[inline(always)]
    fn free(&mut self, a: u32, size: u32) {
        #[cfg(debug_assertions)]
        let (a, size) = if size <= SMALL_MAX {
            // Debug runtime: quarantine freed small blocks (FIFO) so a stale pointer cannot
            // alias a new object of the same size (the live bitmap is keyed by address).
            // Large blocks are not held back: they would pin memory.
            // SAFETY: single-threaded.
            let (old_a, old_size) = unsafe { quarantine(a, size) };
            if old_a == 0 {
                return;
            }
            (old_a, old_size)
        } else {
            (a, size)
        };
        let sz = (size + 7) & !7;
        if sz <= SMALL_MAX {
            // SAFETY: single-threaded; `a` was returned by `alloc`/`alloc_uninit`.
            unsafe { (*addr_of_mut!(POOL)).free_small(a) };
            return;
        }
        dl_free(a, sz);
    }
    #[inline(always)]
    fn st8(&mut self, a: u32, v: u8) {
        // SAFETY: wasm bounds-checks the access; addresses come from the runtime.
        unsafe { core::ptr::write(a as usize as *mut u8, v) }
    }
    fn mem_size(&self) -> u64 {
        core::arch::wasm32::memory_size(0) as u64 * 65536
    }
    fn trap(&mut self, kind: u32) -> ! {
        // SAFETY: single-threaded; provided by compiler/linker (sets `oxi_trap_kind`).
        unsafe { oxi_set_trap(kind) };
        core::arch::wasm32::unreachable()
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: single-threaded. 10 = TrapKind::Panic
    unsafe { oxi_set_trap(10) };
    core::arch::wasm32::unreachable()
}

// ---- ABI exports (names and signatures: oxiscript_abi::exports::EXPORTS) ----

#[no_mangle]
pub extern "C" fn oxi_trap(kind: u32) {
    rt().heap.trap(kind);
}
#[no_mangle]
pub extern "C" fn oxi_alloc(type_id: u32) -> u32 {
    rt().alloc(type_id)
}
#[no_mangle]
pub extern "C" fn oxi_alloc_bytes(size: u32) -> u32 {
    rt().alloc_bytes(size)
}
#[no_mangle]
pub extern "C" fn oxi_free_bytes(ptr: u32, size: u32) {
    rt().free_bytes(ptr, size);
}
#[no_mangle]
pub extern "C" fn oxi_retain(p: u32) {
    rt().retain(p);
}
#[no_mangle]
pub extern "C" fn oxi_release(p: u32) {
    rt().release(p);
}
#[no_mangle]
pub extern "C" fn oxi_clone(p: u32) -> u32 {
    rt().clone_obj(p)
}
#[no_mangle]
pub extern "C" fn oxi_is_unique(p: u32) -> u32 {
    rt().is_unique(p)
}
#[no_mangle]
pub extern "C" fn oxi_collect_cycles() -> u32 {
    rt().collect_cycles()
}
#[no_mangle]
pub extern "C" fn oxi_live_bytes() -> u32 {
    rt().live_bytes()
}
#[no_mangle]
pub extern "C" fn oxi_live_objects() -> u32 {
    rt().live_objects()
}
#[no_mangle]
pub extern "C" fn oxi_str_new(ptr: u32, len: u32) -> u32 {
    rt().str_new(ptr, len)
}
#[no_mangle]
pub extern "C" fn oxi_str_concat(a: u32, b: u32) -> u32 {
    rt().str_concat(a, b)
}
#[no_mangle]
pub extern "C" fn oxi_str_eq(a: u32, b: u32) -> u32 {
    rt().str_eq(a, b)
}
#[no_mangle]
pub extern "C" fn oxi_str_cmp(a: u32, b: u32) -> u32 {
    rt().str_cmp(a, b)
}
#[no_mangle]
pub extern "C" fn oxi_str_slice(s: u32, start: u32, end: u32) -> u32 {
    rt().str_slice(s, start, end)
}
#[no_mangle]
pub extern "C" fn oxi_str_from_i64(v: i64) -> u32 {
    rt().str_from_i64(v)
}
#[no_mangle]
pub extern "C" fn oxi_str_from_f64(v: f64) -> u32 {
    rt().str_from_f64(v)
}
#[no_mangle]
pub extern "C" fn oxi_str_from_f32(v: f32) -> u32 {
    rt().str_from_f32(v)
}
#[no_mangle]
pub extern "C" fn oxi_array_new(type_id: u32, len: u32) -> u32 {
    rt().array_new(type_id, len)
}
#[no_mangle]
pub extern "C" fn oxi_array_grow(a: u32, min_cap: u32) {
    rt().array_grow(a, min_cap);
}

/// `oxi_rt_init(typedesc_base, typedesc_count, heap_base)`: must run before
/// any other runtime call (the compiler's `oxi_init` calls it first).
/// `heap_base` is the 8-aligned end of the compiler's static data (`0` keeps
/// the runtime's default start at `__heap_base` in standalone builds; linked
/// modules additionally guard against any `heap_base < static_end`).
/// Calling `oxi_rt_init` again with valid arguments is a harmless no-op once the heap
/// has been used; before that the latest call wins. Any allocation before the first
/// `oxi_rt_init` traps (`T_BAD_TYPEDESC`).
#[no_mangle]
pub extern "C" fn oxi_rt_init(base: u32, count: u32, heap_base: u32) {
    if heap_base != 0 {
        let rt_end = (&raw const __heap_base as usize) as u32;
        let mem = core::arch::wasm32::memory_size(0) as u64 * PAGE_SIZE as u64;
        if !heap_base.is_multiple_of(8) || heap_base < rt_end || u64::from(heap_base) > mem {
            rt().heap.trap(T_BAD_TYPEDESC);
        }
    }
    // SAFETY: single-threaded.
    let (inited, used) = unsafe { (*addr_of!(RT_INITED), *addr_of!(HEAP_USED)) };
    if inited && used {
        // Heap already in use: the installed table stays (a harmless no-op).
        return;
    }
    // First call, or a repeat before anything was allocated (an early call by a
    // host): the latest valid call wins, so the program's own `oxi_init` always
    // installs its own table and floor (BL2-06).
    rt().init(base, count);
    if heap_base != 0 {
        // SAFETY: single-threaded; sets the allocator floor before heap use.
        unsafe {
            let alloc = (*addr_of_mut!(DL)).allocator();
            if alloc.cur.get() < heap_base as usize {
                alloc.cur.set(heap_base as usize);
            }
        }
    }
    // SAFETY: single-threaded.
    unsafe {
        *addr_of_mut!(RT_INITED) = true;
    }
}

// ---- extension exports (not in the ABI table; see crate docs) ----
/// Size of the scratch region (a Wasm global in the ABI; see "Linking model").
#[no_mangle]
pub extern "C" fn oxi_rt_scratch_cap() -> u32 {
    // SAFETY: only the length of the static is read.
    unsafe { (*addr_of!(oxi_scratch_ptr)).0.len() as u32 }
}
/// Bounded collection step; returns `STEP_*` bits.
#[no_mangle]
pub extern "C" fn oxi_gc_step(budget: u32) -> u32 {
    rt().gc_step(budget)
}
#[no_mangle]
pub extern "C" fn oxi_gc_stat(key: u32) -> u32 {
    rt().stat(key)
}
#[no_mangle]
pub extern "C" fn oxi_gc_config(key: u32, value: u32) {
    rt().config(key, value);
}
