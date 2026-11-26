//! BAD ACTOR 8.G: codegen limits and boundary programs. Each program verifies;
//! the answer must be correct code or a clean diagnostic.
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::{parse_module, verify};
use oxiscript_runtime::{Error, Value};
use oxiscript_wasm::{compile, validate};
use std::fmt::Write as _;

fn rt_for(src: &str) -> (oxiscript_runtime::Host, Vec<u8>) {
    let m = parse_module(src).unwrap();
    assert_eq!(verify(&m).len(), 0, "{:?}", verify(&m));
    (host(), build(&m))
}

#[test]
fn from_utf8_trap_order_and_bounds() {
    let src = r#"oxiir 1
module "u"
export func @f(%n: i32, %fill: u8) -> i32 {
e:
  %c: i32 = const 4
  %a: [u8] = array.new %c
  %z: i32 = const 0
  %one: i32 = const 1
  br l(%z)
l(%i: i32):
  %d: bool = ge %i, %c
  br_if %d, go(), body()
body:
  array.set %a, %i, %fill
  %i2: i32 = add %i, %one
  br l(%i2)
go:
  %s: str = str.from_utf8 %a, %n
  %r: i32 = str.len %s
  release %s
  release %a
  ret %r
}"#;
    let (h, wasm) = rt_for(src);
    let mut rt = instantiate(&h, &wasm);
    let mut call = |n: i32, fill: i32| -> Result<i32, Option<TrapKind>> {
        if rt.is_poisoned() {
            rt = instantiate(&h, &wasm);
        }
        match rt.call_export("f", &[Value::I32(n), Value::I32(fill)]) {
            Ok(v) => match v[0] {
                Value::I32(x) => Ok(x),
                _ => panic!(),
            },
            Err(Error::Trap(t)) => Err(t.guest_kind),
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(call(4, 0x41), Ok(4));
    assert_eq!(call(0, 0xff), Ok(0));
    assert_eq!(call(5, 0x41), Err(Some(TrapKind::Bounds)));
    assert_eq!(call(-1, 0x41), Err(Some(TrapKind::Bounds)));
    assert_eq!(call(i32::MIN, 0xff), Err(Some(TrapKind::Bounds)));
    // out of range wins over invalid bytes
    assert_eq!(call(5, 0xff), Err(Some(TrapKind::Bounds)));
    assert_eq!(call(4, 0xff), Err(Some(TrapKind::BadUtf8)));
    assert_eq!(call(1, 0x80), Err(Some(TrapKind::BadUtf8)));
    // an invalid tail beyond n is not looked at
    assert_eq!(call(0, 0x80), Ok(0));
}

#[test]
fn array_new_extremes() {
    let src = r#"oxiir 1
module "a"
export func @n8(%n: i32) -> i32 {
e:
  %a: [u8] = array.new %n
  %l: i32 = array.len %a
  release %a
  ret %l
}
export func @n64(%n: i32) -> i32 {
e:
  %a: [i64] = array.new %n
  %l: i32 = array.len %a
  release %a
  ret %l
}"#;
    let (h, wasm) = rt_for(src);
    let mut rt = instantiate(&h, &wasm);
    let mut kind = |f: &str, n: i32| -> Result<i32, Option<TrapKind>> {
        if rt.is_poisoned() {
            rt = instantiate(&h, &wasm);
        }
        match rt.call_export(f, &[Value::I32(n)]) {
            Ok(v) => Ok(match v[0] {
                Value::I32(x) => x,
                _ => panic!(),
            }),
            Err(Error::Trap(t)) => Err(t.guest_kind),
            Err(e) => panic!("{e:?}"),
        }
    };
    assert_eq!(kind("n8", 0), Ok(0));
    assert_eq!(kind("n8", 1000), Ok(1000));
    assert_eq!(kind("n8", -1), Err(Some(TrapKind::Bounds)));
    assert_eq!(kind("n8", i32::MIN), Err(Some(TrapKind::Bounds)));
    // 2^31-1 i64 elements = 16 GiB: must be out_of_memory, not wraparound
    let r = kind("n64", i32::MAX);
    assert!(matches!(r, Err(Some(TrapKind::OutOfMemory)) | Err(None)), "{r:?}");
    let r = kind("n64", 0x2000_0000); // 4 GiB
    assert!(matches!(r, Err(Some(TrapKind::OutOfMemory)) | Err(None)), "{r:?}");
    let r = kind("n8", i32::MAX);
    assert!(matches!(r, Err(Some(TrapKind::OutOfMemory)) | Err(None)), "{r:?}");
}

#[test]
fn many_params_is_a_diagnostic() {
    for n in [31usize, 32, 33, 40, 200] {
        let mut s = String::from("oxiir 1\nmodule \"p\"\nexport func @f(");
        for i in 0..n {
            if i > 0 {
                s.push_str(", ");
            }
            let _ = write!(s, "%p{i}: i32");
        }
        s.push_str(") -> i32 {\ne:\n  ret %p0\n}\n");
        let m = parse_module(&s).unwrap();
        match compile(&m, &opts()) {
            Ok(w) => {
                validate(&w).unwrap();
                assert!(n <= 32, "n={n} accepted");
                let h = host();
                let mut rt = h.instantiate(&w, oxiscript_runtime::Grants::all()).unwrap();
                let args: Vec<Value> = (0..n as i32).map(Value::I32).collect();
                assert_eq!(rt.call_export("f", &args).unwrap(), vec![Value::I32(0)]);
            }
            Err(e) => assert_eq!(e.0[0].code, oxiscript_wasm::E_LIMIT, "n={n}: {e}"),
        }
    }
    // str params flatten to two wasm params each: 17 strs = 34 wasm params
    for n in [15usize, 16, 17, 20] {
        let mut s = String::from("oxiir 1\nmodule \"p\"\nexport func @f(");
        for i in 0..n {
            if i > 0 {
                s.push_str(", ");
            }
            let _ = write!(s, "%p{i}: str");
        }
        s.push_str(") -> i32 {\ne:\n  %l: i32 = str.len %p0\n  ret %l\n}\n");
        let m = parse_module(&s).unwrap();
        match compile(&m, &opts()) {
            Ok(w) => validate(&w).unwrap(),
            Err(e) => assert_eq!(e.0[0].code, oxiscript_wasm::E_LIMIT, "n={n}: {e}"),
        }
    }
}

#[test]
fn many_results_is_a_diagnostic() {
    for n in [31usize, 32, 33, 100] {
        let rets = vec!["i32"; n].join(", ");
        let mut s = format!("oxiir 1\nmodule \"r\"\nexport func @f() -> ({rets}) {{\ne:\n  %z: i32 = const 5\n  ret ");
        s.push_str(&vec!["%z"; n].join(", "));
        s.push_str("\n}\n");
        let m = parse_module(&s).unwrap();
        match compile(&m, &opts()) {
            Ok(w) => validate(&w).unwrap(),
            Err(e) => assert_eq!(e.0[0].code, oxiscript_wasm::E_LIMIT, "n={n}: {e}"),
        }
    }
}

/// A struct with more managed fields than the descriptor's 16-bit `ptr_count`
/// can hold: releasing the object must still release every child.
/// Build a struct with `n` managed (`str?`) fields, initialise all, overwrite the last, release.
fn wide_struct_src(n: usize) -> String {
    let mut s = String::from("oxiir 1\nmodule \"wide\"\ntype W = struct { ");
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "f{i}: str?");
    }
    s.push_str(" }\nexport func @f() -> i32 {\ne:\n  %o: ref W = alloc W\n");
    s.push_str("  %n: str? = null\n");
    for i in 0..n {
        let _ = writeln!(s, "  setfield.init W.f{i} %o, %n");
    }
    let last = n - 1;
    s.push_str(&format!("  %z: i32 = const 0\n  %a: [u8] = array.new %z\n  %t: str = str.from_utf8 %a, %z\n  release %a\n  %to: str? = opt %t\n  setfield W.f{last} %o, %to\n  release %o\n  %r: i32 = const 1\n  ret %r\n}}\n"));
    s
}

