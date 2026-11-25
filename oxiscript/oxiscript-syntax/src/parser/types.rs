//! Type syntax (`grammar.md` 3.1).

use super::*;

impl Parser<'_> {
    fn type_height(&self, kind: &TypeExprKind) -> u32 {
        let h = |t: &TypeExpr| self.ht[t.id.0 as usize];
        match kind {
            TypeExprKind::Path { args, .. } => args.iter().map(h).max().unwrap_or(0),
            TypeExprKind::Array(t) | TypeExprKind::Paren(t) => h(t),
            TypeExprKind::Union(v) => v.iter().map(h).max().unwrap_or(0),
            TypeExprKind::Fn { params, ret } => params
                .iter()
                .map(|p| h(&p.ty))
                .max()
                .unwrap_or(0)
                .max(h(ret)),
            TypeExprKind::Object(fs) => fs.iter().map(|f| h(&f.ty)).max().unwrap_or(0),
            TypeExprKind::Lit(_) | TypeExprKind::Null => 0,
        }
    }

    pub(super) fn mk_type(&mut self, kind: TypeExprKind, lo: u32) -> TypeExpr {
        let hh = self.type_height(&kind) + 1;
        let span = self.span_from(lo);
        let t = TypeExpr {
            id: self.nid(),
            span,
            kind,
        };
        self.set_height(t.id, hh);
        t
    }

    pub(super) fn error_type(&mut self) -> TypeExpr {
        let lo = self.cur_lo();
        let id = self.error_ident();
        self.mk_type(
            TypeExprKind::Path {
                segments: vec![id],
                args: Vec::new(),
            },
            lo,
        )
    }

    /// `Type = FnType | UnionType`
    #[inline(never)]
    pub(super) fn parse_type(&mut self) -> TypeExpr {
        if !self.enter() {
            self.leave();
            return self.error_type();
        }
        let t = self.parse_type_body();
        self.leave();
        t
    }

    #[inline(never)]
    fn parse_type_body(&mut self) -> TypeExpr {
        if self.at(Tk::LParen) && self.is_fn_type_start() {
            self.parse_fn_type()
        } else {
            self.parse_union_type()
        }
    }

    /// At `(`: is this `( ... ) =>`?
    pub(super) fn is_fn_type_start(&self) -> bool {
        if self.part != 0 {
            return false;
        }
        let m = self.pm[self.pos];
        m != u32::MAX && self.toks[(m as usize + 1).min(self.toks.len() - 1)].kind == Tk::FatArrow
    }

    fn parse_fn_type(&mut self) -> TypeExpr {
        let lo = self.cur_lo();
        self.bump(); // (
        let mut params = Vec::new();
        while !self.at(Tk::RParen) && !self.at(Tk::Eof) {
            let before = self.pos;
            let plo = self.cur_lo();
            let name = self.parse_ident("a parameter name");
            self.expect(Tk::Colon, "`:` and a parameter type");
            let ty = self.parse_type();
            let span = self.span_from(plo);
            params.push(FnTypeParam {
                id: self.nid(),
                span,
                name,
                ty,
            });
            if !self.eat(Tk::Comma) {
                break;
            }
            if self.pos == before {
                self.bump();
            }
        }
        self.expect(Tk::RParen, "`)`");
        self.expect(Tk::FatArrow, "`=>`");
        let ret = Box::new(self.parse_type());
        self.mk_type(TypeExprKind::Fn { params, ret }, lo)
    }

    #[inline(never)]
    fn parse_union_type(&mut self) -> TypeExpr {
        let lo = self.cur_lo();
        self.eat(Tk::Pipe);
        let first = self.parse_array_type();
        if !self.at(Tk::Pipe) {
            return first;
        }
        let mut members = vec![first];
        while self.eat(Tk::Pipe) {
            let before = self.pos;
            members.push(self.parse_array_type());
            if self.pos == before {
                break;
            }
        }
        self.mk_type(TypeExprKind::Union(members), lo)
    }

    /// `ArrayType = PrimaryType { "[" "]" }`
    #[inline(never)]
    pub(super) fn parse_array_type(&mut self) -> TypeExpr {
        let lo = self.cur_lo();
        let mut t = self.parse_primary_type();
        while self.at(Tk::LBracket) && self.nth(1) == Tk::RBracket {
            self.bump();
            self.bump();
            t = self.mk_type(TypeExprKind::Array(Box::new(t)), lo);
        }
        t
    }

    #[inline(never)]
    fn parse_type_path(&mut self, with_args: bool) -> TypeExpr {
        let lo = self.cur_lo();
        let mut segments = vec![self.parse_ident("a type name")];
        while self.at(Tk::Dot) && matches!(self.nth(1), Tk::Ident | Tk::Reserved) {
            self.bump();
            segments.push(self.parse_ident("a type name"));
        }
        let mut args = Vec::new();
        if with_args && self.at(Tk::Lt) {
            self.bump();
            args = self.parse_type_list();
            if !self.eat_gt() {
                self.expected("`>`");
            }
        }
        self.mk_type(TypeExprKind::Path { segments, args }, lo)
    }

    /// `TypeList = Type { "," Type }`; the caller consumes `>`.
    pub(super) fn parse_type_list(&mut self) -> Vec<TypeExpr> {
        let mut v = Vec::new();
        loop {
            let before = (self.pos, self.part);
            v.push(self.parse_type());
            if (self.pos, self.part) == before {
                break;
            }
            if !self.eat(Tk::Comma) {
                break;
            }
        }
        v
    }

    /// The type after `new`: a path with optional type arguments.
    pub(super) fn parse_new_type(&mut self) -> TypeExpr {
        if !self.enter() {
            self.leave();
            return self.error_type();
        }
        let t = self.parse_type_path(true);
        self.leave();
        t
    }

    #[inline(never)]
    fn parse_primary_type(&mut self) -> TypeExpr {
        let lo = self.cur_lo();
        match self.kind() {
            Tk::Reserved if self.cur_text() == "dyn" => {
                let sp = self.cur_span();
                self.reserved_error("dyn", sp);
                self.bump();
                self.parse_array_type()
            }
            Tk::Ident | Tk::Reserved => self.parse_type_path(true),
            Tk::Kw(Kw::Null) => {
                self.bump();
                self.mk_type(TypeExprKind::Null, lo)
            }
            Tk::Kw(Kw::True | Kw::False) | Tk::Str | Tk::Int => {
                let t = self.bump();
                let l = self.lit_from(&t);
                self.mk_type(TypeExprKind::Lit(l), lo)
            }
            Tk::LBrace => self.parse_object_type(),
            Tk::LParen => {
                self.bump();
                let inner = self.parse_type();
                self.expect(Tk::RParen, "`)`");
                self.mk_type(TypeExprKind::Paren(Box::new(inner)), lo)
            }
            Tk::LBracket => {
                // tuples `[A, B]` and fixed arrays `[T; N]` are reserved
                let sp = self.cur_span();
                self.error(
                    "OXS-E0900",
                    sp,
                    "tuple and fixed-array types are reserved for a future version of OxiScript; \
                     declare a `struct` instead",
                );
                self.bump();
                while !self.at(Tk::RBracket) && !self.at(Tk::Eof) && !self.at(Tk::Semi) {
                    let before = (self.pos, self.part);
                    self.parse_type();
                    if !self.eat(Tk::Comma) {
                        break;
                    }
                    if (self.pos, self.part) == before {
                        break;
                    }
                }
                if self.eat(Tk::Semi) {
                    self.parse_expr();
                }
                self.expect(Tk::RBracket, "`]`");
                self.error_type()
            }
            _ => {
                self.expected("a type");
                self.error_type()
            }
        }
    }

    fn parse_object_type(&mut self) -> TypeExpr {
        let lo = self.cur_lo();
        self.bump(); // {
        let mut fields = Vec::new();
        while !self.at(Tk::RBrace) && !self.at(Tk::Eof) {
            let before = (self.pos, self.part);
            let flo = self.cur_lo();
            let readonly = self.eat_kw(Kw::Readonly);
            let name = self.parse_ident("a field name");
            self.expect(Tk::Colon, "`:` and a field type");
            let ty = self.parse_type();
            let span = self.span_from(flo);
            fields.push(FieldSig {
                id: self.nid(),
                span,
                name,
                ty,
                readonly,
            });
            if !self.eat(Tk::Semi) && !self.eat(Tk::Comma) && !self.at(Tk::RBrace) {
                self.expected("`;`, `,` or `}`");
                if (self.pos, self.part) == before {
                    self.bump();
                }
                if self.at(Tk::Eof) {
                    break;
                }
            }
        }
        self.expect(Tk::RBrace, "`}`");
        self.mk_type(TypeExprKind::Object(fields), lo)
    }

    /// `[ "<" TypeParam { "," TypeParam } [ "," ] ">" ]`
    pub(super) fn parse_type_params(&mut self) -> Vec<TypeParam> {
        let mut v = Vec::new();
        if !self.at(Tk::Lt) {
            return v;
        }
        self.bump();
        while !self.at(Tk::Eof) && !self.at(Tk::Gt) {
            let before = (self.pos, self.part);
            let lo = self.cur_lo();
            let name = self.parse_ident("a type parameter name");
            let bound = if self.eat_kw(Kw::Extends) {
                Some(self.parse_type())
            } else {
                None
            };
            let span = self.span_from(lo);
            v.push(TypeParam {
                id: self.nid(),
                span,
                name,
                bound,
            });
            if !self.eat(Tk::Comma) {
                break;
            }
            if (self.pos, self.part) == before {
                break;
            }
        }
        if !self.eat_gt() {
            self.expected("`>`");
        }
        v
    }
}
