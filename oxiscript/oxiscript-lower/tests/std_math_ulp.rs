//! Special-value tests and ULP accuracy tests for all 14 deterministic transcendental
//! functions in `std/math` against Rust `std::f64`.
//!
//! Run full 10^6 inputs per function:
//!   `cargo test --release -p oxiscript-lower --test std_math_ulp -- --nocapture`
#![allow(clippy::pedantic)]

mod common;

use common::Prog;
use oxiscript_runtime::{Limits, Value};

const MATH_HARNESS: &str = r#"
import {
  sin, cos, tan, asin, acos, atan, atan2,
  exp, log, log2, log10, pow, cbrt, hypot, scalbn
} from "std/math";

@host export function f_sin(x: f64): f64 { return sin(x); }
@host export function f_cos(x: f64): f64 { return cos(x); }
@host export function f_tan(x: f64): f64 { return tan(x); }
@host export function f_asin(x: f64): f64 { return asin(x); }
@host export function f_acos(x: f64): f64 { return acos(x); }
@host export function f_atan(x: f64): f64 { return atan(x); }
@host export function f_atan2(y: f64, x: f64): f64 { return atan2(y, x); }
@host export function f_exp(x: f64): f64 { return exp(x); }
@host export function f_log(x: f64): f64 { return log(x); }
@host export function f_log2(x: f64): f64 { return log2(x); }
@host export function f_log10(x: f64): f64 { return log10(x); }
@host export function f_pow(x: f64, y: f64): f64 { return pow(x, y); }
@host export function f_cbrt(x: f64): f64 { return cbrt(x); }
@host export function f_hypot(x: f64, y: f64): f64 { return hypot(x, y); }
@host export function f_scalbn(x: f64, n: i32): f64 { return scalbn(x, n); }
"#;

fn ulp_diff(a: f64, b: f64) -> u64 {
    if a.is_nan() && b.is_nan() {
        return 0;
    }
    if a.is_nan() || b.is_nan() {
        return u64::MAX;
    }
    if a == b {
        return 0;
    }
    if a.is_infinite() || b.is_infinite() {
        // Adjacent to infinity (f64::MAX vs INF is 1 ULP)
        if a.is_sign_positive() != b.is_sign_positive() {
            return u64::MAX;
        }
        let ua = a.to_bits();
        let ub = b.to_bits();
        return ua.abs_diff(ub);
    }
    // Map IEEE-754 f64 bits to a monotone signed integer order.
    let to_ord = |x: f64| -> i64 {
        let b = x.to_bits() as i64;
        if b < 0 {
            i64::MIN - b
        } else {
            b
        }
    };
    to_ord(a).abs_diff(to_ord(b))
}

fn call1(p: &mut Prog, name: &str, x: f64) -> f64 {
    match p.call(name, &[Value::F64(x)])[0] {
        Value::F64(v) => v,
        _ => panic!("expected f64"),
    }
}

fn call2(p: &mut Prog, name: &str, a: f64, b: f64) -> f64 {
    match p.call(name, &[Value::F64(a), Value::F64(b)])[0] {
        Value::F64(v) => v,
        _ => panic!("expected f64"),
    }
}

struct Rng(u64);
impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    /// Uniform in [0, 1)
    fn unit(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
    }
    /// Uniform in [lo, hi]
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    /// Log-uniform magnitude across [2^e_min, 2^e_max] with optional sign.
    fn log_range(&mut self, e_min: i32, e_max: i32, signed: bool) -> f64 {
        let span = (e_max - e_min + 1) as u64;
        let exp = (e_min + (self.next_u64() % span) as i32 + 1023) as u64;
        let mant = self.next_u64() & 0x000f_ffff_ffff_ffffu64;
        let sign = if signed && (self.next_u64() & 1) != 0 {
            1u64 << 63
        } else {
            0
        };
        f64::from_bits(sign | (exp << 52) | mant)
    }
}