/// 65535 managed fields is the descriptor maximum: it must compile, run and
/// free the child stored in the last slot. 65536 must be a clean E0603 (the
/// pointer count is a u16 in the type descriptor; ABI 1.0 cannot widen it).
#[test]
fn struct_with_more_than_65535_managed_fields_frees_all_children() {
    let t0 = std::time::Instant::now();
    let m = parse_module(&wide_struct_src(65_535)).unwrap_or_else(|e| panic!("parse: {e}"));
    let w = compile(&m, &opts()).unwrap_or_else(|e| panic!("65535 fields must compile: {}", e.0[0]));
    eprintln!("65535 fields: parse+compile {:?}", t0.elapsed());
    let h = host();
    match h.instantiate(&w, oxiscript_runtime::Grants::all()) {
        Err(e) => panic!("host rejects 65535-field module: {e}"),
        Ok(mut rt) => {
            assert_eq!(rt.call_export("f", &[]).unwrap(), vec![Value::I32(1)]);
            assert_eq!(rt.deinit().unwrap(), 0, "children of a 65535-field struct leaked");
        }
    }
    let m = parse_module(&wide_struct_src(65_536)).unwrap_or_else(|e| panic!("parse: {e}"));
    match compile(&m, &opts()) {
        Ok(_) => panic!("65536 managed fields must be rejected, not truncated"),
        Err(e) => assert_eq!(e.0[0].code, "OXS-E0603", "{}", e.0[0]),
    }
}

