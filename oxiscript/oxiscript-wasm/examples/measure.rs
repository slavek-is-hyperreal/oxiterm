#![allow(clippy::pedantic)]
//! Codegen speed and output size. `cargo run --release -p oxiscript-wasm --example measure`
//!
//! The corpus is the `.oxir` programs of `oxiscript-ir` plus seeded random
//! modules (the differential-test generator is not available here, so a
//! synthetic generator builds many medium-sized arithmetic/loop functions).
use oxiscript_abi::idl;
use oxiscript_ir::{parse_module, print_module, VerifyOptions};
use oxiscript_wasm::{compile_with_stats, CompileOptions, RuntimeMode};
use std::fmt::Write as _;
use std::time::Instant;

const PROGRAMS: &[(&str, &str)] = &[
    ("arith", include_str!("../../oxiscript-ir/tests/programs/arith.oxir")),
    ("loop", include_str!("../../oxiscript-ir/tests/programs/loop.oxir")),
    ("structs", include_str!("../../oxiscript-ir/tests/programs/structs.oxir")),
    ("closure", include_str!("../../oxiscript-ir/tests/programs/closure.oxir")),
    ("host", include_str!("../../oxiscript-ir/tests/programs/host.oxir")),
    ("rc", include_str!("../../oxiscript-ir/tests/programs/rc.oxir")),
];

/// A synthetic module of `n` functions, each a loop with branches and calls.
fn synth(n: usize) -> String {
    let mut s = String::from("oxiir 1\nmodule \"synth\"\n");
    for i in 0..n {
        let _ = write!(
            s,
            "export func @f{i}(%a: i64, %b: i32) -> i64 {{\nentry:\n  %z: i32 = const 0\n  %a0: i64 = const 1\n  br loop(%z, %a0)\nloop(%i: i32, %acc: i64):\n  %d: bool = ge %i, %b\n  br_if %d, exit(%acc), body()\nbody:\n  %w: i64 = conv %i\n  %m: i64 = mul.wrap %acc, %a\n  %k: i64 = const 7\n  %r: i64 = rem.wrap %m, %k\n  %odd: bool = gt %r, %k\n  br_if %odd, hi(%acc), lo(%acc)\nhi(%x: i64):\n  %y: i64 = add.wrap %x, %w\n  br join(%y)\nlo(%x2: i64):\n  %y2: i64 = sub.wrap %x2, %w\n  br join(%y2)\njoin(%v: i64):\n  %one: i32 = const 1\n  %i2: i32 = add.wrap %i, %one\n  br loop(%i2, %v)\nexit(%res: i64):\n  ret %res\n}}\n"
        );
    }
    s
}

fn bench(name: &str, src: &str, iters: usize, runtime: RuntimeMode) -> (usize, usize, f64) {
    let mut ifaces = idl::parse(idl::EXAMPLE_LOG_IDL).unwrap();
    ifaces.extend(idl::parse(idl::EXAMPLE_STATE_IDL).unwrap());
    let m = parse_module(src).unwrap();
    let opts = CompileOptions {
        interfaces: ifaces,
        verify: VerifyOptions::default(),
        runtime,
        ..CompileOptions::default()
    };
    let funcs = m.funcs.len();
    let ir_bytes = print_module(&m).len();
    let t = Instant::now();
    let mut out = 0;
    for _ in 0..iters {
        out = compile_with_stats(&m, &opts).unwrap().stats.bytes;
    }
    let secs = t.elapsed().as_secs_f64() / iters as f64;
    println!(
        "{name:<10} {runtime:?}: {funcs:>5} fns, IR {ir_bytes:>8} B -> Wasm {out:>8} B, {:>8.3} ms/compile, {:>9.0} fn/s, {:>8.0} KB/s IR",
        secs * 1e3,
        funcs as f64 / secs,
        ir_bytes as f64 / 1024.0 / secs
    );
    (funcs, out, secs)
}

fn main() {
    for mode in [RuntimeMode::Import, RuntimeMode::Stub] {
        for (n, src) in PROGRAMS {
            bench(n, src, 300, mode);
        }
    }
    let big = synth(2000);
    bench("synth2000", &big, 5, RuntimeMode::Import);
    bench("synth2000", &big, 5, RuntimeMode::Stub);
}
