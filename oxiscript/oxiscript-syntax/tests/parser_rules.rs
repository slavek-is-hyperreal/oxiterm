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
//! The disambiguation rules of grammar.md 3.4 and the must-parenthesise table.
mod common;
use common::*;
use oxiscript_syntax::*;

#[test]
fn precedence_is_typescripts() {
    assert_eq!(init_sx("a + b * c"), "(+ a (* b c))");
    assert_eq!(init_sx("a * b as T"), "(as (* a b))");
    assert_eq!(init_sx("a - b - c"), "(- (- a b) c)");
    assert_eq!(init_sx("a || b && c"), "(|| a (&& b c))");
    assert_eq!(init_sx("a === b && c !== d"), "(&& (=== a b) (!== c d))");
    assert_eq!(
        init_sx("a < b === (c > d)"),
        "(=== (< a b) (paren (> c d)))"
    );
    assert_eq!(init_sx("- a * b"), "(* (Neg a) b)");
    assert_eq!(init_sx("a ? b : c ? d : e"), "(? a b (? c d e))");
    assert_eq!(init_sx("a ?? b ?? c"), "(?? (?? a b) c)");
    assert_eq!(init_sx("a << b >> c"), "(>> (<< a b) c)");
    assert_eq!(init_sx("a +% b +| c"), "(+| (+% a b) c)");
}

#[test]
fn generic_call_versus_comparison() {
    assert_eq!(init_sx("f<i32>(x)"), "(call f<1> x)");
    assert_eq!(
        init_sx("a < b > (c)"),
        "(call a<1> c)",
        "TypeScript reads this as a generic call"
    );
    assert_eq!(init_sx("f(a < b, c > (d))"), "(call f (call a<2> d))");
    assert_eq!(init_sx("(a < b) > c"), "(> (paren (< a b)) c)");
    assert_eq!(
        init_sx("f((a < b), (c > d))"),
        "(call f (paren (< a b)) (paren (> c d)))"
    );
    assert_eq!(init_sx("a < b"), "(< a b)");
    assert_eq!(init_sx("a < b && c > d"), "(&& (< a b) (> c d))");
    assert_eq!(init_sx("x.wrapping<u8>()"), "(call (. x wrapping)<1> )");
    assert_eq!(init_sx("g<Array<Array<i32>>>(1)"), "(call g<1> 1)");
    assert_eq!(init_sx("g<A | B, C[]>(1)"), "(call g<2> 1)");
}

#[test]
fn shift_tokens_are_split_in_type_arguments() {
    let (m, d) = parse_module(
        "let a: Array<Array<i32>>= x;\nlet b: Array<Array<i32>>>= y;\nlet c: Array<i32>= z;",
    );
    // the middle line is a genuine error (`>>>` then `=`), the others split `>>=` and `>=`
    assert!(d.iter().all(|x| x.span.lo > 28 && x.span.lo < 65), "{d:?}");
    assert_eq!(m.items.len(), 3);
    let (_, d) = parse_module(
        "let a: Array<Array<i32>>= x;\nlet c: Array<i32>= z;\nlet e: Array<Array<Array<i32>>>= w;",
    );
    assert_eq!(d.len(), 0, "{d:?}");
    assert_eq!(init_sx("a >> b"), "(>> a b)");
    assert_eq!(init_sx("a >= b"), "(>= a b)");
}

#[test]
fn arrow_versus_parenthesised() {
    assert_eq!(init_sx("x => x"), "(arrow [x] x)");
    assert_eq!(init_sx("(x) => x"), "(arrow [x] x)");
    assert_eq!(init_sx("(x, y) => x"), "(arrow [x,y] x)");
    assert_eq!(init_sx("(x): i32 => x"), "(arrow [x]:T x)");
    assert_eq!(init_sx("(x)"), "(paren x)");
    assert_eq!(init_sx("(a + b) * c"), "(* (paren (+ a b)) c)");
    assert_eq!(init_sx("() => ({ x: 1 })"), "(arrow [] (paren (object 1)))");
    assert_eq!(init_sx("() => { }"), "(arrow [] {})");
}

#[test]
fn ternary_arrow_return_type_rule() {
    // else branch is the arrow function `z => w`
    assert_eq!(init_sx("x ? (y) : z => w"), "(? x (paren y) (arrow [z] w))");
    // true branch is an arrow with a return type, followed by the else `:`
    assert_eq!(init_sx("x ? (y): T => y : z"), "(? x (arrow [y]:T y) z)");
    assert_eq!(init_sx("x ? (y) : z"), "(? x (paren y) z)");
    assert_eq!(init_sx("x ? y : (a): T => a"), "(? x y (arrow [a]:T a))");
}

