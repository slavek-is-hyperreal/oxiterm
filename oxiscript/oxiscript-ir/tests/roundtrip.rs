#![allow(clippy::doc_markdown)]
//! Round-trip, verifier and builder tests for the OxiIR text form.

use oxiscript_ir::build::{target, FuncBuilder};
use oxiscript_ir::*;

const PROGRAMS: &[(&str, &str)] = &[
    ("arith", include_str!("programs/arith.oxir")),
    ("loop", include_str!("programs/loop.oxir")),
    ("structs", include_str!("programs/structs.oxir")),
    ("closure", include_str!("programs/closure.oxir")),
    ("host", include_str!("programs/host.oxir")),
    ("rc", include_str!("programs/rc.oxir")),
    ("raw", include_str!("programs/raw.oxir")),
];

fn strip_comments(t: &str) -> String {
    t.lines()
        .filter(|l| !l.trim_start().starts_with(';'))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

#[test]
fn canonical_text_is_a_fixed_point() {
    for (name, text) in PROGRAMS {
        let m = parse_module(text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(
            print_module(&m),
            strip_comments(text),
            "{name}: print(parse(t)) != t"
        );
    }
}

#[test]
fn print_parse_equal() {
    for (name, text) in PROGRAMS {
        let m = parse_module(text).unwrap();
        let again = parse_module(&print_module(&m)).unwrap();
        assert_eq!(m, again, "{name}");
    }
}

#[test]
fn programs_verify() {
    for (name, text) in PROGRAMS {
        let m = parse_module(text).unwrap();
        let opts = VerifyOptions {
            allow_raw_memory: *name == "raw",
        };
        let errs = verify_with(&m, opts);
        assert!(errs.is_empty(), "{name}: {errs:#?}");
    }
}

#[test]
fn parsed_structure_spot_checks() {
    let m = parse_module(PROGRAMS[4].1).unwrap();
    assert_eq!(m.imports.len(), 4);
    assert_eq!(m.imports[1].sig.rets, vec![Ty::I32, Ty::I32]);
    assert_eq!(m.data[1].text, "counter\u{1}é");
    let main = m.func("main").unwrap();
    assert_eq!(main.blocks.len(), 3);
    let loc = main.blocks[0].insts.iter().find_map(|i| i.loc).unwrap();
    assert_eq!(
        loc,
        Loc {
            file: 0,
            lo: 10,
            hi: 42
        }
    );
    assert_eq!(
        main.blocks[2].term_loc,
        Some(Loc {
            file: 0,
            lo: 50,
            hi: 60
        })
    );
    let s = parse_module(PROGRAMS[2].1).unwrap();
    assert!(s
        .type_decl("Node")
        .unwrap()
        .attrs
        .iter()
        .any(|a| a.name == "cyclic"));
}

#[test]
fn float_constants_round_trip_bit_exactly() {
    // edge cases
    let f64s: Vec<u64> = vec![
        0.0f64.to_bits(),
        (-0.0f64).to_bits(),
        f64::INFINITY.to_bits(),
        f64::NEG_INFINITY.to_bits(),
        f64::NAN.to_bits(),
        0x7ff8_0000_0000_0001,
        0xfff0_0000_0000_0001, // negative signalling NaN with payload
        1,                     // smallest subnormal
        0x000f_ffff_ffff_ffff,
        f64::MAX.to_bits(),
        f64::MIN_POSITIVE.to_bits(),
        0.1f64.to_bits(),
        1e21f64.to_bits(),
        1e-7f64.to_bits(),
    ];
    let f32s: Vec<u32> = vec![
        0,
        0x8000_0000,
        1,
        0x7f80_0000,
        0xff80_0000,
        0x7fc0_0001,
        0xffc0_0001,
        f32::MAX.to_bits(),
        0.1f32.to_bits(),
    ];
    // plus a deterministic pseudo-random sweep over all bit patterns
    let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let mut all64 = f64s;
    let mut all32 = f32s;
    for _ in 0..3000 {
        all64.push(next());
        all32.push(next() as u32);
    }
    let mut b = FuncBuilder::new("f").rets(vec![]);
    let e = b.block("entry");
    for (i, bits) in all64.iter().enumerate() {
        b.inst1(e, Op::Const(Const::F64(*bits)), &format!("a{i}"), Ty::F64);
    }
    for (i, bits) in all32.iter().enumerate() {
        b.inst1(e, Op::Const(Const::F32(*bits)), &format!("b{i}"), Ty::F32);
    }
    b.terminate(e, Terminator::Ret(vec![]));
    let m = Module {
        name: "floats".into(),
        funcs: vec![b.finish()],
        ..Module::default()
    };
    assert!(verify(&m).is_empty());
    let text = print_module(&m);
    let back = parse_module(&text).unwrap();
    assert_eq!(m, back);
}

#[test]
fn integer_extremes_round_trip() {
    let src = "oxiir 1\nmodule \"i\"\n\nfunc @f() -> void {\nentry:\n  %a: i64 = const -9223372036854775808\n  %b: u64 = const 18446744073709551615\n  %c: i8 = const -128\n  %d: u16 = const 65535\n  ret\n}\n";
    let m = parse_module(src).unwrap();
    assert!(verify(&m).is_empty());
    assert_eq!(print_module(&m), src);
}

#[test]
fn builder_matches_text() {
    // build @sum with the builder, textual order = creation order
    let mut b = FuncBuilder::new("sum").export().rets(vec![Ty::I64]);
    let n = b.param("n", Ty::I32, false);
    let entry = b.block("entry");
    let i0 = b.inst1(entry, Op::Const(Const::Int(0)), "i0", Ty::I32);
    let a0 = b.inst1(entry, Op::Const(Const::Int(0)), "a0", Ty::I64);
    let lp = b.block("loop");
    let i = b.block_param(lp, "i", Ty::I32);
    let acc = b.block_param(lp, "acc", Ty::I64);
    b.terminate(entry, Terminator::Br(target(lp, &[i0, a0])));
    let done = b.inst1(
        lp,
        Op::Cmp {
            op: CmpOp::Ge,
            a: i,
            b: n,
        },
        "done",
        Ty::Bool,
    );
    let body = b.block("body");
    let exit = b.block("exit");
    b.terminate(
        lp,
        Terminator::BrIf {
            cond: done,
            then: target(exit, &[acc]),
            els: target(body, &[]),
        },
    );
    let w = b.inst1(
        body,
        Op::Conv {
            ovf: Ovf::Trap,
            a: i,
        },
        "w",
        Ty::I64,
    );
    let acc2 = b.inst1(
        body,
        Op::Bin {
            op: BinOp::Add,
            ovf: Ovf::Trap,
            a: acc,
            b: w,
        },
        "acc2",
        Ty::I64,
    );
    let one = b.inst1(body, Op::Const(Const::Int(1)), "one", Ty::I32);
    let i2 = b.inst1(
        body,
        Op::Bin {
            op: BinOp::Add,
            ovf: Ovf::Trap,
            a: i,
            b: one,
        },
        "i2",
        Ty::I32,
    );
    b.terminate(body, Terminator::Br(target(lp, &[i2, acc2])));
    let res = b.block_param(exit, "res", Ty::I64);
    b.terminate(exit, Terminator::Ret(vec![res]));
    let built = Module {
        name: "loop".into(),
        funcs: vec![b.finish()],
        ..Module::default()
    };
    assert!(verify(&built).is_empty(), "{:?}", verify(&built));

    let text = parse_module(PROGRAMS[1].1).unwrap();
    assert_eq!(built.funcs[0], text.funcs[0]);
    assert_eq!(parse_module(&print_module(&built)).unwrap(), built);
}

// ---- parse errors ---------------------------------------------------------

fn parse_err(src: &str) -> ParseError {
    parse_module(src).unwrap_err()
}

#[test]
fn parse_errors_are_positioned() {
    let e = parse_err("oxiir 2\nmodule \"x\"\n");
    assert!(e.msg.contains("version"), "{e}");
    let e = parse_err(
        "oxiir 1\nmodule \"x\"\nfunc @f() -> void {\nentry:\n  %a: i32 = frob %b\n  ret\n}\n",
    );
    assert_eq!(e.line, 5);
    assert!(e.msg.contains("unknown instruction"), "{e}");
    let e = parse_err(
        "oxiir 1\nmodule \"x\"\nfunc @f() -> void {\nentry:\n  %a: i32 = add %x, %y\n  ret\n}\n",
    );
    assert!(e.msg.contains("not defined"), "{e}");
    let e = parse_err("oxiir 1\nmodule \"x\"\nfunc @f() -> void {\nentry:\n  br nowhere()\n}\n");
    assert!(e.msg.contains("undefined block"), "{e}");
    let e =
        parse_err("oxiir 1\nmodule \"x\"\nfunc @f(%a: i32, %a: i32) -> void {\nentry:\n  ret\n}\n");
    assert!(e.msg.contains("defined twice"), "{e}");
    let e = parse_err("oxiir 1\nmodule \"x\"\nglobal @g: i32?\n");
    assert!(e.msg.contains("nullable"), "{e}");
    let e = parse_err("oxiir 1\nmodule \"x\"\ndata @d = \"unterminated\n");
    assert!(e.msg.contains("unterminated"), "{e}");
    assert!(parse_module("").is_err());
}

// ---- verifier negatives ---------------------------------------------------

fn verify_msgs(src: &str) -> Vec<String> {
    let m = parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    verify(&m).into_iter().map(|e| e.to_string()).collect()
}

fn expect_error(src: &str, needle: &str) {
    let msgs = verify_msgs(src);
    assert!(
        msgs.iter().any(|m| m.contains(needle)),
        "expected `{needle}` in {msgs:#?}"
    );
}

#[test]
fn verifier_rejects_type_errors() {
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%a: i32, %b: i64) -> i32 {\nentry:\n  %c: i32 = add %a, %b\n  ret %c\n}\n",
        "equal operand",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%a: f64) -> f64 {\nentry:\n  %c: f64 = add.wrap %a, %a\n  ret %c\n}\n",
        "overflow mode",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%a: i32) -> i64 {\nentry:\n  ret %a\n}\n",
        "ret does not match",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%a: i32) -> void {\nentry:\n  retain %a\n  ret\n}\n",
        "managed reference",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f() -> i32 {\nentry:\n  %a: i32 = const 5000000000\n  ret %a\n}\n",
        "does not fit",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%a: i32) -> i32 {\nentry:\n  %c: i32 = call @g(%a)\n  ret %c\n}\n",
        "unknown callee",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @g(%a: i32, %b: i32) -> i32 {\nentry:\n  ret %a\n}\nfunc @f(%a: i32) -> i32 {\nentry:\n  %c: i32 = call @g(%a)\n  ret %c\n}\n",
        "expected 2 arguments",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%p: ref Nope) -> void {\nentry:\n  ret\n}\n",
        "unknown type",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\ntype P = struct { x: i32 }\nfunc @f(%p: ref P) -> i64 {\nentry:\n  %a: i64 = getfield P.x %p\n  ret %a\n}\n",
        "field `x` has type",
    );
    expect_error(
        "oxiir 1\nmodule \"x\"\ntype S = union { A, B }\nfunc @f() -> ref S {\nentry:\n  %a: ref S = alloc S\n  ret %a\n}\n",
        "name a variant",
    );
}

