//! Read-only AST visitor. Override the `visit_*` methods you care about and
//! call the matching `walk_*` function to continue into children, in source
//! order.

#![allow(clippy::wildcard_imports)]
use crate::ast::*;

pub trait Visitor<'a>: Sized {
    fn visit_module(&mut self, m: &'a Module) {
        walk_module(self, m);
    }
    fn visit_item(&mut self, i: &'a Item) {
        walk_item(self, i);
    }
    fn visit_attribute(&mut self, a: &'a Attribute) {
        walk_attribute(self, a);
    }
    fn visit_fn(&mut self, f: &'a FnDecl) {
        walk_fn(self, f);
    }
    fn visit_member(&mut self, m: &'a Member) {
        walk_member(self, m);
    }
    fn visit_param(&mut self, p: &'a Param) {
        walk_param(self, p);
    }
    fn visit_type_param(&mut self, p: &'a TypeParam) {
        walk_type_param(self, p);
    }
    fn visit_type(&mut self, t: &'a TypeExpr) {
        walk_type(self, t);
    }
    fn visit_pattern(&mut self, p: &'a Pattern) {
        walk_pattern(self, p);
    }
    fn visit_block(&mut self, b: &'a Block) {
        walk_block(self, b);
    }
    fn visit_stmt(&mut self, s: &'a Stmt) {
        walk_stmt(self, s);
    }
    fn visit_expr(&mut self, e: &'a Expr) {
        walk_expr(self, e);
    }
    fn visit_var_decl(&mut self, v: &'a VarDecl) {
        walk_var_decl(self, v);
    }
    fn visit_case(&mut self, c: &'a Case) {
        walk_case(self, c);
    }
    fn visit_arm(&mut self, a: &'a Arm) {
        walk_arm(self, a);
    }
    fn visit_lit(&mut self, _l: &'a Lit) {}
    fn visit_ident(&mut self, _i: &'a Ident) {}
}

pub fn walk_module<'a, V: Visitor<'a>>(v: &mut V, m: &'a Module) {
    for a in &m.attrs {
        v.visit_attribute(a);
    }
    for i in &m.items {
        v.visit_item(i);
    }
}

pub fn walk_attribute<'a, V: Visitor<'a>>(v: &mut V, a: &'a Attribute) {
    v.visit_ident(&a.name);
    for arg in &a.args {
        if let Some(k) = &arg.key {
            v.visit_ident(k);
        }
        match &arg.value {
            AttrValue::Lit(l) => v.visit_lit(l),
            AttrValue::Path(p) => p.iter().for_each(|i| v.visit_ident(i)),
        }
    }
}

pub fn walk_item<'a, V: Visitor<'a>>(v: &mut V, i: &'a Item) {
    for a in &i.attrs {
        v.visit_attribute(a);
    }
    match &i.kind {
        ItemKind::Import(d) => {
            match &d.clause {
                ImportClause::Named(specs) => {
                    for s in specs {
                        v.visit_ident(&s.name);
                        if let Some(a) = &s.alias {
                            v.visit_ident(a);
                        }
                    }
                }
                ImportClause::Namespace(n) => v.visit_ident(n),
            }
            v.visit_lit(&d.source);
        }
        ItemKind::Var(d) => v.visit_var_decl(d),
        ItemKind::Fn(f) => v.visit_fn(f),
        ItemKind::Record(r) => {
            v.visit_ident(&r.name);
            r.type_params.iter().for_each(|p| v.visit_type_param(p));
            r.implements.iter().for_each(|t| v.visit_type(t));
            r.members.iter().for_each(|m| v.visit_member(m));
        }
        ItemKind::Interface(d) => {
            v.visit_ident(&d.name);
            d.type_params.iter().for_each(|p| v.visit_type_param(p));
            d.extends.iter().for_each(|t| v.visit_type(t));
            d.methods.iter().for_each(|m| v.visit_fn(m));
        }
        ItemKind::Enum(d) => {
            v.visit_ident(&d.name);
            if let Some(r) = &d.repr {
                v.visit_type(r);
            }
            for var in &d.variants {
                v.visit_ident(&var.name);
                if let Some(e) = &var.value {
                    v.visit_expr(e);
                }
            }
        }
        ItemKind::TypeAlias(d) => {
            v.visit_ident(&d.name);
            d.type_params.iter().for_each(|p| v.visit_type_param(p));
            v.visit_type(&d.ty);
        }
        ItemKind::Law(d) => {
            v.visit_ident(&d.name);
            d.type_params.iter().for_each(|p| v.visit_type_param(p));
            d.params.iter().for_each(|p| v.visit_param(p));
            if let Some(w) = &d.when {
                v.visit_expr(w);
            }
            v.visit_block(&d.body);
        }
    }
}

pub fn walk_fn<'a, V: Visitor<'a>>(v: &mut V, f: &'a FnDecl) {
    v.visit_ident(&f.name);
    f.type_params.iter().for_each(|p| v.visit_type_param(p));
    f.params.iter().for_each(|p| v.visit_param(p));
    if let Some(r) = &f.ret {
        v.visit_type(r);
    }
    f.contracts.iter().for_each(|c| v.visit_expr(&c.expr));
    if let Some(b) = &f.body {
        v.visit_block(b);
    }
}

pub fn walk_member<'a, V: Visitor<'a>>(v: &mut V, m: &'a Member) {
    for a in &m.attrs {
        v.visit_attribute(a);
    }
    match &m.kind {
        MemberKind::Field(f) => {
            v.visit_ident(&f.name);
            v.visit_type(&f.ty);
            if let Some(e) = &f.init {
                v.visit_expr(e);
            }
        }
        MemberKind::Method(f) | MemberKind::Constructor(f) => v.visit_fn(f),
    }
}

pub fn walk_param<'a, V: Visitor<'a>>(v: &mut V, p: &'a Param) {
    v.visit_ident(&p.name);
    if let Some(t) = &p.ty {
        v.visit_type(t);
    }
    if let Some(d) = &p.default {
        v.visit_expr(d);
    }
}

pub fn walk_type_param<'a, V: Visitor<'a>>(v: &mut V, p: &'a TypeParam) {
    v.visit_ident(&p.name);
    if let Some(b) = &p.bound {
        v.visit_type(b);
    }
}

pub fn walk_type<'a, V: Visitor<'a>>(v: &mut V, t: &'a TypeExpr) {
    match &t.kind {
        TypeExprKind::Path { segments, args } => {
            for s in segments {
                v.visit_ident(s);
            }
            for a in args {
                v.visit_type(a);
            }
        }
        TypeExprKind::Array(e) | TypeExprKind::Paren(e) => v.visit_type(e),
        TypeExprKind::Union(ms) => ms.iter().for_each(|m| v.visit_type(m)),
        TypeExprKind::Fn { params, ret } => {
            for p in params {
                v.visit_ident(&p.name);
                v.visit_type(&p.ty);
            }
            v.visit_type(ret);
        }
        TypeExprKind::Lit(l) => v.visit_lit(l),
        TypeExprKind::Object(fields) => {
            for f in fields {
                v.visit_ident(&f.name);
                v.visit_type(&f.ty);
            }
        }
        TypeExprKind::Null => {}
    }
}

pub fn walk_pattern<'a, V: Visitor<'a>>(v: &mut V, p: &'a Pattern) {
    match &p.kind {
        PatternKind::Ident(i) => v.visit_ident(i),
        PatternKind::Wildcard => {}
        PatternKind::Object(fields) => {
            for f in fields {
                v.visit_ident(&f.key);
                if let Some(p) = &f.pattern {
                    v.visit_pattern(p);
                }
            }
        }
    }
}

pub fn walk_var_decl<'a, V: Visitor<'a>>(v: &mut V, d: &'a VarDecl) {
    v.visit_pattern(&d.pattern);
    if let Some(t) = &d.ty {
        v.visit_type(t);
    }
    if let Some(e) = &d.init {
        v.visit_expr(e);
    }
}

pub fn walk_block<'a, V: Visitor<'a>>(v: &mut V, b: &'a Block) {
    b.stmts.iter().for_each(|s| v.visit_stmt(s));
    if let Some(t) = &b.tail {
        v.visit_expr(t);
    }
}

pub fn walk_stmt<'a, V: Visitor<'a>>(v: &mut V, s: &'a Stmt) {
    for a in &s.attrs {
        v.visit_attribute(a);
    }
    match &s.kind {
        StmtKind::Var(d) => v.visit_var_decl(d),
        StmtKind::Expr(e) => v.visit_expr(e),
        StmtKind::If { cond, then, els } => {
            v.visit_expr(cond);
            v.visit_stmt(then);
            if let Some(e) = els {
                v.visit_stmt(e);
            }
        }
        StmtKind::While { cond, body } => {
            v.visit_expr(cond);
            v.visit_stmt(body);
        }
        StmtKind::DoWhile { body, cond } => {
            v.visit_stmt(body);
            v.visit_expr(cond);
        }
        StmtKind::For {
            init,
            cond,
            update,
            body,
            ..
        } => {
            if let Some(i) = init {
                v.visit_stmt(i);
            }
            if let Some(c) = cond {
                v.visit_expr(c);
            }
            if let Some(u) = update {
                v.visit_expr(u);
            }
            v.visit_stmt(body);
        }
        StmtKind::ForOf {
            binding,
            iter,
            body,
            ..
        } => {
            v.visit_pattern(binding);
            v.visit_expr(iter);
            v.visit_stmt(body);
        }
        StmtKind::Switch(sw) => {
            v.visit_expr(&sw.scrutinee);
            sw.cases.iter().for_each(|c| v.visit_case(c));
        }
        StmtKind::Return(e) => {
            if let Some(e) = e {
                v.visit_expr(e);
            }
        }
        StmtKind::Break(l) | StmtKind::Continue(l) => {
            if let Some(l) = l {
                v.visit_ident(l);
            }
        }
        StmtKind::Block(b) => v.visit_block(b),
        StmtKind::Labeled { label, body } => {
            v.visit_ident(label);
            v.visit_stmt(body);
        }
        StmtKind::Item(i) => v.visit_item(i),
        StmtKind::Empty => {}
    }
}

pub fn walk_case<'a, V: Visitor<'a>>(v: &mut V, c: &'a Case) {
    c.labels.iter().for_each(|l| v.visit_expr(l));
    c.body.iter().for_each(|s| v.visit_stmt(s));
}

pub fn walk_arm<'a, V: Visitor<'a>>(v: &mut V, a: &'a Arm) {
    a.labels.iter().for_each(|l| v.visit_expr(l));
    v.visit_expr(&a.body);
}

pub fn walk_expr<'a, V: Visitor<'a>>(v: &mut V, e: &'a Expr) {
    match &e.kind {
        ExprKind::Lit(l) => v.visit_lit(l),
        ExprKind::Template(parts) => {
            for p in parts {
                if let TemplatePart::Expr(e) = p {
                    v.visit_expr(e);
                }
            }
        }
        ExprKind::Ident(i) => v.visit_ident(i),
        ExprKind::This => {}
        ExprKind::Array(xs) => xs.iter().for_each(|x| v.visit_expr(x)),
        ExprKind::Object(fields) => {
            for f in fields {
                v.visit_ident(&f.name);
                if let Some(val) = &f.value {
                    v.visit_expr(val);
                }
            }
        }
        ExprKind::Unary { expr, .. } | ExprKind::Try(expr) | ExprKind::Paren(expr) => {
            v.visit_expr(expr);
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            v.visit_expr(lhs);
            v.visit_expr(rhs);
        }
        ExprKind::Assign { target, value, .. } => {
            v.visit_expr(target);
            v.visit_expr(value);
        }
        ExprKind::Update { target, .. } => v.visit_expr(target),
        ExprKind::Call {
            callee,
            type_args,
            args,
            ..
        } => {
            v.visit_expr(callee);
            for t in type_args {
                v.visit_type(t);
            }
            for a in args {
                v.visit_expr(a);
            }
        }
        ExprKind::New { ty, args } => {
            v.visit_type(ty);
            for a in args {
                v.visit_expr(a);
            }
        }
        ExprKind::Member { obj, name, .. } => {
            v.visit_expr(obj);
            v.visit_ident(name);
        }
        ExprKind::Index { obj, index, .. } => {
            v.visit_expr(obj);
            v.visit_expr(index);
        }
        ExprKind::Cast { expr, ty, .. } => {
            v.visit_expr(expr);
            v.visit_type(ty);
        }
        ExprKind::Cond { cond, then, els } => {
            v.visit_expr(cond);
            v.visit_expr(then);
            v.visit_expr(els);
        }
        ExprKind::Arrow(a) => {
            a.params.iter().for_each(|p| v.visit_param(p));
            if let Some(r) = &a.ret {
                v.visit_type(r);
            }
            match &a.body {
                ArrowBody::Expr(e) => v.visit_expr(e),
                ArrowBody::Block(b) => v.visit_block(b),
            }
        }
        ExprKind::Switch(sw) => {
            v.visit_expr(&sw.scrutinee);
            sw.arms.iter().for_each(|a| v.visit_arm(a));
        }
    }
}
