//! Deterministic mutation tests: the verifier must never panic on corrupted
//! modules, and every module that *does* verify must survive print -> parse.
#![allow(clippy::doc_markdown)]

use oxiscript_ir::*;
use std::panic::{catch_unwind, AssertUnwindSafe};

const PROGRAMS: &[(&str, &str)] = &[
    ("arith", include_str!("programs/arith.oxir")),
    ("loop", include_str!("programs/loop.oxir")),
    ("structs", include_str!("programs/structs.oxir")),
    ("closure", include_str!("programs/closure.oxir")),
    ("host", include_str!("programs/host.oxir")),
    ("rc", include_str!("programs/rc.oxir")),
    ("raw", include_str!("programs/raw.oxir")),
];

fn opts(name: &str) -> VerifyOptions {
    VerifyOptions {
        allow_raw_memory: name == "raw",
    }
}

/// Run `verify_with` on `m`, failing the test if it panics; return the result.
fn verify_no_panic(name: &str, what: &str, m: &Module) -> bool {
    let r = catch_unwind(AssertUnwindSafe(|| verify_with(m, opts(name))));
    let Ok(errs) = r else {
        panic!("verify panicked on {name}: {what}");
    };
    errs.is_empty()
}

/// Every verified module prints to text that parses back to an equal module.
fn assert_round_trips_if_verified(name: &str, what: &str, m: &Module) {
    if verify_no_panic(name, what, m) {
        let text = print_module(m);
        let back = parse_module(&text).unwrap_or_else(|e| {
            panic!("{name}: {what}: verified module does not re-parse: {e}\n{text}")
        });
        assert_eq!(
            &back, m,
            "{name}: {what}: print/parse changed a verified module"
        );
    }
}

#[test]
fn corrupted_ids_never_panic_the_verifier() {
    for (name, text) in PROGRAMS {
        let base = parse_module(text).unwrap();
        for (fi, f) in base.funcs.iter().enumerate() {
            for (bi, b) in f.blocks.iter().enumerate() {
                for (ii, inst) in b.insts.iter().enumerate() {
                    let n_ops = inst.op.operands().len();
                    for k in 0..n_ops {
                        for bad in [ValueId(99_999), ValueId(0), ValueId(u32::MAX)] {
                            let mut m = base.clone();
                            *m.funcs[fi].blocks[bi].insts[ii].op.operands_mut()[k] = bad;
                            verify_no_panic(
                                name,
                                &format!("@{} b{bi} i{ii} operand {k} = {bad:?}", f.name),
                                &m,
                            );
                        }
                    }
                    for k in 0..inst.results.len() {
                        let mut m = base.clone();
                        m.funcs[fi].blocks[bi].insts[ii].results[k] = ValueId(77_777);
                        verify_no_panic(name, "result id", &m);
                    }
                }
                let n_ops = b.term.operands().len();
                for k in 0..n_ops {
                    let mut m = base.clone();
                    *m.funcs[fi].blocks[bi].term.operands_mut()[k] = ValueId(99_999);
                    verify_no_panic(name, "terminator operand", &m);
                }
                // corrupt branch targets
                let mut m = base.clone();
                match &mut m.funcs[fi].blocks[bi].term {
                    Terminator::Br(t) => t.block = BlockId(9_999),
                    Terminator::BrIf { then, .. } => then.block = BlockId(9_999),
                    Terminator::Switch { default, .. } => default.block = BlockId(9_999),
                    _ => {}
                }
                verify_no_panic(name, "branch target", &m);
                // corrupt block params
                if !b.params.is_empty() {
                    let mut m = base.clone();
                    m.funcs[fi].blocks[bi].params[0] = ValueId(55_555);
                    verify_no_panic(name, "block param id", &m);
                }
            }
            let mut m = base.clone();
            if !m.funcs[fi].params.is_empty() {
                m.funcs[fi].params[0].value = ValueId(44_444);
                verify_no_panic(name, "param id", &m);
            }
            let mut m = base.clone();
            m.funcs[fi].blocks.clear();
            verify_no_panic(name, "no blocks", &m);
            let mut m = base.clone();
            m.funcs[fi].values.clear();
            verify_no_panic(name, "no values", &m);
        }
    }
}

