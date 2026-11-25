#![allow(clippy::format_push_string)]
//! The deepest inputs the parser accepts must neither overflow a 1 MB stack (the wasm32
//! default) nor panic, and pathological sizes must stay fast.
mod common;
use common::*;
use oxiscript_check::*;
use std::time::{Duration, Instant};

const PRE: &str = "struct B { n: i32; f(): B { return this; } g(x: B): B { return x; } }\n";

/// Check on a thread with a 1 MB stack. Returns (ok, error codes, time).
fn on_small_stack(src: String) -> (bool, Vec<String>, Duration) {
    let kb: usize = std::env::var("OXI_STACK_KB").ok().and_then(|v| v.parse().ok()).unwrap_or(1024);
    std::thread::Builder::new()
        .stack_size(kb << 10)
        .spawn(move || {
            let t = Instant::now();
            let c = check_source(&src, &hosts());
            (c.ok(), c.errors().map(|d| d.code.to_string()).collect::<Vec<_>>(), t.elapsed())
        })
        .unwrap()
        .join()
        .expect("checker overflowed the stack or panicked")
}

/// The largest `n <= hi` for which `mk(n)` still parses without errors (the parser's own limits).
fn deepest(mk: &dyn Fn(usize) -> String, hi: usize) -> (usize, String) {
    let parses = |n: usize| {
        oxiscript_syntax::parse(&format!("{PRE}{}", mk(n)), oxiscript_syntax::FileId(0))
            .diagnostics
            .iter()
            .all(|d| !d.is_error())
    };
    assert!(parses(1), "generator does not produce valid programs");
    let (mut lo, mut h) = (1, hi);
    while lo < h {
        let mid = (lo + h).div_ceil(2);
        if parses(mid) {
            lo = mid;
        } else {
            h = mid - 1;
        }
    }
    (lo, format!("{PRE}{}", mk(lo)))
}

/// Check the deepest parseable instance on a 1 MB stack: accepted, or rejected only with `OXS-E0113`.
fn must_not_crash(label: &str, hi: usize, mk: &dyn Fn(usize) -> String) -> usize {
    must_not_crash_with(label, hi, mk, true)
}

/// `only_limit` false: any diagnostics are fine (the program may be ill-typed); it just must not crash.
fn must_not_crash_with(label: &str, hi: usize, mk: &dyn Fn(usize) -> String, only_limit: bool) -> usize {
    let (n, src) = deepest(mk, hi);
    let (ok, errs, took) = on_small_stack(src);
    assert!(!only_limit || ok || errs.iter().all(|c| c == "OXS-E0113"), "{label} (n={n}): {errs:?}");
    assert!(took < Duration::from_secs(5), "{label} took {took:?}");
    if std::env::var("OXI_SHOW_LIMITS").is_ok() {
        println!("{label}: deepest parseable n={n}, checker ok={ok}");
    }
    n
}

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

#[test]
fn long_chains_do_not_overflow() {
    let n = must_not_crash("method chain", 2000, &|n| format!("function f(b: B): B {{ return b{}; }}", rep(".f()", n)));
    assert!(n >= 300, "method chain parsed only to {n}");
    must_not_crash("additions", 5000, &|n| format!("function f(a: i32): i32 {{ return a{}; }}", rep(" + a", n)));
    must_not_crash("and chain", 5000, &|n| format!("function f(a: bool): bool {{ return a{}; }}", rep(" && a", n)));
    must_not_crash("or chain", 5000, &|n| format!("function f(a: bool): bool {{ return a{}; }}", rep(" || a", n)));
    must_not_crash("coalesce chain", 5000, &|n| format!("function f(a: i32 | null): i32 | null {{ return a{}; }}", rep(" ?? a", n)));
    must_not_crash("assignment chain", 2000, &|n| format!("function f(): void {{ let a = 1; a{}; }}", rep(" = a", n)));
    must_not_crash("unary chain", 2000, &|n| format!("function f(a: i32): i32 {{ return {}a; }}", rep("- ", n)));
    must_not_crash("not chain", 2000, &|n| format!("function f(a: bool): bool {{ return {}a; }}", rep("!", n)));
    must_not_crash("ternary chain", 2000, &|n| format!("function f(a: bool): i32 {{ return {}0; }}", rep("a ? 1 : ", n)));
    must_not_crash("else-if ladder", 3000, &|n| {
        let mut s = String::from("function f(a: i32): i32 {\n  if (a === 0) { return 0; }");
        for i in 1..n {
            s.push_str(&format!(" else if (a === {i}) {{ return {i}; }}"));
        }
        s.push_str(" else { return -1; }\n}\n");
        s
    });
    must_not_crash("many statements", 20_000, &|n| {
        let mut s = String::from("function f(a: i32): i32 {\n  let r = 0;\n");
        for i in 0..n {
            s.push_str(&format!("  r += {i};\n"));
        }
        s.push_str("  return r;\n}\n");
        s
    });
}

