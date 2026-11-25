//! BAD ACTOR 8.G: integer semantics at exact boundaries. 8-bit types are
//! checked exhaustively over all operand pairs, wider types over boundary
//! sets, against an exact i128 model of ir.md section 3.
#![allow(clippy::pedantic)]
mod common;
use common::*;
use oxiscript_abi::trap::TrapKind;
use oxiscript_ir::parse_module;
use wasmi::{Engine, Instance, Linker, Module, Store, Val};

struct W {
    store: Store<()>,
    inst: Instance,
}
impl W {
    fn new(wasm: &[u8]) -> W {
        let engine = Engine::default();
        let module = Module::new(&engine, wasm).unwrap();
        let mut store = Store::new(&engine, ());
        let linker = <Linker<()>>::new(&engine);
        let inst = linker.instantiate_and_start(&mut store, &module).unwrap();
        W { store, inst }
    }
    fn call(&mut self, name: &str, args: &[Val]) -> Result<Val, TrapKind> {
        let g = self.inst.get_global(&self.store, "oxi_trap_kind").unwrap();
        g.set(&mut self.store, Val::I32(0)).unwrap();
        let f = self.inst.get_func(&self.store, name).unwrap();
        let mut res = [Val::I32(0)];
        match f.call(&mut self.store, args, &mut res) {
            Ok(()) => Ok(res[0].clone()),
            Err(_) => {
                let Val::I32(k) = g.get(&self.store) else { panic!() };
                Err(TrapKind::from_code(k as u32).unwrap_or_else(|| panic!("{name}: engine trap without guest kind")))
            }
        }
    }
}
use Val as Value;
use std::fmt::Write as _;

const TYS: [(&str, u32, bool); 8] = [
    ("i8", 8, true),
    ("u8", 8, false),
    ("i16", 16, true),
    ("u16", 16, false),
    ("i32", 32, true),
    ("u32", 32, false),
    ("i64", 64, true),
    ("u64", 64, false),
];

fn range(bits: u32, signed: bool) -> (i128, i128) {
    if signed {
        (-(1i128 << (bits - 1)), (1i128 << (bits - 1)) - 1)
    } else {
        (0, (1i128 << bits) - 1)
    }
}
fn wrapv(bits: u32, signed: bool, v: i128) -> i128 {
    let m = v & ((1i128 << bits) - 1);
    if signed && m >= (1i128 << (bits - 1)) { m - (1i128 << bits) } else { m }
}


/// wasmi's strict limits demand >= 40 bytes per function on average (a known
/// cross-slice finding of 8.G): one big function keeps the average up.
fn pad(src: &mut String) {
    src.push_str("export func @pad(%a: i32) -> i32 {\ne:\n");
    let mut prev = "%a".to_string();
    for i in 0..9000 {
        let _ = writeln!(src, "  %p{i}: i32 = add.wrap {prev}, %a");
        prev = format!("%p{i}");
    }
    let _ = write!(src, "  ret {prev}\n}}\n");
}

type R = Result<i128, TrapKind>;

fn model(op: &str, mode: &str, bits: u32, signed: bool, a: i128, b: i128) -> R {
    let (lo, hi) = range(bits, signed);
    let fin = |v: i128| -> R {
        if v >= lo && v <= hi {
            Ok(v)
        } else {
            match mode {
                "t" => Err(TrapKind::Overflow),
                "w" => Ok(wrapv(bits, signed, v)),
                _ => Ok(v.clamp(lo, hi)),
            }
        }
    };
    match op {
        "add" => fin(a + b),
        "sub" => fin(a - b),
        "mul" => match a.checked_mul(b) {
            Some(v) => fin(v),
            // 64x64 bit products can exceed i128: always out of range, wrap uses the low bits
            None => match mode {
                "t" => Err(TrapKind::Overflow),
                "w" => Ok(wrapv(bits, signed, a.wrapping_mul(b))),
                _ => Ok(if (a < 0) != (b < 0) { lo } else { hi }),
            },
        },
        "div" | "rem" => {
            if b == 0 {
                return Err(TrapKind::DivByZero);
            }
            if signed && a == lo && b == -1 {
                return if mode == "t" { Err(TrapKind::Overflow) } else if op == "div" { Ok(lo) } else { Ok(0) };
            }
            Ok(if op == "div" { a / b } else { a % b })
        }
        "shl" | "shr" => {
            let amt = if mode == "t" {
                if b < 0 || b >= i128::from(bits) {
                    return Err(TrapKind::Bounds);
                }
                b
            } else {
                b & i128::from(bits - 1)
            };
            if op == "shl" {
                Ok(wrapv(bits, signed, a << amt))
            } else {
                Ok(a >> amt) // arithmetic for negative (signed), values are non-negative if unsigned
            }
        }
        _ => unreachable!(),
    }
}

