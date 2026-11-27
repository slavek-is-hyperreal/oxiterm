#![allow(dead_code, clippy::all, clippy::pedantic)]
//! BAD ACTOR round 2026-11-27-1, target 1: formatter F8 comment re-attachment
//! (`fmt/build.rs`), the E0004 continuation hazard and throw/try/catch diagnostics
//! in `parser/mod.rs`. Failing tests are findings (R1-nn in
//! docs/stages/rounds/2026-11-27-1.md).
use oxiscript_syntax::*;

fn clean(src: &str) -> bool {
    parse_module(src).1.iter().all(|d| !d.is_error())
}

/// Remove every comment (replaced by one space) so two sources can be compared
/// for code only.
fn strip_comments(src: &str) -> String {
    let lx = lex(src, FileId(0));
    let mut out = String::new();
    let mut at = 0usize;
    for c in &lx.comments {
        out.push_str(&src[at..c.lo as usize]);
        out.push(' ');
        at = c.hi as usize;
    }
    out.push_str(&src[at..]);
    out
}

fn comment_texts(src: &str) -> Vec<String> {
    let lx = lex(src, FileId(0));
    lx.comments.iter().map(|c| src[c.lo as usize..c.hi as usize].trim().to_string()).collect()
}

/// The oracle: `format` must succeed on a clean input, its output must parse
/// cleanly, be a fixed point, keep every comment, and the code with comments
/// removed must format to the same text as the input with comments removed.
fn oracle(src: &str) -> Result<(), String> {
    if !clean(src) {
        return Err(format!("probe input itself does not parse: {src:?} {:?}", parse_module(src).1));
    }
    let out = format(src).map_err(|e| format!("format failed on clean input {src:?}: {e:?}"))?;
    if !clean(&out) {
        return Err(format!("output does not parse\n--- in\n{src}\n--- out\n{out}"));
    }
    let again = format(&out).map_err(|e| format!("second format failed: {e:?}"))?;
    if again != out {
        return Err(format!("not idempotent\n--- in\n{src}\n--- 1\n{out}\n--- 2\n{again}"));
    }
    let mut ci = comment_texts(src);
    let mut co = comment_texts(&out);
    ci.sort();
    co.sort();
    if ci != co {
        return Err(format!("comments changed {ci:?} -> {co:?}\n--- in\n{src}\n--- out\n{out}"));
    }
    let code_in = format(&strip_comments(src)).map_err(|e| format!("{e:?}"))?;
    let code_out = format(&strip_comments(&out)).map_err(|e| format!("{e:?}"))?;
    let squash = |s: &str| s.lines().filter(|l| !l.trim().is_empty()).collect::<Vec<_>>().join("\n");
    if squash(&code_in) != squash(&code_out) {
        return Err(format!("code changed\n--- in\n{src}\n--- out\n{out}\n--- code in\n{code_in}\n--- code out\n{code_out}"));
    }
    Ok(())
}

const PROBES: &[&str] = &[
    // binary operators, both sides of the operator
    "const x: i32 = a /* b */ + c;\n",
    "const x: i32 = a + /* b */ c;\n",
    "const x: i32 = a // b\n  + c;\n",
    "const x: i32 = a + // b\n  c;\n",
    "const x: i32 = a /* b */ + /* c */ d /* e */ * f;\n",
    "const x: bool = a // one\n  && b // two\n  || c;\n",
    "const x: i32 = (a // p\n  + b) * c;\n",
    "const x = a ?? // n\n  b;\n",
    // calls and arguments
    "const r = f(/* none */);\n",
    "const r = f(a, // first\n  b);\n",
    "const r = f(a /* x */, /* y */ b);\n",
    "const r = f(\n  // leading\n  a,\n);\n",
    "const r = obj // o\n  .m();\n",
    "const r = obj.m // m\n  ();\n",
    "const r = a?.b // c\n  ?.c;\n",
    // arrays, objects
    "const xs = [ // open\n  1,\n];\n",
    "const xs = [1, /* mid */ 2];\n",
    "const xs = [\n  1, // one\n  // own line\n  2,\n];\n",
    "const xs: i32[] = [ /* empty */ ];\n",
    "const o: P = { x: 1, /* y */ y: 2 };\n",
    "const o: P = { x: /* v */ 1 };\n",
    "const o: P = { // open\n  x: 1,\n};\n",
    "const o: P = {\n  x: 1,\n  // dangling\n};\n",
    // conditional, unary, as, assignment, return
    "const v = c // q\n  ? a // t\n  : b;\n",
    "const v = - /* neg */ a;\n",
    "const v = !/* not */ a;\n",
    "const v = a /* c */ as f64;\n",
    "const v = a as /* c */ f64;\n",
    "function f(): i32 {\n  return /* r */ 1;\n}\n",
    "function f(): i32 {\n  return 1 // r\n  ;\n}\n",
    "function f(): void {\n  x = // v\n    1;\n}\n",
    "function f(): void {\n  x += /* v */ 1;\n}\n",
    // try operator and templates
    "function f(): Result<i32, i32> {\n  const v = g()? // t\n  ;\n  return Ok(v);\n}\n",
    "const s = `a ${ /* in */ b } c`;\n",
    "const s = `a ${b // in\n} c`;\n",
    // type positions
    "function f(a: /* t */ i32): i32 {\n  return a;\n}\n",
    "function f(a: i32 /* t */ | null): i32 {\n  return 0;\n}\n",
    "function f<T /* g */>(a: T): T {\n  return a;\n}\n",
    "type S = { kind: \"a\" } // first\n  | { kind: \"b\" };\n",
    "type S =\n  // lead\n  | { kind: \"a\" }\n  | { kind: \"b\" };\n",
    // statements
    "function f(): void {\n  if (a /* c */) {\n  }\n}\n",
    "function f(): void {\n  if (a) // then\n  {\n  }\n}\n",
    "function f(): void {\n  if (a) {\n  } // after then\n  else {\n  }\n}\n",
    "function f(): void {\n  for (const x /* x */ of xs) {\n  }\n}\n",
    "function f(): void {\n  while (/* w */ a) {\n  }\n}\n",
    "function f(): void {\n  switch (k) {\n    // before case\n    case 1: // on case\n      g();\n  }\n}\n",
    "function f(): void {\n  const a = 1; /* one */ const b = 2; // two\n}\n",
    "function f(): void {\n  g(); // a /* b\n  h(); // */ i();\n}\n",
    "function f(): void {\n  /* a */ /* b */ g(); /* c */ /* d */\n}\n",
    // items
    "struct P {\n  x: i32; // x\n  /* y */ y: i32;\n}\n",
    "struct P {\n  x: i32 /* x */;\n}\n",
    "enum E {\n  A, // a\n  B /* b */,\n}\n",
    "import { Log /* l */ } from \"host:log\";\n",
    "export /* e */ function f(): void {\n}\n",
    "@host /* h */ export function f(): void {\n}\n",
    "class C {\n  constructor(/* none */) {\n  }\n}\n",
];

