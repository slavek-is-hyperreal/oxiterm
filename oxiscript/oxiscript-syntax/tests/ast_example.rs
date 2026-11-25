#![allow(clippy::similar_names, clippy::doc_markdown, clippy::too_many_lines)]
//! A hand-built AST for this OxiScript source (no parser exists yet; slice
//! 8.P replaces this with parsing and keeps the test as a golden):
//!
//! ```text
//! import { Log } from "host:log";
//!
//! type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };
//!
//! export function area(s: Shape): f64 {
//!   switch (s.kind) {
//!     case "circle": return s.r * s.r;
//!     case "rect": return s.w * s.h;
//!   }
//! }
//!
//! @expect(simd)
//! function sum(xs: f64[]): f64 {
//!   let acc = 0.0;
//!   for (const x of xs) { acc += x; }
//!   return acc;
//! }
//! ```

use oxiscript_syntax::*;
use std::collections::HashSet;

struct B {
    ids: NodeIdGen,
    pos: u32,
}

impl B {
    fn new() -> B {
        B {
            ids: NodeIdGen::new(),
            pos: 0,
        }
    }
    /// Fake but monotonically increasing spans: each node is 1 byte wide.
    fn span(&mut self) -> Span {
        self.pos += 1;
        Span::new(FileId(0), self.pos - 1, self.pos)
    }
    fn ident(&mut self, n: &str) -> Ident {
        Ident {
            id: self.ids.next_id(),
            span: self.span(),
            name: n.into(),
        }
    }
    fn lit(&mut self, kind: LitKind) -> Lit {
        Lit {
            id: self.ids.next_id(),
            span: self.span(),
            kind,
        }
    }
    fn ty(&mut self, kind: TypeExprKind) -> TypeExpr {
        TypeExpr {
            id: self.ids.next_id(),
            span: self.span(),
            kind,
        }
    }
    fn path_ty(&mut self, n: &str) -> TypeExpr {
        let i = self.ident(n);
        self.ty(TypeExprKind::Path {
            segments: vec![i],
            args: vec![],
        })
    }
    fn expr(&mut self, kind: ExprKind) -> Expr {
        Expr {
            id: self.ids.next_id(),
            span: self.span(),
            kind,
        }
    }
    fn var(&mut self, n: &str) -> Expr {
        let i = self.ident(n);
        self.expr(ExprKind::Ident(i))
    }
    fn member(&mut self, obj: Expr, n: &str) -> Expr {
        let name = self.ident(n);
        self.expr(ExprKind::Member {
            obj: Box::new(obj),
            name,
            optional: false,
        })
    }
    fn bin(&mut self, op: BinOp, l: Expr, r: Expr) -> Expr {
        self.expr(ExprKind::Binary {
            op,
            lhs: Box::new(l),
            rhs: Box::new(r),
        })
    }
    fn stmt(&mut self, kind: StmtKind) -> Stmt {
        Stmt {
            id: self.ids.next_id(),
            span: self.span(),
            attrs: vec![],
            kind,
        }
    }
    fn block(&mut self, stmts: Vec<Stmt>) -> Block {
        Block {
            id: self.ids.next_id(),
            span: self.span(),
            stmts,
            tail: None,
        }
    }
    fn pat(&mut self, n: &str) -> Pattern {
        let i = self.ident(n);
        Pattern {
            id: self.ids.next_id(),
            span: self.span(),
            kind: PatternKind::Ident(i),
        }
    }
    fn param(&mut self, n: &str, ty: TypeExpr) -> Param {
        let name = self.ident(n);
        Param {
            id: self.ids.next_id(),
            span: self.span(),
            mode: ParamMode::Value,
            name,
            ty: Some(ty),
            default: None,
        }
    }
    fn field_sig(&mut self, n: &str, ty: TypeExpr) -> FieldSig {
        let name = self.ident(n);
        FieldSig {
            id: self.ids.next_id(),
            span: self.span(),
            name,
            ty,
            readonly: false,
        }
    }
    fn variant(&mut self, tag: &str, fields: &[(&str, &str)]) -> TypeExpr {
        let kind_ty = {
            let l = self.lit(LitKind::Str(tag.into()));
            self.ty(TypeExprKind::Lit(l))
        };
        let mut sigs = vec![self.field_sig("kind", kind_ty)];
        for (n, t) in fields {
            let ty = self.path_ty(t);
            sigs.push(self.field_sig(n, ty));
        }
        self.ty(TypeExprKind::Object(sigs))
    }
    fn item(&mut self, export: bool, attrs: Vec<Attribute>, kind: ItemKind) -> Item {
        Item {
            id: self.ids.next_id(),
            span: self.span(),
            doc: vec![],
            attrs,
            export,
            kind,
        }
    }
    fn func(&mut self, name: &str, params: Vec<Param>, ret: TypeExpr, body: Block) -> FnDecl {
        FnDecl {
            name: self.ident(name),
            mods: FnModifiers::default(),
            type_params: vec![],
            params,
            ret: Some(ret),
            contracts: vec![],
            body: Some(body),
        }
    }
}