#[test]
fn deep_nesting_does_not_overflow() {
    must_not_crash("parens", 200, &|n| format!("function f(a: i32): i32 {{ return {}a{}; }}", rep("(", n), rep(")", n)));
    must_not_crash("calls", 200, &|n| {
        format!("function g(a: i32): i32 {{ return a; }}\nfunction f(a: i32): i32 {{ return {}a{}; }}", rep("g(", n), rep(")", n))
    });
    must_not_crash("arrays", 200, &|n| format!("function f(): void {{ const a = {}1{}; }}", rep("[", n), rep("]", n)));
    must_not_crash("arrows", 200, &|n| format!("function f(): void {{ const g = {}0; }}", rep("() => ", n)));
    must_not_crash("objects", 200, &|n| {
        format!("struct N {{ c: N | null; }}\nfunction f(): N {{ return {}null{}; }}", rep("{ c: ", n), rep(" }", n))
    });
    must_not_crash("switch expressions", 200, &|n| {
        format!("function f(a: i32): i32 {{ return {}0{}; }}", rep("switch (a) { default: ", n), rep(" }", n))
    });
    must_not_crash("blocks", 200, &|n| format!("function f(): void {{ {}{} }}", rep("{ ", n), rep("}", n)));
    must_not_crash("ifs", 200, &|n| format!("function f(a: bool): void {{ {}{} }}", rep("if (a) { ", n), rep("}", n)));
    must_not_crash("closures", 200, &|n| format!("function f(): void {{ {}{} }}", rep("const g = () => { ", n), rep("};", n)));
    must_not_crash("loops", 200, &|n| format!("function f(a: bool): void {{ {}{} }}", rep("while (a) { ", n), rep("}", n)));
    must_not_crash("generic types", 200, &|n| format!("function f(a: {}i32{}): void {{}}", rep("Array<", n), rep(">", n)));
    must_not_crash("function types", 200, &|n| format!("function f(a: {}i32{}): void {{}}", rep("() => (", n), rep(")", n)));
}

#[test]
fn big_enum_and_switch_stay_fast() {
    let n = 20_000;
    let mut s = String::from("enum E {\n");
    for i in 0..n {
        s.push_str(&format!("  M{i},\n"));
    }
    s.push_str("}\nfunction f(e: E): i32 {\n  switch (e) {\n");
    for i in 0..n {
        s.push_str(&format!("    case E.M{i}: return {i};\n"));
    }
    s.push_str("  }\n}\n");
    let (ok, errs, took) = on_small_stack(s);
    assert!(ok, "{errs:?}");
    assert!(took < Duration::from_secs(if cfg!(debug_assertions) { 40 } else { 10 }), "{took:?}");
}

#[test]
fn many_uninitialised_locals_and_branches_stay_fast() {
    let n = if cfg!(debug_assertions) { 1500 } else { 4000 };
    let mut s = String::from("function f(c: bool): i32 {\n");
    for i in 0..n {
        s.push_str(&format!("  let v{i}: i32;\n"));
    }
    for i in 0..n {
        s.push_str(&format!("  if (c) {{ v{i} = {i}; }} else {{ v{i} = 0; }}\n"));
    }
    s.push_str("  return v0;\n}\n");
    let (ok, errs, took) = on_small_stack(s);
    assert!(ok, "{errs:?}");
    assert!(took < Duration::from_secs(if cfg!(debug_assertions) { 30 } else { 5 }), "{took:?}");
}

