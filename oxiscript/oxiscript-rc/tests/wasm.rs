//! Integration tests of the real wasm build under wasmi.
#![allow(clippy::many_single_char_names, clippy::too_many_lines, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::doc_markdown, clippy::manual_is_multiple_of, clippy::similar_names)]

mod common;

use common::{build_wasm, wasm_available, WasmGuest};
use oxiscript_abi::exports::{self, ExportKind, Provider};
use oxiscript_abi::WasmTy;
use oxiscript_rc::testkit::{fuzz, ty, Guest};
use std::sync::OnceLock;
use wasmi::ExternType;

fn wasm_path() -> &'static std::path::PathBuf {
    static P: OnceLock<std::path::PathBuf> = OnceLock::new();
    P.get_or_init(|| build_wasm("release"))
}

fn conv(t: WasmTy) -> wasmi::ValType {
    match t {
        WasmTy::I32 => wasmi::ValType::I32,
        WasmTy::I64 => wasmi::ValType::I64,
        WasmTy::F32 => wasmi::ValType::F32,
        WasmTy::F64 => wasmi::ValType::F64,
        _ => unreachable!(),
    }
}

#[test]
fn module_imports_only_set_trap_and_exports_every_runtime_row() {
    if !wasm_available() {
        return;
    }
    let g = WasmGuest::load(wasm_path());
    let imps: Vec<_> = g.module.imports().collect();
    assert_eq!(imps.len(), 1, "the runtime must import only oxi_set_trap");
    assert_eq!(imps[0].module(), "oxi:rt");
    assert_eq!(imps[0].name(), "oxi_set_trap");
    match imps[0].ty() {
        ExternType::Func(ft) => {
            assert_eq!(ft.params(), &[wasmi::ValType::I32]);
            assert_eq!(ft.results().len(), 0);
        }
        other => panic!("oxi_set_trap: expected func, got {other:?}"),
    }
    let ex: std::collections::HashMap<String, ExternType> =
        g.module.exports().map(|e| (e.name().to_string(), e.ty().clone())).collect();
    for row in exports::EXPORTS.iter().filter(|r| r.provider == Provider::Runtime) {
        // `oxi_scratch_cap` cannot be a Wasm global from stable Rust (see crate docs);
        // the runtime offers the function `oxi_rt_scratch_cap` and the linker the global.
        if row.name == exports::SCRATCH_CAP {
            assert!(ex.contains_key("oxi_rt_scratch_cap"));
            continue;
        }
        let t = ex.get(row.name).unwrap_or_else(|| panic!("runtime export {} missing", row.name));
        match (row.kind, t) {
            (ExportKind::Func, ExternType::Func(ft)) => {
                let p: Vec<_> = row.params.iter().map(|t| conv(*t)).collect();
                let r: Vec<_> = row.results.iter().map(|t| conv(*t)).collect();
                assert_eq!(ft.params(), &p[..], "params of {}", row.name);
                assert_eq!(ft.results(), &r[..], "results of {}", row.name);
            }
            (ExportKind::Global, ExternType::Global(gt)) => {
                assert_eq!(gt.content(), wasmi::ValType::I32, "{}", row.name);
            }
            (k, t) => panic!("{}: expected {k:?}, found {t:?}", row.name),
        }
    }
    for (name, params, results) in oxiscript_rc::EXTENSION_EXPORTS {
        match ex.get(*name) {
            Some(ExternType::Func(ft)) => {
                assert_eq!((ft.params().len(), ft.results().len()), (*params, *results), "{name}");
            }
            other => panic!("extension export {name}: {other:?}"),
        }
    }
    // only reserved names are exported (plus linker globals)
    for name in ex.keys() {
        assert!(
            exports::is_reserved_export_name(name) || name == "__data_end" || name == "__heap_base",
            "unexpected export {name}"
        );
    }
}

