//! Differential testing: random well-typed IR is interpreted by the
//! reference interpreter and, compiled, run in `oxiscript-runtime` (wasmi);
//! results and trap kinds must agree.
#![allow(clippy::pedantic)]
mod common;

use common::gen::*;
use common::interp::*;
use common::*;
use oxiscript_ir::{verify, Const, Module, Ty};
use oxiscript_runtime::{Error, Value};

fn to_v(c: &Const) -> V {
    match c {
        Const::Bool(b) => V::I(i128::from(*b)),
        Const::Int(v) => V::I(*v),
        Const::F32(b) => V::F32(f32::from_bits(*b)),
        Const::F64(b) => V::F64(f64::from_bits(*b)),
    }
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

struct Stats {
    cases: usize,
    traps: usize,
    skipped: usize,
}

fn check_module(m: &Module, rng: &mut Rng, st: &mut Stats, tag: &str) {
    let errs = verify(m);
    assert!(errs.is_empty(), "{tag}: generated IR does not verify: {errs:#?}");
    let wasm = build(m);
    let h = host();
    let mut rt = instantiate(&h, &wasm);
    for f in &m.funcs {
        let (ps, rtys) = sig_of(m, &f.name);
        for _ in 0..8 {
            let args: Vec<V> = ps
                .iter()
                .map(|p| {
                    if rng.chance(50) {
                        to_v(&rand_const(rng, p))
                    } else {
                        let cs = edge_consts(p);
                        to_v(&cs[rng.below(cs.len())])
                    }
                })
                .collect();
            let mut it = Interp::new(m);
            let want = it.call(&f.name, &args);
            if matches!(want, Err(Out::Fuel)) {
                st.skipped += 1;
                continue;
            }
            st.cases += 1;
            let wargs: Vec<Value> = args.iter().zip(&ps).map(|(a, t)| to_value(*a, t)).collect();
            if rt.is_poisoned() {
                rt = instantiate(&h, &wasm);
            }
            let got = rt.call_export(&f.name, &wargs);
            match (want, got) {
                (Ok(w), Ok(g)) => {
                    assert_eq!(w.len(), g.len(), "{tag} {} result arity", f.name);
                    for ((w, g), t) in w.iter().zip(&g).zip(&rtys) {
                        assert!(
                            same(*w, from_value(g, t)),
                            "{tag} {} {args:?}: interp {w:?} != wasm {g:?}",
                            f.name
                        );
                    }
                }
                (Err(Out::Trap(k)), Err(Error::Trap(t))) => {
                    st.traps += 1;
                    assert_eq!(
                        t.guest_kind,
                        Some(k),
                        "{tag} {} {args:?}: trap kind (wasm cause {:?})",
                        f.name,
                        t.cause
                    );
                }
                (w, g) => panic!("{tag} {} {args:?}: interp {w:?} vs wasm {g:?}", f.name),
            }
        }
    }
}

#[test]
fn random_programs_agree() {
    let n = fuzz_scale(3000, 40);
    let mut st = Stats {
        cases: 0,
        traps: 0,
        skipped: 0,
    };
    for seed in 0..n as u64 {
        let m = gen_module(seed + 1, 3);
        if std::env::var("OXI_TRACE").is_ok() {
            eprintln!("seed {seed}");
        }
        let mut rng = Rng(seed * 7919 + 13);
        check_module(&m, &mut rng, &mut st, &format!("seed {seed}"));
    }
    eprintln!(
        "differential: {n} modules, {} cases ({} trapped, {} skipped)",
        st.cases, st.traps, st.skipped
    );
    assert!(st.cases > n * 3);
}

/// Debug helper: `OXI_SEED=n OXI_DUMP=dir cargo test -p oxiscript-wasm --test diff dump_seed -- --ignored --nocapture`
#[test]
#[ignore = "debug helper"]
fn dump_seed() {
    let seed: u64 = std::env::var("OXI_SEED").unwrap().parse().unwrap();
    let m = gen_module(seed + 1, 3);
    eprintln!("{}", oxiscript_ir::print_module(&m));
    let mut o = opts();
    o.validate_output = false;
    let wasm = oxiscript_wasm::compile(&m, &o).unwrap();
    eprintln!("{:?}", oxiscript_wasm::validate(&wasm));
    if let Err(e) = oxiscript_wasm::validate(&wasm) {
        // which function contains the failing offset?
        let off = u64::from_str_radix(e.rsplit("0x").next().unwrap().trim_end_matches(')'), 16).unwrap();
        let mut idx = 0;
        for p in wasmparser::Parser::new(0).parse_all(&wasm) {
            if let Ok(wasmparser::Payload::CodeSectionEntry(b)) = p {
                let r = b.range();
                if r.start <= off && off < r.end {
                    eprintln!("failing code entry #{idx} (range {r:?})");
                }
                idx += 1;
            }
        }
    }
    if let Ok(t) = wasmprinter::print_bytes(&wasm) {
        let dir = std::env::var("OXI_DUMP").unwrap_or_else(|_| ".".into());
        std::fs::write(format!("{dir}/dump.wat"), t).unwrap();
    }
}