#[test]
fn big_union_is_bounded() {
    let mut s = String::from("type U =\n");
    for i in 0..100 {
        s.push_str(&format!("  | {{ kind: {i} }}\n"));
    }
    s.push_str(";\n");
    let (ok, errs, _) = on_small_stack(s);
    assert!(!ok && errs.contains(&"OXS-E0113".to_string()), "{errs:?}");
}

#[test]
fn deep_generics_are_bounded() {
    // polymorphic recursion: f<T> calls f<Box<T>>; the checker checks `f` once
    let src = "struct Box<T> { v: T; }\nfunction f<T>(x: T, n: i32): i32 {\n  if (n === 0) { return 0; }\n  const b: Box<T> = { v: x };\n  return f<Box<T>>(b, n - 1);\n}\n";
    let (ok, errs, _) = on_small_stack(src.to_string());
    assert!(ok, "{errs:?}");
    // a type nested beyond the limit is an error, not a blowup
    let mut t = String::from("i32");
    for _ in 0..40 {
        t = format!("Box<{t}>");
    }
    let src = format!("struct Box<T> {{ v: T; }}\nfunction f(a: {t}): void {{}}\n");
    let (ok, errs, _) = on_small_stack(src);
    assert!(!ok && errs.iter().any(|c| c == "OXS-E0113"), "{errs:?}");
}

#[test]
fn alias_cycles_terminate() {
    for src in [
        "type A = B; type B = A;",
        "type A = Array<A>;",
        "type A<T> = A<A<T>>;",
        "struct S { a: A; } type A = S | null;",
    ] {
        let (_, _, took) = on_small_stack(src.to_string());
        assert!(took < Duration::from_secs(2), "{src}");
    }
}

/// Heaviest-frame mixes at the parser's depth limit: every level picks a different construct.
#[test]
fn mixed_nesting_does_not_overflow() {
    const EX: [(&str, &str); 6] = [
        ("(", ")"),
        ("g(", ")"),
        ("() => ", ""),
        ("switch (a) { default: ", " }"),
        ("{ c: ", " }"),
        ("[", "]"),
    ];
    for seed in 0..6usize {
        must_not_crash_with("mixed expressions", 200, &|n| {
            let (mut open, mut close) = (String::new(), String::new());
            for i in 0..n {
                let (o, c) = EX[(i * (seed + 1) + seed) % EX.len()];
                open.push_str(o);
                close.insert_str(0, c);
            }
            format!("struct N {{ c: N | null; }}\nfunction g(a: i32): i32 {{ return a; }}\nfunction f(a: i32): void {{ const x = {open}a{close}; }}")
        }, false);
    }
    for seed in 0..4usize {
        must_not_crash_with("mixed statements", 200, &|n| {
            const ST: [(&str, &str); 4] = [("if (a > 0) { ", " }"), ("while (a > 0) { ", " }"), ("const h = () => { ", " };"), ("{ ", " }")];
            let (mut open, mut close) = (String::new(), String::new());
            for i in 0..n {
                let (o, c) = ST[(i * (seed + 1) + seed) % ST.len()];
                open.push_str(o);
                close.insert_str(0, c);
            }
            format!("function f(a: i32): void {{ {open}{close} }}")
        }, false);
    }
}

/// Chains made of many flat declarations: shallow for the parser, deep for the checker.
fn flat_chain(label: &str, src: String, max: Duration) {
    eprintln!("chain: {label}");
    let (ok, errs, took) = on_small_stack(src);
    assert!(ok || errs.iter().all(|c| c == "OXS-E0113"), "{label}: {errs:?}");
    eprintln!("chain: {label} {took:?} ok={ok} {:?}", errs.first());
    assert!(took < max, "{label} took {took:?}");
}

