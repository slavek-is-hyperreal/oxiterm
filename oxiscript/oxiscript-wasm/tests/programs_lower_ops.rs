//! The IR ops added for the lowering (slice 8.L): `str.concat/eq/cmp/slice/byte`,
//! `array.empty`, `array.pop` (text round trip, verification, execution, no leaks).
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::{parse_module, print_module, verify};
use oxiscript_runtime::{Error, Value};

const SRC: &str = r#"oxiir 1
module "ops"
data @a = "foo"
data @b = "foobar"
data @u = "h\u{e9}llo"

export func @concat_len() -> i32 {
e:
  %a: str = str.const @a
  %b: str = str.const @b
  %c: str = str.concat %a, %b
  %n: i32 = str.len %c
  release %c
  ret %n
}
export func @eq() -> bool {
e:
  %a: str = str.const @a
  %b: str = str.const @b
  %zero: i32 = const 0
  %three: i32 = const 3
  %q: str = str.slice %b, %zero, %three
  %r: bool = str.eq %a, %q
  release %q
  ret %r
}
export func @cmp(%x: i32) -> i32 {
e:
  %a: str = str.const @a
  %b: str = str.const @b
  %r: i32 = str.cmp %a, %b
  ret %r
}
export func @byte(%i: i32) -> u8 {
e:
  %b: str = str.const @b
  %r: u8 = str.byte %b, %i
  ret %r
}
export func @bad_slice() -> i32 {
e:
  %u: str = str.const @u
  %zero: i32 = const 0
  %two: i32 = const 2
  %s: str = str.slice %u, %zero, %two
  %n: i32 = str.len %s
  release %s
  ret %n
}
export func @arr() -> i32 {
e:
  %a: [str] = array.empty
  %s: str = str.const @a
  retain %s
  array.push %a, %s
  %n: i32 = array.len %a
  %p: str = array.pop %a
  %m: i32 = array.len %a
  release %p
  release %a
  %ten: i32 = const 10
  %t: i32 = mul %n, %ten
  %r: i32 = add %t, %m
  ret %r
}
export func @pop_empty() -> i32 {
e:
  %a: [i32] = array.empty
  %v: i32 = array.pop %a
  release %a
  ret %v
}
"#;

fn fixed() -> String {
    SRC.to_string()
}

#[test]
fn new_ops_round_trip_and_verify() {
    let m = parse_module(&fixed()).expect("parses");
    assert_eq!(verify(&m).len(), 0, "{:?}", verify(&m));
    let text = print_module(&m);
    let m2 = parse_module(&text).expect("re-parses");
    assert_eq!(m, m2);
    assert_eq!(print_module(&m2), text);
}

#[test]
fn new_ops_execute() {
    let m = parse_module(&fixed()).unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    assert_eq!(rt.call_export("concat_len", &[]).unwrap(), vec![Value::I32(9)]);
    assert_eq!(rt.call_export("eq", &[]).unwrap(), vec![Value::I32(1)]);
    assert_eq!(rt.call_export("cmp", &[Value::I32(0)]).unwrap(), vec![Value::I32(-1)]);
    assert_eq!(rt.call_export("byte", &[Value::I32(3)]).unwrap(), vec![Value::I32(i32::from(b'b'))]);
    assert_eq!(rt.call_export("arr", &[]).unwrap(), vec![Value::I32(10)]);
    assert_eq!(rt.deinit().unwrap(), 0, "leaked objects");
    // traps
    let mut rt = instantiate(&h, &wasm);
    match rt.call_export("byte", &[Value::I32(6)]) {
        Err(Error::Trap(t)) => assert_eq!(t.guest_kind, Some(TrapKind::Bounds)),
        other => panic!("{other:?}"),
    }
    let mut rt = instantiate(&h, &wasm);
    match rt.call_export("bad_slice", &[]) {
        Err(Error::Trap(t)) => assert_eq!(t.guest_kind, Some(TrapKind::BadUtf8)),
        other => panic!("{other:?}"),
    }
    let mut rt = instantiate(&h, &wasm);
    match rt.call_export("pop_empty", &[]) {
        Err(Error::Trap(t)) => assert_eq!(t.guest_kind, Some(TrapKind::Bounds)),
        other => panic!("{other:?}"),
    }
}
