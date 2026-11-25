//! BAD ACTOR 8.G F2: verification must scale (near-)linearly in the number of
//! fields of one struct (was O(n^2): field lists cloned and scanned per op).
use oxiscript_ir::{parse_module, verify};
use std::fmt::Write;

fn wide(n: usize) -> String {
    let mut s = String::from("oxiir 1\nmodule \"wide\"\ntype W = struct { ");
    for i in 0..n {
        if i > 0 {
            s.push_str(", ");
        }
        let _ = write!(s, "f{i}: str?");
    }
    s.push_str(" }\nexport func @f() -> i32 {\ne:\n  %o: ref W = alloc W\n");
    for i in 0..n {
        let _ = writeln!(s, "  %n{i}: str? = null");
        let _ = writeln!(s, "  setfield.init W.f{i} %o, %n{i}");
    }
    s.push_str("  release %o\n  %r: i32 = const 1\n  ret %r\n}\n");
    s
}

#[test]
fn seventy_thousand_fields_verify_fast() {
    let m = parse_module(&wide(70_000)).expect("parse");
    let t = std::time::Instant::now();
    let errs = verify(&m);
    let el = t.elapsed();
    assert!(errs.is_empty(), "{:?}", errs.first());
    eprintln!("verify 70000 fields: {el:?}");
    if !cfg!(debug_assertions) {
        assert!(el < std::time::Duration::from_secs(1), "verify took {el:?}");
    } else {
        assert!(el < std::time::Duration::from_secs(20), "verify took {el:?}");
    }
}