/// Like `flat_chain` but any diagnostics are fine: the chain must only not crash or hang.
fn lenient_chain(label: &str, src: String, max: Duration) {
    let (_, errs, took) = on_small_stack(src);
    eprintln!("chain: {label} {took:?} {:?}", errs.first());
    assert!(took < max, "{label} took {took:?}");
}

fn flat_max() -> Duration {
    Duration::from_secs(if cfg!(debug_assertions) { 20 } else { 2 })
}

#[test]
fn declaration_chains_do_not_overflow() {
    let n = 5000;
    // alias chain (forward and backward references)
    let mut s = String::from("type A0 = i32;\n");
    for i in 1..n {
        s.push_str(&format!("type A{i} = A{};\n", i - 1));
    }
    s.push_str(&format!("function f(a: A{}): i32 {{ return a; }}\n", n - 1));
    flat_chain("alias chain", s, flat_max());
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("type A{i} = A{};\n", i - 1));
    }
    s.push_str("type A0 = i32;\n");
    flat_chain("reversed alias chain", s, flat_max());
    // generic alias chain
    let mut s = String::from("type B0<T> = Array<T>;\n");
    for i in 1..n {
        s.push_str(&format!("type B{i}<T> = B{}<T>;\n", i - 1));
    }
    flat_chain("generic alias chain", s, flat_max());
    // interface extends chain
    let mut s = String::from("interface I0 { m0(): i32; }\n");
    for i in 1..n.min(1500) {
        s.push_str(&format!("interface I{i} extends I{} {{ m{i}(): i32; }}\n", i - 1));
    }
    flat_chain("interface chain", s, flat_max());
    // struct field chain
    let mut s = String::from("struct S0 { v: i32; }\n");
    for i in 1..n {
        s.push_str(&format!("struct S{i} {{ c: S{} | null; }}\n", i - 1));
    }
    s.push_str(&format!("function f(a: S{}): void {{ const b = a.c; }}\n", n - 1));
    flat_chain("struct chain", s, flat_max());
    // enum initialisers referring to the previous member
    let mut s = String::from("enum E {\n  M0 = 1,\n");
    for i in 1..n {
        s.push_str(&format!("  M{i} = M{} + 1,\n", i - 1));
    }
    s.push_str("}\n");
    flat_chain("enum initialiser chain", s, flat_max());
    // module consts referring to the previous one
    let mut s = String::from("const C0: i32 = 1;\n");
    for i in 1..n {
        s.push_str(&format!("const C{i}: i32 = C{} + 1;\n", i - 1));
    }
    flat_chain("const chain", s, flat_max());
    let mut s = String::from("function f0(): i32 { return 1; }\n");
    for i in 1..n {
        s.push_str(&format!("function f{i}(): i32 {{ return f{}() + 1; }}\n", i - 1));
    }
    flat_chain("call chain without annotations", s, flat_max());
    // functions with inferred return types calling the next one (recursion through inference)
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("function g{i}() {{ return g{}(); }}\n", i + 1));
    }
    s.push_str(&format!("function g{n}() {{ return 1; }}\n"));
    flat_chain("inferred return chain", s, flat_max());
}

