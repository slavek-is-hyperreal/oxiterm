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
        let mut cur = self.cur.get();
        if cur == 0 {
            let rt_end = (&raw const __heap_base as usize).saturating_add(7) & !7;
            cur = rt_end;
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

/// Fast size-class free-list allocator in front of `dlmalloc` for small
/// 8-aligned allocations (`8..=256` bytes, 32 classes). Freed small blocks
/// return directly to their class list in O(1) without touching `dlmalloc`;
/// misses carve from 4 KiB slabs obtained from `dlmalloc` (falling back to
/// exact-size `dlmalloc` near OOM so headroom is never wasted).
const SMALL_MAX: u32 = 256;
const SMALL_CLASSES: usize = (SMALL_MAX as usize) / 8;
const SLAB_SIZE: u32 = 4096;

struct SmallPool {
    heads: [u32; SMALL_CLASSES],
    slab_cur: u32,
    slab_end: u32,
}

impl SmallPool {
    const fn new() -> Self {
        SmallPool {
            heads: [0; SMALL_CLASSES],
            slab_cur: 0,
            slab_end: 0,
        }
    }

    #[inline(always)]
    fn alloc_small(&mut self, sz: u32) -> u32 {
        let idx = ((sz - 1) >> 3) as usize;
        // SAFETY: callers only call `alloc_small` with `8 <= sz <= SMALL_MAX` (8-aligned),
        // so `idx < SMALL_CLASSES` (32) always holds; unchecked avoids a panic block in hot path.
        let slot = unsafe { self.heads.get_unchecked_mut(idx) };
        let head = *slot;
        if head != 0 {
            // SAFETY: free blocks in `heads[idx]` are valid 8-aligned allocations >= 8 bytes.
            *slot = unsafe { core::ptr::read(head as usize as *const u32) };
            return head;
        }
        self.alloc_small_slow(sz)
    }

    #[cold]
    #[inline(never)]
    fn alloc_small_slow(&mut self, sz: u32) -> u32 {
        if self.slab_end.wrapping_sub(self.slab_cur) >= sz {
            let p = self.slab_cur;
            self.slab_cur = p + sz;
            return p;
        }
        // Recycle any leftover slab tail into its matching size class.
        let rem = self.slab_end.wrapping_sub(self.slab_cur);
        if (8..=SMALL_MAX).contains(&rem) {
            let ridx = ((rem - 1) >> 3) as usize;
            let p = self.slab_cur;
            // SAFETY: `p` is an 8-aligned slice of `rem` bytes from the current slab.
            unsafe {
                let slot = self.heads.get_unchecked_mut(ridx);
                core::ptr::write(p as usize as *mut u32, *slot);
                *slot = p;
            }
        }
        self.slab_cur = 0;
        self.slab_end = 0;
        // Carve a fresh 4 KiB slab from dlmalloc.
        let slab = dl_malloc(SLAB_SIZE);
        if slab != 0 {
            self.slab_cur = slab + sz;
            self.slab_end = slab + SLAB_SIZE;
            return slab;
        }
        // Near OOM: fall back to an exact-size block so we never waste headroom.
        dl_malloc(sz)
    }

    #[inline(always)]
    fn free_small(&mut self, a: u32, sz: u32) {
        let idx = ((sz - 1) >> 3) as usize;
        // SAFETY: callers only call `free_small` with `8 <= sz <= SMALL_MAX` (8-aligned),
        // so `idx < SMALL_CLASSES` (32) always holds; `a` is an 8-aligned block of `sz >= 8` bytes.
        unsafe {
            let slot = self.heads.get_unchecked_mut(idx);
            core::ptr::write(a as usize as *mut u32, *slot);
            *slot = a;
        }
    }
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
        let sz = (size + 7) & !7;
        if sz <= SMALL_MAX {
            // SAFETY: single-threaded; `a` was returned by `alloc`/`alloc_uninit`.
            unsafe { (*addr_of_mut!(POOL)).free_small(a, sz) };
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
/// Calling `oxi_rt_init` a second time with valid arguments is a harmless no-op.
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
    if unsafe { *addr_of!(RT_INITED) } {
        return;
    }
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
