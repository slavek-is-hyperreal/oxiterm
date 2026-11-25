//! Statements, blocks and patterns (`grammar.md` 3.2, 2.2).

use super::*;

enum Out {
    Stmt(Stmt),
    /// The tail expression of a `law` block.
    Tail(Expr),
}

impl Parser<'_> {
    /// `Block = "{" { Stmt } "}"`
    pub(super) fn parse_block(&mut self) -> Block {
        self.parse_block_impl(false)
    }

    /// `LawBlock = "{" { Stmt } Expr "}"`
    pub(super) fn parse_law_block(&mut self) -> Block {
        self.parse_block_impl(true)
    }

    fn parse_block_impl(&mut self, law: bool) -> Block {
        let lo = self.cur_lo();
        if !self.at(Tk::LBrace) {
            self.expected("`{`");
            return self.mk_block(Vec::new(), None, lo);
        }
        self.bump();
        let saved = (self.no_arrow, self.then_ctx);
        self.no_arrow = false;
        self.then_ctx = false;
        let mut stmts = Vec::new();
        let mut tail: Option<Box<Expr>> = None;
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            match self.parse_stmt_or_tail(law) {
                Some(Out::Stmt(s)) => stmts.push(s),
                Some(Out::Tail(e)) => {
                    tail = Some(Box::new(e));
                    break;
                }
                None => self.sync(false),
            }
            if (self.pos, self.part) == before && !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
                self.bump();
            }
        }
        self.no_arrow = saved.0;
        self.then_ctx = saved.1;
        self.expect(Tk::RBrace, "`}`");
        self.mk_block(stmts, tail, lo)
    }

    /// One statement (never a law tail).
    pub(super) fn parse_stmt(&mut self) -> Stmt {
        let lo = self.cur_lo();
        match self.parse_stmt_or_tail(false) {
            Some(Out::Stmt(s)) => s,
            _ => {
                self.sync(false);
                self.mk_stmt(StmtKind::Empty, Vec::new(), lo)
            }
        }
    }

    fn check_updates(&mut self, mark: usize, allowed: &[NodeId]) {
        let found: Vec<(NodeId, Span)> =
            self.updates.drain(mark.min(self.updates.len())..).collect();
        for (id, sp) in found {
            if !allowed.contains(&id) {
                self.error(
                    "OXS-E0016",
                    sp,
                    "`++` and `--` produce no value; use them only as a statement (`i++;`) or in a `for` update, or write `i += 1`",
                );
            }
        }
    }

    fn parse_stmt_or_tail(&mut self, law: bool) -> Option<Out> {
        if !self.enter() {
            self.leave();
            return None;
        }
        let r = self.parse_stmt_inner(law);
        self.leave();
        r
    }

    /// Is the cursor at the start of a declaration (after attributes)?
    pub(super) fn at_item_start(&self) -> bool {
        match self.kind() {
            Tk::Kw(
                Kw::Export
                | Kw::Function
                | Kw::Struct
                | Kw::Class
                | Kw::Interface
                | Kw::Enum
                | Kw::Import,
            ) => true,
            Tk::Ident => {
                (self.cur_text() == "type"
                    && self.nth(1) == Tk::Ident
                    && matches!(self.nth(2), Tk::Assign | Tk::Lt))
                    || (self.cur_text() == "law"
                        && self.nth(1) == Tk::Ident
                        && matches!(self.nth(2), Tk::LParen | Tk::Lt))
            }
            Tk::Reserved => {
                matches!(self.cur_text(), "async" | "kernel") && self.nth(1) == Tk::Kw(Kw::Function)
            }
            _ => false,
        }
    }

    fn parse_stmt_inner(&mut self, law: bool) -> Option<Out> {
        let lo = self.cur_lo();
        let doc = self.docs_before(lo);
        let mark = self.updates.len();
        let mut attrs = Vec::new();
        while matches!(self.kind(), Tk::At | Tk::AtAt) {
            if self.at(Tk::AtAt) {
                let sp = self.cur_span();
                self.error(
                    "OXS-E0001",
                    sp,
                    "inner attributes `@@name` are only allowed at the top of a file",
                );
            }
            attrs.push(self.parse_attribute());
        }
        if self.at_item_start() {
            let it = self.parse_item_after_attrs(lo, doc, attrs)?;
            return Some(Out::Stmt(self.mk_stmt(
                StmtKind::Item(Box::new(it)),
                Vec::new(),
                lo,
            )));
        }
        let mut allowed: Vec<NodeId> = Vec::new();
        let kind = match self.kind() {
            Tk::LBrace => StmtKind::Block(self.parse_block()),
            Tk::Semi => {
                self.bump();
                StmtKind::Empty
            }
            Tk::Kw(Kw::If) => self.parse_if(),
            Tk::Kw(Kw::While) => {
                self.bump();
                self.expect(Tk::LParen, "`(`");
                let cond = self.bracketed_expr();
                self.expect(Tk::RParen, "`)`");
                let body = Box::new(self.parse_stmt());
                StmtKind::While { cond, body }
            }
            Tk::Kw(Kw::Do) => {
                self.bump();
                let body = Box::new(self.parse_stmt());
                self.expect(Tk::Kw(Kw::While), "`while`");
                self.expect(Tk::LParen, "`(`");
                let cond = self.bracketed_expr();
                self.expect(Tk::RParen, "`)`");
                self.expect_semi();
                StmtKind::DoWhile { body, cond }
            }
            Tk::Kw(Kw::For) => self.parse_for(false, &mut allowed),
            Tk::Reserved if self.cur_text() == "parallel" && self.nth(1) == Tk::Kw(Kw::For) => {
                let sp = self.cur_span();
                self.reserved_error("parallel", sp);
                self.bump();
                self.parse_for(true, &mut allowed)
            }
            Tk::Reserved
                if matches!(self.cur_text(), "arena" | "trusted") && self.nth(1) == Tk::LBrace =>
            {
                let w = self.cur_text();
                let sp = self.cur_span();
                self.reserved_error(w, sp);
                self.bump();
                StmtKind::Block(self.parse_block())
            }
            Tk::Kw(Kw::Switch) => self.parse_switch_stmt(),
            Tk::Kw(Kw::Return) => {
                self.bump();
                let e = if matches!(self.kind(), Tk::Semi | Tk::RBrace | Tk::Eof) {
                    None
                } else {
                    Some(self.parse_expr())
                };
                self.expect_semi();
                StmtKind::Return(e)
            }
            Tk::Kw(k @ (Kw::Break | Kw::Continue)) => {
                self.bump();
                let label = if self.at(Tk::Ident) && !self.cur_nl() {
                    Some(self.parse_ident("a label"))
                } else {
                    None
                };
                self.expect_semi();
                if k == Kw::Break {
                    StmtKind::Break(label)
                } else {
                    StmtKind::Continue(label)
                }
            }
            Tk::Kw(Kw::Let | Kw::Const) => {
                let d = self.parse_var_decl();
                self.expect_semi();
                StmtKind::Var(d)
            }
            Tk::Reserved if self.cur_text() == "var" => {
                let sp = self.cur_span();
                self.reserved_error("var", sp);
                // parse like `let`
                self.bump();
                let pattern = self.parse_pattern();
                let ty = if self.eat(Tk::Colon) {
                    Some(self.parse_type())
                } else {
                    None
                };
                let init = if self.eat(Tk::Assign) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                self.expect_semi();
                StmtKind::Var(VarDecl {
                    kind: VarKind::Let,
                    pattern,
                    ty,
                    init,
                })
            }
            Tk::Ident | Tk::Reserved if self.nth(1) == Tk::Colon && self.kind() == Tk::Ident => {
                let label = self.parse_ident("a label");
                self.bump(); // :
                let body = Box::new(self.parse_stmt());
                StmtKind::Labeled { label, body }
            }
            Tk::RBrace | Tk::Eof => return None,
            _ => {
                if matches!(self.kind(), Tk::RParen | Tk::RBracket) {
                    self.expected("a statement");
                    return None;
                }
                let before = (self.pos, self.part);
                let e = self.parse_expr();
                if matches!(e.kind, ExprKind::Update { .. }) {
                    allowed.push(e.id);
                }
                if law && self.at(Tk::RBrace) && attrs.is_empty() {
                    self.check_updates(mark, &allowed);
                    return Some(Out::Tail(e));
                }
                if (self.pos, self.part) == before {
                    // nothing was consumed: the error was already reported
                    self.check_updates(mark, &allowed);
                    return None;
                }
                self.expect_semi();
                StmtKind::Expr(e)
            }
        };
        self.check_updates(mark, &allowed);
        Some(Out::Stmt(self.mk_stmt(kind, attrs, lo)))
    }

    fn bracketed_expr(&mut self) -> Expr {
        let saved = (self.no_arrow, self.then_ctx);
        self.no_arrow = false;
        self.then_ctx = false;
        let e = self.parse_expr();
        self.no_arrow = saved.0;
        self.then_ctx = saved.1;
        e
    }

    fn parse_if(&mut self) -> StmtKind {
        self.bump();
        self.expect(Tk::LParen, "`(`");
        let cond = self.bracketed_expr();
        self.expect(Tk::RParen, "`)`");
        let then = Box::new(self.parse_stmt());
        let els = if self.eat_kw(Kw::Else) {
            Some(Box::new(self.parse_stmt()))
        } else {
            None
        };
        StmtKind::If { cond, then, els }
    }

    fn parse_for(&mut self, parallel: bool, allowed: &mut Vec<NodeId>) -> StmtKind {
        self.bump(); // for
        self.expect(Tk::LParen, "`(`");
        let mut init: Option<Box<Stmt>> = None;
        if matches!(self.kind(), Tk::Kw(Kw::Let | Kw::Const)) {
            let ilo = self.cur_lo();
            let kind = if self.at_kw(Kw::Let) {
                VarKind::Let
            } else {
                VarKind::Const
            };
            self.bump();
            let pattern = self.parse_pattern();
            if self.at_ident_word("of") || self.at_ident_word("in") {
                if self.at_ident_word("in") {
                    let sp = self.cur_span();
                    self.diag(
                        Diagnostic::new(
                            "OXS-E0001",
                            sp,
                            "`for ... in` does not exist; iterate with `for (const x of xs)`",
                        )
                        .with_fix(Fix::replace("use `of`", sp, "of")),
                    );
                }
                self.bump();
                let iter = self.bracketed_expr();
                self.expect(Tk::RParen, "`)`");
                let body = Box::new(self.parse_stmt());
                return StmtKind::ForOf {
                    kind,
                    binding: pattern,
                    iter,
                    body,
                    parallel,
                };
            }
            let ty = if self.eat(Tk::Colon) {
                Some(self.parse_type())
            } else {
                None
            };
            let ini = if self.eat(Tk::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            let d = VarDecl {
                kind,
                pattern,
                ty,
                init: ini,
            };
            init = Some(Box::new(self.mk_stmt(StmtKind::Var(d), Vec::new(), ilo)));
        } else if !self.at(Tk::Semi) {
            let ilo = self.cur_lo();
            let e = self.bracketed_expr();
            if matches!(e.kind, ExprKind::Update { .. }) {
                allowed.push(e.id);
            }
            init = Some(Box::new(self.mk_stmt(StmtKind::Expr(e), Vec::new(), ilo)));
        }
        self.expect(Tk::Semi, "`;`");
        let cond = if self.at(Tk::Semi) {
            None
        } else {
            Some(self.bracketed_expr())
        };
        self.expect(Tk::Semi, "`;`");
        let update = if self.at(Tk::RParen) {
            None
        } else {
            let e = self.bracketed_expr();
            if matches!(e.kind, ExprKind::Update { .. }) {
                allowed.push(e.id);
            }
            Some(e)
        };
        self.expect(Tk::RParen, "`)`");
        let body = Box::new(self.parse_stmt());
        StmtKind::For {
            init,
            cond,
            update,
            body,
            parallel,
        }
    }

    fn parse_switch_stmt(&mut self) -> StmtKind {
        self.bump(); // switch
        self.expect(Tk::LParen, "`(`");
        let scrutinee = self.bracketed_expr();
        self.expect(Tk::RParen, "`)`");
        self.expect(Tk::LBrace, "`{`");
        let mut cases = Vec::new();
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            if !matches!(self.kind(), Tk::Kw(Kw::Case | Kw::Default)) {
                self.expected("`case`, `default` or `}`");
                self.sync(false);
                if (self.pos, self.part) == before {
                    self.bump();
                }
                continue;
            }
            let clo = self.cur_lo();
            let mut labels = Vec::new();
            let mut is_default = false;
            while matches!(self.kind(), Tk::Kw(Kw::Case | Kw::Default)) {
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
            }
            let mut body = Vec::new();
            while !matches!(
                self.kind(),
                Tk::Kw(Kw::Case | Kw::Default) | Tk::RBrace | Tk::Eof
            ) {
                let b = (self.pos, self.part);
                body.push(self.parse_stmt());
                if (self.pos, self.part) == b {
                    self.bump();
                }
            }
            let span = self.span_from(clo);
            cases.push(Case {
                id: self.nid(),
                span,
                labels,
                is_default,
                body,
            });
        }
        self.expect(Tk::RBrace, "`}`");
        StmtKind::Switch(SwitchStmt { scrutinee, cases })
    }

    /// `VarDecl = ( "let" | "const" ) Pattern [ ":" Type ] [ "=" Expr ]` (no `;`).
    pub(super) fn parse_var_decl(&mut self) -> VarDecl {
        let kind = if self.at_kw(Kw::Let) {
            VarKind::Let
        } else {
            VarKind::Const
        };
        self.bump();
        let pattern = self.parse_pattern();
        let ty = if self.eat(Tk::Colon) {
            Some(self.parse_type())
        } else {
            None
        };
        let init = if self.eat(Tk::Assign) {
            Some(self.parse_expr())
        } else {
            None
        };
        VarDecl {
            kind,
            pattern,
            ty,
            init,
        }
    }

    pub(super) fn parse_pattern(&mut self) -> Pattern {
        let lo = self.cur_lo();
        if !self.enter() {
            self.leave();
            let id = self.error_ident();
            let span = id.span;
            return Pattern {
                id: self.nid(),
                span,
                kind: PatternKind::Ident(id),
            };
        }
        let kind = match self.kind() {
            Tk::LBrace => {
                self.bump();
                let mut fields = Vec::new();
                while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
                    let before = (self.pos, self.part);
                    let flo = self.cur_lo();
                    let key = self.parse_ident("a field name");
                    let pattern = if self.eat(Tk::Colon) {
                        Some(self.parse_pattern())
                    } else {
                        None
                    };
                    let span = self.span_from(flo);
                    fields.push(ObjectPatField {
                        id: self.nid(),
                        span,
                        key,
                        pattern,
                    });
                    if !self.eat(Tk::Comma) {
                        break;
                    }
                    if (self.pos, self.part) == before {
                        break;
                    }
                }
                self.expect(Tk::RBrace, "`}`");
                PatternKind::Object(fields)
            }
            Tk::LBracket => {
                let sp = self.cur_span();
                self.error(
                    "OXS-E0900",
                    sp,
                    "array patterns are reserved for a future version of OxiScript; index the array instead",
                );
                self.bump();
                while !self.at(Tk::RBracket) && !self.at(Tk::Eof) && !self.at(Tk::Semi) {
                    let before = (self.pos, self.part);
                    self.parse_pattern();
                    if !self.eat(Tk::Comma) || (self.pos, self.part) == before {
                        break;
                    }
                }
                self.expect(Tk::RBracket, "`]`");
                let id = self.error_ident();
                PatternKind::Ident(id)
            }
            _ => {
                let id = self.parse_ident("a variable name");
                if id.name == "_" {
                    PatternKind::Wildcard
                } else {
                    PatternKind::Ident(id)
                }
            }
        };
        self.leave();
        let span = self.span_from(lo);
        Pattern {
            id: self.nid(),
            span,
            kind,
        }
    }
}
