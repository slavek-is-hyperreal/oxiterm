//! A **test stub** of the runtime-support exports (slice 8.M builds the real
//! one). It is just good enough that generated modules load in
//! `oxiscript-runtime` and run: a bump allocator (memory is never reused),
//! real reference counts with child release through the type descriptors,
//! exact live-object counting for leak checks, UTF-8 validation in
//! `oxi_str_new`, and the string/array primitives the code generator calls.
//! `oxi_str_from_i64`/`oxi_str_from_f64` and cycle collection are stubs.

use crate::body::{Body, Mem};
use oxiscript_abi::layout::{META_COLOUR_SHIFT, RC_STICKY};
use oxiscript_abi::trap::TrapKind;
use wasm_encoder::{BlockType, Function, ValType};

/// Wasm global indices fixed by the module layout (see `lib.rs`).
pub(crate) const G_TRAP: u32 = 0;
pub(crate) const G_TDBASE: u32 = 2;
pub(crate) const G_TDCOUNT: u32 = 3;
pub(crate) const G_HEAP: u32 = 6;
pub(crate) const G_LIVE_OBJS: u32 = 7;
pub(crate) const G_LIVE_BYTES: u32 = 8;

/// The defined runtime functions of the stub, in function-index order.
pub(crate) const FUNCS: &[&str] = &[
    "oxi_alloc",
    "oxi_alloc_bytes",
    "oxi_free_bytes",
    "oxi_retain",
    "oxi_release",
    "oxi_clone",
    "oxi_is_unique",
    "oxi_collect_cycles",
    "oxi_live_bytes",
    "oxi_live_objects",
    "oxi_str_new",
    "oxi_str_concat",
    "oxi_str_eq",
    "oxi_str_cmp",
    "oxi_str_slice",
    "oxi_str_from_i64",
    "oxi_str_from_f64",
    "oxi_array_new",
    "oxi_array_grow",
    "oxi_trap",
    // internal helpers (not exported)
    "bump",
    "free_obj",
    "new_str",
    "utf8_ok",
];

pub(crate) fn is_internal(name: &str) -> bool {
    matches!(name, "bump" | "free_obj" | "new_str" | "utf8_ok")
}

/// Signature of a stub function: (params, results).
pub(crate) fn sig(name: &str) -> (Vec<ValType>, Vec<ValType>) {
    use ValType::{F64, I32, I64};
    match name {
        "bump" | "oxi_alloc" | "oxi_alloc_bytes" | "oxi_clone" | "new_str" => (vec![I32], vec![I32]),
        "oxi_is_unique" => (vec![I32], vec![I32]),
        "oxi_free_bytes" | "oxi_array_grow" => (vec![I32, I32], vec![]),
        "oxi_retain" | "oxi_release" | "free_obj" | "oxi_trap" => (vec![I32], vec![]),
        "oxi_collect_cycles" | "oxi_live_bytes" | "oxi_live_objects" => (vec![], vec![I32]),
        "oxi_str_new" | "oxi_str_concat" | "oxi_str_eq" | "oxi_str_cmp" | "utf8_ok" | "oxi_array_new" => {
            (vec![I32, I32], vec![I32])
        }
        "oxi_str_slice" => (vec![I32, I32, I32], vec![I32]),
        "oxi_str_from_i64" => (vec![I64], vec![I32]),
        "oxi_str_from_f64" => (vec![F64], vec![I32]),
        _ => (vec![], vec![]),
    }
}

fn trap(b: &mut Body, k: TrapKind) {
    b.i32c(k.code() as i32);
    b.s().global_set(G_TRAP).unreachable();
}

fn trap_if(b: &mut Body, k: TrapKind) {
    b.s().if_(BlockType::Empty);
    trap(b, k);
    b.s().end();
}

