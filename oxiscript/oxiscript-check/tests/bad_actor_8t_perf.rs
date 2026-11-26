#![allow(clippy::too_many_lines, clippy::format_collect)]
#![allow(clippy::format_push_string)]
//! BAD ACTOR 8.T: complexity probes. Each generator is timed at n and 4n on a 1 MB stack; the
//! growth exponent is printed (run with --nocapture) and super-quadratic growth fails.
mod common;
use common::*;
use std::time::Instant;

type Gen = fn(usize) -> String;

fn t(src: &str) -> f64 {
    let s = src.to_string();
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let h = hosts();
            let t0 = Instant::now();
            let c = oxiscript_check::check_source(&s, &h);
            let el = t0.elapsed().as_secs_f64();
            std::hint::black_box(&c);
            el
        })
        .unwrap()
        .join()
        .expect("panic/stack overflow in perf probe")
}

fn gens() -> Vec<(&'static str, Gen, usize)> {
    vec![
        ("else_if_nullable_params", |n| {
            let params: Vec<String> = (0..n).map(|i| format!("a{i}: i32 | null")).collect();
            let mut b = String::new();
            for i in 0..n {
                b.push_str(&format!("{}if (a{i} !== null) {{ return a{i}; }}", if i == 0 { "" } else { " else " }));
            }
            format!("function f({}): i32 {{ {} return 0; }}", params.join(", "), b)
        }, 400),
        ("sequential_ifs_many_facts", |n| {
            let params: Vec<String> = (0..n).map(|i| format!("a{i}: i32 | null")).collect();
            let mut b = String::new();
            for i in 0..n {
                b.push_str(&format!("if (a{i} === null) {{ return 0; }}\n"));
            }
            b.push_str("return a0 + 1;");
            format!("function f({}): i32 {{ {} }}", params.join(", "), b)
        }, 400),
        ("joins_many_facts_loop", |n| {
            let params: Vec<String> = (0..n).map(|i| format!("a{i}: i32 | null")).collect();
            let mut b = String::new();
            for i in 0..n {
                b.push_str(&format!("if (a{i} === null) {{ return 0; }}\n"));
            }
            for _ in 0..n {
                b.push_str("if (c) { x = x + 1; } else { x = x + 2; }\n");
            }
            format!("function f(c: bool, {}): i32 {{ let x = 0; {} return x; }}", params.join(", "), b)
        }, 300),
        ("wide_object_literal", |n| {
            let f: Vec<String> = (0..n).map(|i| format!("f{i}: i32;")).collect();
            let v: Vec<String> = (0..n).map(|i| format!("f{i}: {i}")).collect();
            format!("struct S {{ {} }}\nfunction f(): S {{ return {{ {} }}; }}", f.join(" "), v.join(", "))
        }, 2000),
        ("many_variants_switch", |n| {
            let n = n.min(60);
            let v: Vec<String> = (0..n).map(|i| format!("{{ kind: \"k{i}\"; v: i32 }}")).collect();
            let c: Vec<String> = (0..n).map(|i| format!("case \"k{i}\": return {i};")).collect();
            format!("type U = {};\nfunction f(u: U): i32 {{ switch (u.kind) {{ {} }} }}", v.join(" | "), c.join(" "))
        }, 60),
        ("big_enum_switch", |n| {
            let m: Vec<String> = (0..n).map(|i| format!("M{i}")).collect();
            let c: Vec<String> = (0..n).map(|i| format!("case E.M{i}: return {i};")).collect();
            format!("enum E: u16 {{ {} }}\nfunction f(e: E): i32 {{ switch (e) {{ {} }} }}", m.join(", "), c.join(" "))
        }, 3000),
        ("big_int_switch", |n| {
            let c: Vec<String> = (0..n).map(|i| format!("case {i}: return {i};")).collect();
            format!("function f(e: i32): i32 {{ switch (e) {{ {} default: return 0; }} }}", c.join(" "))
        }, 3000),
        ("coalesce_chain", |n| {
            let mut s = String::from("a0");
            for i in 1..n.min(100) {
                s = format!("({s} ?? a{i})");
            }
            let ps: Vec<String> = (0..n.min(100)).map(|i| format!("a{i}: i32 | null")).collect();
            format!("function f({}): i32 {{ return {s} ?? 0; }}", ps.join(", "))
        }, 100),
        ("add_chain", |n| {
            let s: Vec<String> = (0..n).map(|i| format!("{i}")).collect();
            format!("function f(): i32 {{ return {}; }}", s.join(" + "))
        }, 20000),
        ("many_fns_forward_refs", |n| {
            let mut s = String::new();
            for i in 0..n {
                s.push_str(&format!("function f{i}(): i32 {{ return f{}(); }}\n", i + 1));
            }
            s.push_str(&format!("function f{n}(): i32 {{ return 1; }}\n"));
            s
        }, 3000),
        ("many_structs_chain_fields", |n| {
            let mut s = String::new();
            for i in 0..n {
                s.push_str(&format!("struct S{i} {{ a: S{} | null; }}\n", i + 1));
            }
            s.push_str(&format!("struct S{n} {{ x: i32; }}\n"));
            s
        }, 3000),
        ("many_generics_nested", |n| {
            let mut t = "i32".to_string();
            for _ in 0..n.min(30) {
                t = format!("Box<{t}>");
            }
            format!("struct Box<T> {{ v: T; }}\nfunction id<T>(x: T): T {{ return x; }}\nfunction f(b: {t}): {t} {{ return id(b); }}")
        }, 30),
        ("many_closures_capture", |n| {
            let mut s = String::from("function f(): i32 { let x = 0;\n");
            for i in 0..n {
                s.push_str(&format!("const c{i} = (a: i32) => a + x + {i};\nx = x + c{i}(1);\n"));
            }
            s.push_str("return x; }");
            s
        }, 2000),
        ("nested_closures", |n| {
            let n = n.min(25);
            let mut s = String::from("function f(): i32 { let x = 1; return ");
            for _ in 0..n {
                s.push_str("((a: i32) => a + x)(");
            }
            s.push('1');
            for _ in 0..n {
                s.push(')');
            }
            s.push_str("; }");
            s
        }, 25),
        ("generic_calls_many_instances", |n| {
            let mut s = String::from("function id<T>(x: T): T { return x; }\nfunction f(): void {\n");
            for i in 0..n {
                s.push_str(&format!("const a{i} = id<{}>({});\n", if i % 3 == 0 { "i32" } else if i % 3 == 1 { "string" } else { "f64" }, if i % 3 == 0 { "1" } else if i % 3 == 1 { "\"a\"" } else { "1.5" }));
            }
            s.push('}');
            s
        }, 5000),
        ("many_locals_shadow", |n| {
            let mut s = String::from("function f(): i32 { let t = 0;\n");
            for i in 0..n {
                s.push_str(&format!("{{ const v{i} = {i}; t += v{i}; }}\n"));
            }
            s.push_str("return t; }");
            s
        }, 5000),
        ("many_methods_iface", |n| {
            let n = n.min(250);
            let m: Vec<String> = (0..n).map(|i| format!("m{i}(): i32;")).collect();
            let im: Vec<String> = (0..n).map(|i| format!("m{i}(): i32 {{ return {i}; }}")).collect();
            format!("interface I {{ {} }}\nstruct S {{ x: i32; {} }}\nfunction g<T extends I>(t: T): i32 {{ return t.m0(); }}\nfunction f(s: S): i32 {{ return g(s); }}", m.join(" "), im.join(" "))
        }, 250),
        ("long_string_template", |n| {
            let p: Vec<String> = (0..n).map(|i| format!("${{a}}x{i}")).collect();
            format!("function f(a: i32): string {{ return `{}`; }}", p.join(""))
        }, 5000),
        ("array_literal_big", |n| {
            let v: Vec<String> = (0..n).map(|i| format!("{i}")).collect();
            format!("function f(): i32[] {{ return [{}]; }}", v.join(", "))
        }, 20000),
        ("array_literal_nested_objs", |n| {
            let v: Vec<String> = (0..n).map(|i| format!("{{ x: {i}, y: {i} }}")).collect();
            format!("struct P {{ x: i32; y: i32; }}\nfunction f(): P[] {{ return [{}]; }}", v.join(", "))
        }, 5000),
        ("iface_extends_chain", |n| {
            let mut s = String::from("interface I0 { m0(): i32; }\n");
            for i in 1..n { s.push_str(&format!("interface I{i} extends I{} {{ m{i}(): i32; }}\n", i - 1)); }
            s
        }, 20000),
        ("many_params", |n| format!("function f({}): void {{}}", (0..n).map(|i| format!("a{i}: i32")).collect::<Vec<_>>().join(", ")), 20000),
        ("many_type_params", |n| format!("function f<{}>(): void {{}}", (0..n).map(|i| format!("T{i}")).collect::<Vec<_>>().join(", ")), 20000),
        ("enum_big_members", |n| format!("enum E: u32 {{ {} }}", (0..n).map(|i| format!("M{i}")).collect::<Vec<_>>().join(", ")), 40000),
        ("generic_decl_chain", |n| {
            let mut s = String::from("function f0<T>(a: T): T { return a; }\n");
            for i in 1..n { s.push_str(&format!("function f{i}<T>(a: T): T {{ return f{}<T[]>([a])[0]; }}\n", i - 1)); }
            s
        }, 20000),
        ("many_modules_imports", |n| (0..n).map(|i| format!("import {{ x{i} }} from \"./m{i}\";\n")).collect(), 20000),
        ("match_lambdas_infer", |n| {
            let mut s = String::from("function m<T, U>(a: T, f: (x: T) => U): U { return f(a); }\nfunction f(): i32 { return ");
            let n = n.min(60);
            for _ in 0..n {
                s.push_str("m(1, (x) => ");
            }
            s.push('1');
            for _ in 0..n {
                s.push(')');
            }
            s.push_str("; }");
            s
        }, 60),
    ]
}

#[test]
fn growth_is_not_superquadratic() {
    let mut bad = String::new();
    for (name, g, n) in gens() {
        let n1 = (n / 4).max(1);
        let a = g(n1);
        let b = g(n);
        // warm-up + best of 3
        let ta = (0..3).map(|_| t(&a)).fold(f64::MAX, f64::min).max(1e-4);
        let tb = (0..3).map(|_| t(&b)).fold(f64::MAX, f64::min);
        let ratio = tb / ta;
        let size = n as f64 / n1 as f64;
        let exp = ratio.ln() / size.ln();
        println!("{name:34} n={n1:>6}->{n:>6}  {:>8.2}ms -> {:>8.2}ms  exp {exp:.2}", ta * 1e3, tb * 1e3);
        if (exp > 2.4 && tb > 0.05) || (!cfg!(debug_assertions) && tb > 3.0) {
            bad.push_str(&format!("{name}: exponent {exp:.2}, {tb:.3}s\n"));
        }
    }
    assert_eq!(bad.len(), 0, "{bad}");
}