#[test]
fn reversed_declaration_chains_do_not_overflow() {
    let n = 3000;
    let mut s = String::new();
    for i in (1..n.min(1500)).rev() {
        s.push_str(&format!("interface I{i} extends I{} {{ m{i}(): i32; }}\n", i - 1));
    }
    s.push_str("interface I0 { m0(): i32; }\n");
    flat_chain("reversed interface chain", s, flat_max());
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("struct S{i} {{ c: S{} | null; }}\n", i - 1));
    }
    s.push_str("struct S0 { v: i32; }\n");
    flat_chain("reversed struct chain", s, flat_max());
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("type U{i} = {{ kind: \"a\"; v: U{} }} | {{ kind: \"b\" }};\n", i - 1));
    }
    s.push_str("type U0 = { kind: \"z\" };\n");
    flat_chain("reversed union chain", s, flat_max());
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("const C{i}: i32 = C{} + 1;\n", i - 1));
    }
    s.push_str("const C0: i32 = 1;\n");
    flat_chain("reversed const chain", s, flat_max());
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("function f{i}(): i32 {{ return f{}() + 1; }}\n", i - 1));
    }
    s.push_str("function f0(): i32 { return 1; }\n");
    flat_chain("reversed call chain", s, flat_max());
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("function g{i}() {{ return g{}(); }}\n", i + 1));
    }
    s.push_str(&format!("function g{n}() {{ return 1; }}\n"));
    flat_chain("inferred return chain", s, flat_max());
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("struct G{i}<T> {{ c: G{}<Array<T>> | null; }}\n", i - 1));
    }
    s.push_str("struct G0<T> { v: T; }\n");
    flat_chain("reversed generic struct chain", s, flat_max());
}

#[test]
fn bound_and_generic_chains_do_not_overflow() {
    let n = 3000;
    let mut s = String::from("interface I { m(): i32; }\nstruct B<T extends I> { v: T; }\n");
    for i in 0..n {
        s.push_str(&format!("function f{i}<T extends I>(x: T): i32 {{ return x.m(){}; }}\n", if i > 0 { format!(" + f{}(x)", i - 1) } else { String::new() }));
    }
    flat_chain("generic bound chain", s, flat_max());
    // generics instantiating generics of the previous level
    let mut s = String::from("struct W0<T> { v: T; }\n");
    for i in 1..300 {
        s.push_str(&format!("struct W{i}<T> {{ v: W{}<T>; }}\n", i - 1));
    }
    s.push_str("function f(a: W299<i32>): void {}\n");
    flat_chain("generic struct chain", s, flat_max());
}

#[test]
fn import_chains_do_not_overflow() {
    use oxiscript_check::{check_program, MapLoader};
    // 200 modules import in a chain and type-check; 2000 hit the module limit with a diagnostic
    for (n, must_pass) in [(200usize, true), (2000, false)] {
        let mut l = MapLoader::default();
        for i in 0..n {
            let src = if i + 1 < n {
                format!("import {{ v{} }} from \"./m{}\";\nexport const v{i}: i32 = v{} + 1;\n", i + 1, i + 1, i + 1)
            } else {
                format!("export const v{i}: i32 = 1;\n")
            };
            l.0.insert(format!("m{i}"), src);
        }
        let entry = "import { v1 } from \"./m1\";\nexport function f(): i32 { return v1; }\n".to_string();
        let h = hosts();
        let t = Instant::now();
        let c = std::thread::Builder::new()
            .stack_size(1 << 20)
            .spawn(move || {
                let c = check_program("main", &entry, &mut l, &h);
                (c.ok(), c.errors().map(|d| format!("{} {}", d.code, d.message)).collect::<Vec<_>>())
            })
            .unwrap()
            .join()
            .expect("import chain overflowed the stack or panicked");
        assert!(!must_pass || c.0, "{:?}", c.1);
        assert!(c.0 || c.1.iter().any(|x| x.contains("too many modules")), "{:?}", c.1);
        assert!(t.elapsed() < flat_max(), "{:?}", t.elapsed());
    }
}

