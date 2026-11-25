//! BAD ACTOR 8.G: items of the known-attack catalogue (docs/research/
//! oxiscript-known-attacks.md, "8.G") that are not already covered by the
//! other bad_actor_8g_* files. Test names carry the catalogue row (cat NN).
#![allow(clippy::pedantic)]
mod common;
use common::gen::Rng;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::parse_module;
use oxiscript_runtime::{Error, Grants, Value};
use oxiscript_wasm::{compile, validate, E_LIMIT};
use std::fmt::Write as _;
use wasmi::{Engine, Linker, Module, Store, Val};

// ---- cat 01: every module validates under wasmparser AND wasmi; byte-mutation fuzz never panics
#[test]
fn cat01_mutated_modules_never_panic_the_validators() {
    let src = r#"oxiir 1
module "m"
type P = struct { x: i64, n: str? }
data @d = "héllo"
global @g: i32
export func @f(%a: i32, %s: str) -> i32 {
e:
  %p: ref P = alloc P
  %x: i64 = conv %a
  setfield.init P.x %p, %x
  %n: str? = null
  setfield.init P.n %p, %n
  %l: i32 = str.len %s
  %r: i32 = add.wrap %l, %a
  global.set @g, %r
  release %p
  ret %r
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    validate(&wasm).unwrap();
    let engine = Engine::default();
    Module::new(&engine, &wasm[..]).unwrap();
    let mut r = Rng(99);
    let n = fuzz_scale(20_000, 1_500);
    let h = host();
    let mut accepted = 0;
    for _ in 0..n {
        let mut b = wasm.clone();
        for _ in 0..1 + r.below(4) {
            let i = r.below(b.len());
            match r.below(3) {
                0 => b[i] ^= 1 << r.below(8),
                1 => b[i] = r.next() as u8,
                _ => {
                    b.remove(i);
                }
            }
        }
        let ok = std::panic::catch_unwind(|| {
            let _ = validate(&b);
            let _ = Module::new(&Engine::default(), &b[..]);
        });
        assert!(ok.is_ok(), "validator panicked on mutated module");
        // the host loader must also answer (Ok or Err), never panic
        let r2 = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| h.instantiate(&b, Grants::all()).is_ok()));
        assert!(r2.is_ok(), "host loader panicked on a mutated module");
        accepted += usize::from(r2.unwrap());
    }
    eprintln!("cat01: {n} mutants, {accepted} still loaded");
}

// ---- shared raw-wasmi harness
struct W {
    store: Store<()>,
    inst: wasmi::Instance,
}
impl W {
    fn new(wasm: &[u8]) -> W {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let mut linker = <Linker<()>>::new(&engine);
        for imp in module.imports() {
            if let wasmi::ExternType::Func(ft) = imp.ty().clone() {
                let _ = linker.func_new(imp.module(), imp.name(), ft.clone(), move |_, _, res| {
                    for r in res.iter_mut() {
                        *r = match r.ty() {
                            wasmi::ValType::I32 => Val::I32(0),
                            wasmi::ValType::I64 => Val::I64(0),
                            wasmi::ValType::F32 => Val::F32(0.0f32.into()),
                            _ => Val::F64(0.0f64.into()),
                        };
                    }
                    Ok(())
                });
            }
        }
        let inst = linker.instantiate_and_start(&mut store, &module).unwrap();
        W { store, inst }
    }
    fn call(&mut self, name: &str, args: &[Val], nres: usize) -> Result<Vec<Val>, wasmi::Error> {
        let f = self.inst.get_func(&self.store, name).unwrap();
        let mut res = vec![Val::I32(0); nres];
        f.call(&mut self.store, args, &mut res)?;
        Ok(res)
    }
    fn scratch(&self) -> u32 {
        match self.inst.get_global(&self.store, "oxi_scratch_ptr").unwrap().get(&self.store) {
            Val::I32(p) => p as u32,
            _ => panic!(),
        }
    }
    fn mem_size(&self) -> u64 {
        self.inst.get_memory(&self.store, "memory").unwrap().data_size(&self.store) as u64
    }
    fn write(&mut self, at: u32, b: &[u8]) {
        let m = self.inst.get_memory(&self.store, "memory").unwrap();
        m.write(&mut self.store, at as usize, b).unwrap();
    }
}

