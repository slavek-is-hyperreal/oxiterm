#![allow(clippy::format_push_string)]
//! Fuzz-style no-panic / bounded-time tests. Full counts in release or with `OXI_FULL_FUZZ=1`,
//! 1/20 in debug builds. Every input is checked on a thread with a 1 MB stack (the wasm32 default).
mod common;
use common::*;
use std::time::{Duration, Instant};

fn scale(full: usize) -> usize {
    if !cfg!(debug_assertions) || std::env::var_os("OXI_FULL_FUZZ").is_some() {
        full
    } else {
        (full / 20).max(50)
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Check every input on a 1 MB stack; panics or overflow fail the test; each input is time-bounded.
fn check_all(inputs: Vec<String>) -> (usize, usize) {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let h = hosts();
            let (mut ok, mut bad) = (0, 0);
            let mut worst = Duration::ZERO;
            for src in &inputs {
                let t = Instant::now();
                let c = oxiscript_check::check_source(src, &h);
                worst = worst.max(t.elapsed());
                // every diagnostic must carry a span inside the source
                for d in &c.diagnostics {
                    assert!((d.span.hi as usize) <= src.len() + 1, "span out of range in: {src}");
                }
                assert!(c.diagnostics.iter().all(|d| d.code != "OXS-E0990"), "poisoned table in: {src}");
                if c.ok() {
                    ok += 1;
                } else {
                    bad += 1;
                }
            }
            assert!(worst < Duration::from_secs(5), "slow input: {worst:?}");
            (ok, bad)
        })
        .unwrap()
        .join()
        .expect("checker panicked or overflowed the stack")
}

const SOUP: &[&str] = &[
    "function", "f", "g", "(", ")", "{", "}", "[", "]", "<", ">", ":", ";", ",", ".", "?.", "?", "??", "=", "===", "!==", "=>", "+", "-", "*",
    "/", "%", "&&", "||", "!", "i32", "u8", "f64", "bool", "string", "null", "void", "Array", "Result", "Ok", "Err", "const", "let", "return",
    "if", "else", "while", "for", "of", "switch", "case", "default", "break", "continue", "struct", "class", "enum", "interface", "type",
    "law", "import", "export", "from", "as", "new", "this", "true", "false", "0", "1", "255", "1.5", "\"s\"", "x", "y", "a", "kind", "|", "&",
    "@expect", "@inline", "|>", "..", "...", "T", "S", "E", "wrapping", "saturating", "downcast", "length", "push", "\n",
];

#[test]
fn token_soup_never_panics() {
    let mut r = Rng(0x9E37_79B9_7F4A_7C15);
    let inputs = (0..scale(20_000))
        .map(|_| {
            let n = 1 + r.below(60);
            (0..n).map(|_| SOUP[r.below(SOUP.len())]).collect::<Vec<_>>().join(" ")
        })
        .collect();
    check_all(inputs);
}

fn corpus() -> Vec<String> {
    let dir = format!("{}/../oxiscript-syntax/tests/corpus/valid", env!("CARGO_MANIFEST_DIR"));
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| std::fs::read_to_string(e.unwrap().path()).ok())
        .collect();
    v.sort();
    v
}

/// Token-level mutations of valid programs reach deep into the checker (types mostly resolve).
#[test]
fn mutated_corpus_never_panics() {
    let base = corpus();
    assert!(base.len() >= 40);
    let mut r = Rng(0xDEAD_BEEF_1234_5678);
    let mut inputs = Vec::new();
    for _ in 0..scale(12_000) {
        let src = &base[r.below(base.len())];
        let mut toks: Vec<&str> = src.split_inclusive(|c: char| c.is_whitespace() || "(){}[];,".contains(c)).collect();
        for _ in 0..=r.below(4) {
            if toks.is_empty() {
                break;
            }
            let i = r.below(toks.len());
            match r.below(6) {
                0 => {
                    toks.remove(i);
                }
                1 => {
                    let t = toks[i];
                    toks.insert(i, t);
                }
                2 => {
                    let j = r.below(toks.len());
                    toks.swap(i, j);
                }
                3 => toks[i] = SOUP[r.below(SOUP.len())],
                4 => toks.truncate(i.max(1)),
                _ => {
                    let j = r.below(toks.len());
                    toks[i] = toks[j];
                }
            }
        }
        inputs.push(toks.concat());
    }
    let (ok, bad) = check_all(inputs);
    assert!(ok + bad > 0);
}

