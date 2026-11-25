//! BAD ACTOR 8.G: differential fuzz over *arbitrary* random CFGs (forward and
//! back edges, critical edges, multi-latch loops, switches with block args,
//! unreachable blocks). Irreducible graphs must give OXS-E0600 (and nothing
//! else); every accepted program must agree with the reference interpreter.
#![allow(clippy::pedantic)]
mod common;

use common::gen::Rng;
use common::interp::*;
use common::*;
use oxiscript_ir::{parse_module, verify, Module, Ty};
use oxiscript_runtime::{Error, Value};
use oxiscript_wasm::{compile, E_IRREDUCIBLE};
use std::fmt::Write as _;

struct Pools {
    i32s: Vec<String>,
    u8s: Vec<String>,
    i64s: Vec<String>,
}

fn pick(r: &mut Rng, v: &[String]) -> String {
    v[r.below(v.len())].clone()
}

fn ovf(r: &mut Rng) -> &'static str {
    ["", ".wrap", ".sat", ".wrap"][r.below(4)]
}

pub fn gen_cfg(seed: u64, n: usize, wide: bool) -> String {
    gen_cfg_x(seed, n, wide, false)
}

/// `noparam`: no block parameters, so every switch is eligible for the jump-table plan.
pub fn gen_cfg_x(seed: u64, n: usize, wide: bool, noparam: bool) -> String {
    let mut r = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut s = String::from("oxiir 1\nmodule \"cfg\"\nexport func @f(%a: i32, %b: i32) -> i32 {\n");
    // entry values
    let mut ent = Pools {
        i32s: vec!["%a".into(), "%b".into()],
        u8s: vec![],
        i64s: vec![],
    };
    s.push_str("L0:\n");
    for (i, c) in [0i64, 1, -1, 7, 255, i32::MAX as i64, i32::MIN as i64, 3]
        .iter()
        .enumerate()
    {
        let _ = writeln!(s, "  %k{i}: i32 = const {c}");
        ent.i32s.push(format!("%k{i}"));
    }
    let _ = writeln!(s, "  %e8: u8 = conv.wrap %b");
    ent.u8s.push("%e8".into());
    let _ = writeln!(s, "  %e64: i64 = conv.wrap %a");
    ent.i64s.push("%e64".into());
    let mut ctr = 0;
    for bi in 0..n {
        let mut p = Pools {
            i32s: ent.i32s.clone(),
            u8s: ent.u8s.clone(),
            i64s: ent.i64s.clone(),
        };
        if bi > 0 && noparam {
            let _ = writeln!(s, "L{bi}:");
        } else if bi > 0 {
            let _ = writeln!(s, "L{bi}(%p{bi}: i32, %q{bi}: u8, %r{bi}: i64):");
            p.i32s.push(format!("%p{bi}"));
            p.u8s.push(format!("%q{bi}"));
            p.i64s.push(format!("%r{bi}"));
        }
        // the reference interpreter only checks its fuel inside instructions: an
        // instruction-free loop would spin forever, so every block has one
        let _ = writeln!(s, "  %nop{bi}: i32 = const 0");
        let ninst = r.below(5);
        let mut bools: Vec<String> = vec![];
        for _ in 0..ninst {
            ctr += 1;
            let d = format!("%v{bi}_{ctr}");
            match r.below(12) {
                0 | 1 => {
                    let op = ["add", "sub", "mul", "div", "rem"][r.below(5)];
                    let o = if op == "div" || op == "rem" { [".wrap", ""][r.below(2)] } else { ovf(&mut r) };
                    let (x, y) = (pick(&mut r, &p.i32s), pick(&mut r, &p.i32s));
                    let _ = writeln!(s, "  {d}: i32 = {op}{o} {x}, {y}");
                    p.i32s.push(d);
                }
                2 => {
                    let op = ["and", "or", "xor"][r.below(3)];
                    let (x, y) = (pick(&mut r, &p.i32s), pick(&mut r, &p.i32s));
                    let _ = writeln!(s, "  {d}: i32 = {op} {x}, {y}");
                    p.i32s.push(d);
                }
                3 => {
                    let op = ["shl", "shr"][r.below(2)];
                    let (x, y) = (pick(&mut r, &p.i32s), pick(&mut r, &p.i32s));
                    let o = [".wrap", ""][r.below(2)];
                    let _ = writeln!(s, "  {d}: i32 = {op}{o} {x}, {y}");
                    p.i32s.push(d);
                }
                4 => {
                    let op = ["add", "sub", "mul"][r.below(3)];
                    let (x, y) = (pick(&mut r, &p.u8s), pick(&mut r, &p.u8s));
                    let o = ovf(&mut r);
                    let _ = writeln!(s, "  {d}: u8 = {op}{o} {x}, {y}");
                    p.u8s.push(d);
                }
                5 => {
                    let x = pick(&mut r, &p.i32s);
                    let _ = writeln!(s, "  {d}: u8 = conv.wrap {x}");
                    p.u8s.push(d);
                }
                6 => {
                    let x = pick(&mut r, &p.u8s);
                    let _ = writeln!(s, "  {d}: i32 = conv {x}");
                    p.i32s.push(d);
                }
                7 => {
                    let op = ["add", "sub", "mul", "shl"][r.below(4)];
                    let (x, y) = (pick(&mut r, &p.i64s), pick(&mut r, &p.i64s));
                    let o = ovf(&mut r);
                    let o = if op == "shl" { [".wrap", ""][r.below(2)] } else { o };
                    let _ = writeln!(s, "  {d}: i64 = {op}{o} {x}, {y}");
                    p.i64s.push(d);
                }
                8 => {
                    let x = pick(&mut r, &p.i64s);
                    let _ = writeln!(s, "  {d}: i32 = conv.wrap {x}");
                    p.i32s.push(d);
                }
                9 => {
                    let x = pick(&mut r, &p.i32s);
                    let _ = writeln!(s, "  {d}: i64 = conv {x}");
                    p.i64s.push(d);
                }
                10 => {
                    let op = ["lt", "le", "eq", "ne", "gt", "ge"][r.below(6)];
                    let (x, y) = (pick(&mut r, &p.i32s), pick(&mut r, &p.i32s));
                    let _ = writeln!(s, "  {d}: bool = {op} {x}, {y}");
                    bools.push(d);
                }
                _ => {
                    if bools.is_empty() {
                        let (x, y) = (pick(&mut r, &p.u8s), pick(&mut r, &p.u8s));
                        let _ = writeln!(s, "  {d}: bool = lt {x}, {y}");
                        bools.push(d);
                    } else {
                        let c = pick(&mut r, &bools);
                        let (x, y) = (pick(&mut r, &p.i32s), pick(&mut r, &p.i32s));
                        let _ = writeln!(s, "  {d}: i32 = select {c}, {x}, {y}");
                        p.i32s.push(d);
                    }
                }
            }
        }
        // terminator
        let tgt = |r: &mut Rng, p: &Pools| -> String {
            // forward-biased, back edges ~25%, never to the entry (no params there)
            let t = if r.chance(if noparam { 12 } else { 25 }) && bi >= 1 {
                1 + r.below(bi)
            } else if bi + 1 < n {
                bi + 1 + r.below(n - bi - 1)
            } else {
                1 + r.below(n - 1)
            };
            if noparam {
                return format!("L{t}()");
            }
            let (a, b, c) = (pick(r, &p.i32s), pick(r, &p.u8s), pick(r, &p.i64s));
            format!("L{t}({a}, {b}, {c})")
        };
        let last = bi + 1 == n;
        match if last { 0 } else { r.below(10) } {
            0 | 1 => {
                // ret
                let x = pick(&mut r, &p.i32s);
                let _ = writeln!(s, "  ret {x}");
            }
            2 => {
                let k = ["unreachable", "assert", "panic"][r.below(3)];
                let _ = writeln!(s, "  trap {k}");
            }
            3 | 4 => {
                let t = tgt(&mut r, &p);
                let _ = writeln!(s, "  br {t}");
            }
            5..=7 => {
                let c = if bools.is_empty() {
                    ctr += 1;
                    let (x, y) = (pick(&mut r, &p.i32s), pick(&mut r, &p.i32s));
                    let nm = format!("%c{bi}_{ctr}");
                    let _ = writeln!(s, "  {nm}: bool = lt {x}, {y}");
                    nm
                } else {
                    pick(&mut r, &bools)
                };
                let (t1, t2) = (tgt(&mut r, &p), tgt(&mut r, &p));
                let _ = writeln!(s, "  br_if {c}, {t1}, {t2}");
            }
            _ => {
                let v = pick(&mut r, &p.i32s);
                ctr += 1;
                let nm = format!("%sw{bi}_{ctr}");
                let m = [3, 4, 7, 16][r.below(4)];
                // scrutinee is masked so cases are hit
                let _ = writeln!(s, "  %m{bi}_{ctr}: i32 = const {m}");
                let _ = writeln!(s, "  {nm}: i32 = rem.wrap {v}, %m{bi}_{ctr}");
                let ncase = if noparam { 3 + r.below(8) } else { 1 + r.below(if wide { 6 } else { 4 }) };
                let ncase = ncase.min(m as usize + 2);
                let mut keys: Vec<i64> = Vec::new();
                while keys.len() < ncase {
                    let k = r.below(m as usize + 2) as i64 - 1;
                    if !keys.contains(&k) {
                        keys.push(k);
                    }
                }
                let mut cs = String::new();
                for (i, k) in keys.iter().enumerate() {
                    if i > 0 {
                        cs.push_str(", ");
                    }
                    let _ = write!(cs, "{k}: {}", tgt(&mut r, &p));
                }
                let d = tgt(&mut r, &p);
                let _ = writeln!(s, "  switch {nm}, [{cs}], default {d}");
            }
        }
    }
    s.push('}');
    s
}