#[test]
fn transcendentals_special_values_and_edges() {
    let wasm = common::compile(MATH_HARNESS);
    let mut p = Prog::from_wasm_limits(
        wasm,
        Limits {
            fuel_per_call: 50_000_000,
            ..Limits::default()
        },
    );

    // Special values for unary functions
    let specials = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        2.0,
        -2.0,
        std::f64::consts::PI,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::FRAC_PI_4,
        1e-308,
        1e-315, // subnormal
        1e-16,
        1e10,
        1e20,
        1e100,
        1e200,
        1e300,
        -1e100,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];

    type UnaryCase = (&'static str, fn(f64) -> f64, u64);
    let unary: &[UnaryCase] = &[
        ("f_sin", f64::sin, 2),
        ("f_cos", f64::cos, 2),
        ("f_tan", f64::tan, 2),
        ("f_asin", f64::asin, 2),
        ("f_acos", f64::acos, 2),
        ("f_atan", f64::atan, 2),
        ("f_exp", f64::exp, 2),
        ("f_log", f64::ln, 2),
        ("f_log2", f64::log2, 2),
        ("f_log10", f64::log10, 2),
        ("f_cbrt", f64::cbrt, 2),
    ];

    for &(name, ref_fn, max_ok) in unary {
        for &x in &specials {
            let got = call1(&mut p, name, x);
            let want = ref_fn(x);
            let u = ulp_diff(got, want);
            assert!(
                u <= max_ok,
                "{name}({x:e}): got={got:e} ({:#018x}) want={want:e} ({:#018x}) ulp={u}",
                got.to_bits(),
                want.to_bits()
            );
        }
    }

    // pow special matrix (IEEE-754)
    let pow_vals = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        2.0,
        -2.0,
        3.0,
        -3.0,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NAN,
    ];
    for &x in &pow_vals {
        for &y in &pow_vals {
            let got = call2(&mut p, "f_pow", x, y);
            let want = x.powf(y);
            let u = ulp_diff(got, want);
            assert!(
                u <= 2,
                "pow({x}, {y}): got={got} ({:#018x}) want={want} ({:#018x}) ulp={u}",
                got.to_bits(),
                want.to_bits()
            );
        }
    }

    // atan2 & hypot special matrix
    for &y in &pow_vals {
        for &x in &pow_vals {
            let got_a = call2(&mut p, "f_atan2", y, x);
            let want_a = y.atan2(x);
            let ua = ulp_diff(got_a, want_a);
            assert!(ua <= 2, "atan2({y}, {x}): got={got_a} want={want_a} ulp={ua}");

            let got_h = call2(&mut p, "f_hypot", y, x);
            let want_h = y.hypot(x);
            let uh = ulp_diff(got_h, want_h);
            assert!(uh <= 2, "hypot({y}, {x}): got={got_h} want={want_h} ulp={uh}");
        }
    }

    p.finish();
}