#[test]
fn question_mark_rule() {
    assert_eq!(init_sx("f()?"), "(try (call f ))");
    assert_eq!(init_sx("f() ? a : b"), "(? (call f ) a b)");
    assert_eq!(init_sx("f()?.x"), "(?. (call f ) x)");
    assert_eq!(init_sx("f() ?.x"), "(?. (call f ) x)");
    assert_eq!(init_sx("f()\n  ?.x"), "(?. (call f ) x)");
    assert_eq!(init_sx("a ?.5 : 1"), "(? a .5 1)");
    assert_eq!(init_sx("a?.[0]"), "(?[] a 0)");
    assert_eq!(init_sx("a?.(1)"), "(call? a 1)");
    assert_eq!(init_sx("(f()?).x"), "(. (paren (try (call f ))) x)");
    assert_eq!(init_sx("f()? + 1"), "(+ (try (call f )) 1)");
    assert_eq!(init_sx("a ? b : c ? d : e"), "(? a b (? c d e))");
}

#[test]
fn must_parenthesise_table() {
    assert_eq!(codes("const x = a & b === c;"), ["OXS-E0007"]);
    assert_eq!(codes("const x = a === b & c;"), ["OXS-E0007"]);
    assert_eq!(codes("const x = a | b < c;"), ["OXS-E0007"]);
    assert_eq!(codes("const x = a ^ b | c;"), ["OXS-E0007"]);
    assert_eq!(codes("const x = a << b + c;"), ["OXS-E0008"]);
    assert_eq!(codes("const x = a +| b >> c;"), ["OXS-E0008"]);
    assert_eq!(codes("const x = a ?? b || c;"), ["OXS-E0009"]);
    assert_eq!(codes("const x = a && b ?? c;"), ["OXS-E0009"]);
    assert_eq!(codes("const x = a < b < c;"), ["OXS-E0001"]);
    assert_eq!(codes("const x = a === b !== c;"), ["OXS-E0001"]);
    assert_eq!(codes("const x = a as u8 | 1;"), ["OXS-E0010"]);
    // exempt: parenthesised operands and repeated operators
    assert_eq!(codes("const x = (a & b) === c; const y = a & b & c; const z = a + b + c; const w = (a ?? b) || c;").len(), 0);
    assert!(codes("const x = a * b << c; const y = a < b == false;")
        .iter()
        .all(|c| *c != "OXS-E0008"));
}

#[test]
fn fixes_insert_parentheses_around_the_tighter_pair() {
    let src = "const x = a & b === c;";
    let (_, d) = parse_module(src);
    let fixed = apply_edits(src, &d[0].fixes[0].edits).unwrap();
    assert_eq!(fixed, "const x = a & (b === c);");
    let src = "const x = a ?? b || c;";
    let (_, d) = parse_module(src);
    assert_eq!(
        apply_edits(src, &d[0].fixes[0].edits).unwrap(),
        "const x = a ?? (b || c);"
    );
    let src = "const x = a == b;";
    let (_, d) = parse_module(src);
    assert_eq!(
        apply_edits(src, &d[0].fixes[0].edits).unwrap(),
        "const x = a === b;"
    );
    let src = "const x = 1f32;";
    let (_, d) = parse_module(src);
    assert_eq!(
        apply_edits(src, &d[0].fixes[0].edits).unwrap(),
        "const x = 1.0f32;"
    );
    let src = "let x = 1";
    let (_, d) = parse_module(src);
    assert_eq!(
        apply_edits(src, &d[0].fixes[0].edits).unwrap(),
        "let x = 1;"
    );
    let src = "const x = a as u8 | 1;";
    let (_, d) = parse_module(src);
    let outs: Vec<String> = d[0]
        .fixes
        .iter()
        .map(|f| apply_edits(src, &f.edits).unwrap())
        .collect();
    assert_eq!(
        outs,
        ["const x = (a as u8) | 1;", "const x = a as (u8 | 1);"]
    );
    // after applying a fix the code parses cleanly
    for s in &outs {
        assert_eq!(parse_module(s).1.len(), 0, "{s}");
    }
}

#[test]
fn switch_statement_versus_expression() {
    let (m, d) = parse_module("function f(x: i32): i32 {\n switch (x) { case 1: return 1; default: return 2; }\n}\nconst y = switch (1) { case 1: 2, default: 3, };");
    assert_eq!(d.len(), 0, "{d:?}");
    let ItemKind::Fn(f) = &m.items[0].kind else {
        panic!()
    };
    assert!(matches!(
        f.body.as_ref().unwrap().stmts[0].kind,
        StmtKind::Switch(_)
    ));
    let ItemKind::Var(v) = &m.items[1].kind else {
        panic!()
    };
    assert!(matches!(v.init.as_ref().unwrap().kind, ExprKind::Switch(_)));
    // stacked labels share one clause
    let (m, d) =
        parse_module("function f(x: i32): void { switch (x) { case 1: case 2: default: g(); } }");
    assert_eq!(d.len(), 0);
    let ItemKind::Fn(f) = &m.items[0].kind else {
        panic!()
    };
    let StmtKind::Switch(s) = &f.body.as_ref().unwrap().stmts[0].kind else {
        panic!()
    };
    assert_eq!(s.cases.len(), 1);
    assert_eq!(s.cases[0].labels.len(), 2);
    assert!(s.cases[0].is_default);
    // an arm label with a top-level ?: must be parenthesised
    assert_eq!(
        codes("const y = switch (x) { case a ? b : c: 1, default: 2 };"),
        ["OXS-E0001"]
    );
    assert_eq!(codes("const y = switch (x) { case (a ? b : c): 1, default: 2 };").len(), 0);
    // `case x => v` is not an arrow function
    assert_eq!(
        codes("const y = switch (x) { case a => 1, default: 2 };"),
        ["OXS-E0001"]
    );
}

