#![allow(clippy::pedantic, clippy::nursery)]
//! Stage 8.M-link gate tests: real `oxiscript-rc` runtime linked into one Core Wasm module.
//!
//! Covers:
//! - `${x}` with `f64` and `f64.toString()` exact formatting (abi.md 3.1)
//! - Runtime-raised traps (`BadUtf8`, `OutOfMemory`, `Bounds`) via `oxi_trap_kind`
//! - Cycle collection (closures and class graphs) -> 0 live objects after `deinit`
//! - Stub vs Linked output parity + zero `oxi:rt` imports + `hello.oxs` size comparison
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_lower::{compile_source, Options};
use oxiscript_runtime::{Grants, Limits};
use oxiscript_wasm::RuntimeMode;
use std::path::PathBuf;

const HDR: &str = "import { Log } from \"host:log\";\n";

#[test]
fn linked_f64_template_and_to_string_formatting() {
    run_log(
        &format!(
            "{HDR}{}",
            r#"
export function main(log: Log): void {
  const zero: f64 = 0.0;
  const negZero: f64 = -0.0;
  const one: f64 = 1.0;
  const negInt: f64 = -42.0;
  const tenth: f64 = 0.1;
  const half: f64 = 1.5;
  const pi: f64 = -3.14;
  const big: f64 = 1e21;
  const small: f64 = 1e-7;
  const nanVal: f64 = zero / zero;
  const posInf: f64 = one / zero;
  const negInf: f64 = -one / zero;

  log.write(2, `${zero}|${negZero}|${one}|${negInt}`);
  log.write(2, `${tenth}|${half}|${pi}`);
  log.write(2, `${big}|${small}`);
  log.write(2, `${nanVal}|${posInf}|${negInf}`);
  log.write(2, one.toString() + "," + tenth.toString() + "," + negZero.toString() + "," + nanVal.toString() + "," + posInf.toString() + "," + negInf.toString());
  const f: f32 = 2.5f32;
  log.write(2, `${f}|${f.toString()}`);
}
"#
        ),
        &[
            "0|-0|1|-42",
            "0.1|1.5|-3.14",
            "1e21|1e-7",
            "NaN|Infinity|-Infinity",
            "1,0.1,-0,NaN,Infinity,-Infinity",
            "2.5|2.5",
        ],
    );
}

#[test]
fn linked_runtime_traps_surface_through_oxi_trap_kind() {
    // 1. BadUtf8 from `oxi_str_slice` (mid-code-point slice of 2-byte UTF-8 "é")
    let mut p = Prog::new(&format!(
        "{HDR}{}",
        r#"
@host export function bad_slice(): i32 {
  const s = "h\u{00e9}llo";
  const cut = s.slice(0, 2);
  return cut.byteLength;
}
@host export function oob_slice(): i32 {
  const s = "abc";
  const cut = s.slice(2, 1);
  return cut.byteLength;
}
export function main(log: Log): void { log.write(2, "up"); }
"#
    ));
    let e = p.call_err("bad_slice", &[]);
    assert_eq!(trap_kind(&e), Some(TrapKind::BadUtf8), "got {e}");

    // 2. Bounds from `oxi_str_slice`
    let mut p2 = Prog::from_wasm(p.wasm.clone());
    let e2 = p2.call_err("oob_slice", &[]);
    assert_eq!(trap_kind(&e2), Some(TrapKind::Bounds), "got {e2}");

    // 3. OutOfMemory from `oxi_alloc` / `oxi_array_grow` under a tight memory page limit
    let oom_src = format!(
        "{HDR}{}",
        r#"
export function main(log: Log): void {
  const chunks: i32[][] = [];
  for (let i = 0; i < 10000; i++) {
    const a: i32[] = [];
    for (let j = 0; j < 2000; j++) {
      a.push(j);
    }
    chunks.push(a);
  }
  log.write(2, `${chunks.length}`);
}
"#
    );
    let oom_wasm = compile(&oom_src);
    let tight = Limits {
        max_memory_pages: 4, // 256 KiB cap
        fuel_per_call: u64::MAX / 4,
        ..Limits::default()
    };
    let env = env_with_limits(tight);
    let err = match env.host.instantiate(&oom_wasm, Grants::all()) {
        Ok(_) => panic!("expected OutOfMemory trap"),
        Err(e) => e,
    };
    assert_eq!(trap_kind(&err), Some(TrapKind::OutOfMemory), "OOM got {err}");
}

#[test]
fn linked_collects_closure_and_object_cycles_to_zero() {
    let src = format!(
        "{HDR}{}",
        r#"
class Node {
  label: string;
  next: Node | null = null;
  cb: (() => string) | null = null;
  constructor(label: string) { this.label = label; }
}
export function main(log: Log): void {
  // 2-node object cycle + closure capturing self
  const a = new Node("A" + "1");
  const b = new Node("B" + "2");
  a.next = b;
  b.next = a;
  a.cb = () => {
    const nxt = a.next;
    return a.label + ":" + (nxt !== null ? nxt.label : "");
  };
  const f = a.cb;
  if (f !== null) { log.write(2, f()); }
  // mutually recursive closures
  let even: (n: i32) => bool = (n) => n === 0;
  let odd: (n: i32) => bool = (n) => n !== 0;
  even = (n) => n === 0 ? true : odd(n - 1);
  odd = (n) => n === 0 ? false : even(n - 1);
  log.write(2, `${even(10)} ${odd(9)} ${even(7)}`);
}
"#
    );
    let mut p = Prog::new(&src);
    assert_eq!(p.log(), vec!["A1:B2", "true true false"]);
    // Before deinit/collect, the cyclic garbage is still counted live.
    assert!(p.rt.live_objects().unwrap() > 0, "cycle should exist before collection");
    let freed = p.rt.collect_cycles().unwrap();
    assert!(freed >= 4, "expected cycle collector to free cycle nodes, freed {freed}");
    p.finish();
}

#[test]
fn linked_has_no_rt_imports_matches_stub_and_reports_hello_size() {
    let examples_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples");
    for name in ["hello", "counter", "errors", "laws", "shapes", "structs"] {
        let src = std::fs::read_to_string(examples_dir.join(format!("{name}.oxs"))).unwrap();
        let stub_wasm = compile_source(
            &src,
            &interfaces(),
            &Options {
                runtime: RuntimeMode::Stub,
                ..Options::default()
            },
        )
        .unwrap();
        let linked_wasm = compile_source(
            &src,
            &interfaces(),
            &Options {
                runtime: RuntimeMode::Linked,
                ..Options::default()
            },
        )
        .unwrap();
        if name == "hello" {
            eprintln!(
                "hello.oxs module size: Stub = {} bytes, Linked = {} bytes",
                stub_wasm.len(),
                linked_wasm.len()
            );
        }
        // Verify merged module passes strict ABI 1.0 Core Wasm validation and strict host load checks
        // (which enforce zero non-manifest / `oxi:rt` imports, 1 memory, 1 table, and full ABI export table).
        oxiscript_wasm::validate(&linked_wasm).unwrap();

        // Stub and Linked produce identical log output and zero leaks.
        let mut ps = Prog::from_wasm(stub_wasm);
        let mut pl = Prog::from_wasm(linked_wasm);
        assert_eq!(ps.log(), pl.log(), "{name}: Stub vs Linked log mismatch");
        ps.finish();
        pl.finish();
    }
}
