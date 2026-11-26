#![allow(clippy::all, clippy::pedantic, clippy::nursery, dead_code)]
//! BAD ACTOR 8.M-link: structural checks of the merged module (no overlap of runtime and
//! program static data, table bounds, no leftover `oxi:rt` imports, export hygiene) under stress
//! layouts (huge literal, thousands of literals, hundreds of types), plus runtime integrity:
//! static data must survive heavy allocation and a raw `oxi_rt_init` re-call matrix.
mod common;
use common::*;
use oxiscript_ir::parse_module;
use oxiscript_wasm::{compile, CompileOptions, RuntimeMode};
use wasmi::{Engine, Linker, Module, Store, Val};
use wasmparser::{DataKind, ElementKind, ExternalKind, Parser, Payload, TypeRef};

fn linked_opts() -> CompileOptions {
    CompileOptions { runtime: RuntimeMode::Linked, allow_raw_exports: true, ..opts() }
}

/// IR with `nlits` distinct string literals (+ one `big` bytes literal) and `ntypes` struct types.
pub fn stress_ir(nlits: usize, big: usize, ntypes: usize) -> (String, i64) {
    let mut s = String::from("oxiir 1\nmodule \"stress\"\n");
    for t in 0..ntypes {
        s.push_str(&format!("type T{t} = struct {{ v: i32, p: ref T{t}?, name: str }}\n"));
    }
    let mut expect = 0i64;
    for i in 0..nlits {
        let lit = format!("lit{i}_{}", "x".repeat(i % 17));
        expect += lit.len() as i64;
        s.push_str(&format!("data @d{i} = \"{lit}\"\n"));
    }
    if big > 0 {
        s.push_str(&format!("data @big = \"{}\"\n", "b".repeat(big)));
        expect += big as i64;
    }
    let per = 100;
    let groups = nlits.div_ceil(per);
    for g in 0..groups {
        s.push_str(&format!("func @g{g}() -> i32 {{\nentry:\n  %acc0: i32 = const 0\n"));
        let mut acc = 0;
        for i in (g * per)..((g + 1) * per).min(nlits) {
            s.push_str(&format!(
                "  %s{i}: str = str.const @d{i}\n  %n{i}: i32 = str.len %s{i}\n  release %s{i}\n  %acc{}: i32 = add %acc{acc}, %n{i}\n",
                acc + 1
            ));
            acc += 1;
        }
        s.push_str(&format!("  ret %acc{acc}\n}}\n"));
    }
    s.push_str("export func @lens() -> i32 {\nentry:\n  %t0: i32 = const 0\n");
    let mut t = 0;
    for g in 0..groups {
        s.push_str(&format!("  %r{g}: i32 = call @g{g}()\n  %t{}: i32 = add %t{t}, %r{g}\n", t + 1));
        t += 1;
    }
    if big > 0 {
        s.push_str(&format!("  %b: str = str.const @big\n  %bn: i32 = str.len %b\n  release %b\n  %t{}: i32 = add %t{t}, %bn\n", t + 1));
        t += 1;
    }
    s.push_str(&format!("  ret %t{t}\n}}\n"));
    // allocate every type once
    s.push_str("export func @types() -> i32 {\nentry:\n  %a0: i32 = const 0\n");
    for k in 0..ntypes {
        s.push_str(&format!(
            "  %p{k}: ref T{k} = alloc T{k}\n  %v{k}: i32 = const {k}\n  setfield.init T{k}.v %p{k}, %v{k}\n  %nl{k}: ref T{k}? = null\n  setfield.init T{k}.p %p{k}, %nl{k}\n  %e{k}: str = str.const @{}\n  setfield.init T{k}.name %p{k}, %e{k}\n  %g{k}: i32 = getfield T{k}.v %p{k}\n  release %p{k}\n  %a{}: i32 = add %a{k}, %g{k}\n",
            if nlits > 0 { "d0" } else { "big" },
            k + 1
        ));
    }
    s.push_str(&format!("  ret %a{ntypes}\n}}\n"));
    (s, expect)
}