/// Build the body of one stub function; `ix` maps a stub name to its function index.
#[allow(clippy::too_many_lines)]
pub(crate) fn build(name: &str, ix: &dyn Fn(&str) -> u32) -> Function {
    use ValType::{I32, I64};
    let (params, _) = sig(name);
    let mut body = Body::new(params.len());
    let b = &mut body;
    match name {
        "bump" => {
            // n -> ptr (8-aligned, zeroed because memory only ever grows)
            let (n, p, end, cur) = (0, b.local(I32), b.local(I32), b.local(I64));
            b.get(n).i32c(0x7fff_ff00);
            b.s().i32_gt_u();
            trap_if(b, TrapKind::OutOfMemory);
            b.get(n).i32c(7);
            b.s().i32_add();
            b.i32c(-8);
            b.s().i32_and();
            b.set(n);
            b.s().global_get(G_HEAP);
            b.set(p);
            b.get(p).get(n);
            b.s().i32_add();
            b.set(end);
            b.get(end).get(p);
            b.s().i32_lt_u();
            trap_if(b, TrapKind::OutOfMemory);
            b.s().memory_size(0).i64_extend_i32_u();
            b.i64c(16);
            b.s().i64_shl();
            b.set(cur);
            b.get(end);
            b.s().i64_extend_i32_u();
            b.get(cur);
            b.s().i64_gt_u().if_(BlockType::Empty);
            // pages = ((end - cur) + 65535) >> 16
            b.get(end);
            b.s().i64_extend_i32_u();
            b.get(cur);
            b.s().i64_sub();
            b.i64c(65535);
            b.s().i64_add();
            b.i64c(16);
            b.s().i64_shr_u().i32_wrap_i64().memory_grow(0);
            b.i32c(-1);
            b.s().i32_eq();
            trap_if(b, TrapKind::OutOfMemory);
            b.s().end();
            b.get(end);
            b.s().global_set(G_HEAP);
            b.get(p);
        }
        "oxi_alloc" => {
            let (tid, desc, size, p) = (0, b.local(I32), b.local(I32), b.local(I32));
            b.get(tid);
            b.s().global_get(G_TDCOUNT).i32_ge_u();
            trap_if(b, TrapKind::Unreachable);
            b.s().global_get(G_TDBASE);
            b.get(tid).i32c(4);
            b.s().i32_shl().i32_add();
            b.set(desc);
            b.get(desc).load(Mem::I32, 0);
            b.set(size);
            b.get(size).i32c(8);
            b.s().i32_add().call(ix("bump"));
            b.set(p);
            b.get(p).i32c(1);
            b.store(Mem::I32, 0);
            // meta = tid | (acyclic << 26)   (green = colour 4 << 24)
            b.get(p).get(tid);
            b.get(desc).load(Mem::I32, 4);
            b.i32c(8);
            b.s().i32_shr_u();
            b.i32c(1);
            b.s().i32_and();
            b.i32c(META_COLOUR_SHIFT as i32 + 2);
            b.s().i32_shl().i32_or();
            b.store(Mem::I32, 4);
            b.s().global_get(G_LIVE_OBJS);
            b.i32c(1);
            b.s().i32_add().global_set(G_LIVE_OBJS);
            b.s().global_get(G_LIVE_BYTES);
            b.get(size).i32c(8);
            b.s().i32_add().i32_add().global_set(G_LIVE_BYTES);
            b.get(p).i32c(8);
            b.s().i32_add();
        }
        "oxi_alloc_bytes" => {
            let n = 0;
            // like the real runtime: a zero-sized block is the null pointer
            b.get(n);
            b.s().i32_eqz().if_(BlockType::Empty).i32_const(0).return_().end();
            b.s().global_get(G_LIVE_BYTES);
            b.get(n).i32c(7);
            b.s().i32_add();
            b.i32c(-8);
            b.s().i32_and().i32_add().global_set(G_LIVE_BYTES);
            b.get(n).s().call(ix("bump"));
        }
        "oxi_free_bytes" => {
            b.s().global_get(G_LIVE_BYTES);
            b.get(1).i32c(7);
            b.s().i32_add();
            b.i32c(-8);
            b.s().i32_and().i32_sub().global_set(G_LIVE_BYTES);
        }
        "oxi_retain" => {
            let (p, rc) = (0, b.local(I32));
            b.get(p);
            b.s().i32_eqz().if_(BlockType::Empty).return_().end();
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.load(Mem::I32, 0);
            b.set(rc);
            b.get(rc).i32c(RC_STICKY as i32);
            b.s().i32_ge_u().if_(BlockType::Empty).return_().end();
            b.get(p).i32c(8);
            b.s().i32_sub();
            // rc == 0x7fffffff -> sticky, else rc + 1
            b.get(rc).i32c(0x7fff_ffff);
            b.s().i32_eq().if_(BlockType::Result(I32));
            b.i32c(RC_STICKY as i32);
            b.s().else_();
            b.get(rc).i32c(1);
            b.s().i32_add().end();
            b.store(Mem::I32, 0);
        }
        "oxi_release" => {
            let (p, rc) = (0, b.local(I32));
            b.get(p);
            b.s().i32_eqz().if_(BlockType::Empty).return_().end();
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.load(Mem::I32, 0);
            b.set(rc);
            b.get(rc).i32c(RC_STICKY as i32);
            b.s().i32_ge_u().if_(BlockType::Empty).return_().end();
            b.get(rc);
            b.s().i32_eqz();
            trap_if(b, TrapKind::Unreachable);
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.get(rc).i32c(1);
            b.s().i32_sub(); b.tee(rc);
            b.store(Mem::I32, 0);
            b.get(rc);
            b.s().i32_eqz().if_(BlockType::Empty);
            b.get(p).s().call(ix("free_obj"));
            b.s().end();
        }
        "free_obj" => {
            let p = 0;
            let desc = b.local(I32);
            let w1 = b.local(I32);
            let kind = b.local(I32);
            let pc = b.local(I32);
            let map = b.local(I32);
            let len = b.local(I32);
            let data = b.local(I32);
            let elem = b.local(I32);
            let i = b.local(I32);
            let k = b.local(I32);
            let total = b.local(I32);
            // iterative free (L06): children that reach rc 0 are chained through their (dead) rc word instead of
            // recursing into `oxi_release`; `head` is the pending list, so the native stack stays flat for any depth.
            let head = b.local(I32);
            let q = b.local(I32);
            let rc = b.local(I32);
            b.get(p);
            b.set(head);
            b.i32c(0).set(rc);
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.i32c(0);
            b.store(Mem::I32, 0);
            b.s().block(BlockType::Empty).loop_(BlockType::Empty);
            b.get(head);
            b.s().i32_eqz().br_if(1);
            b.get(head);
            b.set(p);
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.load(Mem::I32, 0);
            b.set(head);
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.i32c(0);
            b.store(Mem::I32, 0);
            desc_of(b, p, desc);
            b.get(desc).load(Mem::I32, 4);
            b.set(w1);
            b.get(w1).i32c(0xff);
            b.s().i32_and();
            b.set(kind);
            b.get(w1).i32c(16);
            b.s().i32_shr_u();
            b.set(pc);
            b.get(desc).load(Mem::I32, 8);
            b.set(map);
            b.get(kind).i32c(5);
            b.s().i32_eq().if_(BlockType::Empty);
            {
                b.get(p).load(Mem::I32, 0);
                b.set(len);
                b.get(p).load(Mem::I32, 8);
                b.set(data);
                b.get(desc).load(Mem::I32, 0);
                b.set(elem);
                b.get(pc);
                b.s().if_(BlockType::Empty);
                b.i32c(0);
                b.set(i);
                b.s().block(BlockType::Empty).loop_(BlockType::Empty);
                b.get(i).get(len);
                b.s().i32_ge_u().br_if(1);
                b.i32c(0);
                b.set(k);
                b.s().block(BlockType::Empty).loop_(BlockType::Empty);
                b.get(k).get(pc);
                b.s().i32_ge_u().br_if(1);
                b.get(data).get(i).get(elem);
                b.s().i32_mul().i32_add();
                b.get(map).get(k).i32c(2);
                b.s().i32_shl().i32_add();
                b.load(Mem::I32, 0);
                b.s().i32_add();
                b.load(Mem::I32, 0);
                b.set(q);
                release_child(b, q, rc, head);
                b.get(k).i32c(1);
                b.s().i32_add();
                b.set(k);
                b.s().br(0).end().end();
                b.get(i).i32c(1);
                b.s().i32_add();
                b.set(i);
                b.s().br(0).end().end();
                b.s().end();
                // free the data buffer
                b.get(data);
                b.get(p).load(Mem::I32, 4).get(elem);
                b.s().i32_mul().call(ix("oxi_free_bytes"));
                b.i32c(24);
                b.set(total);
            }
            b.s().else_();
            {
                b.get(kind).i32c(4);
                b.s().i32_eq().if_(BlockType::Empty);
                // string: object = 8 + round8(8 + len + 1)
                b.get(p).load(Mem::I32, 0);
                b.i32c(9 + 7 + 8);
                b.s().i32_add();
                b.i32c(-8);
                b.s().i32_and();
                b.set(total);
                b.s().else_();
                b.i32c(0);
                b.set(k);
                b.s().block(BlockType::Empty).loop_(BlockType::Empty);
                b.get(k).get(pc);
                b.s().i32_ge_u().br_if(1);
                b.get(p);
                b.get(map).get(k).i32c(2);
                b.s().i32_shl().i32_add();
                b.load(Mem::I32, 0);
                b.s().i32_add();
                b.load(Mem::I32, 0);
                b.set(q);
                release_child(b, q, rc, head);
                b.get(k).i32c(1);
                b.s().i32_add();
                b.set(k);
                b.s().br(0).end().end();
                b.get(desc).load(Mem::I32, 0);
                b.i32c(8);
                b.s().i32_add();
                b.set(total);
                b.s().end();
            }
            b.s().end();
            b.s().global_get(G_LIVE_OBJS);
            b.i32c(1);
            b.s().i32_sub().global_set(G_LIVE_OBJS);
            b.s().global_get(G_LIVE_BYTES);
            b.get(total);
            b.s().i32_sub().global_set(G_LIVE_BYTES);
            b.s().br(0).end().end();
        }
        "oxi_clone" => {
            let p = 0;
            let desc = b.local(I32);
            let w1 = b.local(I32);
            let kind = b.local(I32);
            let pc = b.local(I32);
            let map = b.local(I32);
            let len = b.local(I32);
            let elem = b.local(I32);
            let q = b.local(I32);
            let i = b.local(I32);
            let k = b.local(I32);
            let nd = b.local(I32);
            desc_of(b, p, desc);
            b.get(desc).load(Mem::I32, 4);
            b.set(w1);
            b.get(w1).i32c(0xff);
            b.s().i32_and();
            b.set(kind);
            b.get(w1).i32c(16);
            b.s().i32_shr_u();
            b.set(pc);
            b.get(desc).load(Mem::I32, 8);
            b.set(map);
            b.get(kind).i32c(5);
            b.s().i32_eq().if_(BlockType::Empty);
            {
                b.get(p).load(Mem::I32, 0);
                b.set(len);
                b.get(desc).load(Mem::I32, 0);
                b.set(elem);
                b.get(p).i32c(8);
                b.s().i32_sub();
                b.load(Mem::I32, 4);
                b.i32c(0x00ff_ffff);
                b.s().i32_and();
                b.get(len).s().call(ix("oxi_array_new"));
                b.set(q);
                b.get(q).load(Mem::I32, 8);
                b.set(nd);
                b.get(nd);
                b.get(p).load(Mem::I32, 8);
                b.get(len).get(elem);
                b.s().i32_mul().memory_copy(0, 0);
                b.get(pc);
                b.s().if_(BlockType::Empty);
                b.i32c(0);
                b.set(i);
                b.s().block(BlockType::Empty).loop_(BlockType::Empty);
                b.get(i).get(len);
                b.s().i32_ge_u().br_if(1);
                b.i32c(0);
                b.set(k);
                b.s().block(BlockType::Empty).loop_(BlockType::Empty);
                b.get(k).get(pc);
                b.s().i32_ge_u().br_if(1);
                b.get(nd).get(i).get(elem);
                b.s().i32_mul().i32_add();
                b.get(map).get(k).i32c(2);
                b.s().i32_shl().i32_add();
                b.load(Mem::I32, 0);
                b.s().i32_add();
                b.load(Mem::I32, 0);
                b.s().call(ix("oxi_retain"));
                b.get(k).i32c(1);
                b.s().i32_add();
                b.set(k);
                b.s().br(0).end().end();
                b.get(i).i32c(1);
                b.s().i32_add();
                b.set(i);
                b.s().br(0).end().end();
                b.s().end();
                b.get(q);
                b.s().return_();
            }
            b.s().end();
            b.get(kind).i32c(4);
            b.s().i32_eq().if_(BlockType::Empty);
            b.get(p).i32c(8);
            b.s().i32_add();
            b.get(p).load(Mem::I32, 0);
            b.s().call(ix("oxi_str_new")).return_().end();
            // record / closure / union variant
            b.get(p).i32c(8);
            b.s().i32_sub();
            b.load(Mem::I32, 4);
            b.i32c(0x00ff_ffff);
            b.s().i32_and().call(ix("oxi_alloc"));
            b.set(q);
            b.get(q).get(p);
            b.get(desc).load(Mem::I32, 0);
            b.s().memory_copy(0, 0);
            b.i32c(0);
            b.set(k);
            b.s().block(BlockType::Empty).loop_(BlockType::Empty);
            b.get(k).get(pc);
            b.s().i32_ge_u().br_if(1);
            b.get(q);
            b.get(map).get(k).i32c(2);
            b.s().i32_shl().i32_add();
            b.load(Mem::I32, 0);
            b.s().i32_add();
            b.load(Mem::I32, 0);
            b.s().call(ix("oxi_retain"));
            b.get(k).i32c(1);
            b.s().i32_add();
            b.set(k);
            b.s().br(0).end().end();
            b.get(q);
        }
        "oxi_is_unique" => {
            b.get(0).i32c(8);
            b.s().i32_sub();
            b.load(Mem::I32, 0);
            b.i32c(1);
            b.s().i32_eq();
        }
        "oxi_collect_cycles" => {
            b.i32c(0);
        }
        "oxi_live_bytes" => {
            b.s().global_get(G_LIVE_BYTES);
        }
        "oxi_live_objects" => {
            b.s().global_get(G_LIVE_OBJS);
        }
        "new_str" => {
            // len -> string payload pointer (rc 1, len and terminating NUL written)
            let (len, tot, p) = (0, b.local(I32), b.local(I32));
            b.get(len).i32c(0x7fff_ff00);
            b.s().i32_gt_u();
            trap_if(b, TrapKind::OutOfMemory);
            b.get(len).i32c(9 + 7 + 8);
            b.s().i32_add();
            b.i32c(-8);
            b.s().i32_and();
            b.set(tot);
            b.get(tot).s().call(ix("bump"));
            b.set(p);
            b.get(p).i32c(1);
            b.store(Mem::I32, 0);
            b.get(p).i32c(1 | (4 << META_COLOUR_SHIFT));
            b.store(Mem::I32, 4);
            b.get(p).get(len);
            b.store(Mem::I32, 8);
            b.get(p).get(len);
            b.s().i32_add();
            b.i32c(0);
            b.store(Mem::U8, 16);
            b.s().global_get(G_LIVE_OBJS);
            b.i32c(1);
            b.s().i32_add().global_set(G_LIVE_OBJS);
            b.s().global_get(G_LIVE_BYTES);
            b.get(tot);
            b.s().i32_add().global_set(G_LIVE_BYTES);
            b.get(p).i32c(8);
            b.s().i32_add();
        }
        "utf8_ok" => {
            let (ptr, len) = (0, 1);
            let i = b.local(I32);
            let c = b.local(I32);
            let need = b.local(I32);
            let lo = b.local(I32);
            let hi = b.local(I32);
            let j = b.local(I32);
            b.s().block(BlockType::Result(I32)).loop_(BlockType::Empty);
            // if i >= len -> 1
            b.get(i).get(len);
            b.s().i32_ge_u().if_(BlockType::Empty);
            b.i32c(1);
            b.s().br(2).end();
            b.get(ptr).get(i);
            b.s().i32_add();
            b.load(Mem::U8, 0);
            b.set(c);
            // ascii
            b.get(c).i32c(0x80);
            b.s().i32_lt_u().if_(BlockType::Empty);
            b.get(i).i32c(1);
            b.s().i32_add();
            b.set(i);
            b.s().br(1).end();
            // lead byte classes
            b.get(c).i32c(0xC2);
            b.s().i32_lt_u().if_(BlockType::Empty);
            b.i32c(0);
            b.s().br(2).end();
            b.i32c(0x80);
            b.set(lo);
            b.i32c(0xBF);
            b.set(hi);
            b.get(c).i32c(0xE0);
            b.s().i32_lt_u().if_(BlockType::Empty);
            b.i32c(1);
            b.set(need);
            b.s().else_();
            b.get(c).i32c(0xF0);
            b.s().i32_lt_u().if_(BlockType::Empty);
            b.i32c(2);
            b.set(need);
            // E0: A0..BF ; ED: 80..9F
            b.get(c).i32c(0xE0);
            b.s().i32_eq().if_(BlockType::Empty);
            b.i32c(0xA0);
            b.set(lo);
            b.s().end();
            b.get(c).i32c(0xED);
            b.s().i32_eq().if_(BlockType::Empty);
            b.i32c(0x9F);
            b.set(hi);
            b.s().end();
            b.s().else_();
            b.get(c).i32c(0xF4);
            b.s().i32_gt_u().if_(BlockType::Empty);
            b.i32c(0);
            b.s().br(4).end();
            b.i32c(3);
            b.set(need);
            b.get(c).i32c(0xF0);
            b.s().i32_eq().if_(BlockType::Empty);
            b.i32c(0x90);
            b.set(lo);
            b.s().end();
            b.get(c).i32c(0xF4);
            b.s().i32_eq().if_(BlockType::Empty);
            b.i32c(0x8F);
            b.set(hi);
            b.s().end();
            b.s().end();
            b.s().end();
            // need bytes must exist: i + need < len
            b.get(i).get(need);
            b.s().i32_add();
            b.get(len);
            b.s().i32_ge_u().if_(BlockType::Empty);
            b.i32c(0);
            b.s().br(2).end();
            // second byte range
            b.get(ptr).get(i);
            b.s().i32_add();
            b.load(Mem::U8, 1);
            b.set(c);
            b.get(c).get(lo);
            b.s().i32_lt_u();
            b.get(c).get(hi);
            b.s().i32_gt_u().i32_or().if_(BlockType::Empty);
            b.i32c(0);
            b.s().br(2).end();
            // remaining continuation bytes
            b.i32c(2);
            b.set(j);
            b.s().block(BlockType::Empty).loop_(BlockType::Empty);
            b.get(j).get(need);
            b.s().i32_gt_u().br_if(1);
            b.get(ptr).get(i);
            b.s().i32_add();
            b.get(j);
            b.s().i32_add();
            b.load(Mem::U8, 0);
            b.i32c(0xC0);
            b.s().i32_and();
            b.i32c(0x80);
            b.s().i32_ne().if_(BlockType::Empty);
            b.i32c(0);
            b.s().br(5).end();
            b.get(j).i32c(1);
            b.s().i32_add();
            b.set(j);
            b.s().br(0).end().end();
            b.get(i).get(need);
            b.s().i32_add();
            b.i32c(1);
            b.s().i32_add();
            b.set(i);
            b.s().br(0).end();
            b.s().i32_const(0).end();
        }
        "oxi_str_new" => {
            let (ptr, len, r) = (0, 1, b.local(I32));
            b.get(ptr).get(len).s().call(ix("utf8_ok"));
            b.s().i32_eqz();
            trap_if(b, TrapKind::BadUtf8);
            b.get(len).s().call(ix("new_str"));
            b.set(r);
            b.get(r).i32c(8);
            b.s().i32_add();
            b.get(ptr).get(len);
            b.s().memory_copy(0, 0);
            b.get(r);
        }
        "oxi_str_concat" => {
            let (a, bb) = (0, 1);
            let (la, lb, r) = (b.local(I32), b.local(I32), b.local(I32));
            b.get(a).load(Mem::I32, 0);
            b.set(la);
            b.get(bb).load(Mem::I32, 0);
            b.set(lb);
            b.get(la).get(lb);
            b.s().i32_add().call(ix("new_str"));
            b.set(r);
            b.get(r).i32c(8);
            b.s().i32_add();
            b.get(a).i32c(8);
            b.s().i32_add();
            b.get(la);
            b.s().memory_copy(0, 0);
            b.get(r).i32c(8);
            b.s().i32_add();
            b.get(la);
            b.s().i32_add();
            b.get(bb).i32c(8);
            b.s().i32_add();
            b.get(lb);
            b.s().memory_copy(0, 0);
            b.get(r);
        }
        "oxi_str_eq" | "oxi_str_cmp" => {
            let (a, bb) = (0, 1);
            let (la, lb, n, i, x, y) = (
                b.local(I32),
                b.local(I32),
                b.local(I32),
                b.local(I32),
                b.local(I32),
                b.local(I32),
            );
            let is_eq = name == "oxi_str_eq";
            b.get(a).load(Mem::I32, 0);
            b.set(la);
            b.get(bb).load(Mem::I32, 0);
            b.set(lb);
            if is_eq {
                b.get(la).get(lb);
                b.s().i32_ne().if_(BlockType::Empty);
                b.i32c(0);
                b.s().return_().end();
            }
            // n = min(la, lb)
            b.get(la).get(lb).get(la).get(lb);
            b.s().i32_lt_u().select();
            b.set(n);
            b.s().block(BlockType::Empty).loop_(BlockType::Empty);
            b.get(i).get(n);
            b.s().i32_ge_u().br_if(1);
            b.get(a).get(i);
            b.s().i32_add();
            b.load(Mem::U8, 8);
            b.set(x);
            b.get(bb).get(i);
            b.s().i32_add();
            b.load(Mem::U8, 8);
            b.set(y);
            b.get(x).get(y);
            b.s().i32_ne().if_(BlockType::Empty);
            if is_eq {
                b.i32c(0);
            } else {
                b.i32c(-1).i32c(1).get(x).get(y);
                b.s().i32_lt_u().select();
            }
            b.s().return_().end();
            b.get(i).i32c(1);
            b.s().i32_add();
            b.set(i);
            b.s().br(0).end().end();
            if is_eq {
                b.i32c(1);
            } else {
                // equal prefix: compare lengths -> -1/0/1
                b.get(la).get(lb);
                b.s().i32_gt_u();
                b.get(la).get(lb);
                b.s().i32_lt_u().i32_sub();
            }
        }
        "oxi_str_slice" => {
            let (s, start, end) = (0, 1, 2);
            let (len, r) = (b.local(I32), b.local(I32));
            b.get(s).load(Mem::I32, 0);
            b.set(len);
            b.get(start).get(end);
            b.s().i32_gt_u();
            b.get(end).get(len);
            b.s().i32_gt_u().i32_or();
            trap_if(b, TrapKind::Bounds);
            for pos in [start, end] {
                // a boundary inside the string must not be a continuation byte
                b.get(pos).get(len);
                b.s().i32_lt_u().if_(BlockType::Empty);
                b.get(s).get(pos);
                b.s().i32_add();
                b.load(Mem::U8, 8);
                b.i32c(0xC0);
                b.s().i32_and();
                b.i32c(0x80);
                b.s().i32_eq();
                trap_if(b, TrapKind::BadUtf8);
                b.s().end();
            }
            b.get(end).get(start);
            b.s().i32_sub().call(ix("new_str"));
            b.set(r);
            b.get(r).i32c(8);
            b.s().i32_add();
            b.get(s).i32c(8);
            b.s().i32_add();
            b.get(start);
            b.s().i32_add();
            b.get(end).get(start);
            b.s().i32_sub().memory_copy(0, 0);
            b.get(r);
        }
        "oxi_str_from_i64" | "oxi_str_from_f64" => {
            trap(b, TrapKind::Unreachable);
        }
        "oxi_array_new" => {
            let (tid, len) = (0, 1);
            let (desc, elem, bytes, obj, data) = (
                b.local(I32),
                b.local(I32),
                b.local(I64),
                b.local(I32),
                b.local(I32),
            );
            b.get(len).i32c(0);
            b.s().i32_lt_s();
            trap_if(b, TrapKind::Bounds);
            b.get(tid);
            b.s().global_get(G_TDCOUNT).i32_ge_u();
            trap_if(b, TrapKind::Unreachable);
            b.s().global_get(G_TDBASE);
            b.get(tid).i32c(4);
            b.s().i32_shl().i32_add();
            b.set(desc);
            b.get(desc).load(Mem::I32, 0);
            b.set(elem);
            b.get(len);
            b.s().i64_extend_i32_u();
            b.get(elem);
            b.s().i64_extend_i32_u().i64_mul();
            b.set(bytes);
            b.get(bytes).i64c(0x7fff_ffff);
            b.s().i64_gt_u();
            trap_if(b, TrapKind::OutOfMemory);
            b.i32c(24).s().call(ix("bump"));
            b.set(obj);
            b.get(obj).i32c(1);
            b.store(Mem::I32, 0);
            b.get(obj).get(tid);
            b.get(desc).load(Mem::I32, 4);
            b.i32c(8);
            b.s().i32_shr_u();
            b.i32c(1);
            b.s().i32_and();
            b.i32c(META_COLOUR_SHIFT as i32 + 2);
            b.s().i32_shl().i32_or();
            b.store(Mem::I32, 4);
            b.get(bytes);
            b.s().i32_wrap_i64().call(ix("oxi_alloc_bytes"));
            b.set(data);
            b.get(obj).get(len);
            b.store(Mem::I32, 8);
            b.get(obj).get(len);
            b.store(Mem::I32, 12);
            b.get(obj).get(data);
            b.store(Mem::I32, 16);
            b.s().global_get(G_LIVE_OBJS);
            b.i32c(1);
            b.s().i32_add().global_set(G_LIVE_OBJS);
            b.s().global_get(G_LIVE_BYTES);
            b.i32c(24);
            b.s().i32_add().global_set(G_LIVE_BYTES);
            b.get(obj).i32c(8);
            b.s().i32_add();
        }
        "oxi_array_grow" => {
            let (arr, min_cap) = (0, 1);
            let (cap, newcap, len, data, elem, bytes, nd) = (
                b.local(I32),
                b.local(I32),
                b.local(I32),
                b.local(I32),
                b.local(I32),
                b.local(I64),
                b.local(I32),
            );
            b.get(arr).load(Mem::I32, 4);
            b.set(cap);
            b.get(cap).get(min_cap);
            b.s().i32_ge_u().if_(BlockType::Empty).return_().end();
            b.get(min_cap).i32c(0x7fff_ffff);
            b.s().i32_gt_u();
            trap_if(b, TrapKind::OutOfMemory);
            // newcap = max(min_cap, cap * 2, 4), capped at 0x7fffffff
            b.get(cap).i32c(1);
            b.s().i32_shl();
            b.set(newcap);
            b.get(newcap).get(min_cap).get(newcap).get(min_cap);
            b.s().i32_gt_u().select();
            b.set(newcap);
            b.get(newcap).i32c(4).get(newcap).i32c(4);
            b.s().i32_gt_u().select();
            b.set(newcap);
            b.get(newcap).i32c(0x7fff_ffff);
            b.s().i32_gt_u().if_(BlockType::Empty);
            b.get(min_cap);
            b.set(newcap);
            b.s().end();
            b.get(arr).load(Mem::I32, 0);
            b.set(len);
            b.get(arr).load(Mem::I32, 8);
            b.set(data);
            // elem size from the descriptor
            b.s().global_get(G_TDBASE);
            b.get(arr).i32c(8);
            b.s().i32_sub();
            b.load(Mem::I32, 4);
            b.i32c(0x00ff_ffff);
            b.s().i32_and();
            b.i32c(4);
            b.s().i32_shl().i32_add();
            b.load(Mem::I32, 0);
            b.set(elem);
            b.get(newcap);
            b.s().i64_extend_i32_u();
            b.get(elem);
            b.s().i64_extend_i32_u().i64_mul();
            b.set(bytes);
            b.get(bytes).i64c(0x7fff_ffff);
            b.s().i64_gt_u();
            trap_if(b, TrapKind::OutOfMemory);
            b.get(bytes);
            b.s().i32_wrap_i64().call(ix("oxi_alloc_bytes"));
            b.set(nd);
            b.get(nd).get(data);
            b.get(len).get(elem);
            b.s().i32_mul().memory_copy(0, 0);
            b.get(data);
            b.get(cap).get(elem);
            b.s().i32_mul().call(ix("oxi_free_bytes"));
            b.get(arr).get(nd);
            b.store(Mem::I32, 8);
            b.get(arr).get(newcap);
            b.store(Mem::I32, 4);
        }
        "oxi_trap" => {
            b.get(0);
            b.s().global_set(G_TRAP).unreachable();
        }
        other => unreachable!("unknown stub function {other}"),
    }
    body.finish()
}

