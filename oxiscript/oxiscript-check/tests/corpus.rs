mod common;
use common::*;
use oxiscript_check::*;
use oxiscript_syntax::LineIndex;

fn valid_dir() -> String {
    format!("{}/../oxiscript-syntax/tests/corpus/valid", env!("CARGO_MANIFEST_DIR"))
}

fn loader() -> MapLoader {
    let mut m = MapLoader::default();
    m.0.insert("util".into(), "export function a(): i32 { return 1; }\nexport const b: i32 = 2;\nexport struct d { x: i32; }\nexport struct Box<T> { v: T; }\n".into());
    m.0.insert("std/math".into(), "export function sqrt(x: f64): f64 { return x; }\n".into());
    m
}

#[test]
fn every_valid_program_type_checks() {
    let mut files: Vec<_> = std::fs::read_dir(valid_dir()).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "oxs")).collect();
    files.sort();
    assert!(files.len() >= 40);
    let mut failures = String::new();
    for f in &files {
        let src = std::fs::read_to_string(f).unwrap();
        let name = f.file_name().unwrap().to_string_lossy().to_string();
        let mut l = loader();
        let c = check_program("main", &src, &mut l, &hosts());
        let li = LineIndex::new(&src);
        for d in c.errors() {
            failures.push_str(&format!("{name}:{}: {} {}\n", li.line(d.span.lo), d.code, d.message));
        }
    }
    assert!(failures.is_empty(), "\n{failures}");
}