#[test]
fn huge_switch_chain_and_table() {
    for (n, stride) in [(3000usize, 1i64), (3000, 7), (1024, 1), (1025, 1)] {
        let mut s = String::from("oxiir 1\nmodule \"sw\"\nexport func @f(%x: i32) -> i32 {\ne:\n  switch %x, [");
        for i in 0..n {
            if i > 0 {
                s.push_str(", ");
            }
            let _ = write!(s, "{}: t{i}(%x)", i as i64 * stride);
        }
        s.push_str("], default d(%x)\n");
        for i in 0..n {
            let _ = write!(s, "t{i}(%a{i}: i32):\n  %c{i}: i32 = const {}\n  %r{i}: i32 = add.wrap %a{i}, %c{i}\n  ret %r{i}\n", i * 3);
        }
        s.push_str("d(%z: i32):\n  %m: i32 = const -1\n  ret %m\n}\n");
        let m = parse_module(&s).unwrap();
        let w = compile(&m, &opts()).unwrap_or_else(|e| panic!("n={n}: {e}"));
        let h = host();
        let mut rt = h.instantiate(&w, oxiscript_runtime::Grants::all()).unwrap();
        for i in [0usize, 1, n / 2, n - 1] {
            let x = i as i64 * stride;
            let want = x as i32 + (i * 3) as i32;
            assert_eq!(rt.call_export("f", &[Value::I32(x as i32)]).unwrap(), vec![Value::I32(want)], "n={n} stride={stride} x={x}");
        }
        for x in [-1i32, i32::MIN, i32::MAX, (n as i64 * stride) as i32 + 1, 5 * stride as i32 + 1] {
            let want = if (0..n as i64).any(|i| i * stride == x as i64) { None } else { Some(-1) };
            if let Some(w) = want {
                assert_eq!(rt.call_export("f", &[Value::I32(x)]).unwrap(), vec![Value::I32(w)], "n={n} stride={stride} x={x}");
            }
        }
    }
}

#[test]
fn array_push_growth_all_element_sizes() {
    let src = r#"oxiir 1
module "g"
export func @sum_u8(%n: i32) -> i64 {
entry:
  %z: i32 = const 0
  %a: [u8] = array.new %z
  br l(%z)
l(%i: i32):
  %d: bool = ge %i, %n
  br_if %d, done(), body()
body:
  %v: u8 = conv.wrap %i
  array.push %a, %v
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br l(%i2)
done:
  %len: i32 = array.len %a
  %s: i64 = const 0
  br s(%z, %s)
s(%j: i32, %acc: i64):
  %e: bool = ge %j, %len
  br_if %e, out(%acc), sb()
sb:
  %x: u8 = array.get %a, %j
  %xl: i64 = conv %x
  %acc2: i64 = add %acc, %xl
  %o: i32 = const 1
  %j2: i32 = add %j, %o
  br s(%j2, %acc2)
out(%r: i64):
  release %a
  ret %r
}
export func @sum_f64(%n: i32) -> f64 {
entry:
  %z: i32 = const 0
  %a: [f64] = array.new %z
  br l(%z)
l(%i: i32):
  %d: bool = ge %i, %n
  br_if %d, done(), body()
body:
  %v: f64 = conv %i
  array.push %a, %v
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br l(%i2)
done:
  %len: i32 = array.len %a
  %s: f64 = const 0.0
  br s(%z, %s)
s(%j: i32, %acc: f64):
  %e: bool = ge %j, %len
  br_if %e, out(%acc), sb()
sb:
  %x: f64 = array.get %a, %j
  %acc2: f64 = add %acc, %x
  %o: i32 = const 1
  %j2: i32 = add %j, %o
  br s(%j2, %acc2)
out(%r: f64):
  release %a
  ret %r
}"#;
    let (h, wasm) = rt_for(src);
    let mut rt = instantiate(&h, &wasm);
    for n in [0, 1, 3, 4, 5, 63, 64, 65, 1000, 70_000] {
        let want: i64 = (0..n as i64).map(|i| i & 0xff).sum();
        assert_eq!(rt.call_export("sum_u8", &[Value::I32(n)]).unwrap(), vec![Value::I64(want)], "n={n}");
        let wantf: f64 = (0..n).map(|i| i as f64).sum();
        assert_eq!(rt.call_export("sum_f64", &[Value::I32(n)]).unwrap(), vec![Value::F64(wantf)], "n={n}");
    }
    assert_eq!(rt.deinit().unwrap(), 0);
}

