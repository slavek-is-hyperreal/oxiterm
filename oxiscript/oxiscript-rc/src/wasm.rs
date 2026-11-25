//! The wasm32 guest: the exports of the memory runtime over raw linear memory
//! and dlmalloc. Single instance = single thread, so one global `Rt`.

use crate::heap::Heap;
use crate::rt::Rt;
use core::ptr::{addr_of, addr_of_mut};
use dlmalloc::Dlmalloc;

pub struct WasmHeap;

static mut DL: Dlmalloc = Dlmalloc::new();
static mut RT: Rt<WasmHeap> = Rt::new(WasmHeap);

/// Last trap kind, written by `oxi_trap` just before `unreachable`. Stable
/// Rust cannot define or write a Wasm *global*, so the module part that
/// exports the mutable global `oxi_trap_kind` mirrors this cell (see the
/// crate docs, "Linking model").
#[no_mangle]
pub static mut oxi_trap_cell: u32 = 0;

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
        // SAFETY: single-threaded.
        unsafe { *addr_of_mut!(oxi_trap_cell) = kind };
        core::arch::wasm32::unreachable()
    }
}

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: single-threaded. 10 = TrapKind::Panic
    unsafe { *addr_of_mut!(oxi_trap_cell) = 10 };
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

// ---- extension exports (not in the ABI table; see crate docs) ----

/// `oxi_rt_init(typedesc_base, typedesc_count)`: must run before any other
/// runtime call (the compiler's `oxi_init` calls it first).
#[no_mangle]
pub extern "C" fn oxi_rt_init(base: u32, count: u32) {
    rt().init(base, count);
}
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
