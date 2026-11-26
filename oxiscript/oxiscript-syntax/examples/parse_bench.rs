#![allow(clippy::doc_markdown)]
//! Parse/format throughput on a large generated file (run with `--release`):
//! `cargo run --release -p oxiscript-syntax --example parse_bench -- [MB]`
use oxiscript_syntax::{format, lex, parse_module, FileId};
use std::time::Instant;

fn main() {
    let mb: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(20);
    let dir = format!("{}/tests/corpus/valid", env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| std::fs::read_to_string(e.unwrap().path()).unwrap())
        .collect();
    files.sort();
    let mut src = String::new();
    let mut i = 0;
    while src.len() < mb << 20 {
        // drop BOM'd files and the CRLF one: keep the benchmark text uniform
        let f = &files[i % files.len()];
        if !f.starts_with('\u{feff}') && !f.contains("@@") {
            src.push_str(f);
            src.push('\n');
        }
        i += 1;
    }
    let bytes = src.len() as f64;
    let t = Instant::now();
    let lx = lex(&src, FileId(0));
    let tl = t.elapsed().as_secs_f64();
    let ntok = lx.tokens.len();
    drop(lx);
    let t = Instant::now();
    let (m, d) = parse_module(&src);
    let tp = t.elapsed().as_secs_f64();
    assert_eq!(d.len(), 0, "{:?}", d.first());
    let items = m.items.len();
    drop(m);
    let t = Instant::now();
    let out = format(&src).unwrap();
    let tf = t.elapsed().as_secs_f64();
    println!(
        "input {:.1} MB, {} tokens, {} items\nlex          {:.3}s  {:.1} MB/s\nlex+parse    {:.3}s  {:.1} MB/s\nparse+format {:.3}s  {:.1} MB/s (output {:.1} MB)",
        bytes / 1e6,
        ntok,
        items,
        tl,
        bytes / 1e6 / tl,
        tp,
        bytes / 1e6 / tp,
        tf,
        bytes / 1e6 / tf,
        out.len() as f64 / 1e6
    );
}