/// Nested generic calls with context-sensitive lambdas and unions must not double the work per level.
#[test]
fn nested_inference_is_not_exponential() {
    let cap = if cfg!(debug_assertions) { Duration::from_secs(10) } else { Duration::from_secs(1) };
    let pre = "function ap<T>(f: (x: T) => T, v: T): T { return f(v); }\nfunction id<T>(x: T): T { return x; }\nstruct P { x: i32; c: P | null; }\ntype U = { kind: \"a\"; v: U | null } | { kind: \"b\" };\n";
    let (n, src) = deepest_with(pre, &|n| format!("function f(): i32 {{ return {}0{}; }}", rep("ap((a) => ap((b) => a + b, a), ", n), rep(")", n)), 200);
    let (_, errs, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    eprintln!("ap n={n} {took:?} {errs:?}");
    assert!(took < cap, "nested ap n={n} took {took:?} {errs:?}");
    let (n, src) = deepest_with(pre, &|n| format!("function f(): i32 {{ return {}0{}; }}", rep("id(", n), rep(")", n)), 200);
    let (_, _, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    assert!(took < cap, "nested id n={n} took {took:?}");
    let (n, src) = deepest_with(pre, &|n| format!("function f(): P {{ return {}null{}; }}", rep("{ x: 1, c: ", n), rep(" }", n)), 200);
    let (ok, errs, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    assert!(ok, "{errs:?}");
    assert!(took < cap, "nested object literals n={n} took {took:?}");
    let (n, src) = deepest_with(pre, &|n| format!("function f(): U {{ return {}{{ kind: \"b\" }}{}; }}", rep("{ kind: \"a\", v: ", n), rep(" }", n)), 200);
    let (ok, errs, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    assert!(ok, "{errs:?}");
    assert!(took < cap, "nested union literals n={n} took {took:?}");
    let (n, src) = deepest_with(pre, &|n| format!("function f(a: i32): i32 {{ return {}0{}; }}", rep("ap((x) => ", n), rep(", 1)", n)), 200);
    let (_, _, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    assert!(took < cap, "lambda chain n={n} took {took:?}");
    let (n, src) = deepest_with(pre, &|n| format!("function f(a: i32): i32 {{ return {}a{}; }}", rep("(a > 0 ? ", n), rep(" : a)", n)), 200);
    let (_, _, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    assert!(took < cap, "ternary nest n={n} took {took:?}");
    let (n, src) = deepest_with(pre, &|n| format!("function f(a: i32): Array<i32> {{ return {}a{}; }}", rep("id([a, ", n), rep("])", n)), 200);
    let (_, _, took) = on_small_stack(src);
    assert!(n >= 20, "generator parses only to {n}");
    assert!(took < cap, "spread nest n={n} took {took:?}");
}

fn deepest_with(pre: &str, mk: &dyn Fn(usize) -> String, hi: usize) -> (usize, String) {
    let parses = |n: usize| {
        oxiscript_syntax::parse(&format!("{pre}{}", mk(n)), oxiscript_syntax::FileId(0)).diagnostics.iter().all(|d| !d.is_error())
    };
    if !parses(1) {
        return (0, format!("{pre}{}", mk(1)));
    }
    let (mut lo, mut h) = (1, hi);
    while lo < h {
        let mid = (lo + h).div_ceil(2);
        if parses(mid) {
            lo = mid;
        } else {
            h = mid - 1;
        }
    }
    (lo, format!("{pre}{}", mk(lo)))
}

/// Parser-maximal nesting inside every level of a declaration chain: the budgets must be shared.
#[test]
fn nested_budgets_do_not_overflow() {
    // the deepest paren / Array<> nesting the parser accepts in each position
    let (pn, _) = deepest_with("", &|n| format!("function f(a: i32): i32 {{ return {}a{}; }}", rep("(", n), rep(")", n)), 200);
    let (an, _) = deepest_with("", &|n| format!("function f(a: {}i32{}): void {{}}", rep("Array<", n), rep(">", n)), 200);
    assert!(pn >= 40 && an >= 40, "{pn} {an}");
    let n = 150;
    let wrap_ty = |inner: &str| format!("{}{inner}{}", rep("Array<", an - 1), rep(">", an - 1));
    let wrap_ex = |inner: &str| format!("{}{inner}{}", rep("(", pn - 1), rep(")", pn - 1));
    let max = Duration::from_secs(if cfg!(debug_assertions) { 30 } else { 3 });
    // reversed alias chain, each body wrapped in Array<...>
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("type A{i} = {};\n", wrap_ty(&format!("A{}", i - 1))));
    }
    s.push_str("type A0 = i32;\n");
    flat_chain("nested reversed alias chain", s, max);
    // inferred-return chain with maximal parens around the call
    let mut s = String::new();
    for i in 0..n {
        s.push_str(&format!("function g{i}() {{ return {}; }}\n", wrap_ex(&format!("g{}()", i + 1))));
    }
    s.push_str(&format!("function g{n}() {{ return 1; }}\n"));
    flat_chain("nested inferred return chain", s, max);
    // reversed const chain
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("const C{i}: i32 = {};\n", wrap_ex(&format!("C{} + 1", i - 1))));
    }
    s.push_str("const C0: i32 = 1;\n");
    flat_chain("nested reversed const chain", s, max);
    // reversed extends chain with deep type arguments
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("interface I{i}<T> extends I{}<{}> {{ m{i}(): i32; }}\n", i - 1, wrap_ty("T")));
    }
    s.push_str("interface I0<T> { m0(): i32; }\n");
    flat_chain("nested reversed extends chain", s, max);
    // reversed struct chain with deep field types, and a generic function chain with deep arguments
    let mut s = String::new();
    for i in (1..n).rev() {
        s.push_str(&format!("struct S{i} {{ c: {}S{}{}; }}\n", rep("Array<", an - 8), i - 1, rep(">", an - 8)));
    }
    s.push_str("struct S0 { v: i32; }\n");
    flat_chain("nested reversed struct chain", s, max);
    let mut s = String::from("function id<T>(x: T): T { return x; }\n");
    for i in (0..n).rev() {
        s.push_str(&format!("function h{i}(): i32 {{ return {}; }}\n", wrap_ex(&format!("id(h{}())", i + 1))));
    }
    s.push_str(&format!("function h{n}(): i32 {{ return 1; }}\n"));
    flat_chain("nested call chain", s, max);
}