/// Drop one reference to `q` inside the iterative `free_obj`: a child that dies is pushed on the pending list
/// `head` (linked through its own dead rc word) instead of being freed recursively.
fn release_child(b: &mut Body, q: u32, rc: u32, head: u32) {
    b.get(q);
    b.s().i32_eqz().if_(BlockType::Empty).else_();
    {
        b.get(q).i32c(8);
        b.s().i32_sub();
        b.load(Mem::I32, 0);
        b.set(rc);
        b.get(rc).i32c(RC_STICKY as i32);
        b.s().i32_lt_u().if_(BlockType::Empty);
        {
            b.get(rc);
            b.s().i32_eqz();
            trap_if(b, TrapKind::Unreachable);
            b.get(rc).i32c(1);
            b.s().i32_sub();
            b.set(rc);
            b.get(q).i32c(8);
            b.s().i32_sub();
            b.get(rc);
            b.s().i32_eqz().if_(BlockType::Result(ValType::I32));
            // dead: the rc word becomes the link to the previous head
            b.get(head);
            b.s().else_();
            b.get(rc);
            b.s().end();
            b.store(Mem::I32, 0);
            b.get(rc);
            b.s().i32_eqz().if_(BlockType::Empty);
            b.get(q);
            b.set(head);
            b.s().end();
        }
        b.s().end();
    }
    b.s().end();
}

/// `desc = typedesc_base + (meta(p) & 0xffffff) * 16`.
fn desc_of(b: &mut Body, p: u32, desc: u32) {
    b.s().global_get(G_TDBASE);
    b.get(p).i32c(8);
    b.s().i32_sub();
    b.load(Mem::I32, 4);
    b.i32c(0x00ff_ffff);
    b.s().i32_and();
    b.i32c(4);
    b.s().i32_shl().i32_add();
    b.set(desc);
}