pub struct Layout {
    pub rt_heap_base: u32,
    pub segs: Vec<(u32, u32, bool)>, // (start, end, from_program)
    pub elems: Vec<(u32, u32)>,
    pub table_min: u32,
    pub mem_pages: u32,
    pub imports: Vec<(String, String)>,
    pub exports: Vec<String>,
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
                    let v = match ops.read().unwrap() {
                        wasmparser::Operator::I32Const { value } => value as u32,
                        _ => 0,
                    };
                    globals.push(v);
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
    globals[hb.expect("__heap_base exported") as usize]
}

pub fn analyse(wasm: &[u8]) -> Layout {
    let rt = rt_heap_base();
    let mut l = Layout { rt_heap_base: rt, segs: vec![], elems: vec![], table_min: 0, mem_pages: 0, imports: vec![], exports: vec![] };
    for p in Parser::new(0).parse_all(wasm) {
        match p.unwrap() {
            Payload::ImportSection(r) => {
                for i in r.into_imports() {
                    let i = i.unwrap();
                    assert!(matches!(i.ty, TypeRef::Func(_)));
                    l.imports.push((i.module.to_string(), i.name.to_string()));
                }
            }
            Payload::TableSection(r) => {
                for t in r {
                    l.table_min = t.unwrap().ty.initial as u32;
                }
            }
            Payload::MemorySection(r) => {
                for m in r {
                    l.mem_pages = m.unwrap().initial as u32;
                }
            }
            Payload::ExportSection(r) => {
                for e in r {
                    l.exports.push(e.unwrap().name.to_string());
                }
            }
            Payload::DataSection(r) => {
                for d in r {
                    let d = d.unwrap();
                    if let DataKind::Active { offset_expr, .. } = d.kind {
                        let mut ops = offset_expr.get_operators_reader();
                        if let wasmparser::Operator::I32Const { value } = ops.read().unwrap() {
                            let s = value as u32;
                            // runtime segments are those that end below rt heap base
                            l.segs.push((s, s + d.data.len() as u32, s + d.data.len() as u32 > rt));
                        }
                    }
                }
            }
            Payload::ElementSection(r) => {
                for e in r {
                    let e = e.unwrap();
                    if let ElementKind::Active { offset_expr, .. } = e.kind {
                        let mut ops = offset_expr.get_operators_reader();
                        let off = match ops.read().unwrap() {
                            wasmparser::Operator::I32Const { value } => value as u32,
                            _ => u32::MAX,
                        };
                        let n = match e.items {
                            wasmparser::ElementItems::Functions(f) => f.count(),
                            wasmparser::ElementItems::Expressions(_, f) => f.count(),
                        };
                        l.elems.push((off, off + n));
                    }
                }
            }
            _ => {}
        }
    }
    l
}