/// A definition abandoned by the recursion guard must still be usable without panics.
#[test]
fn guarded_definitions_can_still_be_used() {
    let n = 200;
    let max = Duration::from_secs(30);
    // chains whose head is the thing the program then uses
    let mut s = String::new();
    s.push_str("function useAll(): void {\n  const a: A1 = 1;\n  const g = G1 + 1;\n  const r = f1();\n  const s: S1 = { c: null };\n}\n");
    for i in (1..n).rev() {
        s.push_str(&format!("type A{i} = A{};\nconst G{i}: i32 = G{} + 1;\nfunction f{i}() {{ return f{}(); }}\nstruct S{i} {{ c: S{} | null; }}\n", i - 1, i - 1, i - 1, i - 1));
    }
    s.push_str("type A0 = i32;\nconst G0: i32 = 1;\nfunction f0() { return 1; }\nstruct S0 { v: i32; }\n");
    lenient_chain("chains used after the guard", s, max);
    let mut s = String::from("struct Item implements I200 { }\nfunction use<T extends I200>(x: T): i32 { return x.m200(); }\nfunction go(i: Item): i32 { return use(i); }\n");
    for i in (1..=n).rev() {
        s.push_str(&format!("interface I{i} extends I{} {{ m{i}(): i32; }}\n", i - 1));
    }
    s.push_str("interface I0 { m0(): i32; }\n");
    lenient_chain("interface chain used as bound", s, max);
    let mut s = String::from("function go(): i32 { return E200.M0; }\n");
    for i in (1..=n).rev() {
        s.push_str(&format!("enum E{i} {{ M0 = C{} }}\nconst C{i}: i32 = E{}.M0;\n", i - 1, i - 1));
    }
    s.push_str("enum E0 { M0 = 1 }\nconst C0: i32 = 1;\n");
    lenient_chain("enum/const mutual chain", s, max);
    let mut s = String::from("function go(u: U1): i32 { return switch (u.kind) { case \"a\": 1, default: 0 }; }\n");
    for i in (1..n).rev() {
        s.push_str(&format!("type U{i} = {{ kind: \"a\"; v: U{} }} | {{ kind: \"b\" }};\n", i - 1));
    }
    s.push_str("type U0 = { kind: \"z\" };\n");
    lenient_chain("union chain used in a switch", s, max);
}