/// Functions with very many SSA values (one Wasm local each) must either be
/// a diagnostic or load AND run in the strict engine.
#[test]
fn many_locals_load_and_run_or_diagnose() {
    for n in [1000usize, 8_000, 11_000, 11_990, 12_010, 16_000, 20_000, 32_000, 50_010] {
        let mut s = String::from("oxiir 1\nmodule \"big\"\nexport func @f(%a: i32) -> i32 {\ne:\n");
        let mut prev = "%a".to_string();
        for i in 0..n {
            let _ = writeln!(s, "  %v{i}: i32 = add.wrap {prev}, %a");
            prev = format!("%v{i}");
        }
        let _ = write!(s, "  ret {prev}\n}}");
        let m = parse_module(&s).unwrap();
        match compile(&m, &opts()) {
            Ok(w) => {
                validate(&w).unwrap();
                let h = host();
                match h.instantiate(&w, oxiscript_runtime::Grants::all()) {
                    Ok(mut rt) => {
                        let r = rt.call_export("f", &[Value::I32(3)]);
                        match r {
                            Ok(v) => assert_eq!(v, vec![Value::I32(3 * (n as i32 + 1))], "n={n}"),
                            Err(e) => panic!("n={n}: accepted by codegen but the call fails: {e:?}"),
                        }
                    }
                    Err(e) => panic!("n={n}: host rejects a module codegen accepted: {e}"),
                }
            }
            Err(e) => assert_eq!(e.0[0].code, oxiscript_wasm::E_LIMIT, "n={n}: {e}"),
        }
    }
}

/// `n` loops one after another (the natural shape of a long function). Each
/// sums 0..3 into the accumulator.
fn seq_loops(n: usize) -> String {
    let mut s = String::from("oxiir 1\nmodule \"sl\"\nexport func @f(%a: i32) -> i32 {\nentry:\n  %z: i32 = const 0\n  br h0(%z, %a)\n");
    for i in 0..n {
        let _ = write!(
            s,
            "h{i}(%i{i}: i32, %acc{i}: i32):\n  %lim{i}: i32 = const 3\n  %c{i}: bool = lt %i{i}, %lim{i}\n  br_if %c{i}, b{i}(), x{i}()\nb{i}:\n  %acc2_{i}: i32 = add.wrap %acc{i}, %i{i}\n  %one{i}: i32 = const 1\n  %n{i}: i32 = add.wrap %i{i}, %one{i}\n  br h{i}(%n{i}, %acc2_{i})\nx{i}:\n  %z{i}: i32 = const 0\n  br h{}(%z{i}, %acc{i})\n",
            i + 1
        );
    }
    let _ = write!(s, "h{n}(%iN: i32, %accN: i32):\n  ret %accN\n}}");
    s
}

#[test]
fn many_sequential_loops_compile_or_diagnose_cleanly() {
    for n in [10usize, 100, 250, 299, 301, 600, 2000] {
        let m = parse_module(&seq_loops(n)).unwrap();
        let r = std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || compile(&m, &opts()).map_err(|e| e.0[0].code.to_string()))
            .unwrap()
            .join()
            .expect("compiler stack overflow");
        match r {
            Ok(w) => {
                validate(&w).unwrap();
                let h = host();
                let mut rt = h.instantiate(&w, oxiscript_runtime::Grants::all()).unwrap();
                assert_eq!(rt.call_export("f", &[Value::I32(5)]).unwrap(), vec![Value::I32(5 + 3 * n as i32)], "n={n}");
                eprintln!("sequential loops n={n}: ok");
            }
            Err(code) => {
                // documented limit: loop exits are nested in the Wasm structure (MAX_DEPTH 300)
                assert_eq!(code, oxiscript_wasm::E_LIMIT, "n={n}");
                assert!(n > 250, "n={n}: a modest number of loops must compile");
            }
        }
    }
}