#[test]
fn verifier_rejects_ssa_errors() {
    // branch argument count / type mismatch
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f() -> void {\nentry:\n  br next()\nnext(%a: i32):\n  ret\n}\n",
        "expected 1 arguments",
    );
    // use not dominated: value defined in one arm used in the join
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%c: bool) -> i32 {\nentry:\n  br_if %c, a(), b()\na:\n  %x: i32 = const 1\n  br join()\nb:\n  br join()\njoin:\n  ret %x\n}\n",
        "not dominated",
    );
    // entry block parameters
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f() -> void {\nentry(%a: i32):\n  ret\n}\n",
        "entry block must not have parameters",
    );
    // duplicate symbols
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f() -> void {\nentry:\n  ret\n}\nfunc @f() -> void {\nentry:\n  ret\n}\n",
        "duplicate symbol",
    );
}

#[test]
fn verifier_gates_raw_memory_and_intrinsics() {
    expect_error(
        "oxiir 1\nmodule \"x\"\nfunc @f(%p: ptr) -> u32 {\nentry:\n  %a: u32 = load %p, 0\n  ret %a\n}\n",
        "trusted raw-memory",
    );
    expect_error(
        "oxiir 1\nmodule \"x\" #raw_memory\nfunc @f(%a: i64) -> void {\nentry:\n  intrinsic oxi_retain(%a)\n  ret\n}\n",
        "operand types",
    );
    expect_error(
        "oxiir 1\nmodule \"x\" #raw_memory\nfunc @f(%a: i32) -> void {\nentry:\n  intrinsic oxi_nope(%a)\n  ret\n}\n",
        "not a runtime function",
    );
}

#[test]
fn verifier_checks_closures_and_imports() {
    // closure target must take the env first
    expect_error(
        "oxiir 1\nmodule \"x\"\ntype E = closure { n: i32 }\nfunc @g(%x: i32) -> i32 {\nentry:\n  ret %x\n}\nfunc @f(%n: i32) -> fn(i32) -> i32 {\nentry:\n  %c: fn(i32) -> i32 = closure.new E @g(%n)\n  ret %c\n}\n",
        "must take `ref E` first",
    );
    // imports use the typed boundary: scalars, str, [u8], fn
    expect_error(
        "oxiir 1\nmodule \"x\"\nimport @h = \"host:a@1\" \"f\" ([i32]) -> void\n",
        "not allowed on a host import",
    );
}

#[test]
fn text_is_stable_under_reprint() {
    // idempotence on every program (print(parse(print(m))) == print(m))
    for (name, text) in PROGRAMS {
        let m = parse_module(text).unwrap();
        let t1 = print_module(&m);
        let t2 = print_module(&parse_module(&t1).unwrap());
        assert_eq!(t1, t2, "{name}");
    }
}
