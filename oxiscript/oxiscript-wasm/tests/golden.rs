//! Golden text dumps of the generated code (runtime imported, so only the
//! compiler's own functions show) to review codegen shape. Bless with
//! `OXI_BLESS=1 cargo test -p oxiscript-wasm --test golden`.
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_ir::parse_module;
use oxiscript_wasm::{compile, CompileOptions, RuntimeMode};

const CASES: &[(&str, &str)] = &[
    ("loop", include_str!("../../oxiscript-ir/tests/programs/loop.oxir")),
    ("arith", include_str!("../../oxiscript-ir/tests/programs/arith.oxir")),
    ("structs", include_str!("../../oxiscript-ir/tests/programs/structs.oxir")),
    ("closure", include_str!("../../oxiscript-ir/tests/programs/closure.oxir")),
];

#[test]
fn golden_dumps() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let bless = std::env::var("OXI_BLESS").is_ok();
    for (name, src) in CASES {
        let m = parse_module(src).unwrap();
        let wasm = compile(
            &m,
            &CompileOptions {
                runtime: RuntimeMode::Import,
                ..opts()
            },
        )
        .unwrap();
        let text = wasmprinter::print_bytes(&wasm).unwrap();
        let path = dir.join(format!("{name}.wat"));
        if bless {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(&path, &text).unwrap();
        } else {
            let want = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("missing golden {path:?}; run with OXI_BLESS=1"));
            assert_eq!(text, want, "golden {name} changed (OXI_BLESS=1 to update)");
        }
    }
}
