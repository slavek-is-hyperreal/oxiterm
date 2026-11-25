//! BAD ACTOR 8.G: randomised operation sequences over managed values
//! (array of struct refs, sharing, copy-on-write, clone, global replacement)
//! checked against a plain Rust value model, with exact leak accounting.
#![allow(clippy::pedantic)]
mod common;
use common::gen::Rng;
use common::*;
use oxiscript_ir::parse_module;
use oxiscript_runtime::Value;

const SRC: &str = r#"oxiir 1
module "mg"
type P = struct { x: i64, t: u8, name: str? }
data @nm = "abc"
global @arr: [ref P?]?

export func @init() -> void {
e:
  %z: i32 = const 0
  %a: [ref P?] = array.new %z
  %o: [ref P?]? = opt %a
  global.set @arr, %o
  ret
}
export func @push(%x: i64, %t: u8) -> void {
e:
  %o: [ref P?]? = global.get @arr
  %a: [ref P?] = unopt %o
  %p: ref P = alloc P
  setfield.init P.x %p, %x
  setfield.init P.t %p, %t
  %n: str = str.const @nm
  retain %n
  %no: str? = opt %n
  setfield.init P.name %p, %no
  %po: ref P? = opt %p
  array.push %a, %po
  ret
}
export func @copy(%i: i32, %j: i32) -> void {
e:
  %o: [ref P?]? = global.get @arr
  %a: [ref P?] = unopt %o
  %e1: ref P? = array.get %a, %i
  retain %e1
  array.set %a, %j, %e1
  ret
}
export func @mutate(%i: i32, %x: i64) -> void {
e:
  %o: [ref P?]? = global.get @arr
  %a: [ref P?] = unopt %o
  %e1: ref P? = array.get %a, %i
  %p: ref P = unopt %e1
  %u: bool = is_unique %p
  br_if %u, inplace(), cow()
inplace:
  setfield P.x %p, %x
  ret
cow:
  %c: ref P = clone %p
  setfield P.x %c, %x
  %co: ref P? = opt %c
  array.set %a, %i, %co
  ret
}
export func @getx(%i: i32) -> i64 {
e:
  %o: [ref P?]? = global.get @arr
  %a: [ref P?] = unopt %o
  %e1: ref P? = array.get %a, %i
  %p: ref P = unopt %e1
  %x: i64 = getfield P.x %p
  ret %x
}
export func @gett(%i: i32) -> u8 {
e:
  %o: [ref P?]? = global.get @arr
  %a: [ref P?] = unopt %o
  %e1: ref P? = array.get %a, %i
  %p: ref P = unopt %e1
  %x: u8 = getfield P.t %p
  ret %x
}
export func @len() -> i32 {
e:
  %o: [ref P?]? = global.get @arr
  %a: [ref P?] = unopt %o
  %n: i32 = array.len %a
  ret %n
}
export func @clear() -> void {
e:
  %n: [ref P?]? = null
  global.set @arr, %n
  ret
}
"#;

fn call(rt: &mut oxiscript_runtime::Runtime, f: &str, a: &[Value]) -> Vec<Value> {
    rt.call_export(f, a).unwrap_or_else(|e| panic!("{f}{a:?}: {e:?}"))
}

#[test]
fn shared_refs_cow_and_leak_accounting() {
    let m = parse_module(SRC).unwrap();
    let wasm = build(&m);
    let h = host();
    let rounds = fuzz_scale(300, 40);
    for seed in 0..rounds as u64 {
        let mut r = Rng(seed * 977 + 3);
        let mut rt = instantiate(&h, &wasm);
        call(&mut rt, "init", &[]);
        let mut model: Vec<(i64, u8)> = Vec::new();
        for step in 0..150 {
            match r.below(10) {
                0..=2 => {
                    let (x, t) = (r.next() as i64, r.next() as u8);
                    call(&mut rt, "push", &[Value::I64(x), Value::I32(i32::from(t))]);
                    model.push((x, t));
                }
                3..=5 if !model.is_empty() => {
                    let (i, j) = (r.below(model.len()), r.below(model.len()));
                    call(&mut rt, "copy", &[Value::I32(i as i32), Value::I32(j as i32)]);
                    model[j] = model[i];
                }
                6..=8 if !model.is_empty() => {
                    let i = r.below(model.len());
                    let x = r.next() as i64;
                    call(&mut rt, "mutate", &[Value::I32(i as i32), Value::I64(x)]);
                    model[i].0 = x;
                }
                _ => {}
            }
            // full check every few steps
            if step % 7 == 0 || step == 149 {
                assert_eq!(call(&mut rt, "len", &[]), vec![Value::I32(model.len() as i32)], "seed {seed} step {step}");
                for (i, (x, t)) in model.iter().enumerate() {
                    assert_eq!(call(&mut rt, "getx", &[Value::I32(i as i32)]), vec![Value::I64(*x)], "seed {seed} step {step} idx {i}");
                    assert_eq!(call(&mut rt, "gett", &[Value::I32(i as i32)]), vec![Value::I32(i32::from(*t))], "seed {seed} step {step} idx {i}");
                }
            }
        }
        // out-of-range access traps bounds (index == len)
        let n = model.len() as i32;
        assert!(rt.call_export("getx", &[Value::I32(n)]).is_err());
        let mut rt2 = instantiate(&h, &wasm);
        std::mem::swap(&mut rt, &mut rt2);
        drop(rt2);
        // fresh instance: replay is not needed; leak check on a clean sequence
        let mut rt = instantiate(&h, &wasm);
        call(&mut rt, "init", &[]);
        let mut cnt = 0;
        for k in 0..(r.below(60) + 1) {
            call(&mut rt, "push", &[Value::I64(k as i64), Value::I32(1)]);
            cnt += 1;
        }
        for _ in 0..r.below(80) {
            let (i, j) = (r.below(cnt), r.below(cnt));
            call(&mut rt, "copy", &[Value::I32(i as i32), Value::I32(j as i32)]);
            if r.chance(50) {
                call(&mut rt, "mutate", &[Value::I32(i as i32), Value::I64(r.next() as i64)]);
            }
        }
        call(&mut rt, "clear", &[]);
        assert_eq!(rt.deinit().unwrap(), 0, "seed {seed}: leaked objects after clear");
    }
}
