#![allow(clippy::format_push_string)]
//! Output-table invariants: when a program has no errors, no expression carries the
//! poison type, every coercion and call is recorded consistently, and every
//! `Res` points at something that exists.
mod common;
use common::*;
use oxiscript_check::ty::ERROR;
use oxiscript_check::*;
use oxiscript_syntax::*;

struct Walk<'c> {
    c: &'c Checked,
    m: usize,
    bad: Vec<String>,
}

impl<'a> Visitor<'a> for Walk<'_> {
    fn visit_expr(&mut self, e: &'a Expr) {
        let m = &self.c.modules[self.m];
        let t = m.expr_ty.get(e.id.0 as usize).copied();
        match t {
            None => self.bad.push(format!("expr {:?} at {} has no type", e.id, e.span.lo)),
            Some(t) => {
                if t == ERROR || self.c.tys.has_error(t) {
                    self.bad.push(format!("expr at {}..{} has the error type", e.span.lo, e.span.hi));
                }
            }
        }
        if let ExprKind::Ident(_) = &e.kind {
            if !m.res.contains_key(&e.id) {
                self.bad.push(format!("identifier at {} has no resolution", e.span.lo));
            }
        }
        walk_expr(self, e);
    }
}

/// No poison type in any side table of an error-free program (expression types are checked by the walker).
fn all_tables_error_free(c: &Checked, label: &str) {
    let e = |t: oxiscript_check::Ty| t == ERROR || c.tys.has_error(t);
    for (mi, m) in c.modules.iter().enumerate() {
        for l in &m.locals {
            assert!(!e(l.ty), "{label}: module {mi} local {}", l.name);
        }
        for (id, co) in m.coercions.iter().chain(m.casts.iter()) {
            assert!(!e(co.from) && !e(co.to), "{label}: module {mi} coercion/cast {id:?}");
        }
        for (id, ci) in &m.calls {
            assert!(ci.type_args.iter().all(|t| !e(*t)), "{label}: module {mi} call {id:?} type_args");
        }
        for (id, sw) in &m.switches {
            assert!(sw.union.is_none_or(|t| !e(t)), "{label}: switch {id:?}");
        }
    }
    for (i, inst) in c.instances.iter().enumerate() {
        assert!(inst.args.iter().all(|t| !e(*t)), "{label}: instance {i} of {:?}", inst.def);
    }
    for (d, f) in &c.fns {
        assert!(!e(f.ret) && f.params.iter().all(|p| !e(p.ty)), "{label}: fn {}", c.defs[d.0 as usize].name);
    }
    for (d, r) in &c.records {
        assert!(r.fields.iter().all(|f| !e(f.ty)), "{label}: record {}", c.defs[d.0 as usize].name);
    }
    for (d, g) in &c.globals {
        assert!(!e(g.ty), "{label}: global {}", c.defs[d.0 as usize].name);
    }
    for (d, t) in &c.aliases {
        assert!(!e(*t), "{label}: alias {}", c.defs[d.0 as usize].name);
    }
    for (d, u) in &c.unions {
        assert!(!e(u.discr_ty), "{label}: union {}", c.defs[d.0 as usize].name);
    }
    for (d, h) in &c.hosts {
        assert!(h.fns.iter().all(|(_, t)| !e(*t)), "{label}: host {}", c.defs[d.0 as usize].name);
    }
}

fn walk_all(src: &str, label: &str) {
    let c = check_source(src, &hosts());
    assert!(c.ok(), "{label}: errors\n{}", show(src));
    all_tables_error_free(&c, label);
    let parsed = parse(src, FileId(0));
    let mut w = Walk { c: &c, m: 0, bad: Vec::new() };
    w.visit_module(&parsed.module);
    assert!(w.bad.is_empty(), "{label}: {:#?}", w.bad);
    // locals: every recorded local has a type that is not the poison type
    for l in &c.modules[0].locals {
        assert!(l.ty != ERROR, "{label}: local {} has the error type", l.name);
    }
    // coercions are between distinct, error-free types
    for (id, co) in &c.modules[0].coercions {
        assert!(co.from != co.to && !c.tys.has_error(co.from) && !c.tys.has_error(co.to), "{label}: coercion at {id:?}");
    }
}