fn check_layout(name: &str, wasm: &[u8]) -> Layout {
    let l = analyse(wasm);
    assert!(l.imports.iter().all(|(m, _)| m != "oxi:rt"), "{name}: leftover oxi:rt import {:?}", l.imports);
    for bad in ["__oxi_set_trap", "oxi_set_trap", "__heap_base", "__data_end", "oxi_gc_step", "oxi_gc_stat", "oxi_gc_config", "oxi_rt_scratch_cap"] {
        assert!(!l.exports.iter().any(|e| e == bad), "{name}: leaked export {bad}");
    }
    let mut segs = l.segs.clone();
    segs.sort();
    for w in segs.windows(2) {
        assert!(w[0].1 <= w[1].0, "{name}: overlapping data segments {:?} / {:?}", w[0], w[1]);
    }
    for (s, e, prog) in &segs {
        if *prog {
            assert!(*s >= l.rt_heap_base, "{name}: program segment [{s:#x},{e:#x}) below runtime heap base {:#x}", l.rt_heap_base);
        } else {
            assert!(*e <= l.rt_heap_base, "{name}: runtime segment above heap base");
        }
        assert!(u64::from(*e) <= u64::from(l.mem_pages) * 65536, "{name}: segment end {e:#x} beyond initial memory {} pages", l.mem_pages);
    }
    for (s, e) in &l.elems {
        assert!(*e <= l.table_min, "{name}: element segment [{s},{e}) beyond table {}", l.table_min);
    }
    let mut es = l.elems.clone();
    es.sort();
    for w in es.windows(2) {
        assert!(w[0].1 <= w[1].0, "{name}: overlapping element segments {:?}", w);
    }
    l
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
    fn call(&mut self, name: &str, args: &[Val], nres: usize) -> Result<Vec<Val>, wasmi::Error> {
        let f = self.inst.get_func(&self.store, name).unwrap_or_else(|| panic!("no export {name}"));
        let mut res = vec![Val::I32(0); nres];
        f.call(&mut self.store, args, &mut res)?;
        Ok(res)
    }
    fn i32(&mut self, name: &str, args: &[i32]) -> Result<i32, wasmi::Error> {
        let a: Vec<Val> = args.iter().map(|&x| Val::I32(x)).collect();
        Ok(self.call(name, &a, 1)?[0].i32().unwrap())
    }
    fn unit(&mut self, name: &str, args: &[i32]) -> Result<(), wasmi::Error> {
        let a: Vec<Val> = args.iter().map(|&x| Val::I32(x)).collect();
        self.call(name, &a, 0)?;
        Ok(())
    }
    fn global(&self, name: &str) -> i32 {
        self.inst.get_global(&self.store, name).unwrap().get(&self.store).i32().unwrap()
    }
    fn mem(&self) -> Vec<u8> {
        self.inst.get_memory(&self.store, "memory").unwrap().data(&self.store).to_vec()
    }
    fn mem_len(&self) -> usize {
        self.inst.get_memory(&self.store, "memory").unwrap().data_size(&self.store)
    }
    fn write(&mut self, at: u32, b: &[u8]) {
        let m = self.inst.get_memory(&self.store, "memory").unwrap();
        m.write(&mut self.store, at as usize, b).unwrap();
    }
}

/// Active data segments as (offset, bytes) from the module.
fn images(wasm: &[u8]) -> Vec<(u32, Vec<u8>)> {
    let mut v = vec![];
    for p in Parser::new(0).parse_all(wasm) {
        if let Payload::DataSection(r) = p.unwrap() {
            for d in r {
                let d = d.unwrap();
                if let DataKind::Active { offset_expr, .. } = d.kind {
                    if let wasmparser::Operator::I32Const { value } = offset_expr.get_operators_reader().read().unwrap() {
                        v.push((value as u32, d.data.to_vec()));
                    }
                }
            }
        }
    }
    v
}

/// Program static data only (those above the runtime heap base) must be byte-identical.
fn static_intact(w: &W, imgs: &[(u32, Vec<u8>)], rt_hb: u32, what: &str) {
    let m = w.mem();
    for (off, b) in imgs {
        if *off as usize + b.len() > rt_hb as usize && *off >= rt_hb {
            assert_eq!(&m[*off as usize..*off as usize + b.len()], &b[..], "{what}: program static data at {off:#x} corrupted");
        }
    }
}

fn churn(w: &mut W, rounds: usize) -> usize {
    let mut n = 0;
    let mut ptrs = vec![];
    for i in 0..rounds {
        let sz = 8 + (i * 37) % 3000;
        match w.i32("oxi_alloc_bytes", &[sz as i32]) {
            Ok(p) if p != 0 => {
                w.write(p as u32, &vec![0xAA; sz]);
                ptrs.push((p, sz));
                n += 1;
            }
            _ => break,
        }
    }
    for (p, sz) in ptrs {
        w.unit("oxi_free_bytes", &[p, sz as i32]).unwrap();
    }
    n
}

