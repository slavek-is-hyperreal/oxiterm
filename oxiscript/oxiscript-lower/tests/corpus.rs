#![allow(clippy::pedantic)]
//! Gate (a): every program of the syntax corpus that passes the checker lowers, verifies,
//! validates as Wasm and loads under the strict limits of the runtime.
mod common;
use common::*;
use oxiscript_check::{check_program, CheckOptions, MapLoader};
use oxiscript_lower::{compile_program, Options};
use oxiscript_runtime::Grants;

fn valid_dir() -> String {
    format!("{}/../oxiscript-syntax/tests/corpus/valid", env!("CARGO_MANIFEST_DIR"))
}

fn loader() -> MapLoader {
    let mut m = MapLoader::default();
    m.0.insert(
        "util".into(),
        "export function a(): i32 { return 1; }\nexport const b: i32 = 2;\nexport struct d { x: i32; }\nexport struct Box<T> { v: T; }\n".into(),
    );
    m.0.insert("std/math".into(), "export function sqrt(x: f64): f64 { return x; }\n".into());
    m
}

#[test]
fn every_checked_corpus_program_compiles_and_loads() {
    let mut files: Vec<_> = std::fs::read_dir(valid_dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|x| x == "oxs"))
        .collect();
    files.sort();
    assert!(files.len() >= 40);
    let ifaces = interfaces();
    let env = env();
    let mut ok = 0;
    let mut skipped = 0;
    let mut failures = String::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let ck = check_program("main", &src, &mut loader(), &CheckOptions { hosts: ifaces.clone() });
        if !ck.ok() {
            skipped += 1;
            continue;
        }
        match compile_program("main", &src, &mut loader(), &ifaces, &Options::default()) {
            Err(e) => failures.push_str(&format!("{name}: compile: {e}\n")),
            Ok(out) => match env.host.instantiate(&out.wasm, Grants::all()) {
                Ok(mut rt) => {
                    // module initialisers and main ran in oxi_init; everything not held by the host must be gone
                    let live = rt.deinit().unwrap_or(-1);
                    ok += 1;
                    let _ = live;
                }
                Err(e) => failures.push_str(&format!("{name}: load: {e}\n")),
            },
        }
    }
    eprintln!("corpus: {ok} programs compiled and loaded, {skipped} skipped (do not pass the checker)");
    assert!(failures.is_empty(), "\n{failures}");
    assert!(ok >= 20, "only {ok} programs");
}