#[test]
fn renamed_modules_that_verify_round_trip() {
    let names = [
        "zz", "a.b", "9x", "_", "a b", "", "ret", "x:y", "é", "A\"B", "br_if", "a%b",
    ];
    for (name, text) in PROGRAMS {
        let base = parse_module(text).unwrap();
        assert_round_trips_if_verified(name, "unmodified", &base);
        for new in names {
            for (fi, f) in base.funcs.iter().enumerate() {
                // value names
                for vi in 0..f.values.len().min(6) {
                    let mut m = base.clone();
                    m.funcs[fi].values[vi].name = new.into();
                    assert_round_trips_if_verified(name, &format!("value name {new:?}"), &m);
                }
                // labels
                let mut m = base.clone();
                m.funcs[fi].blocks[0].label = new.into();
                assert_round_trips_if_verified(name, &format!("label {new:?}"), &m);
                // function / symbol names
                let mut m = base.clone();
                m.funcs[fi].name = new.into();
                assert_round_trips_if_verified(name, &format!("func name {new:?}"), &m);
            }
            for ti in 0..base.types.len() {
                let mut m = base.clone();
                m.types[ti].name = new.into();
                assert_round_trips_if_verified(name, &format!("type name {new:?}"), &m);
            }
            for di in 0..base.data.len() {
                let mut m = base.clone();
                m.data[di].name = new.into();
                assert_round_trips_if_verified(name, &format!("data name {new:?}"), &m);
            }
            let mut m = base.clone();
            m.name = new.into();
            assert_round_trips_if_verified(name, "module name", &m);
            let mut m = base.clone();
            m.attrs.push(Attr {
                name: new.into(),
                args: vec![new.into(), "1".into(), "01".into(), "host:log".into()],
            });
            assert_round_trips_if_verified(name, "attribute", &m);
        }
    }
}

#[test]
fn block_order_is_free_for_verified_modules() {
    // reversing every function's non-entry blocks (fixing targets) keeps
    // dominance; if it verifies it must print and re-parse (forward value refs).
    for (name, text) in PROGRAMS {
        let base = parse_module(text).unwrap();
        for fi in 0..base.funcs.len() {
            let n = base.funcs[fi].blocks.len();
            if n < 3 {
                continue;
            }
            let mut m = base.clone();
            let f = &mut m.funcs[fi];
            // permutation: entry stays, rest reversed
            let perm: Vec<usize> = std::iter::once(0).chain((1..n).rev()).collect();
            let mut inv = vec![0usize; n];
            for (new, old) in perm.iter().enumerate() {
                inv[*old] = new;
            }
            let old_blocks = f.blocks.clone();
            f.blocks = perm.iter().map(|o| old_blocks[*o].clone()).collect();
            for b in &mut f.blocks {
                let fix = |t: &mut Target| t.block = BlockId(inv[t.block.0 as usize] as u32);
                match &mut b.term {
                    Terminator::Br(t) => fix(t),
                    Terminator::BrIf { then, els, .. } => {
                        fix(then);
                        fix(els);
                    }
                    Terminator::Switch { cases, default, .. } => {
                        for (_, t) in cases.iter_mut() {
                            fix(t);
                        }
                        fix(default);
                    }
                    _ => {}
                }
            }
            // value ids are no longer in textual order: renumber canonically
            build::renumber(f);
            assert_round_trips_if_verified(name, "reversed blocks", &m);
        }
    }
}

#[test]
fn deep_but_legal_types_round_trip() {
    let depth = 60;
    let ty = format!("{}i32{}?", "[".repeat(depth), "]".repeat(depth));
    let src = format!("oxiir 1\nmodule \"d\"\n\nglobal @g: {ty}\n");
    let m = parse_module(&src).unwrap();
    assert_eq!(verify(&m).len(), 0, "{:?}", verify(&m));
    assert_eq!(parse_module(&print_module(&m)).unwrap(), m);
    let too_deep = format!(
        "oxiir 1\nmodule \"d\"\n\nglobal @g: {}i32{}?\n",
        "[".repeat(70),
        "]".repeat(70)
    );
    assert!(parse_module(&too_deep).is_err());
}

#[test]
fn programmatically_built_deep_types_are_rejected_without_overflow() {
    // 100k-deep type built iteratively; verify must report an error, not recurse.
    let mut t = Ty::I32;
    for _ in 0..100_000 {
        t = Ty::Array(Box::new(t));
    }
    let m = Module {
        name: "deep".into(),
        globals: vec![Global {
            name: "g".into(),
            ty: t,
            init: None,
        }],
        ..Module::default()
    };
    let errs = verify(&m);
    assert!(
        errs.iter().any(|e| e.msg.contains("deeper than")),
        "{errs:?}"
    );
    // Ty's Drop is recursive by decision (parsed input is depth-bounded, code-built input is trusted):
    // leak the value rather than drop it here.
    std::mem::forget(m);
}

#[test]
fn renumber_restores_canonical_numbering_after_insertions() {
    // insert an instruction at the start of a function, then renumber: the module verifies again
    let mut m = parse_module(PROGRAMS[0].1).unwrap();
    let f = &mut m.funcs[0];
    let new_id = ValueId(f.values.len() as u32); // non-canonical on purpose
    f.values.push(ValueInfo {
        name: "extra".into(),
        ty: Ty::I32,
    });
    f.blocks[0].insts.insert(
        0,
        Inst {
            results: vec![new_id],
            op: Op::Const(Const::Int(7)),
            loc: None,
        },
    );
    assert!(
        !verify(&m).is_empty(),
        "non-canonical numbering must be rejected"
    );
    build::renumber(&mut m.funcs[0]);
    assert_eq!(verify(&m).len(), 0, "{:?}", verify(&m));
    assert_eq!(parse_module(&print_module(&m)).unwrap(), m);
}
