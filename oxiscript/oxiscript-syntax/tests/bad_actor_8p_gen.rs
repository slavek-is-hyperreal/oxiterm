#![allow(dead_code, clippy::all, clippy::pedantic)]
//! BAD ACTOR 8.P: structured program generator + oracle checks.
//!
//! Oracle for every generated program that parses clean:
//!   1. format() succeeds, is idempotent;
//!   2. format_with(braces:false) re-parses to the same AST shape;
//!   3. the multiset of comments (trimmed text) is preserved;
//!   4. comment text is not duplicated.
//! Run with `BA_ITERS=20000 cargo test -p oxiscript-syntax --release --test bad_actor_8p_gen -- --nocapture`.
mod common;
use common::shape;
use oxiscript_syntax::*;

pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
    pub fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
}

/// A token and whether it must be glued to the previous token (no whitespace).
#[derive(Clone)]
pub struct Tok(pub String, pub bool);
fn t(s: &str) -> Vec<Tok> {
    vec![Tok(s.to_string(), false)]
}
fn glue(s: &str) -> Vec<Tok> {
    vec![Tok(s.to_string(), true)]
}
fn cat(parts: Vec<Vec<Tok>>) -> Vec<Tok> {
    parts.into_iter().flatten().collect()
}
fn paren(e: Vec<Tok>) -> Vec<Tok> {
    cat(vec![t("("), e, t(")")])
}

pub struct Gen {
    pub r: Rng,
    pub comments: bool,
}

const IDENTS: &[&str] = &["a", "b", "c", "x", "y", "foo", "T", "U"];
const BINOPS: &[&str] = &[
    "+", "-", "*", "/", "%", "+%", "-%", "*%", "+|", "-|", "*|", "&", "|", "^", "<<", ">>", "&&",
    "||", "??", "===", "!==", "<", "<=", ">", ">=",
];
const ASSIGN: &[&str] = &[
    "=", "+=", "-=", "*=", "/=", "%=", "+%=", "-%=", "*%=", "+|=", "-|=", "*|=", "&=", "|=", "^=",
    "<<=", ">>=",
];

