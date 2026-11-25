//! Expressions (`grammar.md` 3.3 and the disambiguation rules of 3.4).

use super::*;

#[derive(Clone, Copy)]
enum BinKind {
    Op(BinOp),
    As,
    /// `==` / `!=`: reported (`OXS-E0005`) and parsed as `===` / `!==`.
    Loose(BinOp),
    Instanceof,
}

fn is_cmp(op: BinOp) -> bool {
    matches!(op, BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
}
fn is_eq(op: BinOp) -> bool {
    matches!(op, BinOp::Eq | BinOp::Ne)
}
fn is_bitwise(op: BinOp) -> bool {
    matches!(op, BinOp::BitAnd | BinOp::BitOr | BinOp::BitXor)
}
fn is_additive(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Add | BinOp::Sub | BinOp::AddWrap | BinOp::SubWrap | BinOp::AddSat | BinOp::SubSat
    )
}

pub fn op_text(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::AddWrap => "+%",
        BinOp::SubWrap => "-%",
        BinOp::MulWrap => "*%",
        BinOp::AddSat => "+|",
        BinOp::SubSat => "-|",
        BinOp::MulSat => "*|",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::Eq => "===",
        BinOp::Ne => "!==",
        BinOp::And => "&&",
        BinOp::Or => "||",
        BinOp::Coalesce => "??",
    }
}

fn binop_of(e: &Expr) -> Option<BinOp> {
    match &e.kind {
        ExprKind::Binary { op, .. } => Some(*op),
        _ => None,
    }
}

/// Does the expression end in an unparenthesised `as T` (so a following `?` cannot attach)?
fn ends_with_cast(e: &Expr) -> bool {
    let mut cur = e;
    loop {
        match &cur.kind {
            ExprKind::Cast { .. } => return true,
            ExprKind::Binary { rhs, .. } => cur = rhs,
            ExprKind::Assign { value, .. } => cur = value,
            ExprKind::Cond { els, .. } => cur = els,
            ExprKind::Unary { expr, .. } => cur = expr,
            _ => return false,
        }
    }
}

/// Could this expression also be read as a (member path or literal) type?
fn type_like(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Ident(_) | ExprKind::Lit(_) => true,
        ExprKind::Member {
            obj,
            optional: false,
            ..
        } => type_like(obj),
        _ => false,
    }
}

fn is_place(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Ident(_)
        | ExprKind::Member {
            optional: false, ..
        }
        | ExprKind::Index {
            optional: false, ..
        } => true,
        ExprKind::Paren(inner) => is_place(inner),
        _ => false,
    }
}