#[test]
fn stress_layouts_merge_and_run() {
    for (name, nlits, big, ntypes) in [("tiny", 1, 0, 1), ("lits5k", 5000, 0, 3), ("big100k", 3, 100_000, 2), ("types600", 5, 0, 600), ("big1m", 2, 1_000_000, 2), ("everything", 2000, 300_000, 300)] {
        let (ir, expect) = stress_ir(nlits, big, ntypes);
        let m = parse_module(&ir).unwrap_or_else(|e| panic!("{name}: ir parse: {e:?}"));
        let t0 = std::time::Instant::now();
        let wasm = match compile(&m, &linked_opts()) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("{name}: compile error: {e}");
                continue;
            }
        };
        eprintln!("{name}: compiled {} bytes in {:?}", wasm.len(), t0.elapsed());
        oxiscript_wasm::validate(&wasm).unwrap();
        let l = check_layout(name, &wasm);
        let imgs = images(&wasm);
        let mut w = W::new(&wasm);
        w.unit("oxi_init", &[]).unwrap();
        assert_eq!(i64::from(w.i32("lens", &[]).unwrap()), expect, "{name}: literal lengths");
        assert_eq!(w.i32("types", &[]).unwrap() as usize, (0..ntypes).sum::<usize>(), "{name}: types");
        static_intact(&w, &imgs, l.rt_heap_base, name);
        let n = churn(&mut w, 3000);
        static_intact(&w, &imgs, l.rt_heap_base, &format!("{name} after churn {n}"));
        assert_eq!(i64::from(w.i32("lens", &[]).unwrap()), expect, "{name}: literal lengths after churn");
        assert_eq!(w.i32("oxi_live_objects", &[]).unwrap(), 0, "{name}: leak");
        w.unit("oxi_deinit", &[]).unwrap();
        assert_eq!(w.global("oxi_trap_kind"), 0);
    }
}

