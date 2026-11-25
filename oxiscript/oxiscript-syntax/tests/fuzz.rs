#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::match_same_arms,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::wildcard_imports,
    clippy::struct_excessive_bools,
    clippy::large_enum_variant,
    clippy::option_option,
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::self_only_used_in_recursion,
    clippy::unused_self,
    clippy::return_self_not_must_use,
    clippy::doc_markdown,
    clippy::needless_lifetimes,
    clippy::manual_let_else,
    clippy::unnecessary_wraps,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::cognitive_complexity,
    clippy::collapsible_else_if,
    clippy::while_let_loop,
    clippy::naive_bytecount
)]
//! Fuzz-style tests on stable Rust: seeded random token streams, mutations of
//! the valid corpus, pathological nesting and huge literals. Every input must
//! parse without panicking, within a time bound, and whatever parses cleanly
//! must format idempotently and re-parse cleanly.
mod common;
use common::*;
use oxiscript_syntax::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static CASES: AtomicUsize = AtomicUsize::new(0);

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Parse (and, when clean, format) one input; panics on contract violations.
fn run_case(src: &str, limit: Duration) {
    CASES.fetch_add(1, Ordering::Relaxed);
    let t = Instant::now();
    let (m, d) = parse_module(src);
    if !d.iter().any(Diagnostic::is_error) {
        let once = format(src).expect("clean input must format");
        let twice = format(&once).expect("formatted output must parse");
        assert_eq!(once, twice, "not idempotent for {src:?}");
        let (_, d2) = parse_module(&once);
        assert!(
            d2.is_empty(),
            "formatted output has errors for {src:?}: {d2:?}"
        );
        if src.len() < 200_000 {
            let o = FormatOptions {
                braces: false,
                ..FormatOptions::default()
            };
            let nb = format_with(src, &o).expect("clean input must format");
            let (m2, d3) = parse_module(&nb);
            assert!(
                d3.is_empty(),
                "structure-preserving output has errors for {src:?}"
            );
            assert_eq!(shape(&m), shape(&m2), "AST changed by formatting {src:?}");
        }
    } else {
        for x in &d {
            assert!(x.span.lo <= x.span.hi, "{x:?}");
            assert!(x.code.starts_with("OXS-"), "{x:?}");
            assert!(
                x.span.hi as usize <= src.len() + 1,
                "{x:?} in {} bytes",
                src.len()
            );
            for f in &x.fixes {
                // fixes must be well-formed edits (they may not apply cleanly to
                // every recovered input, but must never be out of range)
                for e in &f.edits {
                    assert!(e.span.hi as usize <= src.len(), "{x:?}");
                }
            }
        }
    }
    let el = t.elapsed();
    assert!(
        el < limit,
        "took {el:?} (> {limit:?}) on input of {} bytes starting {:?}",
        src.len(),
        src.chars().take(60).collect::<String>()
    );
}

const FAST: Duration = Duration::from_secs(2);
const SLOW: Duration = Duration::from_secs(20);

fn token_pieces(src: &str) -> Vec<(usize, usize)> {
    let lx = lex(src, FileId(0));
    lx.tokens
        .iter()
        .filter(|t| t.hi > t.lo)
        .map(|t| (t.lo as usize, t.hi as usize))
        .collect()
}

#[test]
fn mutate_valid_corpus() {
    let files = corpus("valid");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for (name, src) in &files {
        let toks = token_pieces(src);
        if toks.len() < 3 {
            continue;
        }
        run_case(src, FAST);
        for round in 0..150 {
            let mut s = src.clone();
            let n_mut = 1 + rng.below(3);
            for _ in 0..n_mut {
                let toks = token_pieces(&s);
                if toks.len() < 2 {
                    break;
                }
                let i = rng.below(toks.len());
                let (lo, hi) = toks[i];
                match rng.below(5) {
                    0 => s.replace_range(lo..hi, ""),
                    1 => {
                        let piece = s[lo..hi].to_string();
                        s.insert_str(hi, &format!(" {piece}"));
                    }
                    2 => {
                        let j = rng.below(toks.len());
                        let (lo2, hi2) = toks[j];
                        if hi <= lo2 {
                            let a = s[lo..hi].to_string();
                            let b = s[lo2..hi2].to_string();
                            s.replace_range(lo2..hi2, &a);
                            s.replace_range(lo..hi, &b);
                        }
                    }
                    3 => {
                        let junk = [
                            "(", ")", "{", "}", "[", "]", "<", ">", "?", ":", ";", ",", "=>", "`",
                            "\"", "/*", "${", "@", "..", ".5", "?.", ">>", ">=",
                        ];
                        let j = junk[rng.below(junk.len())];
                        s.insert_str(lo, j);
                    }
                    _ => {
                        // truncate at a char boundary
                        let mut cut = lo;
                        while !s.is_char_boundary(cut) {
                            cut -= 1;
                        }
                        s.truncate(cut);
                    }
                }
            }
            run_case(&s, FAST);
            let _ = (name, round);
        }
    }
}