#[test]
fn probe_formatter_comment_oracle_over_placements() {
    let mut fails = Vec::new();
    for p in PROBES {
        if let Err(e) = oracle(p) {
            fails.push(e);
        }
    }
    for f in &fails {
        eprintln!("=====\n{f}");
    }
    assert!(fails.is_empty(), "{} of {} comment placements fail the formatter oracle (see stderr)", fails.len(), PROBES.len());
}

/// R1-11: E0004 continuation hazard: the parser must not offer a `;` insertion fix
/// whose application changes meaning, and must not miss a hazard.
#[test]
fn r1_11_continuation_hazard_fix_never_changes_meaning() {
    // Each input is missing a `;` at the end of line 1. Inserting it there is
    // only harmless when line 2 cannot continue line 1's expression.
    let hazards = [
        "function f(): void {\n  const a = b\n  (c);\n}\n",
        "function f(): void {\n  const a = b\n  [c];\n}\n",
        "function f(): void {\n  const a = b\n  -c;\n}\n",
        "function f(): void {\n  const a = b\n  `t`;\n}\n",
        "function f(): void {\n  const a = b\n  .c;\n}\n",
        "function f(): void {\n  const a = b\n  ?.c;\n}\n",
        "function f(): void {\n  const a = b\n  as f64;\n}\n",
        "function f(): void {\n  const a = b\n  ?? c;\n}\n",
        "function f(): void {\n  const a = b\n  ++c;\n}\n",
        "function f(): void {\n  const a = b\n  /c/;\n}\n",
        "function f(): void {\n  const a = b\n  < c;\n}\n",
    ];
    let mut bad = Vec::new();
    for src in hazards {
        let (_, diags) = parse_module(src);
        let eol1 = (src.find("= b\n").unwrap() + 3) as u32;
        for d in &diags {
            for fx in &d.fixes {
                if fx.edits.iter().any(|e| e.replacement.contains(';') && e.span.lo == eol1 && e.span.hi == eol1) {
                    bad.push(format!("{src:?}: {} offers {:?} at the end of line 1", d.code, fx.title));
                }
            }
        }
        eprintln!("{src:?} -> {:?}", diags.iter().map(|d| (d.code, d.message.clone(), d.fixes.len())).collect::<Vec<_>>());
    }
    assert!(bad.is_empty(), "E0004 `;` fixes applied across a continuation line:\n{}", bad.join("\n"));
}

/// throw/try/catch: the new diagnostic must not fire on ordinary code that
/// uses these words where they are not keywords (property names, strings),
/// and must fire (with no `;` fix) where they start a statement.
#[test]
fn probe_throw_try_catch_diagnostic_precision() {
    let ok = [
        "const s = \"throw try catch\";\n",
        "function f(o: O): void {\n  o.throw();\n  o.try = 1;\n  o.catch(1);\n}\n",
        "const o: O = { throw: 1, try: 2, catch: 3 };\n",
    ];
    for src in ok {
        let (_, d) = parse_module(src);
        let hit: Vec<_> = d.iter().filter(|d| d.message.contains("exception")).collect();
        assert!(hit.is_empty(), "no-exceptions diagnostic on non-statement use {src:?}: {hit:?}");
    }
    let bad = [
        "function f(): void {\n  throw new E();\n}\n",
        "function f(): void {\n  try {\n  } catch (e) {\n  }\n}\n",
        "function f(): void {\n  try {\n  } finally {\n  }\n}\n",
        "function f(): void {\n  if (a) throw x;\n}\n",
        "const g = () => { throw 1; };\n",
    ];
    for src in bad {
        let (_, d) = parse_module(src);
        assert!(!d.is_empty(), "no diagnostic for {src:?}");
        let semi_fix = d.iter().any(|d| d.fixes.iter().any(|f| f.edits.iter().any(|e| e.replacement == ";")));
        assert!(!semi_fix, "bogus `;` fix offered for {src:?}: {d:?}");
    }
}