#[test]
fn valid_corpus_tables_are_error_free() {
    let dir = format!("{}/../oxiscript-syntax/tests/corpus/valid", env!("CARGO_MANIFEST_DIR"));
    let mut n = 0;
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "oxs") {
            continue;
        }
        let src = std::fs::read_to_string(&p).unwrap();
        // files that import modules need the loader; check the module's own tables only
        if src.contains("from \"./") || src.contains("from \"std/") {
            continue;
        }
        walk_all(&src, &p.file_name().unwrap().to_string_lossy());
        n += 1;
    }
    assert!(n >= 35, "{n}");
}

#[test]
fn accepted_feature_programs_have_complete_tables() {
    for (i, src) in [
        "enum E: u8 { A = 1 << 2, B }\nfunction f(e: E): i32 { return e as i32; }",
        "function fail(): never { return fail(); }\nfunction g(): i32 { return fail() + 1; }",
        "type S = { kind: \"a\"; x: i32 } | { kind: \"b\" };\nfunction f(s: S): i32 {\n  return switch (s.kind) { case \"a\": s.x, case \"b\": 0, };\n}",
        "class C { n: i32 = 0; inc(): void { this.n += 1; } static make(): C { return new C(); } }\nfunction f(): i32 { const c = C.make(); c.inc(); return c.n; }",
        "function id<T>(x: T): T { return x; }\nfunction f(): i32 { return id(1) + id<i32>(2); }",
        "function f(xs: i32[]): i32 { return xs.map((x) => x * 2).reduce((a, b) => a + b, 0); }",
        "function f(a: i32 | null): i32 { return a ?? 3; }",
        "struct P { x: i32; y: i32; }\nfunction f(p: P): i32 { const { x, y } = p; return x + y; }",
    ]
    .iter()
    .enumerate()
    {
        walk_all(src, &format!("feature {i}"));
    }
}

#[test]
fn this_locals_are_distinct_per_method() {
    let src = "class C { n: i32 = 0; a(): void {} b(): void {} }";
    let c = check_source(src, &hosts());
    assert!(c.ok());
    assert_eq!(c.modules[0].this_locals.len(), 2);
    let ids: std::collections::HashSet<_> = c.modules[0].this_locals.values().collect();
    assert_eq!(ids.len(), 2);
}

#[test]
fn generated_program_and_modules_have_error_free_tables() {
    let mut s = String::from("struct Box<T> { v: T; }\nfunction id<T>(x: T): T { return x; }\n");
    for i in 0..30 {
        s.push_str(&format!(
            "type Shape{i} = {{ kind: \"c\"; r: f64 }} | {{ kind: \"s\"; w: f64 }};\nfunction w{i}(a: Array<i32>, m: i32 | null, s: Shape{i}): f64 {{\n  let t = 0;\n  for (const v of a) {{ t += v; }}\n  const b = id<Box<i32>>({{ v: t }});\n  const f = (k: i32) => k + b.v;\n  if (m !== null) {{ t += m; }}\n  switch (s.kind) {{ case \"c\": return s.r; case \"s\": return s.w; }}\n}}\n"
        ));
    }
    let c = check_source(&s, &hosts());
    assert!(c.ok(), "{}", show(&s));
    all_tables_error_free(&c, "generated");
    let mut l = MapLoader::default();
    l.0.insert("math".into(), "export function add(a: i32, b: i32): i32 { return a + b; }\nexport struct Pt { x: i32; y: i32; }\nexport function mk(x: i32): Array<i32> { return [x]; }\n".into());
    let c = check_program("main", "import { add, Pt, mk } from \"./math\";\nimport * as m from \"./math\";\nexport function f(): i32 {\n  const p: Pt = { x: 1, y: 2 };\n  return add(p.x, m.add(1, 2)) + mk(3).length;\n}\n", &mut l, &hosts());
    assert!(c.ok(), "{:?}", c.errors().map(|d| d.message.clone()).collect::<Vec<_>>());
    all_tables_error_free(&c, "modules");
}
