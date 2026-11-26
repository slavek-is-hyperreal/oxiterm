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

static mut DL: Dlmalloc<WasmSys> = Dlmalloc::new_with_allocator(WasmSys { cur: Cell::new(0) });
static mut RT: Rt<WasmHeap> = Rt::new(WasmHeap);

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
    fn alloc(&mut self, size: u32) -> u32 {
        // SAFETY: single-threaded; valid size/align.
        unsafe { (*addr_of_mut!(DL)).calloc(size as usize, 8) as usize as u32 }
    }
    fn free(&mut self, a: u32, size: u32) {
        // SAFETY: `a` was returned by `alloc`.
        unsafe { (*addr_of_mut!(DL)).free(a as usize as *mut u8, size as usize, 8) }
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
/// the runtime's default start at `__heap_base`).
#[no_mangle]
pub extern "C" fn oxi_rt_init(base: u32, count: u32, heap_base: u32) {
    if heap_base != 0 {
        let rt_end = (&raw const __heap_base as usize) as u32;
        let mem = core::arch::wasm32::memory_size(0) as u64 * PAGE_SIZE as u64;
        if !heap_base.is_multiple_of(8) || heap_base < rt_end || u64::from(heap_base) > mem {
            rt().heap.trap(T_BAD_TYPEDESC);
        }
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