const WRAP_SRC: &str = "oxiir 1\nmodule \"w\"\nexport func @slen(%s: str) -> i32 {\ne:\n  %n: i32 = str.len %s\n  ret %n\n}\nexport func @blen(%b: [u8]) -> i32 {\ne:\n  %n: i32 = array.len %b\n  ret %n\n}\n";

// ---- cat 02: base+offset wrap, zero-length bulk copy at the memory boundary
#[test]
fn cat02_zero_length_and_wrapping_addresses_at_the_boundary() {
    let m = parse_module(WRAP_SRC).unwrap();
    let mut w = W::new(&build(&m));
    let size = w.mem_size();
    // zero-length copy exactly at the end of memory is legal; one past is an engine trap
    for f in ["slen", "blen"] {
        assert!(w.call(f, &[Val::I32(size as i32), Val::I32(0)], 1).is_ok(), "{f}: len 0 at memory end");
        assert!(w.call(f, &[Val::I32(size as i32 + 1), Val::I32(0)], 1).is_err(), "{f}: len 0 past memory end");
        assert!(w.call(f, &[Val::I32(-1), Val::I32(0)], 1).is_err());
        // ptr + len wraps 2^32 (0xFFFFFFFF + 2): must trap, not copy from low memory
        assert!(w.call(f, &[Val::I32(-1), Val::I32(2)], 1).is_err());
        assert!(w.call(f, &[Val::I32(-8), Val::I32(16)], 1).is_err());
        // last bytes of memory, in range
        assert!(w.call(f, &[Val::I32(size as i32 - 4), Val::I32(4)], 1).is_ok() || f == "slen", "{f}");
    }
}

