#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.M-link round 2: call orders around `oxi_init` / `oxi_rt_init` on linked modules.
//! The BL03 guard covers `oxi_rt_init` only; the ABI says it "must run before any other runtime
//! call", but nothing enforces that. A FAILING test is a finding (docs/stages/8.M-link/bad-actor-2.md).
mod common;
use common::*;
use oxiscript_ir::parse_module;
use oxiscript_wasm::{compile, CompileOptions, RuntimeMode};
use wasmi::{Engine, Linker, Module, Store, Val};
use wasmparser::{DataKind, ExternalKind, Parser, Payload};

const IR: &str = r#"
oxiir 1
module "preinit"
type T = struct { v: i32, name: str }
data @d0 = "hello-literal-0123456789-abcdefghijklmnopqrstuvwxyz"
data @d1 = "second literal ABCDEFGHIJKLMNOPQRSTUVWXYZ-0123456789"
export func @lens() -> i32 {
entry:
  %a: str = str.const @d0
  %n: i32 = str.len %a
  release %a
  %b: str = str.const @d1
  %m: i32 = str.len %b
  release %b
  %p: ref T = alloc T
  %v: i32 = const 7
  setfield.init T.v %p, %v
  %e: str = str.const @d0
  setfield.init T.name %p, %e
  %g: i32 = getfield T.v %p
  release %p
  %r0: i32 = add %n, %m
  %r: i32 = add %r0, %g
  ret %r
}
"#;

fn linked(debug: bool) -> Vec<u8> {
    let m = parse_module(IR).unwrap();
    compile(&m, &CompileOptions { runtime: RuntimeMode::Linked, debug_runtime: debug, allow_raw_exports: true, ..opts() }).unwrap()
}

fn rt_heap_base() -> u32 {
    let bytes = std::fs::read(concat!(env!("OUT_DIR"), "/oxiscript_rc.wasm")).unwrap();
    let mut globals = Vec::new();
    let mut hb = None;
    for p in Parser::new(0).parse_all(&bytes) {
        match p.unwrap() {
            Payload::GlobalSection(r) => {
                for g in r {
                    let g = g.unwrap();
                    let mut ops = g.init_expr.get_operators_reader();
                    globals.push(match ops.read().unwrap() {
                        wasmparser::Operator::I32Const { value } => value as u32,
                        _ => 0,
                    });
                }
            }
            Payload::ExportSection(r) => {
                for e in r {
                    let e = e.unwrap();
                    if e.name == "__heap_base" && e.kind == ExternalKind::Global {
                        hb = Some(e.index);
                    }
                }
            }
            _ => {}
        }
    }
    globals[hb.unwrap() as usize]
}

/// Program data segments (start >= the runtime's own `__heap_base`): (offset, bytes).
fn program_images(wasm: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let rt = rt_heap_base();
    let mut v = vec![];
    for p in Parser::new(0).parse_all(wasm) {
        if let Payload::DataSection(r) = p.unwrap() {
            for d in r {
                let d = d.unwrap();
                if let DataKind::Active { offset_expr, .. } = d.kind {
                    let mut ops = offset_expr.get_operators_reader();
                    if let wasmparser::Operator::I32Const { value } = ops.read().unwrap() {
                        if value as u32 >= rt {
                            v.push((value as u32, d.data.to_vec()));
                        }
                    }
                }
            }
        }
    }
    v
}

struct W {
    store: Store<()>,
    inst: wasmi::Instance,
}
impl W {
    fn new(wasm: &[u8]) -> W {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let linker = <Linker<()>>::new(&engine);
        let inst = linker.instantiate_and_start(&mut store, &module).unwrap();
        W { store, inst }
    }
    fn call(&mut self, name: &str, args: &[i32]) -> Result<Option<i32>, String> {
        let f = self.inst.get_func(&self.store, name).ok_or(format!("no export {name}"))?;
        let n = f.ty(&self.store).results().len();
        let a: Vec<Val> = args.iter().map(|&x| Val::I32(x)).collect();
        let mut out = vec![Val::I32(0); n];
        f.call(&mut self.store, &a, &mut out).map_err(|e| e.to_string())?;
        Ok(out.first().and_then(|v| v.i32()))
    }
    fn global(&self, name: &str) -> i32 {
        self.inst.get_global(&self.store, name).unwrap().get(&self.store).i32().unwrap()
    }
    fn mem(&self) -> &[u8] {
        self.inst.get_memory(&self.store, "memory").unwrap().data(&self.store)
    }
    fn write(&mut self, a: u32, b: &[u8]) {
        let m = self.inst.get_memory(&self.store, "memory").unwrap();
        m.data_mut(&mut self.store)[a as usize..a as usize + b.len()].copy_from_slice(b);
    }
}

