//! Stage 8.Sec regression tests for `oxiscript-lower`:
//! - `[R3-69, R3-163, BL04]` reproducible compiler output (byte-identical IR & Wasm)
//! - `[R3-213]` fuel and memory-limit fault-injection sweep on a compiled program
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::*;
use oxiscript_abi::target::Target;
use oxiscript_lower::{compile_program, Options};
use oxiscript_runtime::{Error, Grants, Host, HostOptions, Limits, TrapCause, Value};

const RICH_PROGRAM: &str = r#"
import { Log } from "host:log";
import { State } from "host:state";
import { sqrt, clamp } from "std/math";
import { Map } from "std/collections";

struct Vec2 {
  x: f64;
  y: f64;
}

type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };

class Acc {
  total: i32;
  constructor(init: i32) {
    this.total = init;
  }
  add(n: i32): i32 {
    this.total = this.total + n;
    return this.total;
  }
}

function area(s: Shape): f64 {
  switch (s.kind) {
    case "circle":
      return 3.14159 * s.r * s.r;
    case "rect":
      return s.w * s.h;
  }
}

function makeAdder(base: i32): (x: i32) => i32 {
  return (x: i32): i32 => base + x;
}

export function main(log: Log, state: State): void {
  const acc = new Acc(10);
  const add5 = makeAdder(5);
  const m = new Map<string, i32>();
  m.set("a", add5(acc.add(3)));
  m.set("b", (clamp(sqrt(area({ kind: "rect", w: 4.0, h: 9.0 })), 0.0, 100.0)) as i32);
  const v = (m.get("a") ?? 0) + (m.get("b") ?? 0);
  log.write(2, `v=${v}`);
  const slot = state.open("v");
  switch (slot.kind) {
    case "ok":
      const _ = state.set_i64(slot.value, v as i64);
    case "err":
      log.write(4, "err");
  }
}
"#;

#[test]
fn reproducible_compiler_output_is_byte_identical_across_builds() {
    for target in [Target::Device, Target::Web] {
        let opts = Options {
            target,
            ..Options::default()
        };
        let a = compile_program("main", RICH_PROGRAM, &mut oxiscript_check::NoLoader, &interfaces(), &opts)
            .expect("first compile");
        let b = compile_program("main", RICH_PROGRAM, &mut oxiscript_check::NoLoader, &interfaces(), &opts)
            .expect("second compile");
        assert_eq!(
            oxiscript_ir::print_module(&a.module),
            oxiscript_ir::print_module(&b.module),
            "IR text differed across builds for target {target}"
        );
        assert_eq!(
            a.wasm, b.wasm,
            "emitted .wasm bytes differed across builds for target {target}"
        );
    }
}

#[test]
fn compiled_program_fuel_and_oom_fault_injection_sweep() {
    let src = r#"
class Node {
  v: i32;
  next: Node | null;
  constructor(v: i32, next: Node | null) {
    this.v = v;
    this.next = next;
  }
}

@host export function build(n: i32): i32 {
  let head: Node | null = null;
  let arr: i32[] = [];
  for (const i of range(0, n)) {
    head = new Node(i, head);
    arr.push(i);
  }
  let sum: i32 = 0;
  let cur: Node | null = head;
  while (cur !== null) {
    sum = sum + cur.v;
    cur = cur.next;
  }
  return sum + arr.length;
}
"#;
    let wasm = compile(src);

    // Measure full completion fuel for n=25.
    let base_host = Host::with_options(Limits::default(), test_host_options());
    let mut base_rt = base_host.instantiate(&wasm, Grants::all()).unwrap();
    let expected = base_rt.call_export("build", &[Value::I32(25)]).unwrap();
    assert_eq!(expected, vec![Value::I32(300 + 25)]);
    let full_fuel = base_rt.last_fuel_used();

    // Sweep fuel across 30 points from 1 up to full_fuel + 100.
    let stride = (full_fuel / 30).max(1);
    let mut saw_trap = false;
    let mut saw_ok = false;
    let mut f = 1u64;
    while f <= full_fuel + stride {
        let host = Host::with_options(
            Limits {
                fuel_per_call: f,
                ..Limits::default()
            },
            test_host_options(),
        );
        match host.instantiate(&wasm, Grants::all()) {
            Err(Error::Trap(info)) => {
                assert_eq!(info.cause, TrapCause::OutOfFuel);
                saw_trap = true;
            }
            Ok(mut rt) => match rt.call_export("build", &[Value::I32(25)]) {
                Ok(v) => {
                    assert_eq!(v, expected);
                    saw_ok = true;
                }
                Err(Error::Trap(info)) => {
                    assert_eq!(info.cause, TrapCause::OutOfFuel);
                    assert!(rt.is_poisoned());
                    saw_trap = true;
                }
                Err(e) => panic!("unexpected error at fuel={f}: {e:?}"),
            },
            Err(e) => panic!("unexpected init error at fuel={f}: {e:?}"),
        }
        f += stride;
    }
    assert!(saw_trap && saw_ok);

    // Sweep memory page caps (1..=4 pages) with large allocation: must trap cleanly on OOM, never panic.
    let mut saw_oom = false;
    for pages in 1..=4u32 {
        let host = Host::with_options(
            Limits {
                max_memory_pages: pages,
                fuel_per_call: 50_000_000,
                ..Limits::default()
            },
            HostOptions {
                max_target: test_target(),
                ..HostOptions::default()
            },
        );
        let mut rt = match host.instantiate(&wasm, Grants::all()) {
            Ok(r) => r,
            Err(Error::Rejected(r)) if r.code == "OXS-E0409" => {
                saw_oom = true;
                continue;
            }
            Err(e) => panic!("unexpected init error for pages={pages}: {e:?}"),
        };
        match rt.call_export("build", &[Value::I32(20_000)]) {
            Err(Error::Trap(_)) => {
                assert!(rt.is_poisoned());
                saw_oom = true;
            }
            Ok(_) => {}
            Err(e) => panic!("unexpected error for pages={pages}: {e:?}"),
        }
    }
    assert!(saw_oom, "expected tight memory page cap to trigger clean OOM trap");
}
