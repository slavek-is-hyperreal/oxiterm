//! Declarations and attributes (`grammar.md` 2 and 4).

use super::*;

impl Parser<'_> {
    /// `AttrArgs`, with the leading `@` or `@@` at the cursor.
    pub(super) fn parse_attribute(&mut self) -> Attribute {
        let lo = self.cur_lo();
        let inner = self.at(Tk::AtAt);
        self.bump();
        let name = self.parse_ident("an attribute name");
        let mut args = Vec::new();
        if self.at(Tk::LParen) {
            self.bump();
            while !self.at(Tk::RParen) && !self.at(Tk::Eof) {
                let before = (self.pos, self.part);
                let alo = self.cur_lo();
                let key = if self.at(Tk::Ident) && self.nth(1) == Tk::Colon {
                    let k = self.parse_ident("an argument name");
                    self.bump();
                    Some(k)
                } else {
                    None
                };
                let value = match self.kind() {
                    Tk::Str | Tk::Int | Tk::Kw(Kw::True | Kw::False) => {
                        let t = self.bump();
                        AttrValue::Lit(self.lit_from(&t))
                    }
                    Tk::Ident | Tk::Reserved => {
                        let mut path = vec![self.parse_ident("a name")];
                        while self.at(Tk::Dot) {
                            self.bump();
                            path.push(self.parse_ident("a name"));
                        }
                        AttrValue::Path(path)
                    }
                    _ => {
                        self.expected(
                            "an attribute argument (a string, integer, `true`, `false` or a name)",
                        );
                        AttrValue::Path(vec![self.error_ident()])
                    }
                };
                let span = self.span_from(alo);
                args.push(AttrArg {
                    id: self.nid(),
                    span,
                    key,
                    value,
                });
                if !self.eat(Tk::Comma) {
                    break;
                }
                if (self.pos, self.part) == before {
                    break;
                }
            }
            self.expect(Tk::RParen, "`,` or `)`");
        }
        let span = self.span_from(lo);
        Attribute {
            id: self.nid(),
            span,
            inner,
            name,
            args,
        }
    }

    /// A top-level or nested declaration, with its doc comments and attributes.
    pub(super) fn parse_item(&mut self, _nested: bool) -> Option<Item> {
        let lo = self.cur_lo();
        let doc = self.docs_before(lo);
        let mut attrs = Vec::new();
        while matches!(self.kind(), Tk::At | Tk::AtAt) {
            if self.at(Tk::AtAt) {
                let sp = self.cur_span();
                self.error(
                    "OXS-E0001",
                    sp,
                    "inner attributes `@@name` are only allowed at the top of a file, before the first declaration",
                );
            }
            attrs.push(self.parse_attribute());
        }
        self.parse_item_after_attrs(lo, doc, attrs)
    }

    #[inline(never)]
    pub(super) fn parse_item_after_attrs(
        &mut self,
        lo: u32,
        doc: Vec<String>,
        attrs: Vec<Attribute>,
    ) -> Option<Item> {
        if !self.enter_w(STMT_WEIGHT) {
            self.leave_w(STMT_WEIGHT);
            return None;
        }
        let mark = self.updates.len();
        let r = self.parse_item_inner(lo, doc, attrs);
        self.updates.truncate(mark.min(self.updates.len()));
        self.leave_w(STMT_WEIGHT);
        r
    }

    #[inline(never)]
    fn parse_item_inner(
        &mut self,
        lo: u32,
        doc: Vec<String>,
        attrs: Vec<Attribute>,
    ) -> Option<Item> {
        let export_span = self.cur_span();
        let export = self.eat_kw(Kw::Export);
        let kind = match self.kind() {
            Tk::Kw(Kw::Import) => {
                if export {
                    self.error(
                        "OXS-E0001",
                        export_span,
                        "`export` cannot be applied to `import` (there is no re-export)",
                    );
                }
                ItemKind::Import(self.parse_import())
            }
            Tk::Kw(Kw::Let | Kw::Const) => {
                let d = self.parse_var_decl();
                self.expect_semi();
                ItemKind::Var(d)
            }
            Tk::Kw(Kw::Function) => ItemKind::Fn(self.parse_fn(FnModifiers::default())),
            Tk::Reserved
                if matches!(self.cur_text(), "async" | "kernel")
                    && self.nth(1) == Tk::Kw(Kw::Function) =>
            {
                let w = self.cur_text();
                let mut mods = FnModifiers::default();
                if w == "async" {
                    mods.is_async = true;
                } else {
                    mods.kernel = true;
                }
                let sp = self.cur_span();
                self.reserved_error(w, sp);
                self.bump();
                ItemKind::Fn(self.parse_fn(mods))
            }
            Tk::Kw(Kw::Struct | Kw::Class) => ItemKind::Record(self.parse_record()),
            Tk::Kw(Kw::Interface) => ItemKind::Interface(self.parse_interface()),
            Tk::Kw(Kw::Enum) => ItemKind::Enum(self.parse_enum()),
            Tk::Ident if self.at_item_start() && self.cur_text() == "type" => {
                self.bump();
                let name = self.parse_ident("a type name");
                let type_params = self.parse_type_params();
                self.expect(Tk::Assign, "`=`");
                let ty = self.parse_type();
                self.expect_semi();
                ItemKind::TypeAlias(TypeAliasDecl {
                    name,
                    type_params,
                    ty,
                })
            }
            Tk::Ident if self.at_item_start() && self.cur_text() == "law" => {
                ItemKind::Law(self.parse_law())
            }
            Tk::Reserved
                if matches!(
                    self.cur_text(),
                    "declare" | "namespace" | "trait" | "impl" | "unsafe"
                ) =>
            {
                let w = self.cur_text();
                let sp = self.cur_span();
                self.reserved_error(w, sp);
                return None;
            }
            _ => {
                self.expected(
                    "a declaration (`function`, `struct`, `class`, `interface`, `enum`, `type`, `law`, `let`, `const` or `import`)",
                );
                return None;
            }
        };
        Some(self.mk_item(lo, doc, attrs, export, kind))
    }

    fn parse_import(&mut self) -> ImportDecl {
        self.bump(); // import
        let clause = if self.at(Tk::LBrace) {
            self.bump();
            let mut specs = Vec::new();
            while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
                let before = (self.pos, self.part);
                let lo = self.cur_lo();
                let name = self.parse_ident("an imported name");
                let alias = if self.eat_kw(Kw::As) {
                    Some(self.parse_ident("a local name"))
                } else {
                    None
                };
                let span = self.span_from(lo);
                specs.push(ImportSpec {
                    id: self.nid(),
                    span,
                    name,
                    alias,
                });
                if !self.eat(Tk::Comma) {
                    break;
                }
                if (self.pos, self.part) == before {
                    break;
                }
            }
            self.expect(Tk::RBrace, "`,` or `}`");
            ImportClause::Named(specs)
        } else if self.eat(Tk::Star) {
            self.expect(Tk::Kw(Kw::As), "`as`");
            ImportClause::Namespace(self.parse_ident("a namespace name"))
        } else {
            self.expected("`{` or `* as name`");
            ImportClause::Named(Vec::new())
        };
        if self.at_ident_word("from") {
            self.bump();
        } else {
            self.expected("`from`");
        }
        let source = if self.at(Tk::Str) {
            let t = self.bump();
            self.lit_from(&t)
        } else {
            self.expected("a module path string");
            Lit {
                id: self.nid(),
                span: self.prev_end_span(),
                kind: LitKind::Str(String::new()),
            }
        };
        self.expect_semi();
        ImportDecl { clause, source }
    }

    fn parse_contracts(&mut self) -> Vec<Contract> {
        let mut v = Vec::new();
        while self.at(Tk::Reserved) && matches!(self.cur_text(), "requires" | "ensures") {
            let kind = if self.cur_text() == "requires" {
                ContractKind::Requires
            } else {
                ContractKind::Ensures
            };
            let w = self.cur_text();
            let sp = self.cur_span();
            self.reserved_error(w, sp);
            self.bump();
            let expr = self.parse_expr();
            v.push(Contract { kind, expr });
        }
        v
    }

    /// `function name<T>(params): Ret contracts { body }` (cursor at `function`).
    fn parse_fn(&mut self, mods: FnModifiers) -> FnDecl {
        self.bump(); // function
        let name = self.parse_ident("a function name");
        self.parse_fn_rest(name, mods, true)
    }

    /// Everything after the name: type params, params, return type, contracts, body.
    fn parse_fn_rest(&mut self, name: Ident, mods: FnModifiers, body_required: bool) -> FnDecl {
        let type_params = self.parse_type_params();
        self.expect(Tk::LParen, "`(`");
        let saved = (self.no_arrow, self.then_ctx);
        self.no_arrow = false;
        self.then_ctx = false;
        let params = self.parse_params(true);
        self.no_arrow = saved.0;
        self.then_ctx = saved.1;
        self.expect(Tk::RParen, "`)`");
        let ret = if self.eat(Tk::Colon) {
            Some(self.parse_type())
        } else {
            None
        };
        let contracts = self.parse_contracts();
        let body = if body_required {
            Some(self.parse_block())
        } else {
            self.expect_semi();
            None
        };
        FnDecl {
            name,
            mods,
            type_params,
            params,
            ret,
            contracts,
            body,
        }
    }

    fn parse_record(&mut self) -> RecordDecl {
        let kind = if self.at_kw(Kw::Struct) {
            RecordKind::Struct
        } else {
            RecordKind::Class
        };
        self.bump();
        let name = self.parse_ident("a type name");
        let type_params = self.parse_type_params();
        let mut implements = Vec::new();
        if self.at_kw(Kw::Extends) {
            let sp = self.cur_span();
            self.error(
                "OXS-E0900",
                sp,
                "`extends` on a class is reserved: classes cannot inherit, use interfaces (`implements`) and composition",
            );
            self.bump();
            let _ = self.parse_type();
        }
        if self.eat_kw(Kw::Implements) {
            implements = self.parse_type_list();
        }
        self.expect(Tk::LBrace, "`{`");
        let mut members = Vec::new();
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            if let Some(m) = self.parse_member() {
                members.push(m);
            } else {
                self.sync(true);
            }
            if (self.pos, self.part) == before && !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
                self.bump();
            }
        }
        self.expect(Tk::RBrace, "`}`");
        RecordDecl {
            kind,
            name,
            type_params,
            implements,
            members,
        }
    }

    #[inline(never)]
    fn parse_member(&mut self) -> Option<Member> {
        if !self.enter_w(STMT_WEIGHT) {
            self.leave_w(STMT_WEIGHT);
            return None;
        }
        let r = self.parse_member_inner();
        self.leave_w(STMT_WEIGHT);
        r
    }

    #[inline(never)]
    fn parse_member_inner(&mut self) -> Option<Member> {
        let lo = self.cur_lo();
        let doc = self.docs_before(lo);
        let mut attrs = Vec::new();
        while matches!(self.kind(), Tk::At | Tk::AtAt) {
            attrs.push(self.parse_attribute());
        }
        let mut private = false;
        let mut readonly = false;
        let mut is_static = false;
        let mut mutating = false;
        loop {
            match self.kind() {
                Tk::Kw(Kw::Private) => private = true,
                Tk::Kw(Kw::Readonly) => readonly = true,
                Tk::Kw(Kw::Static) => is_static = true,
                Tk::Ident
                    if self.cur_text() == "mutating"
                        && matches!(self.nth(1), Tk::Ident | Tk::Reserved) =>
                {
                    mutating = true;
                }
                Tk::Reserved if matches!(self.cur_text(), "public" | "protected") => {
                    let w = self.cur_text();
                    let sp = self.cur_span();
                    self.reserved_error(w, sp);
                }
                _ => break,
            }
            self.bump();
        }
        let kind = if self.at_ident_word("constructor") && self.nth(1) == Tk::LParen {
            let t = self.bump();
            let name = self.mk_ident(
                "constructor".to_string(),
                mk_span(self.file, t.lo as usize, t.hi as usize),
            );
            let f = self.parse_fn_rest(name, FnModifiers::default(), true);
            MemberKind::Constructor(f)
        } else if matches!(self.kind(), Tk::Ident | Tk::Reserved) {
            let name = self.parse_ident("a member name");
            if self.at(Tk::LParen) || self.at(Tk::Lt) {
                let mods = FnModifiers {
                    is_static,
                    mutating,
                    private,
                    kernel: false,
                    is_async: false,
                };
                MemberKind::Method(self.parse_fn_rest(name, mods, true))
            } else {
                self.expect(Tk::Colon, "`:` and a field type, or `(` for a method");
                let ty = self.parse_type();
                let init = if self.eat(Tk::Assign) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                if !self.eat(Tk::Semi) && !self.eat(Tk::Comma) && !self.at(Tk::RBrace) {
                    let at = self.prev_end_span();
                    self.diag(
                        Diagnostic::new("OXS-E0004", at, "fields end with `;` or `,`")
                            .with_fix(Fix::insert("insert `;`", at, ";")),
                    );
                }
                MemberKind::Field(FieldDecl {
                    name,
                    ty,
                    init,
                    readonly,
                    private,
                    is_static,
                })
            }
        } else {
            self.expected("a field or method declaration");
            return None;
        };
        let span = self.span_from(lo);
        Some(Member {
            id: self.nid(),
            span,
            doc,
            attrs,
            kind,
        })
    }

    fn parse_interface(&mut self) -> InterfaceDecl {
        self.bump();
        let name = self.parse_ident("an interface name");
        let type_params = self.parse_type_params();
        let extends = if self.eat_kw(Kw::Extends) {
            self.parse_type_list()
        } else {
            Vec::new()
        };
        self.expect(Tk::LBrace, "`{`");
        let mut methods = Vec::new();
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            if matches!(self.kind(), Tk::Ident | Tk::Reserved) {
                let n = self.parse_ident("a method name");
                methods.push(self.parse_fn_rest(n, FnModifiers::default(), false));
            } else {
                self.expected("a method signature");
                self.sync(true);
            }
            if (self.pos, self.part) == before && !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
                self.bump();
            }
        }
        self.expect(Tk::RBrace, "`}`");
        InterfaceDecl {
            name,
            type_params,
            extends,
            methods,
        }
    }

    fn parse_enum(&mut self) -> EnumDecl {
        self.bump();
        let name = self.parse_ident("an enum name");
        let repr = if self.eat(Tk::Colon) {
            Some(self.parse_type())
        } else {
            None
        };
        self.expect(Tk::LBrace, "`{`");
        let mut variants = Vec::new();
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            let lo = self.cur_lo();
            let vname = self.parse_ident("a variant name");
            let value = if self.eat(Tk::Assign) {
                Some(self.parse_expr())
            } else {
                None
            };
            let span = self.span_from(lo);
            variants.push(EnumVariant {
                id: self.nid(),
                span,
                name: vname,
                value,
            });
            if !self.eat(Tk::Comma) {
                break;
            }
            if (self.pos, self.part) == before {
                break;
            }
        }
        self.expect(Tk::RBrace, "`,` or `}`");
        EnumDecl {
            name,
            repr,
            variants,
        }
    }

    fn parse_law(&mut self) -> LawDecl {
        self.bump(); // law
        let name = self.parse_ident("a law name");
        let type_params = self.parse_type_params();
        self.expect(Tk::LParen, "`(`");
        let params = self.parse_params(true);
        self.expect(Tk::RParen, "`)`");
        let when = if self.at_ident_word("when") {
            self.bump();
            Some(self.parse_expr())
        } else {
            None
        };
        let body = self.parse_law_block();
        LawDecl {
            name,
            type_params,
            params,
            when,
            body,
        }
    }
}
