#![allow(clippy::format_collect)]
#![allow(clippy::format_push_string)]
//! BAD ACTOR 8.T: hostile shapes on a 1 MB stack (debug and release), 
//! checked with a wall-clock bound and diagnostic span sanity.
mod common;
use common::*;
use std::time::{Duration, Instant};

fn run(name: &str, src: String) {
    let n = name.to_string();
    let r = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let h = hosts();
            let t = Instant::now();
            let c = oxiscript_check::check_source(&src, &h);
            let el = t.elapsed();
            for d in &c.diagnostics {
                assert!(d.span.lo <= d.span.hi && d.span.hi as usize <= src.len(), "{n}: span {:?} len {}", d.span, src.len());
            }
            if el > Duration::from_millis(300) { println!("SLOW {n} {} bytes: {el:?}", src.len()); }
            assert!(el < Duration::from_secs(20), "{n}: slow {el:?}");
            c.diagnostics.len()
        })
        .unwrap()
        .join();
    assert!(r.is_ok(), "{name}: panic or stack overflow");
}

fn rep(s: &str, n: usize) -> String {
    s.repeat(n)
}

#[test]
fn deep_shapes() {
    let n = [200usize, 2000, 20000];
    for &k in &n {
        run("else_if", format!("function f(a: i32): i32 {{ if (a === 0) {{ return 0; }}{} else {{ return 9; }} }}", (1..k).map(|i| format!(" else if (a === {i}) {{ return {i}; }}")).collect::<String>()));
        run("else_if_nullable_chain", format!("function f(a: i32 | null): i32 {{ if (a === null) {{ return 0; }}{} return 1; }}", rep(" else if (a !== null) { return 1; }", k)));
        run("not_chain", format!("function f(a: bool): bool {{ return {}a; }}", rep("!", k)));
        run("neg_chain", format!("function f(a: i32): i32 {{ return {}a; }}", rep("- ", k)));
        run("assign_chain", format!("function f(): void {{ let a = 0; let b = 0; {}a = 1; }}", rep("b = ", k)));
        run("ternary_chain", format!("function f(a: bool): i32 {{ return {}0; }}", rep("a ? 1 : ", k)));
        run("ternary_nest_true", format!("function f(a: bool): i32 {{ return {}0{}; }}", rep("a ? ", k), rep(" : 1", k)));
        run("as_chain", format!("function f(a: i32): i64 {{ return a{}; }}", rep(" as i64 as i32", k)));
        run("try_chain", format!("function f(a: Result<i32,string>): Result<i32,string> {{ const x = a{}; return Ok(1); }}", rep("?", k)));
        run("block_nest", format!("function f(): void {{ {}{} }}", rep("{ ", k), rep("} ", k)));
        run("array_nest", format!("function f(): void {{ const a = {}1{}; }}", rep("[", k), rep("]", k)));
        run("paren_nest", format!("function f(): i32 {{ return {}1{}; }}", rep("(", k), rep(")", k)));
        run("call_chain", format!("function g(a: i32): i32 {{ return a; }}\nfunction f(): i32 {{ return {}1{}; }}", rep("g(", k), rep(")", k)));
        run("member_chain", format!("struct S {{ s: S | null; }}\nfunction f(a: S): void {{ const b = a{}; }}", rep(".s?.s", k)));
        run("method_chain", format!("struct S {{ m(): S {{ return this; }} }}\nfunction f(a: S): S {{ return a{}; }}", rep(".m()", k)));
        run("index_chain", format!("function f(a: i32[][]): void {{ const b = a{}; }}", rep("[0]", k)));
        run("type_array_nest", format!("function f(a: i32{}): void {{}}", rep("[]", k)));
        run("type_generic_nest", format!("struct B<T> {{ v: T; }}\nfunction f(a: {}i32{}): void {{}}", rep("B<", k), rep(">", k)));
        run("type_fn_nest", format!("function f(a: {}i32): void {{}}", rep("() => ", k)));
        run("type_union_wide", format!("type U = {};", (0..k.min(400)).map(|i| format!("{{ kind: {i}; v: i32 }}")).collect::<Vec<_>>().join(" | ")));
        run("type_alias_chain_rev", {
            let mut s = String::new();
            for i in 0..k { s.push_str(&format!("type A{i} = A{}[];\n", i + 1)); }
            s.push_str(&format!("type A{k} = i32;\nfunction f(a: A0): void {{}}"));
            s
        });
        run("struct_chain_rev", {
            let mut s = String::new();
            for i in 0..k { s.push_str(&format!("struct S{i} {{ a: S{} | null; }}\n", i + 1)); }
            s.push_str(&format!("struct S{k} {{ x: i32; }}\nfunction f(a: S0): void {{}}"));
            s
        });
        run("obj_nest", format!("struct S {{ s: S | null; }}\nfunction f(): S {{ return {}{{ s: null }}{}; }}", rep("{ s: ", k), rep(" }", k)));
        run("lambda_nest", format!("function f(): void {{ const g = {}1; }}", rep("() => ", k)));
        run("lambda_call_nest", format!("function f(): void {{ const g = {}1{}; }}", rep("(() => ", k), rep(")()", k)));
        run("switch_nest", format!("function f(a: i32): i32 {{ {}return 1;{} return 0; }}", rep("switch (a) { case 1: ", k), rep(" default: return 2; }", k)));
        run("switch_expr_nest", format!("function f(a: i32): i32 {{ return {}1{}; }}", rep("switch (a) { default: ", k), rep(", }", k)));
        run("while_nest", format!("function f(a: bool): void {{ {}{} }}", rep("while (a) { ", k), rep("} ", k)));
        run("label_nest", format!("function f(a: bool): void {{ {}{} }}", (0..k).map(|i| format!("l{i}: ")).collect::<String>(), ";"));
        run("template_nest", format!("function f(): string {{ return {}x{}; }}", rep("`${", k), rep("}`", k)));
        run("generic_call_nest", format!("function id<T>(a: T): T {{ return a; }}\nfunction f(): i32 {{ return {}1{}; }}", rep("id(", k), rep(")", k)));
        run("generic_call_array_nest", format!("function id<T>(a: T): T {{ return a; }}\nfunction f(): void {{ const x = {}1{}; }}", rep("id([", k), rep("])", k)));
        run("map_lambda_nest", format!("function m<T, U>(a: T, f: (x: T) => U): U {{ return f(a); }}\nfunction f(): i32 {{ return {}1{}; }}", rep("m(1, (x) => ", k), rep(")", k)));
        run("new_nest", format!("class C {{ constructor(c: C | null) {{}} }}\nfunction f(): C {{ return {}null{}; }}", rep("new C(", k), rep(")", k)));
        run("coalesce_nest", format!("function f(a: i32 | null): i32 {{ return {}a{}; }}", rep("(", k), rep(" ?? 1)", k)));
        run("and_chain", format!("function f(a: i32 | null): bool {{ return a !== null{}; }}", rep(" && a > 1", k)));
        run("or_chain", format!("function f(a: i32 | null): bool {{ return a === null{}; }}", rep(" || a > 1", k)));
        run("generic_decl_chain", {
            let mut s = String::from("function f0<T>(a: T): T { return a; }\n");
            for i in 1..k { s.push_str(&format!("function f{i}<T>(a: T): T {{ return f{}<T[]>([a])[0]; }}\n", i - 1)); }
            s
        });
        run("iface_extends_chain", {
            let mut s = String::from("interface I0 { m0(): i32; }\n");
            for i in 1..k { s.push_str(&format!("interface I{i} extends I{} {{ m{i}(): i32; }}\n", i - 1)); }
            s
        });
        run("iface_extends_chain_rev", {
            let mut s = String::new();
            for i in 0..k { s.push_str(&format!("interface I{i} extends I{} {{ m{i}(): i32; }}\n", i + 1)); }
            s.push_str(&format!("interface I{k} {{ z(): i32; }}\nfunction g<T extends I0>(t: T): i32 {{ return t.z(); }}"));
            s
        });
        run("law_nest", format!("law l(a: i32) {{ {}true }}", rep("{ ", k)));
        run("import_many", (0..k).map(|i| format!("import {{ x{i} }} from \"./m{i}\";\n")).collect());
        run("many_params", format!("function f({}): void {{}}", (0..k).map(|i| format!("a{i}: i32")).collect::<Vec<_>>().join(", ")));
        run("many_type_params", format!("function f<{}>(): void {{}}", (0..k).map(|i| format!("T{i}")).collect::<Vec<_>>().join(", ")));
        run("many_type_params_call", format!("function f<{}>(): void {{}}\nfunction g(): void {{ f<{}>(); }}", (0..k.min(300)).map(|i| format!("T{i}")).collect::<Vec<_>>().join(", "), (0..k.min(300)).map(|_| "i32").collect::<Vec<_>>().join(", ")));
        run("fn_type_wide", format!("function f(a: ({}) => void): void {{}}", (0..k).map(|i| format!("a{i}: i32")).collect::<Vec<_>>().join(", ")));
        run("destructure_nest", format!("struct S {{ s: S; }}\nfunction f(a: S): void {{ const {}x{} = a; }}", rep("{ s: ", k), rep(" }", k)));
        run("enum_big", format!("enum E: u8 {{ {} }}", (0..k).map(|i| format!("M{i}")).collect::<Vec<_>>().join(", ")));
        run("many_attrs", format!("{}function f(): void {{}}", rep("@inline ", k)));
    }
}