#[test]
fn scratch_region_trap_channel_and_heap_base() {
    if !wasm_available() {
        return;
    }
    let mut g = WasmGuest::standard(wasm_path());
    let p = g.global_i32("oxi_scratch_ptr");
    assert!(p >= 1024 && p % 8 == 0);
    assert_eq!(g.call("oxi_rt_scratch_cap", &[]), 4096);
    assert!(p as usize + 4096 <= g.mem_size());
    // the host may write the scratch region; runtime functions can read it
    g.wr_bytes(p, b"hello scratch");
    let s = g.call("oxi_str_new", &[p, 13]);
    assert_eq!(g.str_read(s), "hello scratch");
    g.release(s);
    // traps: call oxi_set_trap(kind) and execute unreachable
    assert!(g.try_call("oxi_alloc", &[999]).is_err());
    assert_eq!(g.trap_kind(), 1);
    let mut g2 = WasmGuest::standard(wasm_path());
    assert!(g2.try_call("oxi_trap", &[7]).is_err());
    assert_eq!(g2.trap_kind(), 7);
    // bad utf-8
    let mut g3 = WasmGuest::standard(wasm_path());
    let p3 = g3.global_i32("oxi_scratch_ptr");
    g3.wr_bytes(p3, &[0xFF, 0xFE]);
    assert!(g3.try_call("oxi_str_new", &[p3, 2]).is_err());
    assert_eq!(g3.trap_kind(), 9);
    // heap_base: allocator starts at or above the passed static_end
    let mut g4 = WasmGuest::load(wasm_path());
    let rt_hb = g4.global_i32("__heap_base");
    let td_base = (rt_hb + 15) & !15;
    // minimal valid descriptor table: 16 entries, id 1 = string (kind 4, acyclic 1)
    g4.wr(td_base + 16 + 4, 4 | (1 << 8));
    let heap_base = (td_base + 16 * 16 + 256 + 7) & !7;
    g4.call("oxi_rt_init", &[td_base, 16, heap_base]);
    let blk = g4.alloc_bytes(32);
    assert!(blk >= heap_base, "allocation {blk:#x} overlapped static region ending at {heap_base:#x}");
    g4.free_bytes(blk, 32);
    // invalid heap_base below __heap_base traps 64
    let mut g5 = WasmGuest::load(wasm_path());
    assert!(g5.try_call("oxi_rt_init", &[td_base, 16, 1024]).is_err());
    assert_eq!(g5.trap_kind(), 64);
}

#[test]
fn fuzz_wasm_matches_reference_model() {
    if !wasm_available() {
        return;
    }
    let mut total = 0;
    for seed in 0..40 {
        let mut g = WasmGuest::standard(wasm_path());
        let st = fuzz(&mut g, seed, 150);
        total += st.ops;
    }
    eprintln!("wasm fuzz: {total} ops");
}