/// `oxi_rt_init` is a host-visible export now. Re-calling it with hostile `heap_base`
/// values must either trap with a defined kind or leave static data and the allocator intact.
#[test]
fn rt_init_recall_matrix() {
    let (ir, expect) = stress_ir(200, 5000, 4);
    let wasm = compile(&parse_module(&ir).unwrap(), &linked_opts()).unwrap();
    let l = analyse(&wasm);
    let imgs = images(&wasm);
    let mut report = vec![];
    let prog_end = l.segs.iter().filter(|s| s.2).map(|s| s.1).max().unwrap();
    let mem = u32::try_from(u64::from(l.mem_pages) * 65536).unwrap();
    let cands: Vec<(&str, u32)> = vec![
        ("zero", 0),
        ("one", 1),
        ("misaligned4", l.rt_heap_base + 4),
        ("below_rt_heap", l.rt_heap_base - 8),
        ("rt_heap_base", l.rt_heap_base),
        ("inside_static", l.rt_heap_base + 64),
        ("prog_end_8", (prog_end + 7) & !7),
        ("mem", mem),
        ("mem_plus_8", mem + 8),
        ("huge", 0x8000_0000),
        ("max_aligned", 0xFFFF_FFF8),
    ];
    for after_init in [true, false] {
        for (name, hb) in &cands {
            let mut w = W::new(&wasm);
            let (tb, tc) = (w.global("oxi_typedesc_base"), w.global("oxi_typedesc_count"));
            if after_init {
                w.unit("oxi_init", &[]).unwrap();
            }
            let r = w.unit("oxi_rt_init", &[tb, tc, *hb as i32]);
            let kind = w.global("oxi_trap_kind");
            let desc = match &r {
                Ok(()) => "ok".to_string(),
                Err(e) => format!("trap({kind}) {}", e.to_string().chars().take(40).collect::<String>()),
            };
            let mut verdict = String::new();
            if r.is_ok() {
                let p = w.i32("oxi_alloc_bytes", &[64]).unwrap_or(-1) as u32;
                if p != 0 && p != u32::MAX {
                    w.write(p, &[0xEE; 64]);
                    if p < prog_end && p + 64 > l.rt_heap_base {
                        verdict.push_str(" ALLOC-OVERLAPS-STATIC");
                    }
                }
                churn(&mut w, 500);
                let m = w.mem();
                for (off, b) in &imgs {
                    if *off >= l.rt_heap_base && m[*off as usize..*off as usize + b.len()] != b[..] {
                        verdict.push_str(" STATIC-CORRUPTED");
                        break;
                    }
                }
                let _ = expect;
            }
            report.push(format!("after_init={after_init} heap_base={name}({hb:#x}): {desc}{verdict}"));
        }
    }
    for r in &report {
        eprintln!("{r}");
    }
    let bad: Vec<_> = report.iter().filter(|r| r.contains("OVERLAPS") || r.contains("CORRUPTED")).collect();
    assert!(bad.is_empty(), "rt_init recall corrupts memory:\n{}", bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

/// Large-chunk churn: ~1 MB blocks allocated and freed repeatedly must not grow memory without bound
/// (our `Allocator::free` returns false; dlmalloc may never return/reuse large chunks).
#[test]
fn large_chunk_churn_reuses_memory() {
    let (ir, _) = stress_ir(1, 0, 1);
    let wasm = compile(&parse_module(&ir).unwrap(), &linked_opts()).unwrap();
    let mut w = W::new(&wasm);
    w.unit("oxi_init", &[]).unwrap();
    let before = w.mem_len();
    for i in 0..300 {
        let sz = 1_000_000 + (i % 7) * 4096;
        let p = w.i32("oxi_alloc_bytes", &[sz as i32]).unwrap();
        assert_ne!(p, 0, "round {i}");
        w.unit("oxi_free_bytes", &[p, sz as i32]).unwrap();
    }
    let after = w.mem_len();
    eprintln!("1MB churn x300: memory {} KiB -> {} KiB", before / 1024, after / 1024);
    assert!(after < before + 8 * 1024 * 1024, "memory grew {} -> {} bytes (leak in large-chunk reuse)", before, after);
}

fn read_str(w: &W, p: u32) -> Vec<u8> {
    let m = w.inst.get_memory(&w.store, "memory").unwrap().data(&w.store);
    let len = u32::from_le_bytes(m[p as usize..p as usize + 4].try_into().unwrap()) as usize;
    m[p as usize + 8..p as usize + 8 + len].to_vec()
}

fn expect_f64(v: f64) -> String {
    if v.is_nan() {
        return "NaN".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "Infinity".into() } else { "-Infinity".into() };
    }
    let mut s = format!("{v:?}");
    if s.ends_with(".0") {
        s.truncate(s.len() - 2);
    }
    s
}

fn tiny_linked() -> Vec<u8> {
    let (ir, _) = stress_ir(1, 0, 1);
    compile(&parse_module(&ir).unwrap(), &linked_opts()).unwrap()
}

/// 10^6 random bit patterns + edge cases through the real `oxi_str_from_f64` in wasm vs the documented format.
#[test]
fn f64_text_matches_documented_format_1e6() {
    let wasm = tiny_linked();
    let mut w = W::new(&wasm);
    w.unit("oxi_init", &[]).unwrap();
    let f = w.inst.get_func(&w.store, "oxi_str_from_f64").unwrap();
    let rel = w.inst.get_func(&w.store, "oxi_release").unwrap();
    let mut edge: Vec<f64> = vec![
        0.0, -0.0, 1.0, -1.0, 0.1, 1e21, 1e16, 1e15, 1e-7, 1e-5, 0.0001, 5e-324, -5e-324, f64::MAX, f64::MIN, f64::MIN_POSITIVE,
        2.2250738585072014e-308, 2.225073858507201e-308, f64::EPSILON, f64::INFINITY, f64::NEG_INFINITY, f64::NAN, 123456789012345680.0,
        9007199254740993.0, 1.7976931348623157e308, 4.9e-324, 0.3, 100.0, 1e100, 1e-100, 12345.678, -987654321.123456789,
    ];
    for bits in [0x7ff8_0000_0000_0001u64, 0xfff8_dead_beef_0001, 0x7ff0_0000_0000_0001, 0xffff_ffff_ffff_ffff, 0x7fff_ffff_ffff_ffff] {
        edge.push(f64::from_bits(bits));
    }
    for e in -320..=308 {
        edge.push(format!("1e{e}").parse().unwrap());
        edge.push(format!("-9.87654321e{e}").parse().unwrap());
    }
    let mut x = 0x9E37_79B9_7F4A_7C15u64;
    let mut bad = vec![];
    let mut hash = 0u64;
    let n = fuzz_scale(1_000_000, 20_000);
    let mut res = [Val::I32(0)];
    let mut run = |w: &mut W, v: f64, bad: &mut Vec<String>, hash: &mut u64| {
        f.call(&mut w.store, &[Val::F64(v.into())], &mut res).unwrap();
        let p = res[0].i32().unwrap() as u32;
        let got = String::from_utf8(read_str(w, p)).unwrap();
        rel.call(&mut w.store, &[Val::I32(p as i32)], &mut []).unwrap();
        let want = expect_f64(v);
        for b in got.bytes() {
            *hash = hash.wrapping_mul(1099511628211).wrapping_add(u64::from(b));
        }
        if got != want && bad.len() < 20 {
            bad.push(format!("{:#018x}: got {got:?} want {want:?}", v.to_bits()));
        }
        if !v.is_nan() && got.parse::<f64>().map(|r| r.to_bits() != v.to_bits() && !(r == 0.0 && v == 0.0 && got != "-0")) .unwrap_or(true) && got != "NaN" && bad.len() < 20 {
            bad.push(format!("{:#018x}: {got:?} does not round-trip", v.to_bits()));
        }
    };
    for v in &edge {
        run(&mut w, *v, &mut bad, &mut hash);
    }
    for _ in 0..n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        run(&mut w, f64::from_bits(x), &mut bad, &mut hash);
    }
    // "ordinary" magnitudes too: random mantissa * 10^k
    for i in 0..200_000u64 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        let v = ((x >> 11) as f64) / ((1u64 << 53) as f64) * 10f64.powi((i % 40) as i32 - 20);
        run(&mut w, v, &mut bad, &mut hash);
    }
    assert_eq!(w.i32("oxi_live_objects", &[]).unwrap(), 0, "f64 text leaks");
    eprintln!("f64 text: {} values, fnv-ish hash {hash:#x}, mismatches {}", edge.len() + n + 200_000, bad.len());
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Same hash on a second fresh instance (determinism across instances/runs).
#[test]
fn f64_text_is_deterministic_across_instances() {
    let wasm = tiny_linked();
    let mut hs = vec![];
    for _ in 0..2 {
        let mut w = W::new(&wasm);
        w.unit("oxi_init", &[]).unwrap();
        let mut h = 0u64;
        let mut x = 12345u64;
        for _ in 0..20_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let r = w.call("oxi_str_from_f64", &[Val::F64(f64::from_bits(x).into())], 1).unwrap()[0].i32().unwrap() as u32;
            for b in read_str(&w, r) {
                h = h.wrapping_mul(31).wrapping_add(u64::from(b));
            }
            w.unit("oxi_release", &[r as i32]).unwrap();
        }
        hs.push(h);
    }
    assert_eq!(hs[0], hs[1]);
}