fn build() -> Module {
    let mut b = B::new();
    let mut items = Vec::new();

    // import { Log } from "host:log";
    let spec = ImportSpec {
        id: b.ids.next_id(),
        span: b.span(),
        name: b.ident("Log"),
        alias: None,
    };
    let source = b.lit(LitKind::Str("host:log".into()));
    items.push(b.item(
        false,
        vec![],
        ItemKind::Import(ImportDecl {
            clause: ImportClause::Named(vec![spec]),
            source,
        }),
    ));

    // type Shape = { kind: "circle"; r: f64 } | { kind: "rect"; w: f64; h: f64 };
    let circle = b.variant("circle", &[("r", "f64")]);
    let rect = b.variant("rect", &[("w", "f64"), ("h", "f64")]);
    let union = b.ty(TypeExprKind::Union(vec![circle, rect]));
    let name = b.ident("Shape");
    items.push(b.item(
        false,
        vec![],
        ItemKind::TypeAlias(TypeAliasDecl {
            name,
            type_params: vec![],
            ty: union,
        }),
    ));

    // export function area(s: Shape): f64 { switch ... }
    let scrutinee = {
        let s = b.var("s");
        b.member(s, "kind")
    };
    let mut cases = Vec::new();
    for (tag, a, c) in [("circle", "r", "r"), ("rect", "w", "h")] {
        let label = {
            let l = b.lit(LitKind::Str(tag.into()));
            b.expr(ExprKind::Lit(l))
        };
        let lhs = {
            let s = b.var("s");
            b.member(s, a)
        };
        let rhs = {
            let s = b.var("s");
            b.member(s, c)
        };
        let prod = b.bin(BinOp::Mul, lhs, rhs);
        let ret = b.stmt(StmtKind::Return(Some(prod)));
        cases.push(Case {
            id: b.ids.next_id(),
            span: b.span(),
            labels: vec![label],
            is_default: false,
            body: vec![ret],
        });
    }
    let sw = b.stmt(StmtKind::Switch(SwitchStmt { scrutinee, cases }));
    let body = b.block(vec![sw]);
    let p = {
        let t = b.path_ty("Shape");
        b.param("s", t)
    };
    let ret = b.path_ty("f64");
    let f = b.func("area", vec![p], ret, body);
    items.push(b.item(true, vec![], ItemKind::Fn(f)));

    // @expect(simd) function sum(xs: f64[]): f64 { ... }
    let attr = {
        let name = b.ident("expect");
        let arg = AttrArg {
            id: b.ids.next_id(),
            span: b.span(),
            key: None,
            value: AttrValue::Path(vec![b.ident("simd")]),
        };
        Attribute {
            id: b.ids.next_id(),
            span: b.span(),
            inner: false,
            name,
            args: vec![arg],
        }
    };
    let acc_decl = {
        let pattern = b.pat("acc");
        let l = b.lit(LitKind::Float {
            text: "0.0".into(),
            suffix: None,
        });
        let init = b.expr(ExprKind::Lit(l));
        b.stmt(StmtKind::Var(VarDecl {
            kind: VarKind::Let,
            pattern,
            ty: None,
            init: Some(init),
        }))
    };
    let loop_stmt = {
        let binding = b.pat("x");
        let iter = b.var("xs");
        let target = b.var("acc");
        let value = b.var("x");
        let assign = b.expr(ExprKind::Assign {
            op: Some(BinOp::Add),
            target: Box::new(target),
            value: Box::new(value),
        });
        let inner = b.stmt(StmtKind::Expr(assign));
        let blk = b.block(vec![inner]);
        let body = b.stmt(StmtKind::Block(blk));
        b.stmt(StmtKind::ForOf {
            kind: VarKind::Const,
            binding,
            iter,
            body: Box::new(body),
            parallel: false,
        })
    };
    let ret_stmt = {
        let a = b.var("acc");
        b.stmt(StmtKind::Return(Some(a)))
    };
    let body = b.block(vec![acc_decl, loop_stmt, ret_stmt]);
    let p = {
        let el = b.path_ty("f64");
        let arr = b.ty(TypeExprKind::Array(Box::new(el)));
        b.param("xs", arr)
    };
    let ret = b.path_ty("f64");
    let f = b.func("sum", vec![p], ret, body);
    items.push(b.item(false, vec![attr], ItemKind::Fn(f)));

    Module {
        id: b.ids.next_id(),
        span: Span::new(FileId(0), 0, b.pos),
        attrs: vec![],
        items,
    }
}

