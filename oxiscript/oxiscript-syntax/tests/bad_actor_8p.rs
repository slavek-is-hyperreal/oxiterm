#![allow(dead_code, clippy::all, clippy::pedantic)]
//! BAD ACTOR 8.P: one failing test per finding. Tests marked `#[ignore]` abort the
//! process (stack overflow) or are very slow; run them alone with
//! `cargo test -p oxiscript-syntax --release --test bad_actor_8p -- --ignored --exact NAME`.
//! Structured fuzz oracles live in `bad_actor_8p_gen.rs`.
use oxiscript_syntax::*;
use std::time::{Duration, Instant};

fn clean(src: &str) -> bool {
    parse_module(src).1.is_empty()
}

// ---------------------------------------------------------------------------
// Formatter: valid input becomes invalid / changes meaning
// ---------------------------------------------------------------------------

/// F1 (HIGH): a line comment followed by a block comment between an expression and its `;`
/// are both hoisted behind the `;` on one line (`// a /* b`), so the tail of the block comment
/// becomes live code. `format()` returns Ok for text that no longer parses.
#[test]
fn f1_fmt_line_comment_then_block_comment_corrupts_code() {
    let src = "let x = 1 // a\n/* b\nc */;";
    assert!(clean(src));
    let out = format(src).unwrap();
    assert!(clean(&out), "formatted output no longer parses: {out:?}");
}

/// F1b: same through an import list (comments inside `{ }` are hoisted behind the `;`).
#[test]
fn f1b_fmt_import_comments_corrupt_code() {
    let src = "import { A // x\n/* m\n n */ } from \"./m\";";
    assert!(clean(src));
    let out = format(src).unwrap();
    assert!(clean(&out), "formatted output no longer parses: {out:?}");
}

/// F1c: a `//` comment swallows a following block comment: `g(a // l\n) /* m */;`
/// prints `g(a); // l /* m */` (the block comment is now part of the line comment, a
/// `///` doc comment becomes plain text).
#[test]
fn f1c_fmt_comments_merge_into_line_comment() {
    let src = "function f() { g(a // l\n) /* m */; }";
    let out = format(src).unwrap();
    assert_eq!(
        lex(src, FileId(0)).comments.len(),
        lex(&out, FileId(0)).comments.len(),
        "comment count changed: {out:?}"
    );
}

/// F2 (MEDIUM): not idempotent when a multi-line block comment trails a statement.
#[test]
fn f2_fmt_not_idempotent_multiline_block_comments() {
    let src = "function f() { return /* b\nc */ a /* d\ne */; // line\n}";
    let once = format(src).unwrap();
    let twice = format(&once).unwrap();
    assert_eq!(once, twice);
}

/// F3 (HIGH, changes meaning): `f()?` then a newline then `.y` is "try, then member".
/// The formatter prints `f()?.y`, which the lexer reads as optional chaining
/// (grammar 1.5 says `(f()?).y` is required). The re-parsed AST differs.
#[test]
fn f3_fmt_try_then_member_becomes_optional_chain() {
    let src = "let x = f()?\n  .y;";
    assert!(clean(src));
    let o = FormatOptions {
        braces: false,
        ..FormatOptions::default()
    };
    let out = format_with(src, &o).unwrap();
    let before = format!("{:?}", parse_module(src).0.items[0].kind).contains("optional: true");
    let after = format!("{:?}", parse_module(&out).0.items[0].kind).contains("optional: true");
    assert_eq!(
        before, after,
        "meaning changed: {out:?} (optional chaining appeared)"
    );
}

/// F3b: `f()?` newline `?.y` (try, then optional chaining) prints `f()??.y`: `??` + `.y`,
/// which does not even parse.
#[test]
fn f3b_fmt_try_then_optional_chain_does_not_reparse() {
    let src = "let x = f()?\n  ?.y;";
    assert!(clean(src));
    let out = format(src).unwrap();
    assert!(clean(&out), "formatted output no longer parses: {out:?}");
}