#[test]
fn contextual_keywords_are_identifiers_elsewhere() {
    assert_eq!(
        codes("const type = 1; const law = type + 1; type A = i32; law l(x: i32) { x === x }").len(),
        0
    );
    let (m, _) = parse_module("type = 5;");
    assert!(m.items.is_empty() || !matches!(m.items[0].kind, ItemKind::TypeAlias(_)));
}

#[test]
fn doc_comments_attach_to_items_and_members() {
    let (m, d) = parse_module("/// one\n/// two\nfunction f(): void {}\n// plain\nstruct S {\n  /// field doc\n  x: i32;\n}\n");
    assert_eq!(d.len(), 0);
    assert_eq!(m.items[0].doc, ["one", "two"]);
    assert_eq!(m.items[1].doc.len(), 0);
    let ItemKind::Record(r) = &m.items[1].kind else {
        panic!()
    };
    assert_eq!(r.members[0].doc, ["field doc"]);
}

#[test]
fn laws_have_tail_expressions_only_in_law_blocks() {
    let (m, d) = parse_module("law l(a: i32) { const b = a; b === a }");
    assert_eq!(d.len(), 0, "{d:?}");
    let ItemKind::Law(l) = &m.items[0].kind else {
        panic!()
    };
    assert_eq!(l.body.stmts.len(), 1);
    assert!(l.body.tail.is_some());
    assert_eq!(codes("function f(a: i32): bool { a === a }"), ["OXS-E0004"]);
}

#[test]
fn reserved_syntax_has_ast_slots() {
    let (m, d) = parse_module("kernel function k(inout p: i32): void requires p > 0 {}");
    assert_eq!(d.iter().filter(|x| x.code == "OXS-E0900").count(), 3);
    let ItemKind::Fn(f) = &m.items[0].kind else {
        panic!()
    };
    assert!(f.mods.kernel);
    assert_eq!(f.params[0].mode, ParamMode::Inout);
    assert_eq!(f.contracts.len(), 1);
    let (_, d) = parse_module("function f(xs: i32[]): void { parallel for (const x of xs) {} }");
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].code, "OXS-E0900");
}

#[test]
fn spans_and_ids() {
    let src = "function f(a: i32): i32 {\n  return a + 1;\n}\n";
    let (m, d) = parse_module(src);
    assert_eq!(d.len(), 0);
    assert_eq!(
        &src[m.items[0].span.lo as usize..m.items[0].span.hi as usize],
        src.trim_end()
    );
    let ItemKind::Fn(f) = &m.items[0].kind else {
        panic!()
    };
    let ret = &f.body.as_ref().unwrap().stmts[0];
    assert_eq!(
        &src[ret.span.lo as usize..ret.span.hi as usize],
        "return a + 1;"
    );
    // ids are unique
    struct Ids(Vec<u32>);
    impl<'a> Visitor<'a> for Ids {
        fn visit_expr(&mut self, e: &'a Expr) {
            self.0.push(e.id.0);
            walk_expr(self, e);
        }
        fn visit_stmt(&mut self, s: &'a Stmt) {
            self.0.push(s.id.0);
            walk_stmt(self, s);
        }
        fn visit_ident(&mut self, i: &'a Ident) {
            self.0.push(i.id.0);
        }
    }
    let mut v = Ids(vec![]);
    v.visit_module(&m);
    let n = v.0.len();
    v.0.sort_unstable();
    v.0.dedup();
    assert_eq!(v.0.len(), n);
}

#[test]
fn error_recovery_reports_many_diagnostics_in_one_pass() {
    let src = "let a = ;\nfunction f( {\nlet b = 1 +;\n}\nconst c = 1 1;\nstruct S { x }\nconst d = 2 == 3;\n";
    let (m, d) = parse_module(src);
    assert!(d.len() >= 4, "{d:#?}");
    assert!(m.items.len() >= 2);
}

#[test]
fn depth_limit_is_reported_not_crashed() {
    let deep = format!("const a = {}1{};", "(".repeat(5000), ")".repeat(5000));
    let (_, d) = parse_module(&deep);
    assert_eq!(
        d.iter().filter(|x| x.code == "OXS-E0011").count(),
        1,
        "{:?}",
        d.len()
    );
    let chain = format!("const a = 1{};", " + 1".repeat(5000));
    let (_, d) = parse_module(&chain);
    assert!(d.iter().any(|x| x.code == "OXS-E0011"));
    let ok = format!("const a = 1{};", " + 1".repeat(900));
    assert_eq!(parse_module(&ok).1.len(), 0);
}

#[test]
fn too_many_errors_stops() {
    let src = "let = ;\n".repeat(1000);
    let (_, d) = parse_module(&src);
    assert!(d.len() <= 202, "{}", d.len());
    assert!(d.iter().any(|x| x.code == "OXS-E0017"));
}