impl Parser<'_> {
    pub(super) fn error_expr(&mut self) -> Expr {
        let id = self.error_ident();
        let sp = id.span;
        self.mk_expr_span(ExprKind::Ident(id), sp)
    }

    /// Run `f` with the arrow/ternary context flags cleared (inside brackets).
    fn bracketed<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let saved = (self.no_arrow, self.then_ctx);
        self.no_arrow = false;
        self.then_ctx = false;
        let r = f(self);
        self.no_arrow = saved.0;
        self.then_ctx = saved.1;
        r
    }

    /// `Expr = Assign`
    pub(super) fn parse_expr(&mut self) -> Expr {
        self.parse_assign()
    }

    fn assign_op(&self) -> Option<Option<BinOp>> {
        Some(match self.kind() {
            Tk::Assign => None,
            Tk::PlusEq => Some(BinOp::Add),
            Tk::MinusEq => Some(BinOp::Sub),
            Tk::StarEq => Some(BinOp::Mul),
            Tk::SlashEq => Some(BinOp::Div),
            Tk::PercentEq => Some(BinOp::Rem),
            Tk::PlusWrapEq => Some(BinOp::AddWrap),
            Tk::MinusWrapEq => Some(BinOp::SubWrap),
            Tk::StarWrapEq => Some(BinOp::MulWrap),
            Tk::PlusSatEq => Some(BinOp::AddSat),
            Tk::MinusSatEq => Some(BinOp::SubSat),
            Tk::StarSatEq => Some(BinOp::MulSat),
            Tk::AmpEq => Some(BinOp::BitAnd),
            Tk::PipeEq => Some(BinOp::BitOr),
            Tk::CaretEq => Some(BinOp::BitXor),
            Tk::ShlEq => Some(BinOp::Shl),
            Tk::ShrEq => Some(BinOp::Shr),
            _ => return None,
        })
    }

    #[inline(never)]
    fn parse_assign(&mut self) -> Expr {
        if !self.enter() {
            self.leave();
            return self.error_expr();
        }
        let r = self.parse_assign_body();
        self.leave();
        r
    }

    #[inline(never)]
    fn parse_assign_body(&mut self) -> Expr {
        let lo = self.cur_lo();
        let lhs = self.parse_cond();
        if let Some(op) = self.assign_op() {
            self.bump();
            if !is_place(&lhs) && !matches!(&lhs.kind, ExprKind::Ident(i) if i.name.is_empty()) {
                self.error(
                    "OXS-E0015",
                    lhs.span,
                    "the left side of an assignment must be a variable, a field (`a.b`) or an element (`a[i]`)",
                );
            }
            let rhs = self.parse_assign();
            self.mk_expr(
                ExprKind::Assign {
                    op,
                    target: Box::new(lhs),
                    value: Box::new(rhs),
                },
                lo,
            )
        } else {
            lhs
        }
    }

    #[inline(never)]
    fn parse_cond(&mut self) -> Expr {
        let lo = self.cur_lo();
        let cond = self.parse_binary(1);
        if self.at(Tk::Question) && self.cur_ws() {
            return self.parse_ternary_rest(cond, lo, false);
        }
        cond
    }

    /// At a `?` that starts a conditional: `cond ? a : b`.
    #[inline(never)]
    fn parse_ternary_rest(&mut self, cond: Expr, lo: u32, quiet: bool) -> Expr {
        // `a ? b : c ? d : e ...` is parsed iteratively (no recursion per
        // branch) and folded to the right afterwards.
        let mut parts: Vec<(Expr, u32, Expr)> = Vec::new();
        let (mut cond, mut lo, mut quiet) = (cond, lo, quiet);
        let els = loop {
            let q = self.cur_span();
            let prev_end = self.last_end.min(q.lo);
            self.bump();
            if matches!(
                self.kind(),
                Tk::Semi | Tk::RParen | Tk::RBracket | Tk::RBrace | Tk::Comma | Tk::Colon | Tk::Eof
            ) {
                if !quiet {
                    // `f() ?;`: removing the space makes it the try operator
                    let ws = Span::new(q.file, prev_end, q.hi);
                    if ends_with_cast(&cond) {
                        // `x as T?` is not valid either: no mechanical fix
                        self.error(
                            "OXS-E0006",
                            q,
                            "`?` after whitespace is the conditional operator and needs `a ? b : c`",
                        );
                    } else {
                        self.report_bare_question(
                            q,
                            "needs `a ? b : c`",
                            ws,
                            "remove the space before `?` (try operator)",
                            "?",
                        );
                    }
                }
                break cond;
            }
            let saved = self.then_ctx;
            self.then_ctx = true;
            let then = self.parse_assign();
            self.then_ctx = saved;
            if !self.eat(Tk::Colon) {
                if !quiet {
                    // `a ? b` without `:`: the conditional is incomplete; the fix
                    // removes the dangling `? b` (removing only the space would
                    // turn `b` into stray code)
                    let dangling = Span::new(q.file, prev_end, then.span.hi.max(q.hi));
                    self.report_bare_question(
                        q,
                        "needs a matching `:`",
                        dangling,
                        "remove the incomplete `? ...` part",
                        "",
                    );
                }
                let els = self.error_expr();
                break self.mk_expr(
                    ExprKind::Cond {
                        cond: Box::new(cond),
                        then: Box::new(then),
                        els: Box::new(els),
                    },
                    lo,
                );
            }
            let lo2 = self.cur_lo();
            let next = self.parse_binary(1);
            parts.push((cond, lo, then));
            cond = next;
            lo = lo2;
            quiet = false;
            if !(self.at(Tk::Question) && self.cur_ws()) {
                break cond;
            }
        };
        let mut els = els;
        while let Some((c, l, t)) = parts.pop() {
            els = self.mk_expr(
                ExprKind::Cond {
                    cond: Box::new(c),
                    then: Box::new(t),
                    els: Box::new(els),
                },
                l,
            );
        }
        els
    }

    /// `OXS-E0006` for a `?` after whitespace that is not a complete conditional.
    /// The fix removes the whitespace before `?` (making it the try operator).
    #[inline(never)]
    fn report_bare_question(
        &mut self,
        q: Span,
        what: &str,
        fix_span: Span,
        fix_title: &str,
        replacement: &str,
    ) {
        self.diag(
            Diagnostic::new(
                "OXS-E0006",
                q,
                format!(
                    "`?` after whitespace is the conditional operator and {what}; \
                     the try operator is written directly after its operand, as in `f()?`"
                ),
            )
            .with_fix(Fix::replace(fix_title, fix_span, replacement)),
        );
    }

    fn binop_info(&self) -> Option<(BinKind, u8)> {
        use BinOp as B;
        Some(match self.kind() {
            Tk::QQ => (BinKind::Op(B::Coalesce), 1),
            Tk::PipePipe => (BinKind::Op(B::Or), 2),
            Tk::AmpAmp => (BinKind::Op(B::And), 3),
            Tk::Pipe => (BinKind::Op(B::BitOr), 4),
            Tk::Caret => (BinKind::Op(B::BitXor), 5),
            Tk::Amp => (BinKind::Op(B::BitAnd), 6),
            Tk::EqEqEq => (BinKind::Op(B::Eq), 7),
            Tk::NotEqEq => (BinKind::Op(B::Ne), 7),
            Tk::EqEq => (BinKind::Loose(B::Eq), 7),
            Tk::NotEq => (BinKind::Loose(B::Ne), 7),
            Tk::Lt => (BinKind::Op(B::Lt), 8),
            Tk::Le => (BinKind::Op(B::Le), 8),
            Tk::Gt => (BinKind::Op(B::Gt), 8),
            Tk::Ge => (BinKind::Op(B::Ge), 8),
            Tk::Kw(Kw::As) => (BinKind::As, 8),
            Tk::Reserved if self.cur_text() == "instanceof" => (BinKind::Instanceof, 8),
            Tk::Shl => (BinKind::Op(B::Shl), 9),
            Tk::Shr => (BinKind::Op(B::Shr), 9),
            Tk::Plus => (BinKind::Op(B::Add), 10),
            Tk::Minus => (BinKind::Op(B::Sub), 10),
            Tk::PlusWrap => (BinKind::Op(B::AddWrap), 10),
            Tk::MinusWrap => (BinKind::Op(B::SubWrap), 10),
            Tk::PlusSat => (BinKind::Op(B::AddSat), 10),
            Tk::MinusSat => (BinKind::Op(B::SubSat), 10),
            Tk::Star => (BinKind::Op(B::Mul), 11),
            Tk::Slash => (BinKind::Op(B::Div), 11),
            Tk::Percent => (BinKind::Op(B::Rem), 11),
            Tk::StarWrap => (BinKind::Op(B::MulWrap), 11),
            Tk::StarSat => (BinKind::Op(B::MulSat), 11),
            _ => return None,
        })
    }

    #[inline(never)]
    pub(super) fn parse_binary(&mut self, min: u8) -> Expr {
        let lo = self.cur_lo();
        let mut lhs = self.parse_unary();
        loop {
            let Some((k, prec)) = self.binop_info() else {
                break;
            };
            if prec < min {
                break;
            }
            match k {
                BinKind::As => {
                    self.bump();
                    let ty = if self.enter() {
                        let t = self.parse_array_type();
                        self.leave();
                        t
                    } else {
                        self.leave();
                        self.error_type()
                    };
                    let ty_span = ty.span;
                    lhs = self.mk_expr(
                        ExprKind::Cast {
                            expr: Box::new(lhs),
                            ty,
                        },
                        lo,
                    );
                    if self.at(Tk::Pipe) {
                        lhs = self.cast_then_pipe(lhs, ty_span, lo, min);
                    }
                }
                BinKind::Instanceof => {
                    let sp = self.cur_span();
                    self.reserved_error("instanceof", sp);
                    self.bump();
                    let _ = self.parse_binary(prec + 1);
                }
                BinKind::Op(_) | BinKind::Loose(_) => {
                    let (op, loose) = match k {
                        BinKind::Op(op) => (op, false),
                        BinKind::Loose(op) => (op, true),
                        _ => unreachable!(),
                    };
                    let tok_span = self.cur_span();
                    if loose {
                        let want = if op == BinOp::Eq { "===" } else { "!==" };
                        let had = if op == BinOp::Eq { "==" } else { "!=" };
                        self.diag(
                            Diagnostic::new(
                                "OXS-E0005",
                                tok_span,
                                format!("`{had}` does not exist in OxiScript; use `{want}`"),
                            )
                            .with_fix(Fix::replace(
                                format!("use `{want}`"),
                                tok_span,
                                want,
                            )),
                        );
                    }
                    self.bump();
                    let rhs = self.parse_binary(prec + 1);
                    lhs = self.build_binary(op, lhs, rhs, lo, tok_span);
                }
            }
        }
        lhs
    }

    /// `x as u8 | 1` (grammar 3.4 rule 8): report `OXS-E0010` with both fixes
    /// and continue with the C reading `(x as u8) | 1`.
    fn cast_then_pipe(&mut self, cast: Expr, ty_span: Span, lo: u32, min: u8) -> Expr {
        let pipe = self.cur_span();
        let cast_span = cast.span;
        if min > 4 {
            self.diag(
                Diagnostic::new(
                    "OXS-E0010",
                    pipe,
                    "`as T | ...` could be a cast to a union or a cast followed by bitwise or; add parentheses",
                )
                .with_fix(Fix::parenthesise("write `(x as T) | ...`", cast_span)),
            );
            return cast;
        }
        self.bump();
        let rhs = self.parse_binary(5);
        let rhs_span = rhs.span;
        let mut d = Diagnostic::new(
            "OXS-E0010",
            pipe,
            "`as T | ...` could be a cast to a union or a cast followed by bitwise or; add parentheses",
        )
        .with_fix(Fix::parenthesise("write `(x as T) | ...`", cast_span));
        if type_like(&rhs) {
            // the second reading is only offered when the right side can be a type
            d = d.with_fix(Fix::parenthesise(
                "write `x as (T | ...)`",
                Span::new(self.file, ty_span.lo, rhs_span.hi),
            ));
        }
        self.diag(d);
        self.build_binary(BinOp::BitOr, cast, rhs, lo, pipe)
    }

    fn build_binary(&mut self, op: BinOp, lhs: Expr, rhs: Expr, lo: u32, tok: Span) -> Expr {
        // non-associative comparison chains
        if is_cmp(op) || is_eq(op) {
            if let Some(lop) = binop_of(&lhs) {
                if (is_cmp(op) && is_cmp(lop)) || (is_eq(op) && is_eq(lop)) {
                    self.diag(
                        Diagnostic::new(
                            "OXS-E0001",
                            tok,
                            format!(
                                "`{}` cannot be chained with `{}`; comparisons are not associative, parenthesise one side",
                                op_text(op),
                                op_text(lop)
                            ),
                        )
                        .with_fix(Fix::parenthesise("parenthesise the left comparison", lhs.span)),
                    );
                }
            }
        }
        let mut offenders: Vec<(Span, BinOp)> = Vec::new();
        let code = if is_bitwise(op) {
            for operand in [&lhs, &rhs] {
                if let Some(o) = binop_of(operand) {
                    if is_cmp(o) || is_eq(o) || (is_bitwise(o) && o != op) {
                        offenders.push((operand.span, o));
                    }
                }
            }
            "OXS-E0007"
        } else if matches!(op, BinOp::Shl | BinOp::Shr) {
            for operand in [&lhs, &rhs] {
                if let Some(o) = binop_of(operand) {
                    if is_additive(o) {
                        offenders.push((operand.span, o));
                    }
                }
            }
            "OXS-E0008"
        } else if op == BinOp::Coalesce {
            for operand in [&lhs, &rhs] {
                if let Some(o) = binop_of(operand) {
                    if matches!(o, BinOp::And | BinOp::Or) {
                        offenders.push((operand.span, o));
                    }
                }
            }
            "OXS-E0009"
        } else {
            ""
        };
        if !offenders.is_empty() {
            let mut d = Diagnostic::new(
                code,
                tok,
                format!(
                    "`{}` mixed with `{}` without parentheses is ambiguous to readers; parenthesise the intended operands",
                    op_text(op),
                    op_text(offenders[0].1)
                ),
            );
            // one fix that parenthesises every offending operand (applying only
            // one of two would leave the other mix in place)
            let mut edits = Vec::new();
            for (sp, _) in &offenders {
                edits.extend(Fix::parenthesise("", *sp).edits);
            }
            d = d.with_fix(Fix {
                title: "add parentheses around the mixed operands".into(),
                edits,
            });
            self.diag(d);
        }
        self.mk_expr(
            ExprKind::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            },
            lo,
        )
    }

    #[inline(never)]
    fn parse_unary(&mut self) -> Expr {
        let lo = self.cur_lo();
        match self.kind() {
            Tk::Minus | Tk::Bang | Tk::Tilde => {
                let op = match self.kind() {
                    Tk::Minus => UnOp::Neg,
                    Tk::Bang => UnOp::Not,
                    _ => UnOp::BitNot,
                };
                self.bump();
                if !self.enter() {
                    self.leave();
                    return self.error_expr();
                }
                let e = self.parse_unary();
                self.leave();
                self.mk_expr(
                    ExprKind::Unary {
                        op,
                        expr: Box::new(e),
                    },
                    lo,
                )
            }
            Tk::PlusPlus | Tk::MinusMinus => {
                let op = if self.at(Tk::PlusPlus) {
                    IncDec::Inc
                } else {
                    IncDec::Dec
                };
                self.bump();
                if !self.enter() {
                    self.leave();
                    return self.error_expr();
                }
                let e = self.parse_unary();
                self.leave();
                self.make_update(op, true, e, lo)
            }
            Tk::Reserved
                if matches!(
                    self.cur_text(),
                    "await" | "typeof" | "delete" | "yield" | "spawn" | "move"
                ) && !matches!(
                    self.nth(1),
                    Tk::Semi | Tk::RParen | Tk::Comma | Tk::Dot | Tk::Eof
                ) =>
            {
                let w = self.cur_text();
                let sp = self.cur_span();
                self.reserved_error(w, sp);
                self.bump();
                if !self.enter() {
                    self.leave();
                    return self.error_expr();
                }
                let e = self.parse_unary();
                self.leave();
                e
            }
            _ => self.parse_postfix(),
        }
    }

    fn make_update(&mut self, op: IncDec, prefix: bool, target: Expr, lo: u32) -> Expr {
        if !is_place(&target) && !matches!(&target.kind, ExprKind::Ident(i) if i.name.is_empty()) {
            self.error(
                "OXS-E0015",
                target.span,
                "`++` and `--` need a variable, a field or an element as operand",
            );
        }
        let e = self.mk_expr(
            ExprKind::Update {
                op,
                prefix,
                target: Box::new(target),
            },
            lo,
        );
        self.updates.push((e.id, e.span));
        e
    }

    /// Postfix operators. The loop is deliberately tiny: every operator is
    /// handled by an out-of-line helper so the recursive path (this function is
    /// on it for every nesting level) keeps a small stack frame.
    #[inline(never)]
    fn parse_postfix(&mut self) -> Expr {
        let lo = self.cur_lo();
        let mut e = self.parse_primary();
        loop {
            let next = match self.kind() {
                Tk::Dot => self.pf_member(e, lo, false),
                Tk::QDot => self.pf_qdot(e, lo),
                Tk::LBracket => self.pf_index(e, lo, false),
                Tk::LParen => self.pf_call(e, lo, Vec::new(), false),
                Tk::Lt => self.pf_generic(e, lo),
                Tk::Question if !self.cur_ws() => self.pf_try(e, lo),
                Tk::PlusPlus | Tk::MinusMinus => self.pf_update(e, lo),
                Tk::Float if self.at_dot_number() => Ok(self.pf_dot_number(e, lo)),
                _ => break,
            };
            match next {
                Ok(n) => e = n,
                Err(done) => return done,
            }
        }
        e
    }

    #[inline(never)]
    fn pf_member(&mut self, e: Expr, lo: u32, optional: bool) -> Result<Expr, Expr> {
        if !optional {
            self.bump();
        }
        let name = self.parse_ident("a member name");
        Ok(self.mk_expr(
            ExprKind::Member {
                obj: Box::new(e),
                name,
                optional,
            },
            lo,
        ))
    }

    #[inline(never)]
    fn pf_qdot(&mut self, e: Expr, lo: u32) -> Result<Expr, Expr> {
        self.bump();
        match self.kind() {
            Tk::LBracket => self.pf_index(e, lo, true),
            Tk::LParen => self.pf_call(e, lo, Vec::new(), true),
            _ => self.pf_member(e, lo, true),
        }
    }

    #[inline(never)]
    fn pf_index(&mut self, e: Expr, lo: u32, optional: bool) -> Result<Expr, Expr> {
        self.bump();
        let idx = self.bracketed(Self::parse_expr);
        self.expect(Tk::RBracket, "`]`");
        Ok(self.mk_expr(
            ExprKind::Index {
                obj: Box::new(e),
                index: Box::new(idx),
                optional,
            },
            lo,
        ))
    }

    #[inline(never)]
    fn pf_call(
        &mut self,
        e: Expr,
        lo: u32,
        type_args: Vec<TypeExpr>,
        optional: bool,
    ) -> Result<Expr, Expr> {
        let args = self.parse_call_args();
        Ok(self.mk_expr(
            ExprKind::Call {
                callee: Box::new(e),
                type_args,
                args,
                optional,
            },
            lo,
        ))
    }

    #[inline(never)]
    fn pf_generic(&mut self, e: Expr, lo: u32) -> Result<Expr, Expr> {
        match self.try_generic_args() {
            Some(type_args) => self.pf_call(e, lo, type_args, false),
            None => Err(e),
        }
    }

    /// `?` glued to its operand: the try operator, unless it is plainly a
    /// conditional written without spaces (`a?b:c`).
    #[inline(never)]
    fn pf_try(&mut self, e: Expr, lo: u32) -> Result<Expr, Expr> {
        let q = self.cur_span();
        let next = self.nth(1);
        let operand_follows = !self.nth_tok(1).ws
            && matches!(
                next,
                Tk::Ident
                    | Tk::Int
                    | Tk::Float
                    | Tk::Str
                    | Tk::TemplateFull
                    | Tk::TemplateHead
                    | Tk::Kw(Kw::True | Kw::False | Kw::Null | Kw::This | Kw::New)
            );
        if operand_follows {
            self.diag(
                Diagnostic::new(
                    "OXS-E0006",
                    q,
                    "`?` directly after an operand is the try operator; \
                     write the conditional operator as `a ? b : c` with a space before `?`",
                )
                .with_fix(Fix::insert("insert a space before `?`", q, " ")),
            );
            return Err(self.parse_ternary_rest(e, lo, true));
        }
        self.bump();
        Ok(self.mk_expr(ExprKind::Try(Box::new(e)), lo))
    }

    #[inline(never)]
    fn pf_update(&mut self, e: Expr, lo: u32) -> Result<Expr, Expr> {
        let op = if self.at(Tk::PlusPlus) {
            IncDec::Inc
        } else {
            IncDec::Dec
        };
        self.bump();
        Ok(self.make_update(op, false, e, lo))
    }

    fn at_dot_number(&self) -> bool {
        !self.cur_ws()
            && self.cur_text().len() > 1
            && self.cur_text().starts_with('.')
            && self.cur_text()[1..].bytes().all(|b| b.is_ascii_digit())
    }

    /// `t.0`: lexed as `t` and the literal `.0`.
    #[inline(never)]
    fn pf_dot_number(&mut self, e: Expr, lo: u32) -> Expr {
        let sp = self.cur_span();
        let digits = self.cur_text()[1..].to_string();
        self.diag(
            Diagnostic::new(
                "OXS-E0001",
                sp,
                "field access by number does not exist (tuples are reserved); index with `t[0]`",
            )
            .with_fix(Fix::replace(
                format!("write `[{digits}]`"),
                sp,
                format!("[{digits}]"),
            )),
        );
        let t = self.bump();
        let lit_span = mk_span(self.file, t.lo as usize + 1, t.hi as usize);
        let value = digits.parse::<u128>().unwrap_or(0);
        let l = Lit {
            id: self.nid(),
            span: lit_span,
            kind: LitKind::Int {
                value,
                suffix: None,
            },
        };
        let idx = self.mk_expr_span(ExprKind::Lit(l), lit_span);
        self.mk_expr(
            ExprKind::Index {
                obj: Box::new(e),
                index: Box::new(idx),
                optional: false,
            },
            lo,
        )
    }

    /// At `<` after an expression: a speculative parse of `"<" TypeList ">"`
    /// that succeeds only when `(` follows (grammar 3.4 rule 1). On success the
    /// type arguments are returned and the cursor is at `(`; otherwise
    /// everything is rewound.
    fn try_generic_args(&mut self) -> Option<Vec<TypeExpr>> {
        if !matches!(
            self.nth(1),
            Tk::Ident
                | Tk::LParen
                | Tk::LBrace
                | Tk::Str
                | Tk::Int
                | Tk::Pipe
                | Tk::Kw(Kw::Null | Kw::True | Kw::False)
        ) {
            return None;
        }
        if !self.generic_scan() {
            return None;
        }
        let snap = self.snapshot();
        self.spec += 1;
        self.bump(); // <
        let args = self.parse_type_list();
        let ok = !self.spec_fail && self.eat_gt() && self.at(Tk::LParen);
        self.spec -= 1;
        if ok {
            Some(args)
        } else {
            self.restore(&snap);
            None
        }
    }

    /// Cheap pre-check before speculating on `<` as type arguments: scan at most
    /// `GENERIC_SCAN` type-like tokens for the matching `>` followed by `(`. This
    /// bounds the cost of every `<` (so long comparison lists stay linear) and
    /// is the documented limit on the length of a generic call's type-argument
    /// list: longer lists are read as comparisons.
    fn generic_scan(&self) -> bool {
        const GENERIC_SCAN: usize = 128;
        let mut angle = 1i32;
        let mut nest = 0i32;
        for k in 1..=GENERIC_SCAN {
            match self.nth(k) {
                Tk::Lt => angle += 1,
                Tk::Gt | Tk::Shr => {
                    angle -= if self.nth(k) == Tk::Gt { 1 } else { 2 };
                    if angle <= 0 {
                        return angle == 0 && self.nth(k + 1) == Tk::LParen;
                    }
                }
                Tk::LParen | Tk::LBracket | Tk::LBrace => nest += 1,
                Tk::RParen | Tk::RBracket | Tk::RBrace => {
                    nest -= 1;
                    if nest < 0 {
                        return false;
                    }
                }
                Tk::Ident
                | Tk::Reserved
                | Tk::Dot
                | Tk::Comma
                | Tk::Pipe
                | Tk::Colon
                | Tk::Semi
                | Tk::FatArrow
                | Tk::Str
                | Tk::Int
                | Tk::Kw(Kw::Null | Kw::True | Kw::False | Kw::Readonly) => {}
                _ => return false,
            }
        }
        false
    }

    /// `( [Args] )`
    #[inline(never)]
    pub(super) fn parse_call_args(&mut self) -> Vec<Expr> {
        self.bump(); // (
        let args = self.bracketed(|p| {
            let mut args = Vec::new();
            while !p.at(Tk::RParen) && !p.at(Tk::Eof) {
                let before = (p.pos, p.part);
                args.push(p.parse_expr());
                if !p.eat(Tk::Comma) {
                    break;
                }
                if (p.pos, p.part) == before {
                    break;
                }
            }
            args
        });
        self.expect(Tk::RParen, "`,` or `)`");
        args
    }

    #[inline(never)]
    fn parse_primary(&mut self) -> Expr {
        match self.kind() {
            Tk::TemplateFull | Tk::TemplateHead => self.parse_template(),
            Tk::Kw(Kw::New) => self.parse_new_expr(),
            Tk::Kw(Kw::Switch) => self.parse_switch_expr(),
            Tk::LParen => self.parse_paren_or_arrow(),
            Tk::LBracket => self.parse_array_lit(),
            Tk::LBrace => self.parse_object_lit(),
            _ => self.prim_atom(),
        }
    }

    /// Literals, identifiers, `this` and primary-expression errors.
    #[inline(never)]
    fn prim_atom(&mut self) -> Expr {
        let lo = self.cur_lo();
        match self.kind() {
            Tk::Int | Tk::Float | Tk::Str | Tk::Kw(Kw::True | Kw::False | Kw::Null) => {
                let t = self.bump();
                let l = self.lit_from(&t);
                self.mk_expr(ExprKind::Lit(l), lo)
            }
            Tk::Ident | Tk::Reserved => {
                if self.nth(1) == Tk::FatArrow && !self.no_arrow {
                    return self.parse_ident_arrow(lo);
                }
                let id = self.parse_ident("an identifier");
                self.mk_expr(ExprKind::Ident(id), lo)
            }
            Tk::Kw(Kw::This) => {
                self.bump();
                self.mk_expr(ExprKind::This, lo)
            }
            Tk::Kw(Kw::Function) => {
                let sp = self.cur_span();
                self.error(
                    "OXS-E0001",
                    sp,
                    "expected an expression, found `function`; function expressions do not exist, use an arrow function `(x) => ...`",
                );
                self.error_expr()
            }
            _ => {
                self.expected("an expression");
                self.error_expr()
            }
        }
    }

    #[inline(never)]
    fn parse_new_expr(&mut self) -> Expr {
        let lo = self.cur_lo();
        self.bump();
        let ty = self.parse_new_type();
        let args = if self.at(Tk::LParen) {
            self.parse_call_args()
        } else {
            self.expected("`(`");
            Vec::new()
        };
        self.mk_expr(ExprKind::New { ty, args }, lo)
    }

    #[inline(never)]
    fn parse_array_lit(&mut self) -> Expr {
        let lo = self.cur_lo();
        self.bump();
        let elems = self.bracketed(|p| {
            let mut v = Vec::new();
            while !p.at(Tk::RBracket) && !p.at(Tk::Eof) {
                let before = (p.pos, p.part);
                v.push(p.parse_expr());
                if !p.eat(Tk::Comma) {
                    break;
                }
                if (p.pos, p.part) == before {
                    break;
                }
            }
            v
        });
        self.expect(Tk::RBracket, "`,` or `]`");
        self.mk_expr(ExprKind::Array(elems), lo)
    }

    #[inline(never)]
    fn parse_object_lit(&mut self) -> Expr {
        let lo = self.cur_lo();
        self.bump();
        let fields = self.bracketed(|p| {
            let mut v = Vec::new();
            while !p.at(Tk::RBrace) && !p.at(Tk::Eof) {
                let before = (p.pos, p.part);
                let flo = p.cur_lo();
                let name = p.parse_ident("a field name");
                let value = if p.eat(Tk::Colon) {
                    Some(p.parse_expr())
                } else {
                    None
                };
                let span = p.span_from(flo);
                v.push(ObjectField {
                    id: p.nid(),
                    span,
                    name,
                    value,
                });
                if !p.eat(Tk::Comma) {
                    break;
                }
                if (p.pos, p.part) == before {
                    p.bump();
                }
            }
            v
        });
        self.expect(Tk::RBrace, "`,` or `}`");
        self.mk_expr(ExprKind::Object(fields), lo)
    }

    #[inline(never)]
    fn parse_template(&mut self) -> Expr {
        let lo = self.cur_lo();
        let mut parts = Vec::new();
        let t = self.bump();
        parts.push(TemplatePart::Str(self.str_data(&t)));
        if t.kind == Tk::TemplateHead {
            loop {
                let e = self.bracketed(Self::parse_expr);
                parts.push(TemplatePart::Expr(e));
                match self.kind() {
                    Tk::TemplateMiddle => {
                        let t = self.bump();
                        parts.push(TemplatePart::Str(self.str_data(&t)));
                    }
                    Tk::TemplateTail => {
                        let t = self.bump();
                        parts.push(TemplatePart::Str(self.str_data(&t)));
                        break;
                    }
                    _ => {
                        self.expected("`}` to close the `${` interpolation");
                        break;
                    }
                }
            }
        }
        self.mk_expr(ExprKind::Template(parts), lo)
    }

    // ---- arrow functions ---------------------------------------------------

    #[inline(never)]
    fn parse_ident_arrow(&mut self, lo: u32) -> Expr {
        let plo = self.cur_lo();
        let name = self.parse_ident("a parameter name");
        let pspan = self.span_from(plo);
        let param = Param {
            id: self.nid(),
            span: pspan,
            mode: ParamMode::Value,
            name,
            ty: None,
            default: None,
        };
        self.bump(); // =>
        let body = self.parse_arrow_body();
        self.mk_expr(
            ExprKind::Arrow(ArrowFn {
                params: vec![param],
                ret: None,
                body,
            }),
            lo,
        )
    }

    #[inline(never)]
    fn parse_arrow_body(&mut self) -> ArrowBody {
        if self.at(Tk::LBrace) {
            let b = self.bracketed(Self::parse_block);
            ArrowBody::Block(b)
        } else {
            let saved = self.no_arrow;
            self.no_arrow = false;
            let e = self.parse_assign();
            self.no_arrow = saved;
            ArrowBody::Expr(Box::new(e))
        }
    }

    /// At `(` known to start an arrow function: `( params ) [: Type] => body`.
    #[inline(never)]
    fn parse_arrow_from_paren(&mut self, lo: u32) -> Expr {
        self.bump(); // (
        let params = self.bracketed(|p| p.parse_params(false));
        self.expect(Tk::RParen, "`)`");
        let ret = if self.eat(Tk::Colon) {
            Some(self.parse_type())
        } else {
            None
        };
        if !self.expect(Tk::FatArrow, "`=>`") {
            return self.error_expr();
        }
        let body = self.parse_arrow_body();
        self.mk_expr(ExprKind::Arrow(ArrowFn { params, ret, body }), lo)
    }

    #[inline(never)]
    fn parse_paren_or_arrow(&mut self) -> Expr {
        let lo = self.cur_lo();
        let m = if self.part == 0 {
            self.pm[self.pos]
        } else {
            u32::MAX
        };
        if m != u32::MAX && !self.no_arrow {
            let after = self.toks[(m as usize + 1).min(self.toks.len() - 1)].kind;
            if after == Tk::FatArrow {
                return self.parse_arrow_from_paren(lo);
            }
            if after == Tk::Colon && self.ret_type_arrow_ahead(m as usize + 2) {
                if !self.then_ctx {
                    return self.parse_arrow_from_paren(lo);
                }
                // inside the then-branch of `?:`: an arrow function only if the
                // body is followed by the `:` of the conditional (rule 2)
                let snap = self.snapshot();
                self.spec += 1;
                let arrow = self.parse_arrow_from_paren(lo);
                let ok = !self.spec_fail && self.at(Tk::Colon);
                self.spec -= 1;
                if ok {
                    return arrow;
                }
                self.restore(&snap);
            }
        }
        self.bump(); // (
        let inner = self.bracketed(Self::parse_expr);
        self.expect(Tk::RParen, "`)`");
        self.mk_expr(ExprKind::Paren(Box::new(inner)), lo)
    }

    /// Speculative: does a type starting at token index `at` end at `=>`?
    fn ret_type_arrow_ahead(&mut self, at: usize) -> bool {
        let snap = self.snapshot();
        self.pos = at.min(self.toks.len() - 1);
        self.part = 0;
        self.spec += 1;
        let _ = self.parse_type();
        let ok = !self.spec_fail && self.at(Tk::FatArrow);
        self.spec -= 1;
        self.restore(&snap);
        ok
    }

    /// `Params = Param { "," Param } [ "," ]` up to (not including) `)`.
    pub(super) fn parse_params(&mut self, require_types: bool) -> Vec<Param> {
        let mut v = Vec::new();
        while !self.at(Tk::RParen) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            let lo = self.cur_lo();
            let mut mode = ParamMode::Value;
            if self.at(Tk::Reserved) && self.cur_text() == "inout" {
                let sp = self.cur_span();
                self.reserved_error("inout", sp);
                self.bump();
                mode = ParamMode::Inout;
            }
            let name = self.parse_ident("a parameter name");
            let ty = if self.eat(Tk::Colon) {
                Some(self.parse_type())
            } else {
                if require_types && !name.name.is_empty() {
                    let msg = format!(
                        "expected `:` and a type for parameter `{}` (declarations need explicit parameter types)",
                        name.name
                    );
                    self.expected_with(&msg);
                }
                None
            };
            let default = if self.eat(Tk::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            let span = self.span_from(lo);
            v.push(Param {
                id: self.nid(),
                span,
                mode,
                name,
                ty,
                default,
            });
            if !self.eat(Tk::Comma) {
                break;
            }
            if (self.pos, self.part) == before {
                break;
            }
        }
        v
    }

    fn expected_with(&mut self, msg: &str) {
        let span = self.cur_span();
        let full = format!("{msg}, found {}", self.found_desc());
        self.error("OXS-E0001", span, full);
    }

    // ---- switch expression -------------------------------------------------

    /// A `case` label: a binary-level expression (a top-level `?:` needs
    /// parentheses) in which `x => y` is not an arrow function.
    pub(super) fn parse_label_expr(&mut self) -> Expr {
        let saved = (self.no_arrow, self.then_ctx);
        self.no_arrow = true;
        self.then_ctx = false;
        let e = self.parse_binary(1);
        self.no_arrow = saved.0;
        self.then_ctx = saved.1;
        e
    }

    /// The `:` after a `case`/`default` label; `=>` and `?` get a specific message.
    pub(super) fn expect_label_colon(&mut self) {
        match self.kind() {
            Tk::Colon => {
                self.bump();
            }
            Tk::FatArrow => {
                let sp = self.cur_span();
                self.diag(
                    Diagnostic::new(
                        "OXS-E0001",
                        sp,
                        "expected `:` after the `case` label, found `=>`; switch arms use `:`, never an arrow",
                    )
                    .with_fix(Fix::replace("use `:`", sp, ":")),
                );
                self.bump();
            }
            Tk::Question => {
                let sp = self.cur_span();
                self.error(
                    "OXS-E0001",
                    sp,
                    "expected `:` after the `case` label; a label containing `?:` must be parenthesised: `case (a ? b : c):`",
                );
                // skip the rest of the conditional and look for the real `:`
                self.bump();
                let _ = self.parse_assign();
                if self.eat(Tk::Colon) {
                    let _ = self.parse_cond();
                }
                self.expect_label_colon();
            }
            _ => self.expected("`:` after the `case` label"),
        }
    }

    #[inline(never)]
    fn parse_switch_expr(&mut self) -> Expr {
        let lo = self.cur_lo();
        self.bump(); // switch
        self.expect(Tk::LParen, "`(`");
        let scrutinee = self.bracketed(Self::parse_expr);
        self.expect(Tk::RParen, "`)`");
        self.expect(Tk::LBrace, "`{`");
        let mut arms = Vec::new();
        while matches!(self.kind(), Tk::Kw(Kw::Case | Kw::Default)) {
            let before = (self.pos, self.part);
            let alo = self.cur_lo();
            let mut labels = Vec::new();
            let mut is_default = false;
            if self.eat_kw(Kw::Case) {
                loop {
                    labels.push(self.parse_label_expr());
                    if !self.eat(Tk::Comma) {
                        break;
                    }
                }
            } else {
                self.bump();
                is_default = true;
            }
            self.expect_label_colon();
            let body = self.bracketed(Self::parse_expr);
            let span = self.span_from(alo);
            arms.push(Arm {
                id: self.nid(),
                span,
                labels,
                is_default,
                body,
            });
            if !self.eat(Tk::Comma) && matches!(self.kind(), Tk::Kw(Kw::Case | Kw::Default)) {
                let at = self.prev_end_span();
                self.diag(
                    Diagnostic::new("OXS-E0001", at, "expected `,` after the switch arm")
                        .with_fix(Fix::insert("insert `,`", at, ",")),
                );
            }
            if (self.pos, self.part) == before {
                break;
            }
        }
        if arms.is_empty() {
            self.expected("`case` or `default` (a switch expression needs at least one arm)");
        }
        self.expect(Tk::RBrace, "`}`");
        self.mk_expr(
            ExprKind::Switch(SwitchExpr {
                scrutinee: Box::new(scrutinee),
                arms,
            }),
            lo,
        )
    }
}