fn to_val(bits: u32, v: i128) -> Val {
    if bits == 64 { Val::I64(v as i64) } else { Val::I32(v as i64 as i32) }
}
fn from_val(_bits: u32, signed: bool, v: &Val) -> i128 {
    match v {
        Value::I32(x) => if signed { i128::from(*x) } else { i128::from(*x as u32) },
        Value::I64(x) => if signed { i128::from(*x) } else { i128::from(*x as u64) },
        _ => panic!(),
    }
}

fn edges(bits: u32, signed: bool) -> Vec<i128> {
    let (lo, hi) = range(bits, signed);
    let mut v = vec![lo, lo + 1, lo + 2, hi, hi - 1, hi - 2, 0, 1, 2, 3, 7, 8, 15, 16, 31, 32, 33, 63, 64, 65, 127, 128, 255, 256, 1 << (bits - 1), (1 << (bits - 1)) - 1, -1, -2, -3, -8, -64, -128];
    v.retain(|x| *x >= lo && *x <= hi);
    v.sort();
    v.dedup();
    v
}

#[test]
fn binary_ops_match_model() {
    let ops: [(&str, &[&str]); 7] = [
        ("add", &["t", "w", "s"]),
        ("sub", &["t", "w", "s"]),
        ("mul", &["t", "w", "s"]),
        ("div", &["t", "w"]),
        ("rem", &["t", "w"]),
        ("shl", &["t", "w"]),
        ("shr", &["t", "w"]),
    ];
    let mut src = String::from("oxiir 1\nmodule \"int\"\n");
    for (t, _, _) in TYS {
        for (op, modes) in ops {
            for mode in modes {
                let suffix = match *mode { "t" => "", "w" => ".wrap", _ => ".sat" };
                let _ = write!(src, "export func @{op}_{t}_{mode}(%a: {t}, %b: {t}) -> {t} {{\ne:\n  %r: {t} = {op}{suffix} %a, %b\n  ret %r\n}}\n");
            }
        }
    }
    pad(&mut src);
    let m = parse_module(&src).unwrap();
    let wasm = build(&m);
    let mut rt = W::new(&wasm);
    let mut cases = 0usize;
    for (t, bits, signed) in TYS {
        let vals: Vec<i128> = if bits == 8 {
            let (lo, hi) = range(bits, signed);
            (lo..=hi).collect()
        } else {
            edges(bits, signed)
        };
        for (op, modes) in ops {
            for mode in modes {
                let f = format!("{op}_{t}_{mode}");
                for &a in &vals {
                    for &b in &vals {
                        // shift amounts: also pass non-canonical-range amounts as typed values only
                        let want = model(op, mode, bits, signed, a, b);
                        let got = rt.call(&f, &[to_val(bits, a), to_val(bits, b)]);
                        cases += 1;
                        match (want, got) {
                            (Ok(w), Ok(g)) => assert_eq!(from_val(bits, signed, &g), w, "{f}({a}, {b})"),
                            (Err(k), Err(tr)) => assert_eq!(tr, k, "{f}({a}, {b})"),
                            (w, g) => panic!("{f}({a}, {b}): model {w:?} vs wasm {g:?}"),
                        }
                    }
                }
            }
        }
    }
    eprintln!("int binary cases: {cases}");
}