#[test]
fn long_list_does_not_overflow_the_wasm_stack() {
    if !wasm_available() {
        return;
    }
    let mut g = WasmGuest::standard(wasm_path());
    // 1M nodes in release; the unoptimised wasmi host of a debug `cargo test` is ~30x slower
    let n = if cfg!(debug_assertions) { 100_000 } else { 1_000_000 };
    let mut head = 0;
    for _ in 0..n {
        let node = g.alloc(ty::NODE);
        g.wr(node, head); // a = previous head (ownership moves)
        head = node;
    }
    assert_eq!(g.live_objects(), n);
    g.release(head);
    // destruction is deferred and bounded per call: the queue is not empty yet
    assert!(g.stat(0) > 0 || g.live_objects() > 0);
    g.collect();
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn strings_and_arrays_api() {
    if !wasm_available() {
        return;
    }
    let mut g = WasmGuest::standard(wasm_path());
    let a = g.str_new("héllo");
    let b = g.str_new(" wörld");
    let c = g.call("oxi_str_concat", &[a, b]);
    assert_eq!(g.str_read(c), "héllo wörld");
    assert_eq!(g.call("oxi_str_eq", &[a, a]), 1);
    assert_eq!(g.call("oxi_str_eq", &[a, b]), 0);
    assert_eq!(g.call("oxi_str_cmp", &[a, b]) as i32, 1);
    assert_eq!(g.call("oxi_str_cmp", &[b, a]) as i32, -1);
    let s = g.call("oxi_str_slice", &[c, 0, 6]);
    assert_eq!(g.str_read(s), "héllo");
    assert!(g.try_call("oxi_str_slice", &[c, 0, 2]).is_err(), "mid-code-point slice must trap");
    let i = g.call_vals("oxi_str_from_i64", &[wasmi::Val::I64(i64::MIN)]).unwrap()[0].i32().unwrap() as u32;
    assert_eq!(g.str_read(i), "-9223372036854775808");
    for (v, want) in [(1.0, "1"), (0.1, "0.1"), (-0.0, "-0"), (1e21, "1e21"), (f64::NAN, "NaN"), (f64::NEG_INFINITY, "-Infinity")] {
        let r = g.call_vals("oxi_str_from_f64", &[wasmi::Val::F64(v.into())]).unwrap()[0].i32().unwrap() as u32;
        assert_eq!(g.str_read(r), want);
        g.release(r);
    }
    let arr = g.array_new(ty::SCALARARRAY, 3);
    g.array_grow(arr, 100);
    assert!(g.rd(arr + 4) >= 100);
    let cl = g.clone_obj(arr);
    assert_ne!(cl, arr);
    for p in [a, b, c, s, i, arr, cl] {
        g.release(p);
    }
    g.collect();
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn debug_wasm_build_with_poisoning_passes_the_same_fuzz() {
    if !wasm_available() {
        return;
    }
    // debug_assertions poison freed memory (0xDD); a stale read would then show garbage.
    let path = build_wasm("dev");
    let mut total = 0;
    for seed in 100..120 {
        let mut g = WasmGuest::standard(&path);
        total += fuzz(&mut g, seed, 120).ops;
    }
    eprintln!("wasm debug fuzz: {total} ops");
}

#[test]
fn debug_wasm_build_traps_65_on_double_release_and_non_objects() {
    if !wasm_available() {
        return;
    }
    let path = build_wasm("dev");
    let mut g = WasmGuest::standard(&path);
    let a = g.alloc(ty::NODE);
    let _guard = g.alloc(ty::NODE);
    g.release(a);
    assert!(g.try_call("oxi_release", &[a]).is_err());
    assert_eq!(g.trap_kind(), 65, "double release");
    let mut g = WasmGuest::standard(&path);
    let b = g.alloc_bytes(64);
    assert!(g.try_call("oxi_release", &[b + 8]).is_err());
    assert_eq!(g.trap_kind(), 65, "release of an alloc_bytes block");
    // the descriptor table is validated by the real wasm too
    let mut g = WasmGuest::load(&path);
    let t = g.alloc_bytes(32);
    g.wr(t + 16, 16); // id 1: size 16 ...
    g.wr(t + 20, 1); // ... kind 1 (struct) where id 1 must be the string
    assert!(g.try_call("oxi_rt_init", &[t, 2, 0]).is_err());
    assert_eq!(g.trap_kind(), 64, "bad descriptor table");
}

#[test]
fn one_million_node_ring_is_collected_under_a_64k_stack() {
    if !wasm_available() {
        return;
    }
    let mut g = WasmGuest::standard(wasm_path());
    let n = if cfg!(debug_assertions) { 100_000usize } else { 1_000_000 };
    let nodes: Vec<u32> = (0..n).map(|_| g.alloc(ty::NODE)).collect();
    for i in 0..n {
        let next = nodes[(i + 1) % n];
        g.retain(next);
        g.wr(nodes[i], next);
    }
    for &p in &nodes {
        g.release(p);
    }
    assert_eq!(g.live_objects() as usize, n);
    assert_eq!(g.collect() as usize, n);
    assert_eq!(g.live_objects(), 0);
}

#[test]
fn initial_linear_memory_is_small() {
    if !wasm_available() {
        return;
    }
    let g = WasmGuest::load(wasm_path());
    let pages = g.memory.ty(&g.store).minimum();
    eprintln!("initial memory: {pages} pages ({} KiB)", pages * 64);
    assert!(pages <= 3, "64 KiB shadow stack + data + 1 page: {pages} pages");
}