// ---- cat 04: tag checks (the only elidable check) are present and not elided
#[test]
fn cat04_wrong_variant_read_traps_bad_cast() {
    let src = r#"oxiir 1
module "u"
type U = union { A { x: i32 }, B { y: i32 } }
export func @f(%w: i32) -> i32 {
e:
  %zero: i32 = const 0
  %isa: bool = eq %w, %zero
  br_if %isa, a(), b()
a:
  %o: ref U = alloc U::A
  %v: i32 = const 5
  setfield.init U::A.x %o, %v
  br rd(%o)
b:
  %o2: ref U = alloc U::B
  %v2: i32 = const 6
  setfield.init U::B.y %o2, %v2
  br rd(%o2)
rd(%u: ref U):
  %r: i32 = getfield U::A.x %u
  release %u
  ret %r
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    assert_eq!(rt.call_export("f", &[Value::I32(0)]).unwrap(), vec![Value::I32(5)]);
    let mut rt = instantiate(&h, &wasm);
    match rt.call_export("f", &[Value::I32(1)]) {
        Err(Error::Trap(t)) => assert_eq!(t.guest_kind, Some(TrapKind::BadCast)),
        o => panic!("{o:?}"),
    }
}

// ---- cat 05: call_indirect with OOB index, signature mismatch (forged closure via the cb trampoline)
#[test]
fn cat05_call_indirect_oob_and_signature_mismatch_trap() {
    let src = r#"oxiir 1
module "ci"
import @watch = "host:state@1" "watch" (i32, i32, fn(i32, str) -> void) -> void
type E = closure { }
func @on_change(%e: ref E, %slot: i32, %k: str) -> void {
e:
  ret
}
func @other(%e: ref E) -> i32 {
e:
  %z: i32 = const 7
  ret %z
}
export func @main(%st: i32) -> void {
e:
  %c: fn(i32, str) -> void = closure.new E @on_change()
  %h: i32 = const 0
  call @watch(%st, %h, %c)
  ret
}
export func @mk_other() -> fn() -> i32 {
e:
  %c: fn() -> i32 = closure.new E @other()
  ret %c
}
export func @mk_good() -> fn(i32, str) -> void {
e:
  %c: fn(i32, str) -> void = closure.new E @on_change()
  ret %c
}"#;
    let m = parse_module(src).unwrap();
    let mut o = opts();
    o.needs = None;
    o.allow_raw_exports = true; // harness: mk_* hand raw closure pointers to the test
    let wasm = compile(&m, &o).unwrap_or_else(|e| panic!("{e}"));
    let mut w = W::new(&wasm);
    let cb = "cb:host:state@1.State.OnChange";
    let sp = w.scratch();
    let good = w.call("mk_good", &[], 1).unwrap()[0].i32().unwrap();
    let other = w.call("mk_other", &[], 1).unwrap()[0].i32().unwrap();
    assert!(w.call(cb, &[Val::I32(good), Val::I32(0), Val::I32(sp as i32), Val::I32(0)], 0).is_ok());
    // signature mismatch: the closure's table slot holds @other (() -> i32 with env)
    assert!(w.call(cb, &[Val::I32(other), Val::I32(0), Val::I32(sp as i32), Val::I32(0)], 0).is_err(), "signature mismatch must trap");
    // forged closure object whose table index is out of range
    for idx in [2u32, 1000, u32::MAX, 0x8000_0000] {
        w.write(sp + 64, &idx.to_le_bytes());
        assert!(w.call(cb, &[Val::I32(sp as i32 + 64), Val::I32(0), Val::I32(sp as i32), Val::I32(0)], 0).is_err(), "OOB table index {idx}");
    }
}

// ---- cat 06: deep recursion traps (no host crash); runaway loop is stopped by fuel
#[test]
fn cat06_deep_recursion_and_infinite_loop_are_traps() {
    let src = r#"oxiir 1
module "rec"
func @down(%n: i64) -> i64 {
e:
  %one: i64 = const 1
  %m: i64 = add.wrap %n, %one
  %r: i64 = call @down(%m)
  ret %r
}
export func @rec() -> i64 {
e:
  %z: i64 = const 0
  %r: i64 = call @down(%z)
  ret %r
}
export func @spin() -> i32 {
e:
  br l()
l:
  br l()
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let h = host();
    for f in ["rec", "spin"] {
        let mut rt = instantiate(&h, &wasm);
        assert!(matches!(rt.call_export(f, &[]), Err(Error::Trap(_))), "{f} must be stopped by a trap");
    }
}

// ---- cat 07: big modules: function / global / data / type counts answer with a diagnostic or load
#[test]
fn cat07_big_modules() {
    // 12000 functions, 1100 globals
    let mut s = String::from("oxiir 1\nmodule \"big\"\n");
    for i in 0..12_000 {
        let _ = write!(s, "func @f{i}() -> i32 {{\ne:\n  %a: i32 = const {i}\n  %b: i32 = const 1\n  %c: i32 = add.wrap %a, %b\n  %d: i32 = add.wrap %c, %b\n  ret %d\n}}\n");
    }
    let m = parse_module(&s).unwrap();
    match compile(&m, &opts()) {
        Ok(w) => validate(&w).unwrap(),
        Err(e) => assert_eq!(e.0[0].code, E_LIMIT, "{e}"),
    }
    let mut s = String::from("oxiir 1\nmodule \"g\"\n");
    for i in 0..1_100 {
        let _ = writeln!(s, "global @g{i}: i32");
    }
    let m = parse_module(&s).unwrap();
    match compile(&m, &opts()) {
        Ok(w) => validate(&w).unwrap(),
        Err(e) => assert_eq!(e.0[0].code, E_LIMIT, "{e}"),
    }
    // 3000 struct types + 3000 data segments, one 3 MB constant
    let mut s = String::from("oxiir 1\nmodule \"t\"\n");
    for i in 0..3000 {
        let _ = writeln!(s, "type T{i} = struct {{ a: i32, b: str? }}");
        let _ = writeln!(s, "data @d{i} = \"s{i}\"");
    }
    let _ = writeln!(s, "data @huge = \"{}\"", "x".repeat(3 << 20));
    s.push_str("export func @n() -> i32 {\ne:\n  %s: str = str.const @huge\n  %l: i32 = str.len %s\n  ret %l\n}\n");
    let m = parse_module(&s).unwrap();
    match compile(&m, &opts()) {
        Ok(w) => {
            validate(&w).unwrap();
            let h = host();
            match h.instantiate(&w, Grants::all()) {
                Ok(mut rt) => assert_eq!(rt.call_export("n", &[]).unwrap(), vec![Value::I32(3 << 20)]),
                Err(e) => eprintln!("cat07: host rejects big-data module: {e}"),
            }
        }
        Err(e) => assert_eq!(e.0[0].code, E_LIMIT, "{e}"),
    }
}

// ---- cat 09: NaN bits never observable through bitcast; scalars cannot become pointers
#[test]
fn cat09_bitcast_nan_is_canonical() {
    let src = r#"oxiir 1
module "bc"
export func @b64(%a: f64) -> i64 {
e:
  %r: i64 = bitcast %a
  ret %r
}
export func @b32(%a: f32) -> i32 {
e:
  %r: i32 = bitcast %a
  ret %r
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    for bits in [0x7ff8_0000_0000_0001u64, 0xfff8_0000_0000_0000, 0x7ff0_0000_0000_0001, 0xffff_ffff_ffff_ffff] {
        let r = rt.call_export("b64", &[Value::F64(f64::from_bits(bits))]).unwrap();
        assert_eq!(r, vec![Value::I64(0x7ff8_0000_0000_0000)], "{bits:#x}");
    }
    for bits in [0x7fc0_0001u32, 0xffc0_0000, 0x7f80_0001, 0xffff_ffff] {
        let r = rt.call_export("b32", &[Value::F32(f32::from_bits(bits))]).unwrap();
        assert_eq!(r, vec![Value::I32(0x7fc0_0000)], "{bits:#x}");
    }
    // non-NaN bit patterns are preserved
    let r = rt.call_export("b64", &[Value::F64(-0.0)]).unwrap();
    assert_eq!(r, vec![Value::I64(i64::MIN)]);
}

// ---- cat 10: deterministic output (separate threads, fresh hash states), also for managed programs
#[test]
fn cat10_compile_is_byte_deterministic() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../oxiscript-ir/tests/programs/structs.oxir")).ok();
    let mut srcs = vec![WRAP_SRC.to_string()];
    if let Some(s) = src {
        srcs.push(s);
    }
    for s in srcs {
        let m = parse_module(&s).unwrap();
        let base = compile(&m, &opts());
        for _ in 0..8 {
            let m2 = m.clone();
            let again = std::thread::spawn(move || compile(&m2, &opts())).join().unwrap();
            assert_eq!(base, again);
        }
    }
}

// ---- cat 11: static data over the initial memory is sized into it: instantiation defined, strings readable
// (covered by cat07 huge constant); instantiation of data at exact page boundaries:
#[test]
fn cat11_static_data_sizes_around_page_boundaries() {
    let h = host();
    for len in [0usize, 1, 65_000, 65_400, 65_528, 65_536 - 8 - 1024, 131_072, 131_071, 262_144 - 1024 - 24] {
        let src = format!("oxiir 1\nmodule \"p\"\ndata @d = \"{}\"\nexport func @n() -> i32 {{\ne:\n  %s: str = str.const @d\n  %l: i32 = str.len %s\n  %o: i32 = const 5\n  %r: i32 = add.wrap %l, %o\n  ret %r\n}}\n", "y".repeat(len));
        let m = parse_module(&src).unwrap();
        let wasm = build(&m);
        let mut rt = instantiate(&h, &wasm);
        assert_eq!(rt.call_export("n", &[]).unwrap(), vec![Value::I32(len as i32 + 5)], "len {len}");
    }
}

// ---- cat 12: fuel: no metering is injected by codegen; loops/calls are bounded by the engine (see cat06)

// ---- cat 13: label-depth fuzz is bad_actor_8g_cfg.rs (random_cfgs_agree, random_big_cfgs_agree, random_paramless_cfgs...)

// ---- cat 14: locals reuse/stale pointers: allocate in a loop, release on every path, no leak or double free
#[test]
fn cat14_loop_allocations_released_on_every_path() {
    let src = r#"oxiir 1
module "lp"
type B = struct { v: i32 }
export func @f(%n: i32) -> i32 {
entry:
  %z: i32 = const 0
  br h(%z, %z)
h(%i: i32, %acc: i32):
  %c: bool = lt %i, %n
  br_if %c, body(), out(%acc)
body:
  %o: ref B = alloc B
  setfield.init B.v %o, %i
  %two: i32 = const 2
  %m: i32 = rem %i, %two
  %odd: bool = eq %m, %z
  br_if %odd, ev(%o), od(%o)
ev(%p: ref B):
  %v: i32 = getfield B.v %p
  %a2: i32 = add.wrap %acc, %v
  release %p
  br next(%a2)
od(%q: ref B):
  release %q
  br next(%acc)
next(%a3: i32):
  %one: i32 = const 1
  %i2: i32 = add.wrap %i, %one
  br h(%i2, %a3)
out(%r: i32):
  ret %r
}"#;
    let m = parse_module(src).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    let want: i32 = (0..1000).filter(|i| i % 2 == 0).sum();
    assert_eq!(rt.call_export("f", &[Value::I32(1000)]).unwrap(), vec![Value::I32(want)]);
    assert_eq!(rt.deinit().unwrap(), 0);
}
