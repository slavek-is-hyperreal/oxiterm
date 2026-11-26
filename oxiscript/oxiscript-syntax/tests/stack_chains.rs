#![allow(clippy::pedantic)]
//! Chains of every shape the parser accepts must parse, drop and format on small
//! stacks (no helper thread): the heights are bounded, the recursion is not.
use oxiscript_syntax::*;

fn no_thread() -> ParseOptions {
    ParseOptions {
        use_thread: false,
        ..ParseOptions::default()
    }
}

/// Run `f` on a thread with `kb` KB of stack; `None` if the platform has no threads.
fn on_stack<R: Send + 'static>(kb: usize, f: impl FnOnce() -> R + Send + 'static) -> Option<R> {
    std::thread::Builder::new()
        // debug frames are about 16 times larger than release frames
        .stack_size(kb * 1024 * if cfg!(debug_assertions) { 16 } else { 1 })
        .spawn(f)
        .ok()
        .map(|h| h.join().expect("the thread must not panic"))
}

/// Largest `n` in `1..=hi` for which `make(n)` parses cleanly (shapes grow monotonically).
fn largest_accepted(hi: usize, make: impl Fn(usize) -> String) -> usize {
    let (mut lo, mut hi) = (1usize, hi);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if parse_with(&make(mid), FileId(0), &no_thread())
            .diagnostics
            .is_empty()
        {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

fn check_shape(name: &str, hi: usize, make: impl Fn(usize) -> String + Send + Copy + 'static) {
    let n = largest_accepted(hi, make);
    assert!(n > 20, "{name}: only {n} links accepted");
    let src = make(n);
    // parse + drop on 32 KB
    let s1 = src.clone();
    let r = on_stack(32, move || {
        let p = parse_with(&s1, FileId(0), &no_thread());
        assert_eq!(p.diagnostics.len(), 0);
        drop(p);
    });
    // format (parse, build, print, drop) on 256 KB without a helper thread
    let s2 = src.clone();
    let nm = name.to_string();
    let r2 = on_stack(256, move || {
        let o = FormatOptions {
            parse: no_thread(),
            ..FormatOptions::default()
        };
        let out = format_with(&s2, &o).unwrap();
        assert_eq!(parse_module(&out).1.len(), 0, "{nm}: output must parse");
    });
    eprintln!(
        "{name}: {n} links ok (threads available: {})",
        r.is_some() && r2.is_some()
    );
}

#[test]
fn type_suffix_chain() {
    check_shape("T[][]...", 1000, |n| {
        format!("let a: i32{};", "[]".repeat(n))
    });
    // beyond the nesting limit it is an error, not an abort
    let p = parse(&format!("let a: i32{};", "[]".repeat(5000)), FileId(0));
    assert!(p.diagnostics.iter().any(|d| d.code == "OXS-E0011"));
}

#[test]
fn binary_and_cast_alternation() {
    check_shape("a + b as T + ...", 1000, |n| {
        format!("let x = a{};", " + b as T".repeat(n))
    });
    check_shape("a < b as T < ...", 1000, |n| {
        format!(
            "let x = a{};",
            " < (b as T)".repeat(n).replace(" < (b as T)", " + b as T")
        )
    });
    check_shape("a + b * c + ...", 1000, |n| {
        format!("let x = a{};", " + b * c".repeat(n))
    });
}

#[test]
fn postfix_mixed_chain() {
    check_shape("a.b(c)[d]?.e as T + ...", 1000, |n| {
        format!("let x = a{};", ".b(c)[d]?.e".repeat(n))
    });
}

#[test]
fn ternary_and_else_if_ladders() {
    check_shape("a ? b : c ? ...", 1000, |n| {
        format!("let x = {} z;", "a ? b :".repeat(n))
    });
    check_shape("else if ladder", 1000, |n| {
        format!(
            "function f() {{ if (a) {{}} {} }}",
            "else if (b) {}".repeat(n)
        )
    });
}

#[test]
fn default_depth_input_formats_without_a_thread() {
    // nesting up to the default limit parses AND formats on 256 KB without a helper
    // thread (release; debug frames are about six times larger)
    if cfg!(debug_assertions) {
        return;
    }
    let n = DEFAULT_MAX_DEPTH - 3;
    let kinds: Vec<(&str, String)> = vec![
        (
            "paren",
            format!("let x = {}1{};", "(".repeat(n), ")".repeat(n)),
        ),
        (
            "call",
            format!("let x = {}1{};", "f(".repeat(n), ")".repeat(n)),
        ),
        (
            "array",
            format!("let x = {}1{};", "[".repeat(n), "]".repeat(n)),
        ),
        ("arrow", format!("let x = {}1;", "(a) => ".repeat(n))),
        (
            "switch",
            format!(
                "let x = {}1{};",
                "switch (a) { case 1: ".repeat(n),
                " }".repeat(n)
            ),
        ),
        (
            "type",
            format!("let x: {}i32{};", "Array<".repeat(n), ">".repeat(n)),
        ),
        (
            "block",
            format!(
                "function f(): void {}{}",
                "{".repeat(n / 2),
                "}".repeat(n / 2)
            ),
        ),
    ];
    for (name, src) in kinds {
        let o = FormatOptions {
            parse: no_thread(),
            ..FormatOptions::default()
        };
        let r = on_stack(256, move || format_with(&src, &o).map(|s| s.len()));
        if let Some(r) = r {
            assert!(
                r.is_ok(),
                "{name}: {:?}",
                r.err().map(|e| e.diagnostics.first().cloned())
            );
        }
    }
}
