//! The only thing the runtime algorithms know about memory: a u32-addressed
//! linear memory plus a block allocator. The wasm build implements it with raw
//! pointers and dlmalloc; native tests implement it with a checked arena
//! (`sim`), so the exact same algorithm code is fuzzed natively and shipped
//! as wasm.

/// u32-addressed memory and an 8-aligned zeroing block allocator.
pub trait Heap {
    fn ld32(&self, a: u32) -> u32;
    fn st32(&mut self, a: u32, v: u32);
    /// Write a single byte at `a`.
    #[inline]
    fn st8(&mut self, a: u32, v: u8) {
        self.bytes_mut(a, 1)[0] = v;
    }
    /// `len` bytes at `a` (must lie inside the memory).
    fn bytes(&self, a: u32, len: u32) -> &[u8];
    fn bytes_mut(&mut self, a: u32, len: u32) -> &mut [u8];
    /// memmove inside the memory.
    fn copy(&mut self, dst: u32, src: u32, len: u32);
    /// Zeroed, 8-aligned block of at least `size` (> 0) bytes; `0` when
    /// memory is exhausted.
    fn alloc(&mut self, size: u32) -> u32;
    /// Uninitialised, 8-aligned block of at least `size` (> 0) bytes; `0` when
    /// memory is exhausted. Callers overwrite every ABI-defined byte before
    /// exposing the block. Default delegates to [`Self::alloc`].
    #[inline]
    fn alloc_uninit(&mut self, size: u32) -> u32 {
        self.alloc(size)
    }
    /// Free a block returned by `alloc` or `alloc_uninit` (`size` is the size passed to it).
    fn free(&mut self, a: u32, size: u32);
    /// Size of the memory in bytes (used to validate the descriptor table).
    /// Heaps that cannot tell may keep the default (unbounded).
    fn mem_size(&self) -> u64 {
        u64::MAX
    }
    /// Stop the instance with a `TrapKind` code. Never returns.
    fn trap(&mut self, kind: u32) -> !;
}
