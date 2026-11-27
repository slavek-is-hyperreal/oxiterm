//! Stage 8.Fix end-to-end lowering + execution regression tests:
//! - Closure returning a narrowed-null local (`() => s`) compiles and runs ([R2-111])
//! - IEEE 754 float `%` (`f64 % f64` and `f32 % f32`) compiles and matches Rust `%` ([R3-31])
//! - Optional chaining `this.x?.f` in constructors & generic `x?.f` ([R3-31])
//! - 1000 tiny `@host` exports compile and run in `oxiscript-runtime` ([R2-109, R3-61])
//! - Configurable generic instantiation budget (`Options::max_instantiations`) & count ([R3-142])
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::*;
use oxiscript_check::NoLoader;
use oxiscript_lower::{compile_program, Options};
use oxiscript_runtime::{Grants, Host, Limits, Value};

#[test]
fn closure_returning_narrowed_null_local_compiles_and_runs() {
    let src = r#"
import { Log } from "host:log";
export function main(log: Log): void {
  const s: string | null = null;
  const f = () => s;
  const g = () => { return s; };
  log.write(2, f() ?? "z1");
  log.write(2, g() ?? "z2");
}
"#;
    run_log(src, &["z1", "z2"]);
}

#[test]
fn float_remainder_f64_and_f32_match_ieee754() {
    let src = r#"
@host export function rem64(a: f64, b: f64): f64 {
  return a % b;
}
@host export function rem32(a: f32, b: f32): f32 {
  return a % b;
}
"#;
    let wasm = compile(src);
    let host = Host::with_options(Limits::default(), test_host_options());
    let mut rt = host.instantiate(&wasm, Grants::none()).expect("instantiate");

    let cases64: &[(f64, f64)] = &[
        (5.5, 2.0),
        (-5.5, 2.0),
        (5.5, -2.0),
        (-5.5, -2.0),
        (6.0, 3.0),
        (-6.0, 3.0),
        (1.0, 3.0),
        (-0.0, 1.0),
        (0.0, 1.0),
        (1.0, 0.0),
        (f64::INFINITY, 2.0),
        (2.0, f64::INFINITY),
        (f64::NAN, 2.0),
        (2.0, f64::NAN),
        (1e300, 3.0),
        (f64::from_bits(1), f64::from_bits(1)),
        (f64::from_bits(5), f64::from_bits(2)),
        (1.0, f64::from_bits(1)),
    ];
    for &(a, b) in cases64 {
        let out = rt
            .call_export("rem64", &[Value::F64(a), Value::F64(b)])
            .expect("call rem64");
        let Some(&Value::F64(got)) = out.first() else {
            panic!("expected f64, got {out:?}");
        };
        let want = a % b;
        if want.is_nan() {
            assert!(got.is_nan(), "f64 {a} % {b}: want NaN, got {got}");
        } else {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "f64 {a} % {b}: want {want:?} ({:016x}), got {got:?} ({:016x})",
                want.to_bits(),
                got.to_bits()
            );
        }
    }

    let cases32: &[(f32, f32)] = &[
        (5.5f32, 2.0f32),
        (-5.5f32, 2.0f32),
        (5.5f32, -2.0f32),
        (-6.0f32, 3.0f32),
        (1.0f32, 0.0f32),
        (f32::INFINITY, 2.0f32),
        (2.0f32, f32::INFINITY),
        (1e30f32, 3.0f32),
        (f32::from_bits(5), f32::from_bits(2)),
    ];
    for &(a, b) in cases32 {
        let out = rt
            .call_export("rem32", &[Value::F32(a), Value::F32(b)])
            .expect("call rem32");
        let Some(&Value::F32(got)) = out.first() else {
            panic!("expected f32, got {out:?}");
        };
        let want = a % b;
        if want.is_nan() {
            assert!(got.is_nan(), "f32 {a} % {b}: want NaN, got {got}");
        } else {
            assert_eq!(
                got.to_bits(),
                want.to_bits(),
                "f32 {a} % {b}: want {want:?} ({:08x}), got {got:?} ({:08x})",
                want.to_bits(),
                got.to_bits()
            );
        }
    }
}

#[test]
fn optional_chaining_on_this_field_in_constructor_and_generic_box() {
    let src = r#"
struct Box<T> {
  val: T;
}
class Holder {
  inner: Box<i32> | null;
  copied: i32;
  constructor(b: Box<i32> | null) {
    this.inner = b;
    this.copied = this.inner?.val ?? -1;
  }
}
@host export function test_ctor(present: bool): i32 {
  const b: Box<i32> | null = present ? { val: 42 } : null;
  const h = new Holder(b);
  return h.copied;
}
"#;
    let wasm = compile(src);
    let host = Host::with_options(Limits::default(), test_host_options());
    let mut rt = host.instantiate(&wasm, Grants::none()).expect("instantiate");
    assert_eq!(
        rt.call_export("test_ctor", &[Value::I32(1)]).unwrap(),
        vec![Value::I32(42)]
    );
    assert_eq!(
        rt.call_export("test_ctor", &[Value::I32(0)]).unwrap(),
        vec![Value::I32(-1)]
    );
}

#[test]
fn thousand_tiny_host_exports_compile_and_run_end_to_end() {
    let mut src = String::new();
    for i in 0..1000 {
        src.push_str(&format!(
            "@host export function f{i}(): i32 {{ return {i}; }}\n"
        ));
    }
    let wasm = compile(&src);
    let host = Host::with_options(Limits::default(), test_host_options());
    let mut rt = host
        .instantiate(&wasm, Grants::none())
        .expect("instantiate 1000 exports");
    assert_eq!(rt.call_export("f0", &[]).unwrap(), vec![Value::I32(0)]);
    assert_eq!(rt.call_export("f999", &[]).unwrap(), vec![Value::I32(999)]);
}

#[test]
fn instantiation_budget_is_configurable_and_reported() {
    let src = r#"
function id<T>(x: T): T { return x; }
@host export function run(): i32 {
  const a = id<i32>(1);
  const b = id<i64>(2);
  const c = id<f64>(3.0);
  return a + (b as i32) + (c as i32);
}
"#;
    let ok_opts = Options {
        max_instantiations: 10,
        ..Options::default()
    };
    let out = compile_program("main", src, &mut NoLoader, &interfaces(), &ok_opts)
        .expect("should succeed with budget 10");
    assert!(
        out.instantiations >= 3,
        "expected >= 3 instantiations reported, got {}",
        out.instantiations
    );

    let tight_opts = Options {
        max_instantiations: 2,
        ..Options::default()
    };
    let err = compile_program("main", src, &mut NoLoader, &interfaces(), &tight_opts)
        .err()
        .expect("should exceed budget 2");
    assert!(
        err.0.iter().any(|d| d.code == "OXS-E0701"),
        "expected OXS-E0701, got: {err:?}"
    );
}

#[test]
fn compound_assign_and_update_on_narrowed_nullable_compile_and_run() {
    let src = r#"
import { Log } from "host:log";

function add_if(a: i32 | null): i32 {
  let x: i32 | null = a;
  if (x !== null) {
    x += 5;
    x++;
    return x;
  }
  return 0;
}

function append_if(s: string | null): string {
  let t: string | null = s;
  if (t !== null) {
    t += "!";
    return t;
  }
  return "none";
}

export function main(log: Log): void {
  log.write(2, `${add_if(10)}`);
  log.write(2, `${add_if(null)}`);
  log.write(2, append_if("hi"));
  log.write(2, append_if(null));
}
"#;
    run_log(src, &["16", "0", "hi!", "none"]);
}