/// The host calls an allocating runtime export before `oxi_init` (ABI: `oxi_rt_init` must run
/// first, but nothing enforces it). `WasmSys` seeds its cursor at the RUNTIME's `__heap_base`,
/// which in a linked module is the start of the PROGRAM's static data, and `SmallPool` carves a
/// 4 KiB slab there. `oxi_init` later only raises the cursor: the slab keeps serving small blocks
/// inside program static data (string literals, type descriptors) and they are zero-filled on
/// allocation.
#[test]
fn allocating_before_oxi_init_never_overlaps_program_static_data() {
    // The debug runtime flavour only exists with `--features debug-runtime` (compiling with
    // `debug_runtime: true` is rejected without it); check every flavour that is built.
    let flavours: &[bool] = if cfg!(feature = "debug-runtime") { &[false, true] } else { &[false] };
    for &debug in flavours {
        let wasm = linked(debug);
        let imgs = program_images(&wasm);
        let (lo, hi) = (imgs.iter().map(|i| i.0).min().unwrap(), imgs.iter().map(|i| i.0 + i.1.len() as u32).max().unwrap());
        let mut report = vec![];
        for (first, size) in [("none", 0), ("oxi_alloc_bytes", 16), ("oxi_alloc_bytes", 200), ("oxi_alloc_bytes", 300), ("oxi_alloc_bytes", 5000)] {
            let mut w = W::new(&wasm);
            let pre = if first == "none" { Ok(None) } else { w.call(first, &[size]) };
            let p = pre.clone().ok().flatten().unwrap_or(0) as u32;
            let mut notes = format!("debug={debug} {first}({size}) before oxi_init -> {pre:?}");
            if p != 0 && p < hi && p + size as u32 > lo {
                notes.push_str(&format!(" OVERLAPS program static data [{lo:#x},{hi:#x})"));
            }
            let init = w.call("oxi_init", &[]);
            notes.push_str(&format!("; oxi_init {init:?}"));
            // churn small blocks after init: must come from above static data
            let mut inside = 0;
            for i in 0..400 {
                let sz = 8 + (i % 32) * 8;
                if let Ok(Some(q)) = w.call("oxi_alloc_bytes", &[sz]) {
                    let q = q as u32;
                    if q < hi && q + sz as u32 > lo {
                        inside += 1;
                        w.write(q, &vec![0xEE; sz as usize]);
                    }
                }
            }
            if inside > 0 {
                notes.push_str(&format!("; {inside} post-init blocks INSIDE static data"));
            }
            let corrupted = imgs.iter().any(|(off, b)| w.mem()[*off as usize..*off as usize + b.len()] != b[..]);
            if corrupted {
                notes.push_str("; STATIC DATA CORRUPTED");
            }
            let lens = w.call("lens", &[]);
            notes.push_str(&format!("; lens() {lens:?}"));
            report.push(notes);
        }
        for r in &report {
            eprintln!("{r}");
        }
        assert!(!report[0].contains("INSIDE") && !report[0].contains("CORRUPTED") && report[0].contains("lens() Ok(Some(110))"), "control (no early call) is not clean: {}", report[0]);
        let bad: Vec<_> = report.iter().filter(|r| r.contains("OVERLAPS") || r.contains("INSIDE") || r.contains("CORRUPTED")).collect();
        assert!(bad.is_empty(), "BL2-06: allocation before oxi_init lands in program static data:\n{bad:#?}");
    }
}

/// `oxi_rt_init` called by the host BEFORE `oxi_init` with a valid heap base but a SHORTER type
/// table: `RT_INITED` turns the program's own `oxi_rt_init` into a no-op, so the program runs with
/// the host's table. Expected: the program's own init installs its own table (or the early call
/// traps); documents what happens.
#[test]
fn early_rt_init_with_a_different_table_does_not_survive_oxi_init() {
    let wasm = linked(false);
    let mut w0 = W::new(&wasm);
    w0.call("oxi_init", &[]).unwrap();
    let want = w0.call("lens", &[]).unwrap();
    let imgs = program_images(&wasm);
    let hi = imgs.iter().map(|i| i.0 + i.1.len() as u32).max().unwrap();
    let static_end = (hi + 7) & !7;
    let mut w = W::new(&wasm);
    let (tb, tc) = (w.global("oxi_typedesc_base"), w.global("oxi_typedesc_count"));
    let early = w.call("oxi_rt_init", &[tb, tc - 1, static_end as i32]);
    let init = w.call("oxi_init", &[]);
    let got = w.call("lens", &[]);
    eprintln!("typedesc count {tc}; early oxi_rt_init(count-1) {early:?}; oxi_init {init:?}; lens() {got:?} (want {want:?})");
    assert!(early.is_err() || got == Ok(want), "BL2-06: an early oxi_rt_init with a shorter table is kept by oxi_init: lens() = {got:?}, want {want:?}");
}