impl Gen {
    fn ident(&mut self) -> String {
        IDENTS[self.r.below(IDENTS.len())].to_string()
    }
    fn is_atomic(e: &[Tok]) -> bool {
        e.len() == 1
    }
    pub fn ty(&mut self, d: usize) -> Vec<Tok> {
        if d == 0 {
            return t(["i32", "u8", "string", "bool", "T"][self.r.below(5)]);
        }
        match self.r.below(9) {
            0 => cat(vec![t("Array"), t("<"), self.ty(d - 1), t(">")]),
            1 => cat(vec![self.ty(d - 1), t("["), t("]")]),
            2 => cat(vec![self.ty(d - 1), t("|"), t("null")]),
            3 => cat(vec![
                t("("),
                t("p"),
                t(":"),
                self.ty(d - 1),
                t(")"),
                t("=>"),
                self.ty(d - 1),
            ]),
            4 => cat(vec![
                t("{"),
                t("k"),
                t(":"),
                self.ty(d - 1),
                t(";"),
                t("readonly"),
                t("m"),
                t(":"),
                self.ty(d - 1),
                t("}"),
            ]),
            5 => cat(vec![
                t("Result"),
                t("<"),
                self.ty(d - 1),
                t(","),
                self.ty(d - 1),
                t(">"),
            ]),
            6 => cat(vec![t("("), self.ty(d - 1), t("|"), self.ty(d - 1), t(")")]),
            7 => cat(vec![
                t("Array"),
                t("<"),
                t("Array"),
                t("<"),
                self.ty(d - 1),
                t(">"),
                t(">"),
            ]),
            _ => t(["i32", "u8", "string", "T", "a.b", "\"s\"", "1", "true"][self.r.below(8)]),
        }
    }
    fn operand(&mut self, d: usize) -> Vec<Tok> {
        let e = self.expr(d);
        if Self::is_atomic(&e) || self.r.chance(50) {
            if Self::is_atomic(&e) {
                e
            } else {
                paren(e)
            }
        } else {
            paren(e)
        }
    }
    fn lit(&mut self) -> Vec<Tok> {
        let s = match self.r.below(18) {
            0 => "1",
            1 => "0xFFu8",
            2 => "1.5",
            3 => ".5",
            4 => "1e3",
            5 => "\"s\"",
            6 => "'q\"'",
            7 => "\"a\\n\\t\\u{202E}\\0\"",
            8 => "true",
            9 => "null",
            10 => "this",
            11 => "`t${a}u`",
            12 => "`a\\${b}\\``",
            13 => "1_000i64",
            14 => "\"\\u{1F600}\\x41\"",
            15 => "`${a}${b}`",
            16 => "\"${x}\"",
            _ => "0",
        };
        t(s)
    }
    pub fn expr(&mut self, d: usize) -> Vec<Tok> {
        if d == 0 {
            return if self.r.chance(50) {
                self.lit()
            } else {
                t(&self.ident())
            };
        }
        let d1 = d - 1;
        match self.r.below(24) {
            0 | 1 | 2 | 3 | 4 => {
                // binary: always parenthesise non-atomic operands half of the time; the
                // other half relies on precedence (may be rejected: then the case is skipped)
                let op = BINOPS[self.r.below(BINOPS.len())];
                let l = if self.r.chance(60) {
                    self.operand(d1)
                } else {
                    self.expr(d1)
                };
                let r = if self.r.chance(60) {
                    self.operand(d1)
                } else {
                    self.expr(d1)
                };
                let l = if Self::is_atomic(&l) || self.r.chance(30) {
                    l
                } else {
                    paren(l)
                };
                let r = if Self::is_atomic(&r) || self.r.chance(30) {
                    r
                } else {
                    paren(r)
                };
                cat(vec![l, t(op), r])
            }
            5 => {
                let op = ["-", "!", "~"][self.r.below(3)];
                cat(vec![t(op), self.operand(d1)])
            }
            6 => cat(vec![
                self.operand(d1),
                t("?"),
                self.operand(d1),
                t(":"),
                self.operand(d1),
            ]),
            7 => cat(vec![
                self.operand(d1),
                t("("),
                self.expr(d1),
                t(","),
                self.expr(d1),
                t(")"),
            ]),
            8 => cat(vec![
                self.operand(d1),
                t("<"),
                self.ty(1),
                t(">"),
                t("("),
                self.expr(d1),
                t(")"),
            ]),
            9 => cat(vec![self.operand(d1), t("."), t(&self.ident())]),
            10 => cat(vec![self.operand(d1), t("?."), t(&self.ident())]),
            11 => cat(vec![self.operand(d1), t("["), self.expr(d1), t("]")]),
            12 => cat(vec![self.operand(d1), glue("?")]),
            13 => cat(vec![self.operand(d1), t("as"), self.ty(1)]),
            14 => {
                // arrow
                match self.r.below(4) {
                    0 => cat(vec![
                        t("("),
                        t("p"),
                        t(","),
                        t("q"),
                        t(")"),
                        t("=>"),
                        self.operand(d1),
                    ]),
                    1 => cat(vec![t("p"), t("=>"), self.operand(d1)]),
                    2 => cat(vec![
                        t("("),
                        t("p"),
                        t(":"),
                        self.ty(1),
                        t(")"),
                        t(":"),
                        self.ty(1),
                        t("=>"),
                        self.operand(d1),
                    ]),
                    _ => cat(vec![t("("), t(")"), t("=>"), t("{"), self.stmt(d1), t("}")]),
                }
            }
            15 => cat(vec![
                t("{"),
                t("k"),
                t(":"),
                self.expr(d1),
                t(","),
                t("m"),
                t("}"),
            ]),
            16 => cat(vec![t("["), self.expr(d1), t(","), self.expr(d1), t("]")]),
            17 => cat(vec![t("new"), t("T"), t("("), self.expr(d1), t(")")]),
            18 => cat(vec![
                t("switch"),
                t("("),
                self.expr(d1),
                t(")"),
                t("{"),
                t("case"),
                self.lit(),
                t(","),
                self.lit(),
                t(":"),
                self.expr(d1),
                t(","),
                t("default"),
                t(":"),
                self.expr(d1),
                t("}"),
            ]),
            19 => cat(vec![
                t("`"),
                // template via literal (tokens are joined with spaces, so build as one token)
            ])
            .into_iter()
            .take(0)
            .chain(t("`a${b}c`"))
            .collect(),
            20 => cat(vec![self.operand(d1), t("="), self.expr(d1)])
                .into_iter()
                .collect(),
            21 => paren(self.expr(d1)),
            22 => cat(vec![
                t(&self.ident()),
                t("<"),
                t(&self.ident()),
                t(">"),
                t("("),
                self.expr(d1),
                t(")"),
            ]),
            _ => self.lit(),
        }
    }
    pub fn stmt(&mut self, d: usize) -> Vec<Tok> {
        if d == 0 {
            return cat(vec![t("x"), t("="), t("1"), t(";")]);
        }
        let d1 = d - 1;
        match self.r.below(16) {
            0 => cat(vec![
                t("let"),
                t("v"),
                t(":"),
                self.ty(1),
                t("="),
                self.expr(d1),
                t(";"),
            ]),
            1 => cat(vec![t("const"), t("w"), t("="), self.expr(d1), t(";")]),
            2 => cat(vec![t("f"), t("("), self.expr(d1), t(")"), t(";")]),
            3 => cat(vec![
                t("a"),
                t(ASSIGN[self.r.below(ASSIGN.len())]),
                self.expr(d1),
                t(";"),
            ]),
            4 => cat(vec![
                t("if"),
                t("("),
                self.expr(d1),
                t(")"),
                self.stmt(d1),
                t("else"),
                self.stmt(d1),
            ]),
            5 => cat(vec![t("if"), t("("), self.expr(d1), t(")"), self.stmt(d1)]),
            6 => cat(vec![
                t("while"),
                t("("),
                self.expr(d1),
                t(")"),
                self.stmt(d1),
            ]),
            7 => cat(vec![
                t("for"),
                t("("),
                t("let"),
                t("i"),
                t("="),
                t("0"),
                t(";"),
                t("i"),
                t("<"),
                self.expr(d1),
                t(";"),
                t("i"),
                t("++"),
                t(")"),
                self.stmt(d1),
            ]),
            8 => cat(vec![
                t("for"),
                t("("),
                t("const"),
                t("{"),
                t("p"),
                t(":"),
                t("q"),
                t("}"),
                t("of"),
                self.expr(d1),
                t(")"),
                self.stmt(d1),
            ]),
            9 => cat(vec![
                t("switch"),
                t("("),
                self.expr(d1),
                t(")"),
                t("{"),
                t("case"),
                self.lit(),
                t(":"),
                t("case"),
                self.lit(),
                t(":"),
                self.stmt(d1),
                t("default"),
                t(":"),
                self.stmt(d1),
                t("}"),
            ]),
            10 => cat(vec![t("return"), self.expr(d1), t(";")]),
            11 => cat(vec![t("{"), self.stmt(d1), self.stmt(d1), t("}")]),
            12 => cat(vec![
                t("do"),
                self.stmt(d1),
                t("while"),
                t("("),
                self.expr(d1),
                t(")"),
                t(";"),
            ]),
            13 => cat(vec![t("lbl"), t(":"), self.stmt(d1)]),
            14 => cat(vec![t("i"), t("++"), t(";")]),
            _ => cat(vec![t("{"), t("}")]),
        }
    }
    pub fn item(&mut self, d: usize) -> Vec<Tok> {
        match self.r.below(10) {
            0 => cat(vec![
                t("function"),
                t("f"),
                t("<"),
                t("T"),
                t("extends"),
                self.ty(1),
                t(">"),
                t("("),
                t("a"),
                t(":"),
                self.ty(1),
                t(","),
                t("b"),
                t(":"),
                self.ty(1),
                t("="),
                self.expr(1),
                t(")"),
                t(":"),
                self.ty(1),
                t("{"),
                self.stmt(d),
                self.stmt(d),
                t("}"),
            ]),
            1 => cat(vec![
                t("export"),
                t("function"),
                t("g"),
                t("("),
                t(")"),
                t("{"),
                self.stmt(d),
                t("}"),
            ]),
            2 => cat(vec![
                t("struct"),
                t("S"),
                t("{"),
                t("a"),
                t(":"),
                self.ty(1),
                t(";"),
                t("readonly"),
                t("b"),
                t(":"),
                self.ty(1),
                t("="),
                self.expr(1),
                t(","),
                t("mutating"),
                t("m"),
                t("("),
                t(")"),
                t("{"),
                self.stmt(d),
                t("}"),
                t("}"),
            ]),
            3 => cat(vec![
                t("class"),
                t("C"),
                t("implements"),
                t("I"),
                t("{"),
                t("private"),
                t("a"),
                t(":"),
                self.ty(1),
                t(";"),
                t("constructor"),
                t("("),
                t(")"),
                t("{"),
                self.stmt(d),
                t("}"),
                t("}"),
            ]),
            4 => cat(vec![
                t("interface"),
                t("I"),
                t("extends"),
                t("J"),
                t("{"),
                t("m"),
                t("("),
                t("a"),
                t(":"),
                self.ty(1),
                t(")"),
                t(":"),
                self.ty(1),
                t(";"),
                t("}"),
            ]),
            5 => cat(vec![
                t("enum"),
                t("E"),
                t(":"),
                t("u8"),
                t("{"),
                t("A"),
                t("="),
                self.expr(1),
                t(","),
                t("B"),
                t("}"),
            ]),
            6 => cat(vec![
                t("type"),
                t("U"),
                t("="),
                t("|"),
                t("{"),
                t("kind"),
                t(":"),
                t("\"a\""),
                t(";"),
                t("r"),
                t(":"),
                self.ty(1),
                t("}"),
                t("|"),
                t("{"),
                t("kind"),
                t(":"),
                t("\"b\""),
                t("}"),
                t(";"),
            ]),
            7 if std::env::var("BA_NOIMPORT").is_err() => cat(vec![
                t("import"),
                t("{"),
                t("A"),
                t("as"),
                t("B"),
                t(","),
                t("C"),
                t("}"),
                t("from"),
                t("\"./m\""),
                t(";"),
            ]),
            8 => cat(vec![
                t("law"),
                t("l"),
                t("("),
                t("a"),
                t(":"),
                t("i32"),
                t(")"),
                t("when"),
                self.expr(1),
                t("{"),
                self.stmt(d),
                self.expr(d),
                t("}"),
            ]),
            _ => cat(vec![
                t("@inline"),
                t("function"),
                t("h"),
                t("("),
                t(")"),
                t("{"),
                self.stmt(d),
                t("}"),
            ]),
        }
    }
    pub fn program(&mut self, d: usize) -> Vec<Tok> {
        let n = 1 + self.r.below(3);
        let mut v = Vec::new();
        for _ in 0..n {
            v.extend(self.item(d));
        }
        v
    }
    /// Join tokens with random separators; comments appear in any gap.
    pub fn join(&mut self, toks: &[Tok]) -> String {
        let mut out = String::new();
        for (i, Tok(s, g)) in toks.iter().enumerate() {
            if i > 0 && !*g {
                let sep = if self.comments {
                    self.r.below(14)
                } else {
                    self.r.below(2)
                };
                match sep {
                    0 | 2 | 3 | 4 | 5 => out.push(' '),
                    1 => out.push('\n'),
                    6 => out.push_str(" /* c */ "),
                    7 => out.push_str(" // line\n"),
                    8 => out.push_str("/*m\n  l2*/ "),
                    9 => out.push_str(" /// doc\n"),
                    10 => out.push_str("\n\n"),
                    11 => out.push_str("/**/"),
                    12 => out.push_str(" /* a */ /* b */ // t\n"),
                    _ => out.push_str("\n\n\n// x\n\n"),
                }
            }
            out.push_str(s);
        }
        out
    }
}