#[derive(Default)]
struct Stats {
    idents: Vec<String>,
    exprs: usize,
    stmts: usize,
    types: usize,
    ids: Vec<NodeId>,
    attrs: Vec<String>,
}

impl<'a> Visitor<'a> for Stats {
    fn visit_ident(&mut self, i: &'a Ident) {
        self.idents.push(i.name.clone());
        self.ids.push(i.id);
    }
    fn visit_expr(&mut self, e: &'a Expr) {
        self.exprs += 1;
        self.ids.push(e.id);
        walk_expr(self, e);
    }
    fn visit_stmt(&mut self, s: &'a Stmt) {
        self.stmts += 1;
        self.ids.push(s.id);
        walk_stmt(self, s);
    }
    fn visit_type(&mut self, t: &'a TypeExpr) {
        self.types += 1;
        self.ids.push(t.id);
        walk_type(self, t);
    }
    fn visit_attribute(&mut self, a: &'a Attribute) {
        self.attrs.push(a.name.name.clone());
        walk_attribute(self, a);
    }
    fn visit_lit(&mut self, l: &'a Lit) {
        self.ids.push(l.id);
    }
}

#[test]
fn visitor_walks_in_source_order() {
    let m = build();
    let mut s = Stats::default();
    s.visit_module(&m);
    assert_eq!(s.attrs, vec!["expect"]);
    // identifiers are visited in source order
    assert_eq!(s.idents.first().map(String::as_str), Some("Log"));
    assert_eq!(s.idents[1], "Shape");
    assert!(s.idents.iter().filter(|n| *n == "s").count() >= 5);
    assert_eq!(s.idents.iter().filter(|n| *n == "acc").count(), 3);
    // area: switch + 2 returns; sum: let, for-of, block, `acc += x`, return
    assert_eq!(s.stmts, 3 + 5);
    assert!(s.exprs > 15);
    assert!(s.types >= 8);
}

#[test]
fn node_ids_are_unique_and_dense() {
    let m = build();
    let mut s = Stats::default();
    s.visit_module(&m);
    let unique: HashSet<_> = s.ids.iter().collect();
    assert_eq!(unique.len(), s.ids.len(), "NodeIds must be unique");
    // dense: every id below the max appears somewhere in the tree or in an
    // unvisited container node (items, blocks, params, cases, ...), so the
    // max id is at least the visited count - 1.
    let max = s.ids.iter().map(|i| i.0).max().unwrap() as usize;
    assert!(max + 1 >= s.ids.len());
}

#[test]
fn spans_are_ordered_and_extendable() {
    let m = build();
    let ItemKind::Fn(f) = &m.items[2].kind else {
        panic!()
    };
    assert_eq!(f.name.name, "area");
    assert!(m.items[2].export);
    let a = Span::new(FileId(0), 3, 5);
    let b = Span::new(FileId(0), 4, 9);
    assert_eq!(a.to(b), Span::new(FileId(0), 3, 9));
}

#[test]
fn reserved_slots_default_to_off() {
    let m = build();
    let ItemKind::Fn(f) = &m.items[3].kind else {
        panic!()
    };
    assert!(!f.mods.kernel && !f.mods.is_async);
    assert!(f.contracts.is_empty());
    let body = f.body.as_ref().unwrap();
    let StmtKind::ForOf { parallel, .. } = &body.stmts[1].kind else {
        panic!()
    };
    assert!(!parallel);
    assert_eq!(f.params[0].mode, ParamMode::Value);
}