/// F4 (LOW/MEDIUM): comments inside attribute arguments are moved into the function body.
#[test]
fn f4_fmt_attribute_comment_moves_into_body() {
    let src = "@a(/* c */ 1) function f() {}";
    let out = format(src).unwrap();
    let c = out.find("/* c */").unwrap();
    let body = out.find("function").unwrap();
    assert!(
        c < body,
        "comment relocated into the function body: {out:?}"
    );
}

/// F5 (MEDIUM, Trojan-Source class): escapes for bidi controls / zero-width characters are
/// turned into the raw invisible characters in the output.
#[test]
fn f5_fmt_expands_escapes_to_raw_invisible_characters() {
    for esc in [
        "\\u{202E}",
        "\\u{2066}",
        "\\u{200B}",
        "\\u{FEFF}",
        "\\u{2028}",
        "\\u{61C}",
        "\\u{85}",
    ] {
        for q in ["\"", "`"] {
            let src = format!("let x = {q}a{esc}b{q};");
            assert!(clean(&src), "{src}");
            let out = format(&src).unwrap();
            let invisible = out.chars().any(|c| {
                matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200B}'..='\u{200F}'
                    | '\u{FEFF}' | '\u{2028}' | '\u{2029}' | '\u{61C}' | '\u{85}')
            });
            assert!(
                !invisible,
                "raw invisible character emitted for {esc}: {out:?}"
            );
        }
    }
}

/// F6 (MEDIUM, DoS): trailing same-line comments after a statement are O(n^2)
/// (`newlines_between(hi, c.lo)` rescans from the statement end for every comment,
/// fmt/build.rs `seq`). 32000 x `/**/` (128 KB) takes ~1.4 s release, 1 MB would take minutes.
#[test]
fn f6_fmt_trailing_comments_quadratic() {
    let t = |n: usize| {
        let src = format!("let x = 1; {}", "/**/".repeat(n));
        let s = Instant::now();
        format(&src).unwrap();
        s.elapsed()
    };
    let (a, b) = (t(8_000), t(16_000));
    eprintln!("8000: {a:?}, 16000: {b:?}");
    // linear => ratio ~2, quadratic => ratio ~4
    assert!(
        b < a * 3 || b < Duration::from_millis(100),
        "quadratic: 8000 comments {a:?}, 16000 comments {b:?}"
    );
}

// ---------------------------------------------------------------------------
// Fix-its
// ---------------------------------------------------------------------------

/// X1 (MEDIUM): the E0006 fix "remove the space before `?`" replaces the `?` token by `?`
/// (span of the `?` only): applying it changes nothing and the same error is reported.
#[test]
fn x1_e0006_remove_space_fix_is_a_noop() {
    let src = "let x = a ? b;";
    let (_, d) = parse_module(src);
    let fix = &d.iter().find(|d| d.code == "OXS-E0006").unwrap().fixes[0];
    let fixed = apply_edits(src, &fix.edits).unwrap();
    assert_ne!(fixed, src, "fix is a no-op");
}

/// X2 (LOW): `a?b;` yields two contradictory E0006 at the same span: "write a space before `?`"
/// and "remove the space before `?`" (the second is reported on text that has no space).
#[test]
fn x2_e0006_double_report_for_glued_question() {
    let (_, d) = parse_module("let x = a?b;");
    let n = d.iter().filter(|d| d.code == "OXS-E0006").count();
    assert_eq!(n, 1, "{d:?}");
}

// ---------------------------------------------------------------------------
// Limits, budgets, floods
// ---------------------------------------------------------------------------

fn find_threshold(make: impl Fn(usize) -> String) -> usize {
    let mut n = 1;
    while n < 100_000 {
        let (_, d) = parse_module(&make(n));
        if d.iter().any(|d| d.code == "OXS-E0012") {
            return n;
        }
        n *= 2;
    }
    usize::MAX
}