#[test]
fn transcendentals_seeded_ulp_accuracy() {
    // Full 10^6 inputs per function in release mode (or when OXI_ULP_FULL=1);
    // 2_000 inputs per function in debug mode so `cargo test` finishes quickly.
    let n: usize = if !cfg!(debug_assertions) || std::env::var_os("OXI_ULP_FULL").is_some() {
        1_000_000
    } else {
        2_000
    };

    let wasm = common::compile(MATH_HARNESS);
    let make_prog = || {
        Prog::from_wasm_limits(
            wasm.clone(),
            Limits {
                fuel_per_call: 50_000_000,
                ..Limits::default()
            },
        )
    };
    let mut p = make_prog();

    let mut rng = Rng(0x2026_1126_0008_57d0);
    let mut report: Vec<(&str, u64, f64, f64)> = Vec::new();

    // Helper closure for unary functions (runs on a single instance with zero allocations).
    let mut run_unary = |name: &'static str,
                         gen: &mut dyn FnMut(&mut Rng, usize) -> f64,
                         ref_fn: fn(f64) -> f64,
                         bound: u64| {
        let mut max_u: u64 = 0;
        let mut worst_x: f64 = 0.0;
        for i in 0..n {
            let x = gen(&mut rng, i);
            let got = call1(&mut p, name, x);
            let want = ref_fn(x);
            let u = ulp_diff(got, want);
            if u > max_u {
                max_u = u;
                worst_x = x;
            }
        }
        eprintln!("{name:10} n={n:<7} max_ulp={max_u} (worst x={worst_x:e})");
        assert!(
            max_u <= bound,
            "{name}: max_ulp {max_u} exceeded bound {bound} at x={worst_x:e}"
        );
        report.push((name, max_u, worst_x, 0.0));
    };

    // Trig: mix small [-10, 10], medium [-1e5, 1e5], and large Payne-Hanek inputs [1e6, 1e100]
    let mut trig_gen = |r: &mut Rng, i: usize| -> f64 {
        match i % 5 {
            0 => r.range(-10.0, 10.0),
            1 => r.range(-1e5, 1e5),
            2 => r.log_range(-30, 15, true),
            3 => r.log_range(16, 40, true),
            _ => r.log_range(41, 300, true),
        }
    };
    run_unary("f_sin", &mut trig_gen, f64::sin, 2);
    run_unary("f_cos", &mut trig_gen, f64::cos, 2);
    run_unary("f_tan", &mut trig_gen, f64::tan, 2);

    let mut unit_gen = |r: &mut Rng, i: usize| -> f64 {
        if i.is_multiple_of(3) {
            r.range(-1.0, 1.0)
        } else if i % 3 == 1 {
            // near +/- 1
            let d = r.log_range(-50, -2, false);
            if r.next_u64() & 1 == 0 { 1.0 - d } else { -1.0 + d }
        } else {
            r.log_range(-50, -1, true)
        }
    };
    run_unary("f_asin", &mut unit_gen, f64::asin, 2);
    run_unary("f_acos", &mut unit_gen, f64::acos, 2);

    let mut atan_gen = |r: &mut Rng, i: usize| -> f64 {
        if i.is_multiple_of(2) {
            r.range(-5.0, 5.0)
        } else {
            r.log_range(-40, 60, true)
        }
    };
    run_unary("f_atan", &mut atan_gen, f64::atan, 2);

    let mut exp_gen = |r: &mut Rng, i: usize| -> f64 {
        match i % 3 {
            0 => r.range(-708.0, 708.0),
            1 => r.range(-745.0, -708.0), // subnormal output range
            _ => r.range(-20.0, 20.0),
        }
    };
    run_unary("f_exp", &mut exp_gen, f64::exp, 2);

    let mut pos_gen = |r: &mut Rng, i: usize| -> f64 {
        match i % 4 {
            0 => r.log_range(-1020, 1020, false),
            1 => r.range(0.5, 2.0),
            2 => 1.0 + r.log_range(-45, -2, true),
            _ => r.log_range(-1070, -1023, false).max(f64::MIN_POSITIVE * 1e-10),
        }
    };
    run_unary("f_log", &mut pos_gen, f64::ln, 2);
    run_unary("f_log2", &mut pos_gen, f64::log2, 2);
    run_unary("f_log10", &mut pos_gen, f64::log10, 2);

    let mut cbrt_gen = |r: &mut Rng, i: usize| -> f64 {
        if i.is_multiple_of(4) {
            r.log_range(-1070, -1020, true)
        } else {
            r.log_range(-1000, 1000, true)
        }
    };
    run_unary("f_cbrt", &mut cbrt_gen, f64::cbrt, 2);

    // Binary functions: atan2, pow, hypot
    let mut run_binary = |name: &'static str,
                          gen: &mut dyn FnMut(&mut Rng, usize) -> (f64, f64),
                          ref_fn: fn(f64, f64) -> f64,
                          bound: u64| {
        let mut max_u: u64 = 0;
        let mut worst = (0.0, 0.0);
        for i in 0..n {
            let (a, b) = gen(&mut rng, i);
            let got = call2(&mut p, name, a, b);
            let want = ref_fn(a, b);
            let u = ulp_diff(got, want);
            if u > max_u {
                max_u = u;
                worst = (a, b);
            }
        }
        eprintln!("{name:10} n={n:<7} max_ulp={max_u} (worst=({:e}, {:e}))", worst.0, worst.1);
        assert!(
            max_u <= bound,
            "{name}: max_ulp {max_u} exceeded bound {bound} at ({:e}, {:e})",
            worst.0,
            worst.1
        );
        report.push((name, max_u, worst.0, worst.1));
    };

    run_binary(
        "f_atan2",
        &mut |r, _| (r.log_range(-500, 500, true), r.log_range(-500, 500, true)),
        f64::atan2,
        2,
    );
    run_binary(
        "f_hypot",
        &mut |r, i| {
            if i.is_multiple_of(3) {
                (r.log_range(400, 1020, true), r.log_range(400, 1020, true))
            } else if i % 3 == 1 {
                (r.log_range(-1020, -400, true), r.log_range(-1020, -400, true))
            } else {
                (r.log_range(-100, 100, true), r.log_range(-100, 100, true))
            }
        },
        f64::hypot,
        2,
    );
    run_binary(
        "f_pow",
        &mut |r, i| match i % 4 {
            0 => (r.range(0.01, 100.0), r.range(-100.0, 100.0)),
            1 => (r.log_range(-20, 20, false), r.range(-30.0, 30.0)),
            2 => (1.0 + r.range(-0.1, 0.1), r.range(-500.0, 500.0)),
            _ => {
                // negative base with integer exponent
                let e = ((r.next_u64() % 41) as i32 - 20) as f64;
                (-r.range(0.1, 10.0), e)
            }
        },
        f64::powf,
        2,
    );

    p.finish();
}
