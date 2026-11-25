//! Managed-value, host-boundary and structural programs (see `programs.rs`
//! for the scalar ones).
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::parse_module;
use oxiscript_runtime::caps::state::StateValue;
use oxiscript_runtime::{Error, Value};
use oxiscript_wasm::{compile, CompileOptions, RuntimeMode, E_IRREDUCIBLE};
use Value::{F32, F64, I32, I64};

enum E {
    Ok(Vec<Value>),
    Trap(TrapKind),
}
use E::{Ok as Okv, Trap as Tr};

struct Step(&'static str, Vec<Value>, E);
fn s(f: &'static str, a: Vec<Value>, e: E) -> Step {
    Step(f, a, e)
}

fn same_val(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (F32(x), F32(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        (F64(x), F64(y)) => (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits(),
        _ => a == b,
    }
}

fn check(name: &str, rt: &mut oxiscript_runtime::Runtime, h: &oxiscript_runtime::Host, wasm: &[u8], steps: Vec<Step>) {
    for Step(f, args, want) in steps {
        if rt.is_poisoned() {
            *rt = instantiate(h, wasm);
        }
        let got = rt.call_export(f, &args);
        match (want, got) {
            (Okv(w), Ok(g)) => assert!(
                w.len() == g.len() && w.iter().zip(&g).all(|(a, b)| same_val(a, b)),
                "{name}: {f}{args:?} = {g:?}, want {w:?}"
            ),
            (Tr(k), Err(Error::Trap(t))) => {
                assert_eq!(t.guest_kind, Some(k), "{name}: {f}{args:?}: {:?}", t.cause);
            }
            (_, got) => panic!("{name}: {f}{args:?}: unexpected {got:?}"),
        }
    }
}

fn run(name: &str, src: &str, steps: Vec<Step>, leak: bool) {
    let m = parse_module(src).unwrap_or_else(|e| panic!("{name}: parse: {e}"));
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    check(name, &mut rt, &h, &wasm, steps);
    if leak && !rt.is_poisoned() {
        assert_eq!(rt.deinit().unwrap(), 0, "{name}: leaked objects");
    }
}

#[test]
fn p16_struct_fields_and_release() {
    run(
        "p16",
        r#"oxiir 1
module "p16"
type Point = struct { x: f64, y: f64, name: str }
data @nm = "pt"
export func @run(%a: f64) -> f64 {
e:
  %p: ref Point = alloc Point
  %b: f64 = const 2.0
  setfield.init Point.x %p, %a
  setfield.init Point.y %p, %b
  %n: str = str.const @nm
  retain %n
  setfield.init Point.name %p, %n
  %x: f64 = getfield Point.x %p
  %y: f64 = getfield Point.y %p
  %s: f64 = add %x, %y
  %y2: f64 = const 10.0
  setfield Point.y %p, %y2
  %yy: f64 = getfield Point.y %p
  %t: f64 = add %s, %yy
  release %p
  ret %t
}"#,
        vec![s("run", vec![F64(1.5)], Okv(vec![F64(13.5)]))],
        true,
    );
}

#[test]
fn p17_copy_on_write() {
    run(
        "p17",
        r#"oxiir 1
module "p17"
type P = struct { x: i32 }
func @move_x(%p: own ref P, %dx: i32) -> ref P {
entry:
  %u: bool = is_unique %p
  br_if %u, write(%p), copy()
copy:
  %c: ref P = clone %p
  release %p
  br write(%c)
write(%q: ref P):
  %x: i32 = getfield P.x %q
  %nx: i32 = add %x, %dx
  setfield P.x %q, %nx
  ret %q
}
export func @shared() -> i32 {
e:
  %a: ref P = alloc P
  %one: i32 = const 1
  setfield.init P.x %a, %one
  retain %a
  %five: i32 = const 5
  %b: ref P = call @move_x(%a, %five)
  %ax: i32 = getfield P.x %a
  %bx: i32 = getfield P.x %b
  release %a
  release %b
  %h: i32 = const 100
  %t: i32 = mul %ax, %h
  %r: i32 = add %t, %bx
  ret %r
}
export func @unique() -> i32 {
e:
  %a: ref P = alloc P
  %one: i32 = const 1
  setfield.init P.x %a, %one
  %five: i32 = const 5
  %b: ref P = call @move_x(%a, %five)
  %bx: i32 = getfield P.x %b
  release %b
  ret %bx
}
export func @uniq_flag() -> i32 {
e:
  %a: ref P = alloc P
  %zero: i32 = const 0
  setfield.init P.x %a, %zero
  %u1: bool = is_unique %a
  retain %a
  %u2: bool = is_unique %a
  release %a
  release %a
  %one: i32 = const 1
  %two: i32 = const 2
  %r1: i32 = select %u1, %one, %zero
  %r2: i32 = select %u2, %two, %zero
  %r: i32 = add %r1, %r2
  ret %r
}"#,
        vec![
            s("shared", vec![], Okv(vec![I32(106)])),
            s("unique", vec![], Okv(vec![I32(6)])),
            s("uniq_flag", vec![], Okv(vec![I32(1)])),
        ],
        true,
    );
}

#[test]
fn p18_union_variants() {
    run(
        "p18",
        r#"oxiir 1
module "p18"
type Shape = union { Circle { r: f64 }, Rect { w: f64, h: f64 }, Empty }
export func @area(%k: i32, %a: f64, %b: f64) -> f64 {
entry:
  switch %k, [0: mc(), 1: mr()], default me()
mc:
  %c: ref Shape = alloc Shape::Circle
  setfield.init Shape::Circle.r %c, %a
  br m(%c)
mr:
  %r: ref Shape = alloc Shape::Rect
  setfield.init Shape::Rect.w %r, %a
  setfield.init Shape::Rect.h %r, %b
  br m(%r)
me:
  %e: ref Shape = alloc Shape::Empty
  br m(%e)
m(%s: ref Shape):
  %t: u32 = tag %s
  switch %t, [0: circle(), 1: rect()], default empty()
circle:
  %rr: f64 = getfield Shape::Circle.r %s
  %sq: f64 = mul %rr, %rr
  release %s
  ret %sq
rect:
  %w: f64 = getfield Shape::Rect.w %s
  %h: f64 = getfield Shape::Rect.h %s
  %ar: f64 = mul %w, %h
  release %s
  ret %ar
empty:
  %z: f64 = const 0.0
  release %s
  ret %z
}
export func @bad() -> f64 {
e:
  %c: ref Shape = alloc Shape::Circle
  %one: f64 = const 1.0
  setfield.init Shape::Circle.r %c, %one
  %w: f64 = getfield Shape::Rect.w %c
  release %c
  ret %w
}
export func @tagof(%k: i32) -> u32 {
entry:
  switch %k, [0: mc(), 1: mr()], default me()
mc:
  %c: ref Shape = alloc Shape::Circle
  %z: f64 = const 0.0
  setfield.init Shape::Circle.r %c, %z
  %t0: u32 = tag %c
  release %c
  ret %t0
mr:
  %r: ref Shape = alloc Shape::Rect
  %z2: f64 = const 0.0
  setfield.init Shape::Rect.w %r, %z2
  setfield.init Shape::Rect.h %r, %z2
  %t1: u32 = tag %r
  release %r
  ret %t1
me:
  %e: ref Shape = alloc Shape::Empty
  %t2: u32 = tag %e
  release %e
  ret %t2
}"#,
        vec![
            s("area", vec![I32(0), F64(3.0), F64(0.0)], Okv(vec![F64(9.0)])),
            s("area", vec![I32(1), F64(2.0), F64(5.0)], Okv(vec![F64(10.0)])),
            s("area", vec![I32(2), F64(0.0), F64(0.0)], Okv(vec![F64(0.0)])),
            s("tagof", vec![I32(0)], Okv(vec![I32(0)])),
            s("tagof", vec![I32(1)], Okv(vec![I32(1)])),
            s("tagof", vec![I32(2)], Okv(vec![I32(2)])),
            s("bad", vec![], Tr(TrapKind::BadCast)),
        ],
        true,
    );
}

#[test]
fn p19_arrays() {
    run(
        "p19",
        r#"oxiir 1
module "p19"
export func @fillsum(%n: i32) -> i32 {
entry:
  %a: [i32] = array.new %n
  %i0: i32 = const 0
  br fill(%i0)
fill(%i: i32):
  %done: bool = ge %i, %n
  br_if %done, sumstart(), fbody()
fbody:
  %two: i32 = const 2
  %v: i32 = mul %i, %two
  array.set %a, %i, %v
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br fill(%i2)
sumstart:
  %z: i32 = const 0
  br sum(%z, %z)
sum(%j: i32, %acc: i32):
  %d2: bool = ge %j, %n
  br_if %d2, out(%acc), sbody()
sbody:
  %x: i32 = array.get %a, %j
  %acc2: i32 = add %acc, %x
  %o: i32 = const 1
  %j2: i32 = add %j, %o
  br sum(%j2, %acc2)
out(%r: i32):
  %len: i32 = array.len %a
  %res: i32 = add %r, %len
  release %a
  ret %res
}
export func @get(%i: i32) -> i32 {
e:
  %five: i32 = const 5
  %a: [i32] = array.new %five
  %v: i32 = array.get %a, %i
  release %a
  ret %v
}
export func @newlen(%n: i32) -> i32 {
e:
  %a: [i32] = array.new %n
  %l: i32 = array.len %a
  release %a
  ret %l
}
export func @pushsum(%n: i32) -> i64 {
entry:
  %z: i32 = const 0
  %a: [i64] = array.new %z
  br p(%z)
p(%i: i32):
  %done: bool = ge %i, %n
  br_if %done, s0(), pb()
pb:
  %w: i64 = conv %i
  array.push %a, %w
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br p(%i2)
s0:
  %zl: i64 = const 0
  br s(%z, %zl)
s(%j: i32, %acc: i64):
  %d2: bool = ge %j, %n
  br_if %d2, out(%acc), sb()
sb:
  %x: i64 = array.get %a, %j
  %acc2: i64 = add %acc, %x
  %o: i32 = const 1
  %j2: i32 = add %j, %o
  br s(%j2, %acc2)
out(%r: i64):
  release %a
  ret %r
}
export func @u8canon(%v: i32) -> i32 {
e:
  %three: i32 = const 3
  %a: [u8] = array.new %three
  %b: u8 = conv.wrap %v
  %one: i32 = const 1
  array.set %a, %one, %b
  %g: u8 = array.get %a, %one
  %r: i32 = conv %g
  release %a
  ret %r
}
export func @f64arr() -> f64 {
e:
  %z: i32 = const 0
  %a: [f64] = array.new %z
  %x: f64 = const 1.5
  %y: f64 = const 2.5
  array.push %a, %x
  array.push %a, %y
  %one: i32 = const 1
  %p: f64 = array.get %a, %z
  %q: f64 = array.get %a, %one
  %s: f64 = add %p, %q
  release %a
  ret %s
}
export func @clone_arr() -> i32 {
e:
  %three: i32 = const 3
  %a: [i32] = array.new %three
  %z: i32 = const 0
  %seven: i32 = const 7
  array.set %a, %z, %seven
  %b: [i32] = clone %a
  %ninety: i32 = const 99
  array.set %b, %z, %ninety
  %x: i32 = array.get %a, %z
  %y: i32 = array.get %b, %z
  %k: i32 = const 1000
  %xk: i32 = mul %x, %k
  %r: i32 = add %xk, %y
  release %a
  release %b
  ret %r
}"#,
        vec![
            s("fillsum", vec![I32(5)], Okv(vec![I32(20 + 5)])),
            s("fillsum", vec![I32(0)], Okv(vec![I32(0)])),
            s("get", vec![I32(4)], Okv(vec![I32(0)])),
            s("get", vec![I32(5)], Tr(TrapKind::Bounds)),
            s("get", vec![I32(-1)], Tr(TrapKind::Bounds)),
            s("newlen", vec![I32(-1)], Tr(TrapKind::Bounds)),
            s("newlen", vec![I32(10)], Okv(vec![I32(10)])),
            s("pushsum", vec![I32(1000)], Okv(vec![I64(499500)])),
            s("u8canon", vec![I32(300)], Okv(vec![I32(44)])),
            s("f64arr", vec![], Okv(vec![F64(4.0)])),
            s("clone_arr", vec![], Okv(vec![I32(7099)])),
        ],
        true,
    );
}

#[test]
fn p20_array_of_refs_frees_children() {
    run(
        "p20",
        r#"oxiir 1
module "p20"
type Cell = struct { v: i32 }
export func @run(%n: i32) -> i32 {
entry:
  %z: i32 = const 0
  %a: [ref Cell?] = array.new %z
  br p(%z)
p(%i: i32):
  %done: bool = ge %i, %n
  br_if %done, s0(), pb()
pb:
  %c: ref Cell = alloc Cell
  setfield.init Cell.v %c, %i
  %co: ref Cell? = opt %c
  array.push %a, %co
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br p(%i2)
s0:
  br s(%z, %z)
s(%j: i32, %acc: i32):
  %d2: bool = ge %j, %n
  br_if %d2, out(%acc), sb()
sb:
  %xo: ref Cell? = array.get %a, %j
  %x: ref Cell = unopt %xo
  %v: i32 = getfield Cell.v %x
  %acc2: i32 = add %acc, %v
  %o: i32 = const 1
  %j2: i32 = add %j, %o
  br s(%j2, %acc2)
out(%r: i32):
  release %a
  ret %r
}"#,
        vec![s("run", vec![I32(10)], Okv(vec![I32(45)])), s("run", vec![I32(300)], Okv(vec![I32(44850)]))],
        true,
    );
}

#[test]
fn p21_strings() {
    run(
        "p21",
        r#"oxiir 1
module "p21"
data @h = "h\u{e9}llo"
export func @slen() -> i32 {
e:
  %s: str = str.const @h
  %n: i32 = str.len %s
  ret %n
}
export func @mk(%b1: u8, %b2: u8, %b3: u8, %n: i32) -> i32 {
e:
  %z: i32 = const 0
  %a: [u8] = array.new %z
  array.push %a, %b1
  array.push %a, %b2
  array.push %a, %b3
  %s: str = str.from_utf8 %a, %n
  %l: i32 = str.len %s
  release %s
  release %a
  ret %l
}"#,
        vec![
            s("slen", vec![], Okv(vec![I32(6)])),
            s("mk", vec![I32(0x68), I32(0xC3), I32(0xA9), I32(3)], Okv(vec![I32(3)])),
            s("mk", vec![I32(0x68), I32(0xC3), I32(0xA9), I32(1)], Okv(vec![I32(1)])),
            s("mk", vec![I32(0x68), I32(0xC3), I32(0xA9), I32(2)], Tr(TrapKind::BadUtf8)),
            s("mk", vec![I32(0xFF), I32(0xC3), I32(0xA9), I32(3)], Tr(TrapKind::BadUtf8)),
            s("mk", vec![I32(0xED), I32(0xA0), I32(0x80), I32(3)], Tr(TrapKind::BadUtf8)),
            s("mk", vec![I32(0xE0), I32(0x80), I32(0x80), I32(3)], Tr(TrapKind::BadUtf8)),
            s("mk", vec![I32(0x68), I32(0xC3), I32(0xA9), I32(4)], Tr(TrapKind::Bounds)),
            s("mk", vec![I32(0x68), I32(0xC3), I32(0xA9), I32(-1)], Tr(TrapKind::Bounds)),
        ],
        true,
    );
}

#[test]
fn p22_closures() {
    run(
        "p22",
        r#"oxiir 1
module "p22"
type Env0 = closure { n: i32 }
type Env1 = closure { f: fn(i32) -> i32, g: fn(i32) -> i32 }
func @add_n(%env: ref Env0, %x: i32) -> i32 {
entry:
  %n: i32 = getfield Env0.n %env
  %r: i32 = add %x, %n
  ret %r
}
func @compose_call(%env: ref Env1, %x: i32) -> i32 {
entry:
  %f: fn(i32) -> i32 = getfield Env1.f %env
  %g: fn(i32) -> i32 = getfield Env1.g %env
  %y: i32 = call_closure %g(%x)
  %z: i32 = call_closure %f(%y)
  ret %z
}
export func @adder(%n: i32, %v: i32) -> i32 {
e:
  %c: fn(i32) -> i32 = closure.new Env0 @add_n(%n)
  %r: i32 = call_closure %c(%v)
  release %c
  ret %r
}
export func @comp(%n: i32, %m: i32, %v: i32) -> i32 {
e:
  %a: fn(i32) -> i32 = closure.new Env0 @add_n(%n)
  %b: fn(i32) -> i32 = closure.new Env0 @add_n(%m)
  %c: fn(i32) -> i32 = closure.new Env1 @compose_call(%a, %b)
  %r: i32 = call_closure %c(%v)
  release %c
  ret %r
}"#,
        vec![
            s("adder", vec![I32(3), I32(4)], Okv(vec![I32(7)])),
            s("comp", vec![I32(10), I32(20), I32(1)], Okv(vec![I32(31)])),
        ],
        true,
    );
}

#[test]
fn p23_optionals() {
    run(
        "p23",
        r#"oxiir 1
module "p23"
type Box = struct { v: i32 }
export func @maybe(%k: i32) -> i32 {
entry:
  %z: i32 = const 0
  %isz: bool = eq %k, %z
  br_if %isz, none(), some()
none:
  %n: ref Box? = null
  %nul: bool = is_null %n
  br_if %nul, bad(%n), other()
bad(%m: ref Box?):
  %u: ref Box = unopt %m
  %zz: i32 = const 0
  ret %zz
other:
  trap unreachable
some:
  %b: ref Box = alloc Box
  setfield.init Box.v %b, %k
  %o: ref Box? = opt %b
  %nn: bool = is_null %o
  br_if %nn, other(), use(%o)
use(%q: ref Box?):
  %u2: ref Box = unopt %q
  %v: i32 = getfield Box.v %u2
  release %u2
  ret %v
}"#,
        vec![
            s("maybe", vec![I32(0)], Tr(TrapKind::NullDeref)),
            s("maybe", vec![I32(9)], Okv(vec![I32(9)])),
        ],
        true,
    );
}

#[test]
fn p24_globals() {
    run(
        "p24",
        r#"oxiir 1
module "p24"
type Box = struct { v: i32 }
global @counter: i32 = 5
global @big: i64 = -7
global @half: f64 = 2.5
global @flag: bool = true
global @cur: ref Box?
export func @bump() -> i32 {
e:
  %g: i32 = global.get @counter
  %one: i32 = const 1
  %n: i32 = add %g, %one
  global.set @counter, %n
  ret %n
}
export func @big() -> i64 {
e:
  %g: i64 = global.get @big
  ret %g
}
export func @half() -> f64 {
e:
  %g: f64 = global.get @half
  ret %g
}
export func @flag() -> bool {
e:
  %g: bool = global.get @flag
  ret %g
}
export func @setcur(%v: i32) -> void {
e:
  %b: ref Box = alloc Box
  setfield.init Box.v %b, %v
  %o: ref Box? = opt %b
  global.set @cur, %o
  ret
}
export func @getcur() -> i32 {
entry:
  %o: ref Box? = global.get @cur
  %n: bool = is_null %o
  br_if %n, none(), some()
none:
  %m: i32 = const -1
  ret %m
some:
  %u: ref Box = unopt %o
  %v: i32 = getfield Box.v %u
  ret %v
}"#,
        vec![
            s("bump", vec![], Okv(vec![I32(6)])),
            s("bump", vec![], Okv(vec![I32(7)])),
            s("big", vec![], Okv(vec![I64(-7)])),
            s("half", vec![], Okv(vec![F64(2.5)])),
            s("flag", vec![], Okv(vec![I32(1)])),
            s("getcur", vec![], Okv(vec![I32(-1)])),
            s("setcur", vec![I32(3)], Okv(vec![])),
            s("getcur", vec![], Okv(vec![I32(3)])),
            s("setcur", vec![I32(9)], Okv(vec![])),
            s("getcur", vec![], Okv(vec![I32(9)])),
        ],
        true,
    );
}

#[test]
fn p25_export_wrappers() {
    run(
        "p25",
        r#"oxiir 1
module "p25"
export func @slen(%s: str) -> i32 {
e:
  %n: i32 = str.len %s
  ret %n
}
export func @bsum(%b: [u8]) -> i32 {
entry:
  %n: i32 = array.len %b
  %z: i32 = const 0
  br l(%z, %z)
l(%i: i32, %acc: i32):
  %d: bool = ge %i, %n
  br_if %d, out(%acc), body()
body:
  %x: u8 = array.get %b, %i
  %xi: i32 = conv %x
  %acc2: i32 = add %acc, %xi
  %one: i32 = const 1
  %i2: i32 = add %i, %one
  br l(%i2, %acc2)
out(%r: i32):
  ret %r
}
export func @canon(%a: u8, %b: bool, %c: i8, %d: u16) -> i32 {
e:
  %ai: i32 = conv %a
  %ci: i32 = conv %c
  %di: i32 = conv %d
  %k: i32 = const 1000
  %z: i32 = const 0
  %bi: i32 = select %b, %k, %z
  %s1: i32 = add %ai, %bi
  %s2: i32 = add %s1, %ci
  %s3: i32 = add %s2, %di
  ret %s3
}
export func @own_str(%s: own str) -> i32 {
e:
  %n: i32 = str.len %s
  release %s
  ret %n
}"#,
        vec![
            s("slen", vec![Value::Str("h\u{e9}llo".into())], Okv(vec![I32(6)])),
            s("bsum", vec![Value::Bytes(vec![1, 2, 3, 250])], Okv(vec![I32(256)])),
            s("canon", vec![I32(0x1ff), I32(2), I32(0x80), I32(0x12345)], Okv(vec![I32(255 + 1000 - 128 + 0x2345)])),
            s("own_str", vec![Value::Str("abc".into())], Okv(vec![I32(3)])),
        ],
        true,
    );
}

const HOST_SRC: &str = r#"oxiir 1
module "host"
import @log_write = "host:log@1" "write" (i32, u8, str) -> void
import @state_open = "host:state@1" "open" (i32, str) -> (i32, i32)
import @get_i64 = "host:state@1" "get_i64" (i32, i32) -> (i32, i64)
import @set_i64 = "host:state@1" "set_i64" (i32, i32, i64) -> i32
import @write_str = "host:state@1" "write_str" (i32, i32, str) -> i32
import @read_str = "host:state@1" "read_str" (i32, i32, [u8]) -> (i32, i32)
import @state_watch = "host:state@1" "watch" (i32, i32, fn(i32, str) -> void) -> void
data @hello = "hello"
data @key = "counter"
type Env0 = closure { }
global @st_h: i32
global @slot: i32
global @last_slot: i32
global @last_klen: i32

func @on_change(%env: ref Env0, %slot: i32, %key: str) -> void {
e:
  global.set @last_slot, %slot
  %n: i32 = str.len %key
  global.set @last_klen, %n
  ret
}

export func @main(%log: i32, %st: i32) -> void {
entry:
  global.set @st_h, %st
  %s: str = str.const @hello
  %lvl: u8 = const 2
  call @log_write(%log, %lvl, %s)
  %k: str = str.const @key
  %status: i32, %slot: i32 = call @state_open(%st, %k)
  %zero: i32 = const 0
  %ok: bool = eq %status, %zero
  br_if %ok, store(), fail()
store:
  global.set @slot, %slot
  ret
fail:
  trap panic
}

export func @setv(%v: i64) -> i32 {
e:
  %h: i32 = global.get @st_h
  %sl: i32 = global.get @slot
  %st: i32 = call @set_i64(%h, %sl, %v)
  ret %st
}
export func @getv() -> i64 {
e:
  %h: i32 = global.get @st_h
  %sl: i32 = global.get @slot
  %st: i32, %v: i64 = call @get_i64(%h, %sl)
  ret %v
}
export func @wr(%s: str) -> i32 {
e:
  %h: i32 = global.get @st_h
  %sl: i32 = global.get @slot
  %st: i32 = call @write_str(%h, %sl, %s)
  ret %st
}
export func @rd() -> i32 {
entry:
  %h: i32 = global.get @st_h
  %sl: i32 = global.get @slot
  %c0: i32 = const 4
  %b0: [u8] = array.new %c0
  br try(%b0)
try(%buf: [u8]):
  %st: i32, %len: i32 = call @read_str(%h, %sl, %buf)
  %one: i32 = const 1
  %big: bool = eq %st, %one
  br_if %big, grow(%buf, %len), done(%buf, %st, %len)
grow(%ob: [u8], %need: i32):
  release %ob
  %nb: [u8] = array.new %need
  br try(%nb)
done(%b: [u8], %s: i32, %l: i32):
  %zero: i32 = const 0
  %ok: bool = eq %s, %zero
  br_if %ok, good(%b, %l), bad(%b)
good(%gb: [u8], %gl: i32):
  %str: str = str.from_utf8 %gb, %gl
  %n: i32 = str.len %str
  release %str
  release %gb
  ret %n
bad(%bb: [u8]):
  release %bb
  %m: i32 = const -1
  ret %m
}
export func @arm() -> void {
e:
  %h: i32 = global.get @st_h
  %sl: i32 = global.get @slot
  %c: fn(i32, str) -> void = closure.new Env0 @on_change()
  call @state_watch(%h, %sl, %c)
  ret
}
export func @last_slot() -> i32 {
e:
  %g: i32 = global.get @last_slot
  ret %g
}
export func @last_klen() -> i32 {
e:
  %g: i32 = global.get @last_klen
  ret %g
}
export func @slot() -> i32 {
e:
  %g: i32 = global.get @slot
  ret %g
}"#;

#[test]
fn p26_host_log_state_outbuffer() {
    let m = parse_module(HOST_SRC).unwrap_or_else(|e| panic!("parse: {e}"));
    let wasm = build(&m);
    let e = env();
    let mut rt = instantiate(&e.host, &wasm);
    let lines = e.sink.lines();
    assert_eq!(lines.len(), 1);
    assert_eq!((lines[0].msg.as_str(), lines[0].level), ("hello", 2));
    assert_eq!(rt.call_export("setv", &[I64(41)]).unwrap(), vec![I32(0)]);
    assert_eq!(rt.call_export("getv", &[]).unwrap(), vec![I64(41)]);
    assert_eq!(e.store.get_in(rt.namespace(), "counter"), Some(StateValue::I64(41)));
    // out-buffer retry loop: the 4-byte buffer is too small for the value
    let long = "a string longer than the initial four byte buffer";
    assert_eq!(rt.call_export("wr", &[Value::Str(long.into())]).unwrap(), vec![I32(0)]);
    assert_eq!(rt.call_export("rd", &[]).unwrap(), vec![I32(long.len() as i32)]);
    assert_eq!(rt.deinit().unwrap(), 0, "leak");
}

#[test]
fn p27_callbacks() {
    let m = parse_module(HOST_SRC).unwrap();
    let wasm = build(&m);
    let e = env();
    let mut rt = instantiate(&e.host, &wasm);
    rt.call_export("arm", &[]).unwrap();
    assert_eq!(rt.pump().unwrap().delivered, 0);
    e.store.set("counter", StateValue::I64(5)).unwrap();
    assert_eq!(rt.pump().unwrap().delivered, 1);
    let slot = rt.call_export("slot", &[]).unwrap();
    assert_eq!(rt.call_export("last_slot", &[]).unwrap(), slot);
    assert_eq!(rt.call_export("last_klen", &[]).unwrap(), vec![I32(7)]);
    // the host drops the closure at the next quiescent point: no leak after deinit
    drop(rt);
}

#[test]
fn p28_irreducible_is_rejected() {
    let m = parse_module(
        r#"oxiir 1
module "p28"
export func @f(%c: bool) -> i32 {
entry:
  br_if %c, a(), b()
a:
  br_if %c, b(), out()
b:
  br_if %c, a(), out()
out:
  %x: i32 = const 1
  ret %x
}"#,
    )
    .unwrap();
    let err = compile(&m, &opts()).unwrap_err();
    assert_eq!(err.0[0].code, E_IRREDUCIBLE, "{err}");
}

#[test]
fn p29_unreachable_blocks_are_ignored() {
    run(
        "p29",
        r#"oxiir 1
module "p29"
export func @f() -> i32 {
entry:
  %x: i32 = const 1
  ret %x
dead:
  %q: i32 = const 2
  %r: i32 = add %q, %q
  br dead2(%r)
dead2(%p: i32):
  br dead()
}"#,
        vec![s("f", vec![], Okv(vec![I32(1)]))],
        false,
    );
}

#[test]
fn p30_long_chain_and_many_diamonds() {
    let mut chain = String::from("oxiir 1\nmodule \"p30\"\nexport func @f() -> i32 {\nb0:\n  br b1()\n");
    let n = 3000;
    for i in 1..n {
        chain.push_str(&format!("b{i}:\n  br b{}()\n", i + 1));
    }
    chain.push_str(&format!("b{n}:\n  %x: i32 = const 77\n  ret %x\n}}"));
    run("p30a", &chain, vec![s("f", vec![], Okv(vec![I32(77)]))], false);

    let mut d = String::from("oxiir 1\nmodule \"p30b\"\nexport func @f(%c: bool) -> i32 {\nentry:\n  %z: i32 = const 0\n  br t0(%z)\n");
    let k = 500;
    for i in 0..k {
        d.push_str(&format!(
            "t{i}(%a{i}: i32):\n  br_if %c, y{i}(), n{i}()\ny{i}:\n  %one{i}: i32 = const 1\n  %s{i}: i32 = add %a{i}, %one{i}\n  br t{}(%s{i})\nn{i}:\n  br t{}(%a{i})\n",
            i + 1,
            i + 1
        ));
    }
    d.push_str(&format!("t{k}(%r: i32):\n  ret %r\n}}"));
    run(
        "p30b",
        &d,
        vec![s("f", vec![I32(1)], Okv(vec![I32(k)])), s("f", vec![I32(0)], Okv(vec![I32(0)]))],
        false,
    );
}

#[test]
fn p31_tick_main_and_lifecycle() {
    run(
        "p31a",
        r#"oxiir 1
module "p31a"
func @tk(%now: i64) -> i64 #tick {
e:
  %five: i64 = const 5
  %r: i64 = add %now, %five
  ret %r
}"#,
        vec![],
        false,
    );
    let m = parse_module(
        r#"oxiir 1
module "p31b"
global @n: i32
export func @main() -> void {
e:
  %one: i32 = const 1
  global.set @n, %one
  ret
}
export func @getn() -> i32 {
e:
  %g: i32 = global.get @n
  ret %g
}"#,
    )
    .unwrap();
    let wasm = build(&m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    assert_eq!(rt.call_export("getn", &[]).unwrap(), vec![I32(1)]);
    assert_eq!(rt.tick(10).unwrap(), -1);
    let m = parse_module(
        r#"oxiir 1
module "p31c"
func @tk(%now: i64) -> i64 #tick {
e:
  %five: i64 = const 5
  %r: i64 = add %now, %five
  ret %r
}
export func @one() -> i32 {
e:
  %x: i32 = const 1
  ret %x
}"#,
    )
    .unwrap();
    let wasm = build(&m);
    let mut rt = instantiate(&h, &wasm);
    assert_eq!(rt.tick(10).unwrap(), 15);
}

#[test]
fn p32_safety_profile_matches_default() {
    let body = r#"
type F = struct { x: f64 }
export func @f(%a: f64, %b: f64) -> f64 {
e:
  %m: f64 = min %a, %b
  %o: ref F = alloc F
  setfield.init F.x %o, %m
  %z: i32 = const 0
  %arr: [f64] = array.new %z
  %x: f64 = getfield F.x %o
  array.push %arr, %x
  %g: f64 = array.get %arr, %z
  %f: f32 = conv %g
  %d: f64 = conv %f
  release %arr
  release %o
  ret %d
}"#;
    let a = parse_module(&format!("oxiir 1\nmodule \"p32\"\n{body}")).unwrap();
    let b = parse_module(&format!("oxiir 1\nmodule \"p32\" #profile(safety)\n{body}")).unwrap();
    let h = host();
    let mut ra = instantiate(&h, &build(&a));
    let mut rb = instantiate(&h, &build(&b));
    for (x, y) in [(1.5, 2.5), (f64::NAN, 1.0), (-0.0, 0.0), (1e300, 1.0)] {
        let args = [F64(x), F64(y)];
        let va = ra.call_export("f", &args).unwrap();
        let vb = rb.call_export("f", &args).unwrap();
        assert!(same_val(&va[0], &vb[0]), "{x} {y}: {va:?} vs {vb:?}");
    }
    assert_eq!(rb.deinit().unwrap(), 0);
}

#[test]
fn p33_import_mode_and_manifest() {
    let m = parse_module(HOST_SRC).unwrap();
    let o = CompileOptions {
        runtime: RuntimeMode::Import,
        ..opts()
    };
    let wasm = compile(&m, &o).unwrap();
    let text = wasmprinter::print_bytes(&wasm).unwrap();
    assert!(text.contains("(import \"oxi:rt\" \"oxi_alloc\"") && text.contains("\"oxi_rt_init\""), "rt imports missing");
    // the output is not loadable by itself (imports `oxi:rt`) but is valid Wasm
    oxiscript_wasm::validate(&wasm).unwrap();
}

#[test]
fn p34_deterministic_output() {
    {
        let m = parse_module(HOST_SRC).unwrap();
        let a = build(&m);
        let b = build(&m);
        assert_eq!(a, b);
        let m2 = parse_module(&oxiscript_ir::print_module(&m)).unwrap();
        assert_eq!(a, build(&m2));
    }
    for seed in 0..40 {
        let m = common::gen::gen_module(seed + 1, 3);
        assert_eq!(build(&m), build(&m), "seed {seed}");
    }
}

#[test]
fn p35_closure_capturing_managed_value() {
    run(
        "p35",
        r#"oxiir 1
module "p35"
type Box = struct { v: i32 }
type Env = closure { b: ref Box }
func @get(%env: ref Env, %x: i32) -> i32 {
e:
  %b: ref Box = getfield Env.b %env
  %v: i32 = getfield Box.v %b
  %r: i32 = add %v, %x
  ret %r
}
export func @run(%v: i32, %x: i32) -> i32 {
e:
  %b: ref Box = alloc Box
  setfield.init Box.v %b, %v
  %c: fn(i32) -> i32 = closure.new Env @get(%b)
  %r: i32 = call_closure %c(%x)
  release %c
  ret %r
}"#,
        vec![s("run", vec![I32(40), I32(2)], Okv(vec![I32(42)]))],
        true,
    );
}

#[test]
fn p36_nested_loops_and_early_exit() {
    run(
        "p36",
        r#"oxiir 1
module "p36"
export func @f(%n: i32, %stop: i32) -> i32 {
entry:
  %z: i32 = const 0
  br outer(%z, %z)
outer(%i: i32, %acc: i32):
  %d: bool = ge %i, %n
  br_if %d, exit(%acc), inner0(%i, %acc)
inner0(%i1: i32, %a1: i32):
  %z2: i32 = const 0
  br inner(%i1, %a1, %z2)
inner(%i2: i32, %a2: i32, %j: i32):
  %jd: bool = ge %j, %n
  br_if %jd, next(%i2, %a2), ibody(%i2, %a2, %j)
ibody(%i3: i32, %a3: i32, %j3: i32):
  %hit: bool = eq %j3, %stop
  br_if %hit, exit(%a3), cont(%i3, %a3, %j3)
cont(%i4: i32, %a4: i32, %j4: i32):
  %m: i32 = mul %i4, %j4
  %a5: i32 = add %a4, %m
  %one: i32 = const 1
  %j5: i32 = add %j4, %one
  br inner(%i4, %a5, %j5)
next(%i6: i32, %a6: i32):
  %o: i32 = const 1
  %i7: i32 = add %i6, %o
  br outer(%i7, %a6)
exit(%r: i32):
  ret %r
}"#,
        vec![
            s("f", vec![I32(4), I32(100)], Okv(vec![I32(36)])),
            s("f", vec![I32(4), I32(2)], Okv(vec![I32(0)])),
            s("f", vec![I32(4), I32(3)], Okv(vec![I32(0)])),
            s("f", vec![I32(0), I32(0)], Okv(vec![I32(0)])),
        ],
        false,
    );
}

#[test]
fn p37_unverified_ir_is_rejected() {
    let m = parse_module(
        r#"oxiir 1
module "p37"
export func @f(%a: i32, %b: i64) -> i32 {
e:
  %c: i32 = add %a, %b
  ret %c
}"#,
    );
    // the parser may or may not reject it; either way compile must not produce Wasm
    if let Ok(m) = m {
        let err = compile(&m, &opts()).unwrap_err();
        assert_eq!(err.0[0].code, oxiscript_wasm::E_VERIFY, "{err}");
    }
}

#[test]
fn p38_ungranted_and_forged_imports_are_rejected() {
    let m = parse_module(HOST_SRC).unwrap();
    let err = compile(&m, &CompileOptions::default()).unwrap_err();
    assert_eq!(err.0[0].code, "OXS-E0400", "{err}");
    let forged = HOST_SRC.replace(
        "import @log_write = \"host:log@1\" \"write\" (i32, u8, str) -> void",
        "import @log_write = \"host:log@1\" \"write\" (i32, u8, i32, i32) -> void",
    );
    let m = parse_module(&forged).unwrap();
    let err = compile(&m, &opts()).unwrap_err();
    assert_eq!(err.0[0].code, oxiscript_wasm::E_VERIFY, "{err}");
}

#[test]
fn p39_main_and_needs_consistency() {
    let bad = HOST_SRC.replace("export func @main(%log: i32, %st: i32)", "export func @main(%log: i32, %st: i32, %extra: i32)");
    let m = parse_module(&bad).unwrap();
    let err = compile(&m, &opts()).unwrap_err();
    assert_eq!(err.0[0].code, oxiscript_wasm::E_CONFIG, "{err}");
    let nomain = HOST_SRC.replace("export func @main", "export func @notmain");
    let m = parse_module(&nomain).unwrap();
    let err = compile(&m, &opts()).unwrap_err();
    assert_eq!(err.0[0].code, oxiscript_wasm::E_CONFIG, "{err}");
}

#[test]
fn p40_many_guards_compile() {
    let mut src = String::from("oxiir 1\nmodule \"p40\"\nexport func @f(%a: i32) -> i32 {\nentry:\n  %k: i32 = const 0\n  br g0()\n");
    let n = 2000;
    for i in 0..n {
        src.push_str(&format!(
            "g{i}:\n  %c{i}: bool = eq %a, %k\n  br_if %c{i}, bad{i}(), g{}()\nbad{i}:\n  trap panic\n",
            i + 1
        ));
    }
    src.push_str(&format!("g{n}:\n  ret %a\n}}"));
    run(
        "p40",
        &src,
        vec![s("f", vec![I32(5)], Okv(vec![I32(5)])), s("f", vec![I32(0)], Tr(TrapKind::Panic))],
        false,
    );
}

#[test]
fn p41_callback_trampoline_exported_with_abi_name() {
    let m = parse_module(HOST_SRC).unwrap();
    let wasm = build(&m);
    let text = wasmprinter::print_bytes(&wasm).unwrap();
    assert!(text.contains("(export \"cb:host:state@1.State.OnChange\""), "trampoline missing");
    assert!(text.contains("(export \"oxi_init\""));
    assert!(wasm.windows(12).any(|w| w == b"oxi.manifest"));
}

#[test]
fn p42_feature_set_is_exactly_the_abi() {
    use wasmparser::WasmFeatures as F;
    let f = oxiscript_wasm::allowed_features();
    for off in [F::SATURATING_FLOAT_TO_INT, F::REFERENCE_TYPES, F::SIMD, F::TAIL_CALL, F::MULTI_MEMORY, F::MEMORY64, F::EXCEPTIONS, F::THREADS] {
        assert!(!f.contains(off), "{off:?} must be off");
    }
    for on in [F::MUTABLE_GLOBAL, F::MULTI_VALUE, F::BULK_MEMORY, F::SIGN_EXTENSION, F::FLOATS] {
        assert!(f.contains(on), "{on:?} must be on");
    }
}