#[test]
fn random_token_streams() {
    let vocab = [
        "let",
        "const",
        "function",
        "struct",
        "class",
        "interface",
        "enum",
        "type",
        "law",
        "if",
        "else",
        "while",
        "do",
        "for",
        "of",
        "switch",
        "case",
        "default",
        "return",
        "break",
        "continue",
        "import",
        "export",
        "from",
        "as",
        "new",
        "this",
        "null",
        "true",
        "false",
        "when",
        "private",
        "static",
        "readonly",
        "mutating",
        "constructor",
        "implements",
        "extends",
        "x",
        "y",
        "foo",
        "Bar",
        "T",
        "i32",
        "f64",
        "string",
        "_",
        "1",
        "2.5",
        ".5",
        "0xff",
        "1e3",
        "7u8",
        "1.0f32",
        "\"s\"",
        "'c'",
        "`t`",
        "`a${",
        "}b`",
        "(",
        ")",
        "{",
        "}",
        "[",
        "]",
        ";",
        ",",
        ".",
        ":",
        "?",
        "?.",
        "??",
        "@",
        "@@",
        "=>",
        "+",
        "-",
        "*",
        "/",
        "%",
        "+%",
        "-|",
        "*%",
        "&",
        "|",
        "^",
        "~",
        "<<",
        ">>",
        "!",
        "&&",
        "||",
        "===",
        "!==",
        "==",
        "<",
        "<=",
        ">",
        ">=",
        "=",
        "+=",
        ">>=",
        "++",
        "--",
        "// c\n",
        "/* c */",
        "/// d\n",
        "\n",
        " ",
        "var",
        "any",
        "async",
        "kernel",
        "inout",
        "await",
    ];
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for _ in 0..4000 {
        let n = 1 + rng.below(60);
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(vocab[rng.below(vocab.len())]);
            if rng.below(3) > 0 {
                s.push(' ');
            }
        }
        run_case(&s, FAST);
    }
}

#[test]
fn random_bytes_and_odd_unicode() {
    let mut rng = Rng(42);
    let pieces = [
        "\u{feff}",
        "\0",
        "\r",
        "\r\n",
        "\u{2028}",
        "\u{200b}",
        "é",
        "✓",
        "\u{1F600}",
        "\\",
        "\"",
        "'",
        "`",
        "$",
        "{",
        "/",
        "*",
        "\u{7f}",
        "\u{80}",
        "a",
        "1",
        ".",
        " ",
        "\n",
        "@",
        "#",
        "\\u{110000}",
        "\\x80",
        "\\u{d800}",
    ];
    for _ in 0..3000 {
        let n = 1 + rng.below(40);
        let mut s = String::new();
        for _ in 0..n {
            s.push_str(pieces[rng.below(pieces.len())]);
        }
        run_case(&s, FAST);
    }
    // specific awkward inputs
    for s in [
        "",
        "\u{feff}",
        "\u{feff}\u{feff}",
        "\0",
        "\r",
        "\r\r\n\n",
        "\"",
        "'",
        "`",
        "`${",
        "`${}`",
        "/*",
        "/**/",
        "//",
        "///",
        "////",
        "0x",
        "0b",
        "0o",
        "1e",
        "1e+",
        ".",
        "..",
        "...",
        "1..2",
        "1_",
        "1__0",
        "_1",
        "0xg",
        "1.e3",
        "\"\\u{",
        "\"\\u{1",
        "\"\\x",
        "\"\\",
        "`\\",
        "@@",
        "@",
        "?.",
        "?.5",
        "a?.5:1",
        "let x = `${`${`${1}`}`}`;",
        "x as",
        "x as u8 |",
        "f<",
        "f<a",
        "f<a>",
        "f<a>(",
        "a<b>(c)",
        "a < b > (c)",
        "(",
        "((",
        "(a",
        "(a)",
        "() =>",
        "() => {",
        "(a): i32 =>",
        "x ? (a): T => b : c",
        "switch (x) {",
        "case 1:",
        "default:",
        "law",
        "law f(",
        "type",
        "type x",
        "type x =",
        "enum",
        "enum E {",
        "struct",
        "struct S {",
        "struct S { x }",
        "class C { constructor(",
        "import",
        "import {",
        "import { a",
        "import { a } from",
        "export",
        "export export",
        "@a(",
        "@a(b:",
        "@@a",
        "function",
        "function f",
        "function f(",
        "function f()",
        "function f(): ",
        "for (",
        "for (;;",
        "for (const",
        "for (const x of",
        "do",
        "do ;",
        "while (",
        "return",
        "return ;",
        "break",
        "x++ ++",
        "- - - -",
        "a ?? b || c",
        "a & b === c",
        ">>= >>= >>=",
        "a>>=b",
    ] {
        run_case(s, FAST);
    }
}