fn to_value(v: V, t: &Ty) -> Value {
    match v {
        V::I(x) => {
            if matches!(t, Ty::I64 | Ty::U64) {
                Value::I64(x as i64)
            } else {
                Value::I32(x as i64 as i32)
            }
        }
        V::F32(x) => Value::F32(x),
        V::F64(x) => Value::F64(x),
    }
}

fn from_value(v: &Value, t: &Ty) -> V {
    match v {
        Value::I32(x) => V::I(wrap(t, i128::from(*x))),
        Value::I64(x) => V::I(wrap(t, i128::from(*x))),
        Value::F32(x) => V::F32(*x),
        Value::F64(x) => V::F64(*x),
        _ => panic!(),
    }
}

#[derive(Default)]
struct St {
    modules: usize,
    irreducible: usize,
    cases: usize,
    traps: usize,
    skipped: usize,
}

fn check(src: &str, m: &Module, rng: &mut Rng, st: &mut St, tag: &str) {
    let wasm = match compile(m, &opts()) {
        Ok(w) => w,
        Err(e) => {
            if e.0.len() == 1 && e.0[0].code == E_IRREDUCIBLE {
                st.irreducible += 1;
                return;
            }
            panic!("{tag}: unexpected diagnostic {e}\n{src}");
        }
    };
    // determinism (HashMap state differs per instance) and Import mode validity
    let again = compile(m, &opts()).expect("second compile");
    assert_eq!(again, wasm, "{tag}: compiler output is not deterministic\n{src}");
    let mut io = opts();
    io.runtime = oxiscript_wasm::RuntimeMode::Import;
    let imp = compile(m, &io).unwrap_or_else(|e| panic!("{tag}: import mode: {e}\n{src}"));
    oxiscript_wasm::validate(&imp).unwrap_or_else(|e| panic!("{tag}: import mode invalid wasm: {e}\n{src}"));
    let tr = std::env::var("OXI_TRACE").is_ok();
    if tr {
        eprintln!("  compiled");
    }
    if tr { eprintln!("  inst"); }
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    st.modules += 1;
    let edge = [0i128, 1, -1, 2, 3, 7, 255, 256, i32::MAX as i128, i32::MIN as i128, 100, -100];
    for _ in 0..10 {
        let args = [V::I(edge[rng.below(edge.len())]), V::I(if rng.chance(50) { edge[rng.below(edge.len())] } else { (rng.next() as i32) as i128 })];
        if tr { eprintln!("  interp"); }
        let mut it = Interp::new(m);
        it.max_steps = 20_000;
        let want = it.call("f", &args);
        if matches!(want, Err(Out::Fuel)) {
            st.skipped += 1;
            continue;
        }
        st.cases += 1;
        let wargs: Vec<Value> = args.iter().map(|a| to_value(*a, &Ty::I32)).collect();
        if rt.is_poisoned() {
            rt = instantiate(&h, &wasm);
        }
        if tr {
            eprintln!("  run {args:?} want {want:?}");
        }
        let got = rt.call_export("f", &wargs);
        match (want, got) {
            (Ok(w), Ok(g)) => {
                assert!(same(w[0], from_value(&g[0], &Ty::I32)), "{tag} {args:?}: interp {w:?} != wasm {g:?}\n{src}");
            }
            (Err(Out::Trap(k)), Err(Error::Trap(t))) => {
                st.traps += 1;
                assert_eq!(t.guest_kind, Some(k), "{tag} {args:?}: trap kind (cause {:?})\n{src}", t.cause);
            }
            (w, g) => panic!("{tag} {args:?}: interp {w:?} vs wasm {g:?}\n{src}"),
        }
    }
}