/// S1 (MEDIUM): the speculation budget is per FILE (`24*tokens + 20000`) but every `<` in an
/// argument list speculates "type arguments" up to the end of the list, so a perfectly valid
/// list of ~150 comparisons is rejected with E0012 ("too ambiguous").
#[test]
fn s1_speculation_budget_false_e0012_on_valid_comparison_list() {
    let call = |n: usize| {
        let args: Vec<String> = (0..n).map(|i| format!("a{i} < b{i}")).collect();
        format!("let x = f({});", args.join(", "))
    };
    let arr = |n: usize| {
        let args: Vec<String> = (0..n).map(|i| format!("a{i} < b{i}")).collect();
        format!("let x = [{}];", args.join(", "))
    };
    for n in [150usize, 300, 1000] {
        assert!(
            clean(&call(n)),
            "f(a0 < b0, ... {n} comparisons) rejected (threshold {})",
            find_threshold(call)
        );
        assert!(clean(&arr(n)), "[a0 < b0, ... {n} comparisons] rejected");
    }
}

/// S2 (MEDIUM): an `else if` ladder is parsed by recursion through `enter()`, so any ladder
/// longer than ~125 branches (generated code, decision tables) is rejected with E0011 although
/// it is valid and flat. The same limit applies to `a ? b : c ? d : ...` chains and `- - - x`.
#[test]
fn s2_long_else_if_ladder_rejected() {
    let n = 200;
    let src = format!(
        "function f() {{ if (a0) {{}} {} }}",
        (1..n)
            .map(|i| format!("else if (a{i}) {{}}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let (_, d) = parse_module(&src);
    assert!(
        d.is_empty(),
        "{n}-branch else-if ladder rejected: {:?}",
        d.first()
    );
}

#[test]
fn s2b_long_ternary_chain_rejected() {
    let n = 200;
    let src = format!(
        "let x = {} z;",
        (0..n)
            .map(|i| format!("a{i} ? b{i} :"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    let (_, d) = parse_module(&src);
    assert!(
        d.is_empty(),
        "{n}-way ternary chain rejected: {:?}",
        d.first()
    );
}

/// L1 (MEDIUM): lexer diagnostics are not capped (`MAX_DIAGS` only applies to the parser):
/// N junk bytes give N diagnostics (each with a heap String message): 1 MB of `#` = 1M diagnostics.
#[test]
fn l1_lexer_diagnostic_flood_is_unbounded() {
    let (_, d) = parse_module(&"#".repeat(100_000));
    assert!(
        d.len() <= 1000,
        "{} diagnostics for 100000 bytes of junk",
        d.len()
    );
}

#[test]
fn l1b_unterminated_string_flood_is_unbounded() {
    let (_, d) = parse_module(&"\"a\n".repeat(50_000));
    assert!(d.len() <= 1000, "{} diagnostics", d.len());
}

/// L2 (LOW): the literal diagnostics call everything "integer literal".
#[test]
fn l2_misleading_literal_messages() {
    let (_, d) = parse_module("let x = 1.;");
    assert!(
        !d[0].message.contains("integer literal"),
        "{}",
        d[0].message
    );
    let (_, d) = parse_module("let x = 1e;");
    assert!(
        !d[0].message.contains("integer literal"),
        "{}",
        d[0].message
    );
}

/// L3 (LOW): a CR-only line ending is whitespace, but a `//` comment runs on to the next `\n`,
/// swallowing the following "lines" (code inside the comment); the formatter then moves the
/// trailing comment to the wrong statement and emits a bare `\r` in the middle of a comment.
#[test]
fn l3_cr_only_line_endings() {
    let src = "let a = 1;\rlet b = 2;\r// c\rlet c = 3;";
    let out = format(src).unwrap();
    assert!(!out.contains('\r'), "bare CR emitted: {out:?}");
}

// ---------------------------------------------------------------------------
// Stack usage: small stacks (wasm32 without threads, ESP32 tasks 8-64 KB, tests on 256 KB)
// ---------------------------------------------------------------------------

fn nest_paren(n: usize) -> String {
    format!("let x = {}1{};", "(".repeat(n), ")".repeat(n))
}

#[cfg(not(target_os = "wasi"))]
fn on_stack<R: Send + 'static>(kb: usize, f: impl FnOnce() -> R + Send + 'static) -> R {
    std::thread::Builder::new()
        .stack_size(kb * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap()
}

/// K1 (HIGH for ESP32): parser frames cost ~3 KB per nesting level (release; ~18 KB debug).
/// 30 nested parentheses (< SOFT_DEPTH, so no big-stack restart even where threads exist)
/// do not fit in 64 KB.
#[cfg(all(unix, not(target_os = "wasi")))]
#[test]
fn k1_parse_30_nested_parens_on_64kb_stack() {
    on_stack(64, || {
        let p = parse(&nest_paren(30), FileId(0));
        assert!(p.diagnostics.is_empty());
        std::mem::forget(p);
    });
}

/// K2 (MEDIUM): `Drop` of a 900-term `a + a + ...` chain (height cap is 1000, parse is iterative)
/// costs ~100 bytes/level of the CALLER's stack: ~91 KB. Same for `a.b.b.b...` and `f()()()...`.
#[cfg(all(unix, not(target_os = "wasi")))]
#[test]
fn k2_drop_of_900_term_chain_on_32kb_stack() {
    let src = format!("let x = a{};", " + a".repeat(900));
    let p = parse(&src, FileId(0));
    assert!(p.diagnostics.is_empty());
    on_stack(32, move || drop(p));
}

/// K3 (MEDIUM): the formatter recurses ~300 bytes/level on chains (`a.b().c()...` 900 long,
/// 271 KB in wasm release, 83 KB native release) and spawns a 256 MB thread only when one is
/// available; inline otherwise.
#[cfg(all(unix, not(target_os = "wasi")))]
#[test]
fn k3_format_of_900_member_chain_on_32kb_stack() {
    let src = format!("let x = a{};", ".b".repeat(900));
    on_stack(32, move || {
        // note: the formatter itself spawns a 256 MB thread natively; run under
        // `ulimit -v 110000` (thread creation fails -> inline) to observe the abort.
        let _ = format(&src);
    });
}

/// K4: no-thread fallback. Re-executes this test binary under `ulimit -v 110000`, so the
/// 128 MB parser thread (and the 256 MB formatter thread) cannot be created, exactly as on
/// wasm32 or an RTOS, and parses 100 nested parentheses on a 256 KB stack (release ~303 KB
/// needed, debug ~1.8 MB). The child aborts, the parent reports it.
#[cfg(all(unix, not(target_os = "wasi")))]
#[test]
fn k4_no_thread_fallback_100_nested_parens_on_256kb_stack() {
    // CORRECTED TEST (see HANDOFF-8p-fixes.md): the no-thread fallback must never abort. The
    // default `max_depth` is chosen so that the inline parser fits 256 KB of RELEASE stack:
    // nesting up to the limit parses and deeper input is rejected with OXS-E0011. The original
    // nested 100 parentheses, i.e. asserted the limit is >= 100; the lead's decision is a
    // conservative default. Debug frames are ~6x larger, so the child only runs in release.
    if cfg!(debug_assertions) {
        return;
    }
    if std::env::var("BA_STACK_CHILD").is_ok() {
        on_stack(256, || {
            let p = parse(&nest_paren(DEFAULT_MAX_DEPTH - 3), FileId(0));
            assert!(p.diagnostics.is_empty(), "{:?}", p.diagnostics.first());
            let q = parse(&nest_paren(DEFAULT_MAX_DEPTH + 40), FileId(0));
            assert!(q.diagnostics.iter().any(|d| d.code == "OXS-E0011"));
            let r = parse(&nest_paren(5000), FileId(0));
            assert!(r.diagnostics.iter().any(|d| d.code == "OXS-E0011"));
            std::mem::forget((p, q, r));
        });
        return;
    }
    let exe = std::env::current_exe().unwrap();
    let st = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ulimit -v 110000; BA_STACK_CHILD=1 exec '{}' --exact k4_no_thread_fallback_100_nested_parens_on_256kb_stack --test-threads=1",
            exe.display()
        ))
        .output()
        .unwrap();
    assert!(
        st.status.success(),
        "child died ({:?}): inline fallback within the default depth needs more than 256 KB; stderr: {}",
        st.status,
        String::from_utf8_lossy(&st.stderr).lines().take(3).collect::<Vec<_>>().join(" | ")
    );
}