#[test]
fn pathological_nesting() {
    let n = 10_000;
    let cases: Vec<(&str, String)> = vec![
        (
            "parens",
            format!("const a = {}1{};", "(".repeat(n), ")".repeat(n)),
        ),
        (
            "brackets",
            format!("const a = {}1{};", "[".repeat(n), "]".repeat(n)),
        ),
        (
            "objects",
            format!("const a = {}1{};", "{ a: ".repeat(n), " }".repeat(n)),
        ),
        (
            "blocks",
            format!("function f(): void {}{}", "{".repeat(n), "}".repeat(n)),
        ),
        (
            "calls",
            format!("const a = {}1{};", "f(".repeat(n), ")".repeat(n)),
        ),
        ("unary", format!("const a = {}1;", "- ".repeat(n))),
        ("not", format!("const a = {}1;", "!".repeat(n))),
        (
            "assign",
            format!("function f(): void {{ {}1; }}", "a = ".repeat(n)),
        ),
        (
            "ternary_else",
            format!("const a = {}1;", "x ? 1 : ".repeat(n)),
        ),
        (
            "ternary_then",
            format!("const a = {}1{};", "x ? ".repeat(n), " : 2".repeat(n)),
        ),
        (
            "if_chain",
            format!("function f(): void {{ {}1; }}", "if (a) ".repeat(n)),
        ),
        (
            "else_if",
            format!(
                "function f(): void {{ if (a) {{}} {}}}",
                "else if (b) {} ".repeat(n)
            ),
        ),
        ("arrows", format!("const a = {}1;", "x => ".repeat(n))),
        (
            "arrows_paren",
            format!("const a = {}1;", "(x): i32 => ".repeat(n)),
        ),
        (
            "templates",
            format!("const a = {}1{};", "`${".repeat(n), "}`".repeat(n)),
        ),
        (
            "types_generic",
            format!("let a: {}i32{};", "Array<".repeat(n), ">".repeat(n)),
        ),
        ("types_array", format!("let a: i32{};", "[]".repeat(n))),
        (
            "types_paren",
            format!("let a: {}i32{};", "(".repeat(n), ")".repeat(n)),
        ),
        (
            "types_obj",
            format!("type A = {}i32{};", "{ a: ".repeat(n), " }".repeat(n)),
        ),
        ("types_fn", format!("let a: {}i32;", "() => ".repeat(n))),
        (
            "patterns",
            format!("const {}a{} = 1;", "{ a: ".repeat(n), " }".repeat(n)),
        ),
        (
            "switch_expr",
            format!(
                "const a = {}1{};",
                "switch (x) { case 1: ".repeat(n),
                " }".repeat(n)
            ),
        ),
        (
            "chain_add",
            format!("const a = 1{};", " + 1".repeat(100_000)),
        ),
        (
            "chain_member",
            format!("const a = x{};", ".y".repeat(100_000)),
        ),
        (
            "chain_call",
            format!("const a = f{};", "()".repeat(100_000)),
        ),
        (
            "chain_index",
            format!("const a = x{};", "[0]".repeat(100_000)),
        ),
        ("chain_try", format!("const a = x{};", "?".repeat(100_000))),
        (
            "chain_cast",
            format!("const a = x{};", " as i32".repeat(100_000)),
        ),
        ("chain_lt", format!("const a = 1{};", " < 2".repeat(50_000))),
        (
            "chain_generic",
            format!("const a = f{};", "<a".repeat(50_000)),
        ),
        (
            "chain_generic2",
            format!("const a = f{};", "<a, b".repeat(20_000)),
        ),
        (
            "siblings_in_nested_array_18",
            format!(
                "const a = {}{}{};",
                "[".repeat(18),
                "1,".repeat(500_000),
                "]".repeat(18)
            ),
        ),
        (
            "siblings_in_nested_array_45",
            format!(
                "const a = {}{}{};",
                "[".repeat(45),
                "1,".repeat(200_000),
                "]".repeat(45)
            ),
        ),
        (
            "siblings_in_nested_blocks",
            format!(
                "function f(): void {{ {}{}{} }}",
                "{".repeat(18),
                "a;".repeat(500_000),
                "}".repeat(18)
            ),
        ),
        (
            "siblings_in_nested_types",
            format!(
                "let a: {}{}{};",
                "Array<".repeat(18),
                "i32, ".repeat(200_000) + "i32",
                ">".repeat(18)
            ),
        ),
        (
            "siblings_in_nested_calls_45",
            format!(
                "const a = {}{}{};",
                "f(".repeat(45),
                "g(1), ".repeat(100_000) + "1",
                ")".repeat(45)
            ),
        ),
        (
            "many_stmts",
            format!("function f(): void {{ {} }}", "a = 1; ".repeat(200_000)),
        ),
        ("many_decls", "let a = 1;\n".repeat(200_000)),
        ("many_errors", "let = ;\n".repeat(50_000)),
        ("many_closers", ")".repeat(200_000)),
        ("many_openers", "(".repeat(200_000)),
        ("many_braces", "{".repeat(200_000)),
        ("many_quotes", "\"".repeat(200_000)),
        ("many_backticks", "`".repeat(200_000)),
        ("many_dollar_brace", "`${".repeat(100_000)),
        ("many_at", "@".repeat(100_000)),
        ("many_question", "a ? ".repeat(100_000)),
    ];
    for (name, src) in &cases {
        let t = Instant::now();
        run_case(src, SLOW);
        eprintln!("{name}: {} bytes in {:?}", src.len(), t.elapsed());
    }
}