fn run_range(lo: u64, hi: u64, n: usize, wide: bool, noparam: bool) -> St {
    let mut st = St::default();
    for seed in lo..hi {
        if std::env::var("OXI_TRACE").is_ok() {
            eprintln!("seed {seed}");
        }
        let nb = n.min(3 + (seed as usize % (n - 2)));
        let src = gen_cfg_x(seed, nb, wide, noparam);
        let m = parse_module(&src).unwrap_or_else(|e| panic!("seed {seed}: parse {e}\n{src}"));
        let errs = verify(&m);
        assert!(errs.is_empty(), "seed {seed}: generated IR does not verify: {errs:?}\n{src}");
        let mut rng = Rng(seed * 31 + 5);
        check(&src, &m, &mut rng, &mut st, &format!("seed {seed}"));
    }
    st
}

#[test]
fn random_cfgs_agree() {
    let n = fuzz_scale(20000, 300) as u64;
    let st = run_range(0, n, 10, false, false);
    eprintln!("cfg: {} compiled, {} irreducible, {} cases ({} trapped, {} skipped)", st.modules, st.irreducible, st.cases, st.traps, st.skipped);
    assert!(st.modules > (n as usize) / 10);
}

#[test]
fn random_big_cfgs_agree() {
    let n = fuzz_scale(8000, 100) as u64;
    let st = run_range(1_000_000, 1_000_000 + n, 24, true, false);
    eprintln!("big cfg: {} compiled, {} irreducible, {} cases ({} trapped, {} skipped)", st.modules, st.irreducible, st.cases, st.traps, st.skipped);
}

/// `OXI_SEED=n cargo test ... dump_cfg -- --ignored --nocapture`
#[test]
#[ignore = "debug helper"]
fn dump_cfg() {
    let seed: u64 = std::env::var("OXI_SEED").unwrap().parse().unwrap();
    let n = 10usize;
    let nb = n.min(3 + (seed as usize % (n - 2)));
    eprintln!("{}", gen_cfg(seed, nb, false));
}

#[test]
fn random_paramless_cfgs_with_jump_tables_agree() {
    let n = fuzz_scale(20000, 300) as u64;
    let st = run_range(5_000_000, 5_000_000 + n, 14, true, true);
    eprintln!("noparam cfg: {} compiled, {} irreducible, {} cases ({} trapped, {} skipped)", st.modules, st.irreducible, st.cases, st.traps, st.skipped);
    assert!(st.cases > 100);
}