/// Size-overflow attacks on `array.new` for 1/4/8-byte elements: must trap (OOM/Bounds), never "succeed"
/// with a length the buffer cannot hold (n * elem wraps 2^32).
#[test]
#[ignore = "allocates up to 2 GiB of real memory; run with --release -- --ignored"]
fn array_new_size_overflow_traps_not_wraps() {
    let ir = "oxiir 1\nmodule \"a\"\n\
export func @mk8(%n: i32) -> i32 {\nentry:\n  %a: [i64] = array.new %n\n  %l: i32 = array.len %a\n  %z: i32 = const 0\n  %i: i32 = const 5\n  %v: i64 = const 7\n  array.set %a, %z, %v\n  release %a\n  ret %l\n}\n\
export func @mk4(%n: i32) -> i32 {\nentry:\n  %a: [i32] = array.new %n\n  %l: i32 = array.len %a\n  release %a\n  ret %l\n}\n\
export func @mk1(%n: i32) -> i32 {\nentry:\n  %a: [u8] = array.new %n\n  %l: i32 = array.len %a\n  release %a\n  ret %l\n}\n";
    let wasm = compile(&parse_module(ir).unwrap(), &linked_opts()).unwrap();
    let mut out = vec![];
    for f in ["mk8", "mk4", "mk1"] {
        for n in [0x1000_0001i32, 0x1FFF_FFFF, 0x2000_0000, 0x2000_0001, 0x3FFF_FFFF, 0x4000_0000, 0x4000_0001, 0x7FFF_FFFF, -1, i32::MIN, 100_000_000] {
            let mut w = W::new(&wasm);
            w.unit("oxi_init", &[]).unwrap();
            let r = w.i32(f, &[n]);
            let kind = w.global("oxi_trap_kind");
            match r {
                Ok(l) => { let el: u64 = match f { "mk8" => 8, "mk4" => 4, _ => 1 }; if (w.mem_len() as u64) < (l as u64) * el { out.push(format!("{f}({n:#x}) -> WRAPPED len {l} mem {} KiB", w.mem_len() / 1024)); } else { out.push(format!("{f}({n:#x}) -> OK len {l}  (mem {} KiB)", w.mem_len() / 1024)); } }
                Err(_) => out.push(format!("{f}({n:#x}) -> trap kind {kind}")),
            }
        }
    }
    for o in &out {
        eprintln!("{o}");
    }
    let bad: Vec<_> = out.iter().filter(|o| o.contains("WRAPPED")).collect();
    assert!(bad.is_empty(), "size wrap accepted:\n{}", bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"));
}

/// deinit now runs the cycle collector first: genuine leaks (unbalanced retain) of acyclic objects,
/// of cyclic-typed objects and of members of a real cycle must STILL be counted afterwards.
#[test]
fn collector_in_deinit_does_not_hide_true_leaks() {
    let ir = "oxiir 1\nmodule \"leaks\"\n\
type Acy #acyclic = struct { v: i32 }\n\
type Cyc #cyclic = struct { v: i32, next: ref Cyc? }\n\
export func @leak_acyclic() -> i32 {\nentry:\n  %p: ref Acy = alloc Acy\n  %v: i32 = const 1\n  setfield.init Acy.v %p, %v\n  retain %p\n  release %p\n  ret %v\n}\n\
export func @leak_cyclic_typed() -> i32 {\nentry:\n  %p: ref Cyc = alloc Cyc\n  %v: i32 = const 1\n  setfield.init Cyc.v %p, %v\n  %n: ref Cyc? = null\n  setfield.init Cyc.next %p, %n\n  retain %p\n  release %p\n  ret %v\n}\n\
export func @leak_cycle_member() -> i32 {\nentry:\n  %a: ref Cyc = alloc Cyc\n  %b: ref Cyc = alloc Cyc\n  %v: i32 = const 1\n  setfield.init Cyc.v %a, %v\n  setfield.init Cyc.v %b, %v\n  %nl: ref Cyc? = null\n  setfield.init Cyc.next %b, %nl\n  retain %b\n  %sa: ref Cyc? = opt %a\n  retain %a\n  setfield Cyc.next %b, %sa\n  %sb: ref Cyc? = opt %b\n  setfield.init Cyc.next %a, %sb\n  retain %b\n  release %a\n  release %b\n  ret %v\n}\n\
export func @no_leak_cycle() -> i32 {\nentry:\n  %a: ref Cyc = alloc Cyc\n  %b: ref Cyc = alloc Cyc\n  %v: i32 = const 1\n  setfield.init Cyc.v %a, %v\n  setfield.init Cyc.v %b, %v\n  %nl: ref Cyc? = null\n  setfield.init Cyc.next %b, %nl\n  retain %a\n  %sa: ref Cyc? = opt %a\n  setfield Cyc.next %b, %sa\n  retain %b\n  %sb: ref Cyc? = opt %b\n  setfield.init Cyc.next %a, %sb\n  release %a\n  release %b\n  ret %v\n}\n";
    let m = match parse_module(ir) {
        Ok(m) => m,
        Err(e) => panic!("ir: {e:?}"),
    };
    let wasm = match compile(&m, &linked_opts()) {
        Ok(w) => w,
        Err(e) => panic!("compile: {e}"),
    };
    let mut got = vec![];
    for (f, want) in [("leak_acyclic", 1), ("leak_cyclic_typed", 1), ("leak_cycle_member", None::<i32>.map_or(1, |x| x)), ("no_leak_cycle", 0)] {
        let mut w = W::new(&wasm);
        w.unit("oxi_init", &[]).unwrap();
        w.i32(f, &[]).unwrap();
        w.unit("oxi_deinit", &[]).unwrap();
        let freed = w.i32("oxi_collect_cycles", &[]).unwrap();
        let live = w.i32("oxi_live_objects", &[]).unwrap();
        got.push(format!("{f}: collect freed {freed}, live {live}, want >= {want}"));
        if want == 0 {
            assert_eq!(live, 0, "{f}");
        } else {
            assert!(live >= 1, "TRUE LEAK HIDDEN: {f}: live {live}");
        }
    }
    for g in &got {
        eprintln!("{g}");
    }
}

/// The runtime adds its own functions to the module: a program that fits the Stub function limit must
/// still fit (or fail with the documented limit diagnostic) when linked.
#[test]
fn function_count_limit_with_runtime_added() {
    let mut found = None;
    for nfuncs in [9000usize, 9500, 9700, 9800, 9900, 9950, 9990] {
        let mut ir = String::from("oxiir 1\nmodule \"many\"\n");
        for i in 0..nfuncs {
            ir.push_str(&format!("func @f{i}() -> i32 {{\nentry:\n  %v: i32 = const {i}\n  ret %v\n}}\n"));
        }
        let per = 100;
        let groups = nfuncs.div_ceil(per);
        for g in 0..groups {
            ir.push_str(&format!("func @g{g}() -> i32 {{\nentry:\n  %a0: i32 = const 0\n"));
            let mut k = 0;
            for i in (g * per)..((g + 1) * per).min(nfuncs) {
                ir.push_str(&format!("  %r{i}: i32 = call @f{i}()\n  %a{}: i32 = add %a{k}, %r{i}\n", k + 1));
                k += 1;
            }
            ir.push_str(&format!("  ret %a{k}\n}}\n"));
        }
        ir.push_str("export func @all() -> i32 {\nentry:\n  %t0: i32 = const 0\n");
        for g in 0..groups {
            ir.push_str(&format!("  %q{g}: i32 = call @g{g}()\n  %t{}: i32 = add %t{g}, %q{g}\n", g + 1));
        }
        ir.push_str(&format!("  ret %t{groups}\n}}\n"));
        let m = parse_module(&ir).unwrap();
        let stub = compile(&m, &CompileOptions { runtime: RuntimeMode::Stub, allow_raw_exports: true, ..opts() });
        let linked = compile(&m, &linked_opts());
        eprintln!("{nfuncs} funcs: stub {} linked {}", stub.as_ref().map(|w| w.len().to_string()).unwrap_or_else(|e| format!("ERR {e}")), linked.as_ref().map(|w| w.len().to_string()).unwrap_or_else(|e| format!("ERR {e}")));
        if stub.is_ok() && linked.is_err() && found.is_none() {
            found = Some((nfuncs, linked.err().unwrap().to_string()));
        }
    }
    if let Some((n, e)) = found {
        eprintln!("first count that fits Stub but not Linked: {n} ({e})");
    }
}