#[test]
fn speculation_cannot_explode() {
    let n = 200;
    let cases = [
        // each level could be an arrow with a return type or a parenthesised then-branch
        format!("const a = {}1;", "x ? (a): T => ".repeat(n)),
        format!(
            "const a = {}1{};",
            "x ? (a): T => (b) ".repeat(n),
            " : 2".repeat(n)
        ),
        format!("const a = {}1;", "f<a>(b < c, d > (".repeat(n)),
        format!("const a = {}1;", "(a): X<Y<Z>> => ".repeat(n)),
        format!(
            "const a = {}1{};",
            "f(a < b, c > (".repeat(n),
            ")".repeat(2 * n)
        ),
        format!("const a = {};", "a < b > c < d > e".repeat(n)),
        format!("const a = {}1;", "x ? (a): T => y ? (b): U => ".repeat(n)),
    ];
    for s in &cases {
        let t = Instant::now();
        run_case(s, SLOW);
        eprintln!("speculation case {} bytes: {:?}", s.len(), t.elapsed());
    }
}

#[test]
fn huge_literals_and_names() {
    let big = 1_000_000;
    let cases = [
        format!("const a = {};", "9".repeat(big)),
        format!("const a = 0x{};", "f".repeat(big)),
        format!("const a = 1.{};", "0".repeat(big)),
        format!("const a = 1e{};", "9".repeat(big)),
        format!("const a = \"{}\";", "x".repeat(big)),
        format!("const a = \"{}\";", "\\n".repeat(big / 2)),
        format!("const a = `{}`;", "y".repeat(big)),
        format!("const {} = 1;", "n".repeat(big)),
        format!("// {}\nconst a = 1;", "c".repeat(big)),
        format!("/* {} */ const a = 1;", "c".repeat(big)),
        format!("const a = \"{}\";", "é".repeat(big / 2)),
        " ".repeat(big),
        "\n".repeat(big),
        format!("const a = 1{};", "_1".repeat(big / 2)),
    ];
    for s in &cases {
        run_case(s, SLOW);
    }
}

#[test]
fn zz_report_case_count() {
    // runs last alphabetically within this binary only by convention; the
    // number is informational (printed with --nocapture)
    eprintln!("fuzz cases run so far: {}", CASES.load(Ordering::Relaxed));
}