pub fn comment_texts(src: &str) -> Vec<String> {
    // Relaxed: the multiset of words inside comments (markers removed), so merging
    // two comments into one is tolerated but loss/duplication/leak into code is not.
    let mut v: Vec<String> = Vec::new();
    for c in &lex(src, FileId(0)).comments {
        let raw = &src[c.lo as usize..c.hi as usize];
        let body = raw
            .replace("/*", " ")
            .replace("*/", " ")
            .replace("///", " ")
            .replace("//", " ");
        for w in body.split_whitespace() {
            v.push(w.to_string());
        }
    }
    v.sort();
    v
}

pub fn sh(m: &Module) -> String {
    shape(m)
}

/// Returns Err(reason) when an oracle fails.
pub fn check(src: &str) -> Result<bool, String> {
    let p = parse(src, FileId(0));
    if !p.diagnostics.is_empty() {
        return Ok(false);
    }
    let once = format(src).map_err(|e| format!("format failed on clean input: {e}"))?;
    let twice = format(&once)
        .map_err(|e| format!("formatted output does not parse: {e}\n--once--\n{once}"))?;
    if once != twice {
        return Err(format!(
            "NOT IDEMPOTENT\n--once--\n{once}\n--twice--\n{twice}"
        ));
    }
    let o = FormatOptions {
        braces: false,
        ..FormatOptions::default()
    };
    let nb = format_with(src, &o).map_err(|e| format!("braces=false failed: {e}"))?;
    let (m2, d2) = parse_module(&nb);
    if !d2.is_empty() {
        return Err(format!("braces=false output has errors {d2:?}\n{nb}"));
    }
    if sh(&p.module) != sh(&m2) {
        return Err(format!("AST CHANGED by braces=false format\n--out--\n{nb}"));
    }
    let (m3, d3) = parse_module(&once);
    if !d3.is_empty() {
        return Err(format!("canonical output has errors {d3:?}"));
    }
    let _ = m3;
    let (a, b) = (comment_texts(src), comment_texts(&once));
    if a != b {
        return Err(format!(
            "COMMENTS CHANGED\n in: {a:?}\nout: {b:?}\n--out--\n{once}"
        ));
    }
    // width: a canonical line may exceed 100 only when unbreakable; flag absurd overflow
    for l in once.lines() {
        if l.chars().count() > 400
            && !l.contains("//")
            && !l.contains("/*")
            && !l.contains('"')
            && !l.contains('`')
        {
            return Err(format!("line {} chars wide:\n{l}", l.chars().count()));
        }
    }
    Ok(true)
}