/// Structured generator: mostly well-formed small programs with type-ish variety.
fn gen_ty(r: &mut Rng, d: u32) -> String {
    let base = ["i32", "u8", "i64", "f64", "bool", "string", "void", "S", "T", "E"];
    if d == 0 {
        return base[r.below(base.len())].to_string();
    }
    match r.below(7) {
        0 => format!("Array<{}>", gen_ty(r, d - 1)),
        1 => format!("{} | null", gen_ty(r, d - 1)),
        2 => format!("Result<{}, {}>", gen_ty(r, d - 1), gen_ty(r, d - 1)),
        3 => format!("({}) => {}", gen_ty(r, d - 1), gen_ty(r, d - 1)),
        _ => base[r.below(base.len())].to_string(),
    }
}

fn gen_expr(r: &mut Rng, d: u32) -> String {
    if d == 0 {
        return ["x", "y", "0", "1", "255", "1.5", "true", "null", "\"s\"", "s", "s.n", "arr"][r.below(12)].to_string();
    }
    let a = gen_expr(r, d - 1);
    match r.below(16) {
        0 => format!("({a} + {})", gen_expr(r, d - 1)),
        1 => format!("({a} === {})", gen_expr(r, d - 1)),
        2 => format!("({a} && {})", gen_expr(r, d - 1)),
        3 => format!("({a} ?? {})", gen_expr(r, d - 1)),
        4 => format!("{a}.length"),
        5 => format!("f({a})"),
        6 => format!("{a}?.n"),
        7 => format!("[{a}, {}]", gen_expr(r, d - 1)),
        8 => format!("{{ n: {a} }}"),
        9 => format!("({a} as {})", gen_ty(r, 1)),
        10 => format!("{a}[0]"),
        11 => format!("(v: {}) => {a}", gen_ty(r, 1)),
        12 => format!("({a} ? {} : {})", gen_expr(r, d - 1), gen_expr(r, d - 1)),
        13 => format!("{a}?"),
        14 => format!("Ok({a})"),
        _ => format!("switch ({a}) {{ case 1: {} default: {} }}", gen_expr(r, d - 1), gen_expr(r, d - 1)),
    }
}

fn gen_stmt(r: &mut Rng, d: u32) -> String {
    match r.below(10) {
        0 => format!("let v{} = {};", r.below(5), gen_expr(r, 3)),
        1 => format!("const v{}: {} = {};", r.below(5), gen_ty(r, 2), gen_expr(r, 3)),
        2 => format!("x = {};", gen_expr(r, 3)),
        3 => format!("return {};", gen_expr(r, 3)),
        4 if d > 0 => format!("if ({}) {{ {} }} else {{ {} }}", gen_expr(r, 2), gen_stmt(r, d - 1), gen_stmt(r, d - 1)),
        5 if d > 0 => format!("while ({}) {{ {} }}", gen_expr(r, 2), gen_stmt(r, d - 1)),
        6 if d > 0 => format!("for (const e of {}) {{ {} }}", gen_expr(r, 2), gen_stmt(r, d - 1)),
        7 => format!("s.n += {};", gen_expr(r, 2)),
        8 => format!("if (s.k === \"a\") {{ {} }}", gen_stmt(r, 0)),
        _ => format!("{};", gen_expr(r, 3)),
    }
}

#[test]
fn generated_programs_never_panic() {
    let mut r = Rng(0x0123_4567_89AB_CDEF);
    let mut inputs = Vec::new();
    for _ in 0..scale(15_000) {
        let mut s = String::from(
            "struct S { n: i32; k: string; }\ntype U = { kind: \"a\"; a: i32 } | { kind: \"b\" };\nfunction f<T>(p: T): T { return p; }\nfunction h(x: i32, y: i32 | null, s: S, arr: Array<i32>): i32 {\n",
        );
        for _ in 0..=r.below(6) {
            s.push_str("  ");
            s.push_str(&gen_stmt(&mut r, 2));
            s.push('\n');
        }
        s.push_str("  return 0;\n}\n");
        inputs.push(s);
    }
    let (ok, bad) = check_all(inputs);
    // the generator is deliberately sloppy, but not all programs may be rejected nor all accepted
    assert!(bad > 0 && ok > 0, "ok={ok} bad={bad}");
}

#[test]
fn raw_bytes_and_unicode_never_panic() {
    let mut r = Rng(0xABCD_EF01_2345_6789);
    let pool = ["é", "🙂", "\u{0}", "\"", "'", "`", "/*", "*/", "//", "\\", "\r\n", "\t", "ß", "\u{202e}", "0x", "1e999", "9999999999999999999999"];
    let inputs = (0..scale(5000))
        .map(|_| {
            let n = r.below(40);
            (0..n)
                .map(|_| if r.below(2) == 0 { pool[r.below(pool.len())] } else { SOUP[r.below(SOUP.len())] })
                .collect::<Vec<_>>()
                .join("")
        })
        .collect();
    check_all(inputs);
}