#[test]
fn unary_ops_match_model() {
    let mut src = String::from("oxiir 1\nmodule \"un\"\n");
    for (t, _, signed) in TYS {
        let _ = write!(src, "export func @negw_{t}(%a: {t}) -> {t} {{\ne:\n  %r: {t} = neg.wrap %a\n  ret %r\n}}\n");
        if signed {
            let _ = write!(src, "export func @negt_{t}(%a: {t}) -> {t} {{\ne:\n  %r: {t} = neg %a\n  ret %r\n}}\n");
        }
        for op in ["not", "clz", "ctz", "popcnt"] {
            let _ = write!(src, "export func @{op}_{t}(%a: {t}) -> {t} {{\ne:\n  %r: {t} = {op} %a\n  ret %r\n}}\n");
        }
    }
    pad(&mut src);
    let m = parse_module(&src).unwrap();
    let wasm = build(&m);
    let mut rt = W::new(&wasm);
    for (t, bits, signed) in TYS {
        let (lo, hi) = range(bits, signed);
        let vals: Vec<i128> = if bits <= 16 { (lo..=hi).collect() } else { edges(bits, signed) };
        for &a in &vals {
            let ua = (a as u128) & ((1u128 << bits) - 1);
            let mut checks: Vec<(String, R)> = vec![
                (format!("negw_{t}"), Ok(wrapv(bits, signed, -a))),
                (format!("not_{t}"), Ok(wrapv(bits, signed, !a))),
                (format!("clz_{t}"), Ok(i128::from(ua.leading_zeros() - (128 - bits)))),
                (format!("ctz_{t}"), Ok(if ua == 0 { i128::from(bits) } else { i128::from(ua.trailing_zeros()) })),
                (format!("popcnt_{t}"), Ok(i128::from(ua.count_ones()))),
            ];
            if signed {
                checks.push((format!("negt_{t}"), if a == lo { Err(TrapKind::Overflow) } else { Ok(-a) }));
            }
            for (f, want) in checks {
                let got = rt.call(&f, &[to_val(bits, a)]);
                match (want, got) {
                    (Ok(w), Ok(g)) => assert_eq!(from_val(bits, signed, &g), w, "{f}({a})"),
                    (Err(k), Err(tr)) => assert_eq!(tr, k, "{f}({a})"),
                    (w, g) => panic!("{f}({a}): model {w:?} vs wasm {g:?}"),
                }
            }
        }
        let _ = hi;
    }
}

#[test]
fn int_conversions_match_model() {
    // every (from, to) pair, trap / wrap / sat, boundary and exhaustive-narrow inputs
    let mut src = String::from("oxiir 1\nmodule \"cv\"\n");
    for (f, _, _) in TYS {
        for (t, _, _) in TYS {
            if f == t {
                continue;
            }
            for (m, suffix) in [("t", ""), ("w", ".wrap"), ("s", ".sat")] {
                let _ = write!(src, "export func @c_{f}_{t}_{m}(%a: {f}) -> {t} {{\ne:\n  %r: {t} = conv{suffix} %a\n  ret %r\n}}\n");
            }
        }
    }
    pad(&mut src);
    let m = parse_module(&src).unwrap();
    let wasm = build(&m);
    let mut rt = W::new(&wasm);
    let mut n = 0usize;
    for (f, fb, fs) in TYS {
        let (flo, fhi) = range(fb, fs);
        let vals: Vec<i128> = if fb <= 8 { (flo..=fhi).collect() } else { edges(fb, fs) };
        for (t, tb, ts) in TYS {
            if f == t {
                continue;
            }
            let (tlo, thi) = range(tb, ts);
            for (m, _) in [("t", ""), ("w", ""), ("s", "")] {
                for &a in &vals {
                    let want: R = match m {
                        "t" => if a >= tlo && a <= thi { Ok(a) } else { Err(TrapKind::Overflow) },
                        "w" => Ok(wrapv(tb, ts, a)),
                        _ => Ok(a.clamp(tlo, thi)),
                    };
                    let name = format!("c_{f}_{t}_{m}");
                    let got = rt.call(&name, &[to_val(fb, a)]);
                    n += 1;
                    match (want, got) {
                        (Ok(w), Ok(g)) => assert_eq!(from_val(tb, ts, &g), w, "{name}({a})"),
                        (Err(k), Err(tr)) => assert_eq!(tr, k, "{name}({a})"),
                        (w, g) => panic!("{name}({a}): model {w:?} vs wasm {g:?}"),
                    }
                }
            }
        }
    }
    eprintln!("int conv cases: {n}");
}
