//! Iterative `Drop` for the node kinds that can form very long chains.
//!
//! The parser builds `a + b + c + ...`, `a.b.c.d`, `f()()()`, `a ? b : c ? d : e`
//! and `if .. else if ..` ladders iteratively, up to [`crate::MAX_HEIGHT`] nodes
//! high. The compiler-generated drop glue would recurse once per link, using
//! about 100 bytes of the caller's stack each (tens of KB for a long chain,
//! more in debug builds). These impls detach the children of a node onto an
//! explicit heap stack first, so dropping any tree uses constant native stack
//! along such chains.
//!
//! Consequence for consumers: `Expr` and `Stmt` implement `Drop`, so their
//! fields cannot be moved out by destructuring (`let Expr { kind, .. } = e`
//! is rejected by the compiler); take them with `std::mem::replace` or
//! `std::mem::take`, or match on references.

use crate::ast::*;

fn placeholder() -> Expr {
    Expr {
        id: NodeId(0),
        span: Span::DUMMY,
        kind: ExprKind::This,
    }
}

fn is_leaf(e: &Expr) -> bool {
    matches!(
        e.kind,
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::This
    )
}

fn take_box(b: &mut Box<Expr>, out: &mut Vec<Expr>) {
    if !is_leaf(b) {
        out.push(std::mem::replace(&mut **b, placeholder()));
    }
}

fn take_vec(v: &mut Vec<Expr>, out: &mut Vec<Expr>) {
    out.extend(std::mem::take(v).into_iter().filter(|e| !is_leaf(e)));
}

/// Move the expression children of `kind` (not its types or blocks) into `out`.
fn take_children(kind: &mut ExprKind, out: &mut Vec<Expr>) {
    match kind {
        ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::This => {}
        ExprKind::Template(parts) => {
            for p in parts.iter_mut() {
                if let TemplatePart::Expr(e) = p {
                    if !is_leaf(e) {
                        out.push(std::mem::replace(e, placeholder()));
                    }
                }
            }
        }
        ExprKind::Array(v) => take_vec(v, out),
        ExprKind::Object(fs) => {
            for f in fs.iter_mut() {
                if let Some(e) = &mut f.value {
                    if !is_leaf(e) {
                        out.push(std::mem::replace(e, placeholder()));
                    }
                }
            }
        }
        ExprKind::Unary { expr, .. } | ExprKind::Cast { expr, .. } | ExprKind::Try(expr) => {
            take_box(expr, out);
        }
        ExprKind::Paren(expr) => take_box(expr, out),
        ExprKind::Binary { lhs, rhs, .. } => {
            take_box(lhs, out);
            take_box(rhs, out);
        }
        ExprKind::Assign { target, value, .. } => {
            take_box(target, out);
            take_box(value, out);
        }
        ExprKind::Update { target, .. } => take_box(target, out),
        ExprKind::Call { callee, args, .. } => {
            take_box(callee, out);
            take_vec(args, out);
        }
        ExprKind::New { args, .. } => take_vec(args, out),
        ExprKind::Member { obj, .. } => take_box(obj, out),
        ExprKind::Index { obj, index, .. } => {
            take_box(obj, out);
            take_box(index, out);
        }
        ExprKind::Cond { cond, then, els } => {
            take_box(cond, out);
            take_box(then, out);
            take_box(els, out);
        }
        ExprKind::Arrow(a) => {
            if let ArrowBody::Expr(b) = &mut a.body {
                take_box(b, out);
            }
        }
        ExprKind::Switch(s) => {
            take_box(&mut s.scrutinee, out);
            for arm in &mut s.arms {
                take_vec(&mut arm.labels, out);
                if !is_leaf(&arm.body) {
                    out.push(std::mem::replace(&mut arm.body, placeholder()));
                }
            }
        }
    }
}

impl Drop for Expr {
    fn drop(&mut self) {
        let mut stack: Vec<Expr> = Vec::new();
        take_children(&mut self.kind, &mut stack);
        while let Some(mut e) = stack.pop() {
            take_children(&mut e.kind, &mut stack);
        }
    }
}

impl Drop for Stmt {
    fn drop(&mut self) {
        // `if .. else if .. else if ..` ladders: unlink the `else` chain.
        if let StmtKind::If { els, .. } = &mut self.kind {
            let mut next = els.take();
            while let Some(mut s) = next {
                next = match &mut s.kind {
                    StmtKind::If { els, .. } => els.take(),
                    _ => None,
                };
            }
        }
    }
}