#[test]
#[ignore = "fuzz oracle (BA_ITERS, BA_SEED, BA_NOCOMMENTS, BA_NOIMPORT); with comments it fails on classes covered by F1/F2/F4 in bad_actor_8p.rs; BA_NOCOMMENTS=1 BA_NOIMPORT=1 passes"]
fn structured_roundtrip() {
    let iters: usize = std::env::var("BA_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1500);
    let seed: u64 = std::env::var("BA_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xBADA_C702);
    let with_comments = std::env::var("BA_NOCOMMENTS").is_err();
    let mut g = Gen {
        r: Rng(seed | 1),
        comments: with_comments,
    };
    let (mut clean, mut rejected) = (0usize, 0usize);
    let mut fails: Vec<(String, String)> = Vec::new();
    for _ in 0..iters {
        let d = 1 + g.r.below(4);
        let prog = g.program(d);
        let src = g.join(&prog);
        match std::panic::catch_unwind(|| check(&src)) {
            Ok(Ok(true)) => clean += 1,
            Ok(Ok(false)) => rejected += 1,
            Ok(Err(e)) => fails.push((src, e)),
            Err(_) => fails.push((src, "PANIC".into())),
        }
    }
    eprintln!("clean={clean} rejected={rejected} failures={}", fails.len());
    fails.sort_by_key(|(s, _)| s.len());
    let mut seen: std::collections::BTreeMap<String, usize> = Default::default();
    for (s, e) in fails.iter() {
        let cat: String = e.lines().next().unwrap_or("").chars().take(60).collect();
        let n = seen.entry(cat.clone()).or_insert(0);
        *n += 1;
        if *n <= 2 && s.len() < 600 {
            eprintln!(
                "=== FAIL [{cat}] ({} bytes)\n{s}\n--- reason\n{e}\n",
                s.len()
            );
        }
    }
    eprintln!("categories: {seen:?}");
    assert_eq!(fails.len(), 0, "{} oracle failures", fails.len());
}

/// Apply every suggested fix of every diagnostic of mutated programs; the fix must apply,
/// and the fixed text must have strictly fewer diagnostics, none of the same code at the
/// same place, and no overlapping edits inside one fix.
#[test]
#[ignore = "fuzz oracle; currently finds the E0006 no-op fix (X1 in bad_actor_8p.rs)"]
fn fixit_sweep() {
    let iters: usize = std::env::var("BA_ITERS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1500);
    let seed: u64 = std::env::var("BA_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0xF1F1_7001);
    let mut g = Gen {
        r: Rng(seed | 1),
        comments: false,
    };
    let mut bad: std::collections::BTreeMap<String, (String, String)> = Default::default();
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    let (mut nfix, mut ndiag) = (0usize, 0usize);
    for _ in 0..iters {
        let d = 1 + g.r.below(3);
        let prog = g.program(d);
        let mut toks = prog.clone();
        // mutate: drop, duplicate or swap a token
        for _ in 0..(1 + g.r.below(2)) {
            if toks.len() < 3 {
                break;
            }
            let i = g.r.below(toks.len());
            match g.r.below(4) {
                0 => {
                    toks.remove(i);
                }
                1 => {
                    let x = toks[i].clone();
                    toks.insert(i, x);
                }
                2 => {
                    let j = g.r.below(toks.len());
                    toks.swap(i, j);
                }
                _ => {
                    let junk = [
                        "==", "!=", "?", " ?", "a b", ">>", "as", "|", "<", ">", "=>", ";", "{",
                        "}", "(", ")",
                    ];
                    toks.insert(i, Tok(junk[g.r.below(junk.len())].to_string(), false));
                }
            }
        }
        let src = g.join(&toks);
        let p = parse(&src, FileId(0));
        ndiag += p.diagnostics.len();
        if p.diagnostics.len() != 1 {
            continue;
        }
        for dg in &p.diagnostics {
            *counts.entry(dg.code.to_string()).or_default() += 1;
            // span sanity
            if dg.span.lo > dg.span.hi || dg.span.hi as usize > src.len() + 1 {
                bad.entry(format!("SPAN {}", dg.code))
                    .or_insert((src.clone(), format!("{dg:?}")));
            }
            if !dg.code.starts_with("OXS-") {
                bad.entry("CODE".into())
                    .or_insert((src.clone(), format!("{dg:?}")));
            }
            for f in &dg.fixes {
                nfix += 1;
                let key = format!("{} / {}", dg.code, f.title);
                // overlapping edits
                let mut es: Vec<_> = f.edits.iter().collect();
                es.sort_by_key(|e| (e.span.lo, e.span.hi));
                for w in es.windows(2) {
                    if w[0].span.hi > w[1].span.lo {
                        bad.entry(format!("OVERLAP {key}"))
                            .or_insert((src.clone(), format!("{dg:?}")));
                    }
                }
                let Some(fixed) = apply_edits(&src, &f.edits) else {
                    bad.entry(format!("INAPPLICABLE {key}"))
                        .or_insert((src.clone(), format!("{dg:?}")));
                    continue;
                };
                let p2 = parse(&fixed, FileId(0));
                let same_place = p2.diagnostics.iter().any(|d2| {
                    d2.code == dg.code
                        && d2.span.lo >= dg.span.lo.saturating_sub(2)
                        && d2.span.lo <= dg.span.lo + 2
                });
                if p2.diagnostics.len() > p.diagnostics.len() || same_place {
                    let ent = bad.entry(format!("NOFIX {key}"));
                    let newv = (
                        src.clone(),
                        format!(
                            "{dg:?}\nfixed: {fixed}\nbefore={} after={:?}",
                            p.diagnostics.len(),
                            p2.diagnostics
                                .iter()
                                .map(|d| d.to_string())
                                .collect::<Vec<_>>()
                        ),
                    );
                    match ent {
                        std::collections::btree_map::Entry::Vacant(v) => {
                            v.insert(newv);
                        }
                        std::collections::btree_map::Entry::Occupied(mut o) => {
                            if newv.0.len() < o.get().0.len() {
                                o.insert(newv);
                            }
                        }
                    }
                }
            }
        }
    }
    eprintln!("diags={ndiag} fixes={nfix} codes={counts:?}");
    for (k, (s, e)) in &bad {
        if s.len() < 700 {
            eprintln!("=== {k}\n{s}\n--- {e}\n");
        } else {
            eprintln!("=== {k} (long input)");
        }
    }
    assert_eq!(bad.len(), 0, "{} fix-it problem classes", bad.len());
}
