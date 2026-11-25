//! AST to document conversion.

use super::doc::{concat, group, if_break, indent, join, render_flat, text, Doc};
use super::{comment_kind_is_line, FormatOptions};
use crate::ast::*;
use crate::lexer::is_invisible;
use crate::lexer::{Comment, CommentKind};

pub(crate) struct Builder<'a> {
    src: &'a str,
    cm: &'a [Comment],
    ci: usize,
    o: &'a FormatOptions,
    /// Comments starting before this offset were found inside a printed element
    /// and are printed as plain `//` comments even if they are `///`.
    plain_upto: u32,
    /// Set by the caller of `seq`: per element, the number of `///` lines that
    /// the parser attached to it as documentation (`usize::MAX` = not an item).
    pending_docs: Vec<usize>,
    /// The last thing printed was a `//` comment (nothing may follow on its line).
    ended_line: bool,
}

fn prec(op: BinOp) -> u8 {
    match op {
        BinOp::Coalesce => 1,
        BinOp::Or => 2,
        BinOp::And => 3,
        BinOp::BitOr => 4,
        BinOp::BitXor => 5,
        BinOp::BitAnd => 6,
        BinOp::Eq | BinOp::Ne => 7,
        BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => 8,
        BinOp::Shl | BinOp::Shr => 9,
        BinOp::Add
        | BinOp::Sub
        | BinOp::AddWrap
        | BinOp::SubWrap
        | BinOp::AddSat
        | BinOp::SubSat => 10,
        _ => 11,
    }
}

fn op_text(op: BinOp) -> &'static str {
    crate::parser::op_text(op)
}

fn assign_text(op: Option<BinOp>) -> String {
    match op {
        None => "=".into(),
        Some(o) => format!("{}=", op_text(o)),
    }
}

fn nl() -> Doc {
    Doc::HardLine
}
fn t(s: &str) -> Doc {
    text(s)
}

pub(crate) fn quote_str(s: &str) -> String {
    let mut o = String::with_capacity(s.len() + 2);
    o.push('"');
    for c in s.chars() {
        match c {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '\t' => o.push_str("\\t"),
            '\0' => o.push_str("\\0"),
            c if c.is_ascii_control() => {
                o.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if is_invisible(c) => o.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn template_chunk(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\\' => o.push_str("\\\\"),
            '`' => o.push_str("\\`"),
            '$' if it.peek() == Some(&'{') => o.push_str("\\$"),
            '\r' => o.push_str("\\r"),
            '\0' => o.push_str("\\0"),
            c if c.is_ascii_control() && c != '\n' && c != '\t' => {
                o.push_str(&format!("\\x{:02x}", c as u32));
            }
            c if c != '\n' && c != '\t' && is_invisible(c) => {
                o.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            c => o.push(c),
        }
    }
    o
}

impl<'a> Builder<'a> {
    pub(crate) fn new(src: &'a str, cm: &'a [Comment], o: &'a FormatOptions) -> Self {
        Builder {
            src,
            cm,
            ci: 0,
            o,
            plain_upto: 0,
            pending_docs: Vec::new(),
            ended_line: false,
        }
    }

    // ---- comments ----------------------------------------------------------

    fn take_before(&mut self, pos: u32) -> Vec<Comment> {
        let mut v = Vec::new();
        while let Some(c) = self.cm.get(self.ci) {
            if c.lo < pos {
                v.push(*c);
                self.ci += 1;
            } else {
                break;
            }
        }
        v
    }

    /// A comment that is not the leading doc comment of an item or member
    /// (moved inside or after a statement): printed as `//` so that it cannot
    /// start documenting the next declaration.
    fn plain_comment_doc(&self, c: &Comment) -> Doc {
        if c.kind == CommentKind::Doc {
            let raw = &self.src[c.lo as usize + 3..c.hi as usize];
            return text(format!("//{}", raw.trim_end()));
        }
        self.comment_doc(c)
    }

    fn cdoc(&self, c: &Comment) -> Doc {
        if c.lo < self.plain_upto {
            self.plain_comment_doc(c)
        } else {
            self.comment_doc(c)
        }
    }

    fn comment_doc(&self, c: &Comment) -> Doc {
        let raw = &self.src[c.lo as usize..c.hi as usize];
        if c.kind == CommentKind::Block {
            text(raw.replace("\r\n", "\n").replace('\r', "\n"))
        } else {
            text(raw.trim_end())
        }
    }

    fn newlines_between(&self, a: u32, b: u32) -> usize {
        if b <= a {
            return 0;
        }
        let bytes = self.src.as_bytes();
        let hi = (b as usize).min(bytes.len());
        let lo = (a as usize).min(hi);
        let mut n = 0;
        for (i, &x) in bytes[lo..hi].iter().enumerate() {
            if x == b'\n' || (x == b'\r' && bytes.get(lo + i + 1) != Some(&b'\n')) {
                n += 1;
            }
        }
        n
    }

    /// Comments before `pos` that sit inside an expression, type or parameter.
    fn pre(&mut self, pos: u32) -> Doc {
        let cs = self.take_before(pos);
        if cs.is_empty() {
            return Doc::Nil;
        }
        let mut v = Vec::new();
        for c in &cs {
            v.push(self.plain_comment_doc(c));
            v.push(if comment_kind_is_line(c.kind) {
                nl()
            } else {
                t(" ")
            });
        }
        concat(v)
    }

    /// A sequence of statement-like elements separated by line breaks, with
    /// leading, trailing and dangling comments and at most one blank line.
    fn seq(
        &mut self,
        spans: &[(u32, u32)],
        build: &mut dyn FnMut(&mut Self, usize) -> Doc,
        close: u32,
    ) -> Doc {
        let docs = std::mem::take(&mut self.pending_docs);
        let mut parts: Vec<Doc> = Vec::new();
        let mut first = true;
        let mut prev_end = 0u32;
        let mut last_line = false;
        for (i, &(lo, hi)) in spans.iter().enumerate() {
            let lead = self.take_before(lo);
            // only the `///` lines the parser attached to this item stay `///`
            let keep = match docs.get(i) {
                Some(&d) if d != usize::MAX => {
                    let run = lead
                        .iter()
                        .rev()
                        .take_while(|c| c.kind == CommentKind::Doc)
                        .count();
                    d.min(run)
                }
                _ => usize::MAX,
            };
            for (k, c) in lead.iter().enumerate() {
                if !first {
                    parts.push(nl());
                    if self.newlines_between(prev_end, c.lo) >= 2 {
                        parts.push(nl());
                    }
                }
                let demote =
                    keep != usize::MAX && c.kind == CommentKind::Doc && k + keep < lead.len();
                parts.push(if demote {
                    self.plain_comment_doc(c)
                } else {
                    self.cdoc(c)
                });
                first = false;
                prev_end = c.hi;
            }
            if !first {
                parts.push(nl());
                if self.newlines_between(prev_end, lo) >= 2 {
                    parts.push(nl());
                }
            }
            self.ended_line = false;
            let d = build(self, i);
            parts.push(d);
            first = false;
            prev_end = hi;
            last_line = self.ended_line;
            // trailing comments: on the same line as the end of the last thing
            // printed there; nothing may follow a `//` comment on that line
            let mut line_end = hi;
            let mut open = !self.ended_line;
            while let Some(c) = self.cm.get(self.ci).copied() {
                let inner = c.lo < hi;
                if !inner && (!open || c.lo >= close || !self.same_line_gap(line_end, c.lo)) {
                    break;
                }
                if inner && !open {
                    self.plain_upto = self.plain_upto.max(hi);
                    break;
                }
                parts.push(t(" "));
                if inner {
                    parts.push(self.plain_comment_doc(&c));
                } else {
                    parts.push(self.cdoc(&c));
                }
                prev_end = prev_end.max(c.hi);
                line_end = c.hi;
                last_line = c.kind != CommentKind::Block;
                if c.kind != CommentKind::Block {
                    open = false;
                }
                self.ci += 1;
            }
        }
        for c in self.take_before(close) {
            if !first {
                parts.push(nl());
                if self.newlines_between(prev_end, c.lo) >= 2 {
                    parts.push(nl());
                }
            }
            parts.push(self.cdoc(&c));
            first = false;
            prev_end = c.hi;
            last_line = c.kind != CommentKind::Block;
        }
        self.ended_line = last_line;
        concat(parts)
    }

    /// End of an interface method signature including its `;`.
    fn sig_end(&self, f: &FnDecl) -> u32 {
        let mut hi = f.name.span.hi;
        for p in &f.params {
            hi = hi.max(p.span.hi);
        }
        if let Some(r) = &f.ret {
            hi = hi.max(r.span.hi);
        }
        let bytes = self.src.as_bytes();
        let mut i = hi as usize;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if bytes.get(i) == Some(&b';') {
            i += 1;
        }
        i as u32
    }

    /// Only blanks, `,` and `;` between `a` and `b`, on one line: a comment at `b`
    /// trails whatever ended at `a` (no code in between).
    fn same_line_gap(&self, a: u32, b: u32) -> bool {
        let bytes = self.src.as_bytes();
        let hi = (b as usize).min(bytes.len());
        let lo = (a as usize).min(hi);
        bytes[lo..hi]
            .iter()
            .all(|&x| matches!(x, b' ' | b'\t' | b',' | b';' | 0x0b | 0x0c))
    }

    fn has_comment_before(&self, pos: u32) -> bool {
        self.cm.get(self.ci).is_some_and(|c| c.lo < pos)
    }

    // ---- module and items --------------------------------------------------

    pub(crate) fn module(&mut self, m: &Module) -> Doc {
        enum E<'m> {
            A(&'m Attribute),
            I(&'m Item),
        }
        let mut els: Vec<E> = m.attrs.iter().map(E::A).collect();
        els.extend(m.items.iter().map(E::I));
        let spans: Vec<(u32, u32)> = els
            .iter()
            .map(|e| match e {
                E::A(a) => (a.span.lo, a.span.hi),
                E::I(i) => (i.span.lo, i.span.hi),
            })
            .collect();
        let end = u32::try_from(self.src.len())
            .unwrap_or(u32::MAX)
            .saturating_add(1);
        self.pending_docs = els
            .iter()
            .map(|e| match e {
                E::A(_) => usize::MAX,
                E::I(i) => i.doc.len(),
            })
            .collect();
        self.seq(
            &spans,
            &mut |b, i| match &els[i] {
                E::A(a) => concat(vec![b.attr(a), t(";")]),
                E::I(it) => b.item(it),
            },
            end,
        )
    }

    fn attr(&mut self, a: &Attribute) -> Doc {
        let mut v = vec![t(if a.inner { "@@" } else { "@" }), t(&a.name.name)];
        if !a.args.is_empty() {
            let mut args: Vec<Doc> = Vec::new();
            for x in &a.args {
                let mut p = vec![self.pre(x.span.lo)];
                if let Some(k) = &x.key {
                    p.push(t(&k.name));
                    p.push(t(": "));
                }
                match &x.value {
                    AttrValue::Lit(l) => p.push(t(&self.lit_text(l))),
                    AttrValue::Path(ps) => {
                        let s: Vec<&str> = ps.iter().map(|i| i.name.as_str()).collect();
                        p.push(t(&s.join(".")));
                    }
                }
                args.push(concat(p));
            }
            v.push(t("("));
            v.push(join(|| t(", "), args));
            let tail = self.pre(a.span.hi.saturating_sub(1));
            if !tail.is_nil() {
                v.push(t(" "));
                v.push(tail);
            }
            v.push(t(")"));
        }
        concat(v)
    }

    fn attrs_lines(&mut self, attrs: &[Attribute]) -> Doc {
        let mut v = Vec::new();
        for a in attrs {
            v.push(self.attr(a));
            v.push(nl());
        }
        concat(v)
    }

    fn item(&mut self, it: &Item) -> Doc {
        let mut v = vec![self.attrs_lines(&it.attrs)];
        if it.export {
            v.push(t("export "));
        }
        v.push(match &it.kind {
            ItemKind::Import(i) => self.import(i),
            ItemKind::Var(d) => concat(vec![self.var_decl(d), t(";")]),
            ItemKind::Fn(f) => self.fn_decl(f, "function "),
            ItemKind::Record(r) => self.record(r, it.span.hi.saturating_sub(1)),
            ItemKind::Interface(i) => self.interface(i, it.span.hi.saturating_sub(1)),
            ItemKind::Enum(e) => self.enum_decl(e, it.span.hi.saturating_sub(1)),
            ItemKind::TypeAlias(a) => self.type_alias(a),
            ItemKind::Law(l) => self.law(l),
        });
        concat(v)
    }

    fn import(&mut self, i: &ImportDecl) -> Doc {
        let clause = match &i.clause {
            ImportClause::Named(specs) => {
                if specs.is_empty() {
                    t("{}")
                } else {
                    let docs: Vec<Doc> = specs
                        .iter()
                        .map(|s| {
                            let mut p = vec![t(&s.name.name)];
                            if let Some(a) = &s.alias {
                                p.push(t(" as "));
                                p.push(t(&a.name));
                            }
                            concat(p)
                        })
                        .collect();
                    group(concat(vec![
                        t("{"),
                        indent(concat(vec![
                            Doc::Line,
                            join(|| concat(vec![t(","), Doc::Line]), docs),
                        ])),
                        if_break(t(","), Doc::Nil),
                        Doc::Line,
                        t("}"),
                    ]))
                }
            }
            ImportClause::Namespace(n) => concat(vec![t("* as "), t(&n.name)]),
        };
        concat(vec![
            t("import "),
            clause,
            t(" from "),
            t(&self.lit_text(&i.source)),
            t(";"),
        ])
    }

    fn type_params(&mut self, tps: &[TypeParam]) -> Doc {
        if tps.is_empty() {
            return Doc::Nil;
        }
        let docs: Vec<Doc> = tps
            .iter()
            .map(|p| {
                let mut v = vec![t(&p.name.name)];
                if let Some(b) = &p.bound {
                    v.push(t(" extends "));
                    v.push(self.ty(b));
                }
                concat(v)
            })
            .collect();
        concat(vec![t("<"), join(|| t(", "), docs), t(">")])
    }

    fn param(&mut self, p: &Param) -> Doc {
        let mut v = vec![self.pre(p.span.lo)];
        if p.mode == ParamMode::Inout {
            v.push(t("inout "));
        }
        v.push(t(&p.name.name));
        if let Some(ty) = &p.ty {
            v.push(t(": "));
            v.push(self.ty(ty));
        }
        if let Some(d) = &p.default {
            v.push(t(" = "));
            v.push(self.expr(d));
        }
        concat(v)
    }

    fn params(&mut self, ps: &[Param]) -> Doc {
        if ps.is_empty() {
            return t("()");
        }
        let docs: Vec<Doc> = ps.iter().map(|p| self.param(p)).collect();
        group(concat(vec![
            t("("),
            indent(concat(vec![
                Doc::SoftLine,
                join(|| concat(vec![t(","), Doc::Line]), docs),
            ])),
            if_break(t(","), Doc::Nil),
            Doc::SoftLine,
            t(")"),
        ]))
    }

    fn fn_head(&mut self, f: &FnDecl, kw: &str) -> Doc {
        let mut v = Vec::new();
        if f.mods.is_async {
            v.push(t("async "));
        }
        if f.mods.kernel {
            v.push(t("kernel "));
        }
        v.push(t(kw));
        v.push(t(&f.name.name));
        v.push(self.type_params(&f.type_params));
        v.push(self.params(&f.params));
        if let Some(r) = &f.ret {
            v.push(t(": "));
            v.push(self.ty(r));
        }
        for c in &f.contracts {
            v.push(t(match c.kind {
                ContractKind::Requires => " requires ",
                ContractKind::Ensures => " ensures ",
            }));
            v.push(self.expr(&c.expr));
        }
        concat(v)
    }

    fn fn_decl(&mut self, f: &FnDecl, kw: &str) -> Doc {
        let head = self.fn_head(f, kw);
        match &f.body {
            Some(b) => concat(vec![head, t(" "), self.block(b)]),
            None => concat(vec![head, t(";")]),
        }
    }

    fn record(&mut self, r: &RecordDecl, close: u32) -> Doc {
        let mut v = vec![t(match r.kind {
            RecordKind::Struct => "struct ",
            RecordKind::Class => "class ",
        })];
        v.push(t(&r.name.name));
        v.push(self.type_params(&r.type_params));
        if !r.implements.is_empty() {
            let ds: Vec<Doc> = r.implements.iter().map(|x| self.ty(x)).collect();
            v.push(t(" implements "));
            v.push(join(|| t(", "), ds));
        }
        v.push(t(" "));
        let spans: Vec<(u32, u32)> = r.members.iter().map(|m| (m.span.lo, m.span.hi)).collect();
        self.pending_docs = r.members.iter().map(|m| m.doc.len()).collect();
        v.push(self.braced_seq(&spans, close, &mut |b, i| b.member(&r.members[i])));
        concat(v)
    }

    fn braced_seq(
        &mut self,
        spans: &[(u32, u32)],
        close: u32,
        build: &mut dyn FnMut(&mut Self, usize) -> Doc,
    ) -> Doc {
        if spans.is_empty() && !self.has_comment_before(close) {
            return t("{}");
        }
        let body = self.seq(spans, build, close);
        self.ended_line = false;
        concat(vec![t("{"), indent(concat(vec![nl(), body])), nl(), t("}")])
    }

    fn member(&mut self, m: &Member) -> Doc {
        let mut v = vec![self.attrs_lines(&m.attrs)];
        match &m.kind {
            MemberKind::Field(f) => {
                if f.private {
                    v.push(t("private "));
                }
                if f.is_static {
                    v.push(t("static "));
                }
                if f.readonly {
                    v.push(t("readonly "));
                }
                v.push(t(&f.name.name));
                v.push(t(": "));
                v.push(self.ty(&f.ty));
                if let Some(i) = &f.init {
                    v.push(t(" = "));
                    v.push(self.expr(i));
                }
                v.push(t(";"));
            }
            MemberKind::Method(f) => {
                if f.mods.private {
                    v.push(t("private "));
                }
                if f.mods.is_static {
                    v.push(t("static "));
                }
                if f.mods.mutating {
                    v.push(t("mutating "));
                }
                v.push(self.fn_decl(f, ""));
            }
            MemberKind::Constructor(f) => v.push(self.fn_decl(f, "")),
        }
        concat(v)
    }

    fn interface(&mut self, i: &InterfaceDecl, close: u32) -> Doc {
        let mut v = vec![
            t("interface "),
            t(&i.name.name),
            self.type_params(&i.type_params),
        ];
        if !i.extends.is_empty() {
            let ds: Vec<Doc> = i.extends.iter().map(|x| self.ty(x)).collect();
            v.push(t(" extends "));
            v.push(join(|| t(", "), ds));
        }
        v.push(t(" "));
        let spans: Vec<(u32, u32)> = i
            .methods
            .iter()
            .map(|m| (m.name.span.lo, self.sig_end(m)))
            .collect();
        v.push(self.braced_seq(&spans, close, &mut |b, k| b.fn_decl(&i.methods[k], "")));
        concat(v)
    }

    fn enum_decl(&mut self, e: &EnumDecl, close: u32) -> Doc {
        let mut v = vec![t("enum "), t(&e.name.name)];
        if let Some(r) = &e.repr {
            v.push(t(": "));
            v.push(self.ty(r));
        }
        v.push(t(" "));
        let spans: Vec<(u32, u32)> = e.variants.iter().map(|x| (x.span.lo, x.span.hi)).collect();
        v.push(self.braced_seq(&spans, close, &mut |b, k| {
            let x = &e.variants[k];
            let mut p = vec![t(&x.name.name)];
            if let Some(val) = &x.value {
                p.push(t(" = "));
                p.push(b.expr(val));
            }
            p.push(t(","));
            concat(p)
        }));
        concat(v)
    }

    fn type_alias(&mut self, a: &TypeAliasDecl) -> Doc {
        let head = concat(vec![
            t("type "),
            t(&a.name.name),
            self.type_params(&a.type_params),
            t(" ="),
        ]);
        let rhs = match &a.ty.kind {
            TypeExprKind::Union(ms) => {
                let docs: Vec<Doc> = ms.iter().map(|m| self.ty(m)).collect();
                concat(vec![indent(concat(vec![
                    Doc::Line,
                    if_break(t("| "), Doc::Nil),
                    join(|| concat(vec![Doc::Line, t("| ")]), docs),
                ]))])
            }
            _ => concat(vec![t(" "), self.ty(&a.ty)]),
        };
        group(concat(vec![head, rhs, t(";")]))
    }

    fn law(&mut self, l: &LawDecl) -> Doc {
        let mut v = vec![
            t("law "),
            t(&l.name.name),
            self.type_params(&l.type_params),
            self.params(&l.params),
        ];
        if let Some(w) = &l.when {
            v.push(t(" when "));
            v.push(self.expr(w));
        }
        v.push(t(" "));
        v.push(self.block(&l.body));
        concat(v)
    }

    // ---- types ---------------------------------------------------------------

    fn ty(&mut self, ty: &TypeExpr) -> Doc {
        let pre = self.pre(ty.span.lo);
        let d = match &ty.kind {
            TypeExprKind::Path { segments, args } => {
                let s: Vec<&str> = segments.iter().map(|i| i.name.as_str()).collect();
                let mut v = vec![t(&s.join("."))];
                if !args.is_empty() {
                    let ds: Vec<Doc> = args.iter().map(|a| self.ty(a)).collect();
                    v.push(t("<"));
                    v.push(join(|| t(", "), ds));
                    v.push(t(">"));
                }
                concat(v)
            }
            TypeExprKind::Array(inner) => concat(vec![self.ty(inner), t("[]")]),
            TypeExprKind::Union(ms) => {
                let ds: Vec<Doc> = ms.iter().map(|m| self.ty(m)).collect();
                let mut it = ds.into_iter();
                let first = it.next().unwrap_or(Doc::Nil);
                let mut rest = Vec::new();
                for d in it {
                    rest.push(Doc::Line);
                    rest.push(t("| "));
                    rest.push(d);
                }
                group(concat(vec![first, concat(rest)]))
            }
            TypeExprKind::Fn { params, ret } => {
                let ps: Vec<Doc> = params
                    .iter()
                    .map(|p| concat(vec![t(&p.name.name), t(": "), self.ty(&p.ty)]))
                    .collect();
                concat(vec![t("("), join(|| t(", "), ps), t(") => "), self.ty(ret)])
            }
            TypeExprKind::Lit(l) => t(&self.lit_text(l)),
            TypeExprKind::Object(fs) => {
                if fs.is_empty() {
                    t("{}")
                } else {
                    let ds: Vec<Doc> = fs
                        .iter()
                        .map(|f| {
                            let mut v = Vec::new();
                            if f.readonly {
                                v.push(t("readonly "));
                            }
                            v.push(t(&f.name.name));
                            v.push(t(": "));
                            v.push(self.ty(&f.ty));
                            concat(v)
                        })
                        .collect();
                    group(concat(vec![
                        t("{"),
                        indent(concat(vec![
                            Doc::Line,
                            join(|| concat(vec![t(";"), Doc::Line]), ds),
                        ])),
                        if_break(t(";"), Doc::Nil),
                        Doc::Line,
                        t("}"),
                    ]))
                }
            }
            TypeExprKind::Null => t("null"),
            TypeExprKind::Paren(inner) => concat(vec![t("("), self.ty(inner), t(")")]),
        };
        concat(vec![pre, d])
    }

    // ---- statements ----------------------------------------------------------

    fn block(&mut self, b: &Block) -> Doc {
        let close = b.span.hi.saturating_sub(1);
        let mut spans: Vec<(u32, u32)> = b.stmts.iter().map(|s| (s.span.lo, s.span.hi)).collect();
        if let Some(tail) = &b.tail {
            spans.push((tail.span.lo, tail.span.hi));
        }
        let n = b.stmts.len();
        self.pending_docs = b.stmts.iter().map(stmt_doc_len).collect();
        self.braced_seq(&spans, close, &mut |bd, i| {
            if i < n {
                bd.stmt(&b.stmts[i])
            } else {
                match &b.tail {
                    Some(e) => bd.expr(e),
                    None => Doc::Nil,
                }
            }
        })
    }

    fn stmt(&mut self, s: &Stmt) -> Doc {
        let mut v = vec![self.attrs_lines(&s.attrs)];
        v.push(self.stmt_kind(s));
        concat(v)
    }

    fn var_decl(&mut self, d: &VarDecl) -> Doc {
        let mut v = vec![t(match d.kind {
            VarKind::Let => "let ",
            VarKind::Const => "const ",
        })];
        v.push(self.pattern(&d.pattern));
        if let Some(ty) = &d.ty {
            v.push(t(": "));
            v.push(self.ty(ty));
        }
        if let Some(i) = &d.init {
            return self.assign_like(concat(v), "=", i);
        }
        concat(v)
    }

    fn pattern(&mut self, p: &Pattern) -> Doc {
        match &p.kind {
            PatternKind::Ident(i) => t(&i.name),
            PatternKind::Wildcard => t("_"),
            PatternKind::Object(fs) => {
                let ds: Vec<Doc> = fs
                    .iter()
                    .map(|f| match &f.pattern {
                        Some(sub) => concat(vec![t(&f.key.name), t(": "), self.pattern(sub)]),
                        None => t(&f.key.name),
                    })
                    .collect();
                if ds.is_empty() {
                    t("{}")
                } else {
                    concat(vec![t("{ "), join(|| t(", "), ds), t(" }")])
                }
            }
        }
    }

    /// Body of `if`/`while`/`for`: returns the doc and whether it ends in `}`.
    fn body(&mut self, s: &Stmt) -> (Doc, bool) {
        if matches!(s.kind, StmtKind::Block(_)) {
            return (concat(vec![t(" "), self.stmt(s)]), true);
        }
        let spans = [(s.span.lo, s.span.hi)];
        self.pending_docs = vec![stmt_doc_len(s)];
        let one = self.seq(&spans, &mut |b, _| b.stmt(s), s.span.hi);
        if self.o.braces {
            let inner = if matches!(s.kind, StmtKind::Empty) && s.attrs.is_empty() {
                return (concat(vec![t(" {}"), one_discard(one)]), true);
            } else {
                one
            };
            (
                concat(vec![
                    t(" {"),
                    indent(concat(vec![nl(), inner])),
                    nl(),
                    t("}"),
                ]),
                true,
            )
        } else {
            (indent(concat(vec![nl(), one])), false)
        }
    }

    fn paren_head(&mut self, kw: &str, cond: &Expr) -> Doc {
        let c = self.expr(cond);
        concat(vec![
            t(kw),
            t(" ("),
            group(concat(vec![
                indent(concat(vec![Doc::SoftLine, c])),
                Doc::SoftLine,
            ])),
            t(")"),
        ])
    }

    fn if_doc(&mut self, cond: &Expr, then: &Stmt, els: &Option<Box<Stmt>>) -> Doc {
        // an `else if` ladder is printed with a loop, not recursion
        let mut v = Vec::new();
        let (mut cond, mut then, mut els) = (cond, then, els);
        loop {
            v.push(self.paren_head("if", cond));
            let (b, closed) = self.body(then);
            v.push(b);
            let Some(e) = els else { break };
            if closed {
                v.push(t(" else"));
            } else {
                v.push(nl());
                v.push(t("else"));
            }
            match &e.kind {
                StmtKind::If {
                    cond: c,
                    then: th,
                    els: el,
                } if e.attrs.is_empty() => {
                    v.push(t(" "));
                    (cond, then, els) = (c, th, el);
                }
                _ => {
                    let (b, _) = self.body(e);
                    v.push(b);
                    break;
                }
            }
        }
        concat(v)
    }

    fn stmt_kind(&mut self, s: &Stmt) -> Doc {
        match &s.kind {
            StmtKind::Var(d) => concat(vec![self.var_decl(d), t(";")]),
            StmtKind::Expr(e) => {
                let d = self.expr(e);
                let d = if matches!(e.kind, ExprKind::Binary { .. }) {
                    indent(d)
                } else {
                    d
                };
                concat(vec![d, t(";")])
            }
            StmtKind::If { cond, then, els } => self.if_doc(cond, then, els),
            StmtKind::While { cond, body } => {
                let h = self.paren_head("while", cond);
                let (b, _) = self.body(body);
                concat(vec![h, b])
            }
            StmtKind::DoWhile { body, cond } => {
                let (b, closed) = self.body(body);
                let h = self.paren_head("while", cond);
                concat(vec![
                    t("do"),
                    b,
                    if closed { t(" ") } else { nl() },
                    h,
                    t(";"),
                ])
            }
            StmtKind::For {
                init,
                cond,
                update,
                body,
                parallel,
            } => {
                let init_d = match init {
                    Some(i) => match &i.kind {
                        StmtKind::Var(d) => self.var_decl(d),
                        StmtKind::Expr(e) => self.expr(e),
                        _ => Doc::Nil,
                    },
                    None => Doc::Nil,
                };
                let cond_d = cond.as_ref().map_or(Doc::Nil, |c| self.expr(c));
                let upd_d = update.as_ref().map_or(Doc::Nil, |c| self.expr(c));
                let (b, _) = self.body(body);
                let sep = |d: Doc| {
                    if d.is_nil() {
                        Doc::Nil
                    } else {
                        concat(vec![Doc::Line, d])
                    }
                };
                let head = group(concat(vec![
                    t("("),
                    indent(concat(vec![
                        Doc::SoftLine,
                        init_d,
                        t(";"),
                        sep(cond_d),
                        t(";"),
                        sep(upd_d),
                    ])),
                    Doc::SoftLine,
                    t(")"),
                ]));
                concat(vec![
                    t(if *parallel { "parallel for " } else { "for " }),
                    head,
                    b,
                ])
            }
            StmtKind::ForOf {
                kind,
                binding,
                iter,
                body,
                parallel,
            } => {
                let it = self.expr(iter);
                let (b, _) = self.body(body);
                concat(vec![
                    t(if *parallel { "parallel for (" } else { "for (" }),
                    t(match kind {
                        VarKind::Let => "let ",
                        VarKind::Const => "const ",
                    }),
                    self.pattern(binding),
                    t(" of "),
                    it,
                    t(")"),
                    b,
                ])
            }
            StmtKind::Switch(sw) => self.switch_stmt(sw, s.span.hi),
            StmtKind::Return(e) => match e {
                Some(e) => {
                    let d = self.expr(e);
                    let d = if matches!(e.kind, ExprKind::Binary { .. }) {
                        indent(d)
                    } else {
                        d
                    };
                    concat(vec![t("return "), d, t(";")])
                }
                None => t("return;"),
            },
            StmtKind::Break(l) => match l {
                Some(l) => t(&format!("break {};", l.name)),
                None => t("break;"),
            },
            StmtKind::Continue(l) => match l {
                Some(l) => t(&format!("continue {};", l.name)),
                None => t("continue;"),
            },
            StmtKind::Block(b) => self.block(b),
            StmtKind::Labeled { label, body } => {
                concat(vec![t(&label.name), t(": "), self.stmt(body)])
            }
            StmtKind::Item(it) => self.item(it),
            StmtKind::Empty => t(";"),
        }
    }

    fn switch_stmt(&mut self, sw: &SwitchStmt, end: u32) -> Doc {
        let head = self.paren_head("switch", &sw.scrutinee);
        let spans: Vec<(u32, u32)> = sw.cases.iter().map(|c| (c.span.lo, c.span.hi)).collect();
        let close = end.saturating_sub(1);
        let body = self.braced_seq(&spans, close, &mut |b, i| b.case(&sw.cases[i]));
        concat(vec![head, t(" "), body])
    }

    fn case(&mut self, c: &Case) -> Doc {
        let mut v = Vec::new();
        if !c.labels.is_empty() {
            let ls: Vec<Doc> = c.labels.iter().map(|l| self.expr(l)).collect();
            v.push(t("case "));
            v.push(join(|| t(", "), ls));
            v.push(t(":"));
        }
        if c.is_default {
            if !c.labels.is_empty() {
                v.push(nl());
            }
            v.push(t("default:"));
        }
        if c.body.len() == 1
            && matches!(c.body[0].kind, StmtKind::Block(_))
            && c.body[0].attrs.is_empty()
        {
            v.push(t(" "));
            v.push(self.stmt(&c.body[0]));
        } else if !c.body.is_empty() {
            let spans: Vec<(u32, u32)> = c.body.iter().map(|s| (s.span.lo, s.span.hi)).collect();
            let close = 0; // leftovers belong to the enclosing sequence (stable under re-formatting)
            self.pending_docs = c.body.iter().map(stmt_doc_len).collect();
            let body = self.seq(&spans, &mut |b, i| b.stmt(&c.body[i]), close);
            v.push(indent(concat(vec![nl(), body])));
        }
        concat(v)
    }

    // ---- expressions ---------------------------------------------------------

    fn lit_text(&self, l: &Lit) -> String {
        match &l.kind {
            LitKind::Int { .. } | LitKind::Float { .. } => {
                self.src[l.span.lo as usize..l.span.hi as usize].to_string()
            }
            LitKind::Str(s) => quote_str(s),
            LitKind::Bool(b) => b.to_string(),
            LitKind::Null => "null".into(),
        }
    }

    fn assign_like(&mut self, lhs: Doc, op: &str, rhs: &Expr) -> Doc {
        let r = self.expr(rhs);
        let breaks_after = matches!(rhs.kind, ExprKind::Binary { .. })
            || matches!(&rhs.kind, ExprKind::Lit(l) if matches!(l.kind, LitKind::Str(_)));
        if breaks_after {
            group(concat(vec![
                lhs,
                t(" "),
                t(op),
                group(indent(concat(vec![Doc::Line, r]))),
            ]))
        } else {
            concat(vec![lhs, t(" "), t(op), t(" "), r])
        }
    }

    fn is_hug(e: &Expr) -> bool {
        match &e.kind {
            ExprKind::Arrow(_) => true,
            ExprKind::Object(f) => !f.is_empty(),
            ExprKind::Array(f) => !f.is_empty(),
            _ => false,
        }
    }

    fn args(&mut self, args: &[Expr]) -> Doc {
        if args.is_empty() {
            return t("()");
        }
        let docs: Vec<Doc> = args.iter().map(|a| self.expr(a)).collect();
        let n = docs.len();
        if Self::is_hug(&args[n - 1]) && !docs[..n - 1].iter().any(Doc::hard) {
            return concat(vec![t("("), join(|| t(", "), docs), t(")")]);
        }
        group(concat(vec![
            t("("),
            indent(concat(vec![
                Doc::SoftLine,
                join(|| concat(vec![t(","), Doc::Line]), docs),
            ])),
            if_break(t(","), Doc::Nil),
            Doc::SoftLine,
            t(")"),
        ]))
    }

    fn expr(&mut self, e: &Expr) -> Doc {
        let pre = self.pre(e.span.lo);
        let d = self.expr_inner(e);
        if pre.is_nil() {
            d
        } else {
            concat(vec![pre, d])
        }
    }

    /// The left spine of `a + b as T + c.d()? ...`: binary operators, casts and
    /// postfix operators of any precedence are walked with ONE loop (never
    /// recursion per link, even when the kinds alternate) and printed bottom-up;
    /// each run of equal-precedence binary operators is one flat group.
    /// Parentheses come from `Paren` nodes, so no precedence logic is needed.
    #[inline(never)]
    fn spine(&mut self, e: &Expr) -> Doc {
        let mut nodes: Vec<&Expr> = Vec::new();
        let mut cur = e;
        loop {
            match &cur.kind {
                ExprKind::Binary { lhs: next, .. }
                | ExprKind::Member { obj: next, .. }
                | ExprKind::Index { obj: next, .. }
                | ExprKind::Call { callee: next, .. }
                | ExprKind::Try(next)
                | ExprKind::Cast { expr: next, .. }
                | ExprKind::Update {
                    prefix: false,
                    target: next,
                    ..
                } => {
                    nodes.push(cur);
                    cur = next;
                }
                _ => break,
            }
        }
        nodes.reverse();
        let mut first: Vec<Doc> = vec![self.expr(cur)];
        let mut run: Option<(u8, Vec<Doc>)> = None;
        let mut prev_try = false;
        for node in nodes {
            if let ExprKind::Binary { op, rhs, .. } = &node.kind {
                let r = self.expr(rhs);
                let piece = concat(vec![t(" "), t(op_text(*op)), Doc::Line, r]);
                let p = prec(*op);
                match &mut run {
                    Some((rp, rest)) if *rp == p => rest.push(piece),
                    _ => {
                        let lhs = finish_run(&mut first, run.take());
                        first = vec![lhs];
                        run = Some((p, vec![piece]));
                    }
                }
                prev_try = false;
                continue;
            }
            // postfix operator or cast: closes any binary run
            if run.is_some() {
                let lhs = finish_run(&mut first, run.take());
                first = vec![lhs];
            }
            let is_access = matches!(
                node.kind,
                ExprKind::Member { .. } | ExprKind::Index { .. } | ExprKind::Call { .. }
            );
            if prev_try && is_access {
                // `f()?.y` would read as optional chaining: spell `(f()?).y`
                let inner = std::mem::take(&mut first);
                first.push(concat(vec![t("("), concat(inner), t(")")]));
            }
            prev_try = matches!(node.kind, ExprKind::Try(_));
            let d = self.postfix_op(node);
            first.push(d);
        }
        finish_run(&mut first, run.take())
    }

    fn postfix_op(&mut self, op: &Expr) -> Doc {
        match &op.kind {
            ExprKind::Member { name, optional, .. } => {
                concat(vec![t(if *optional { "?." } else { "." }), t(&name.name)])
            }
            ExprKind::Index {
                index, optional, ..
            } => concat(vec![
                t(if *optional { "?.[" } else { "[" }),
                self.expr(index),
                t("]"),
            ]),
            ExprKind::Call {
                type_args,
                args,
                optional,
                ..
            } => {
                let mut v = Vec::new();
                if *optional {
                    v.push(t("?."));
                }
                if !type_args.is_empty() {
                    let ds: Vec<Doc> = type_args.iter().map(|a| self.ty(a)).collect();
                    v.push(t("<"));
                    v.push(join(|| t(", "), ds));
                    v.push(t(">"));
                }
                v.push(self.args(args));
                concat(v)
            }
            ExprKind::Try(_) => t("?"),
            ExprKind::Cast { ty, .. } => concat(vec![t(" as "), self.ty(ty)]),
            ExprKind::Update { op, .. } => t(if *op == IncDec::Inc { "++" } else { "--" }),
            _ => Doc::Nil,
        }
    }

    /// `a ? b : c ? d : e`: one flat group, built with a loop.
    #[inline(never)]
    fn cond_chain(&mut self, e: &Expr) -> Doc {
        let mut v: Vec<Doc> = Vec::new();
        let mut cur = e;
        let mut first = true;
        while let ExprKind::Cond { cond, then, els } = &cur.kind {
            let c = self.expr(cond);
            let a = self.expr(then);
            if first {
                v.push(c);
                first = false;
            } else {
                v.push(Doc::Line);
                v.push(t(": "));
                v.push(c);
            }
            v.push(Doc::Line);
            v.push(t("? "));
            v.push(a);
            cur = els;
        }
        let last = self.expr(cur);
        v.push(Doc::Line);
        v.push(t(": "));
        v.push(last);
        let head = v.remove(0);
        group(concat(vec![head, indent(concat(v))]))
    }

    #[allow(clippy::too_many_lines)]
    fn expr_inner(&mut self, e: &Expr) -> Doc {
        match &e.kind {
            ExprKind::Lit(l) => t(&self.lit_text(l)),
            ExprKind::Template(parts) => {
                let mut v = vec![t("`")];
                for p in parts {
                    match p {
                        TemplatePart::Str(s) => v.push(t(&template_chunk(s))),
                        TemplatePart::Expr(x) => {
                            let d = self.expr(x);
                            v.push(t("${"));
                            v.push(t(&render_flat(&d)));
                            v.push(t("}"));
                        }
                    }
                }
                v.push(t("`"));
                concat(v)
            }
            ExprKind::Ident(i) => t(&i.name),
            ExprKind::This => t("this"),
            ExprKind::Array(xs) => {
                if xs.is_empty() {
                    return t("[]");
                }
                let ds: Vec<Doc> = xs.iter().map(|x| self.expr(x)).collect();
                group(concat(vec![
                    t("["),
                    indent(concat(vec![
                        Doc::SoftLine,
                        join(|| concat(vec![t(","), Doc::Line]), ds),
                    ])),
                    if_break(t(","), Doc::Nil),
                    Doc::SoftLine,
                    t("]"),
                ]))
            }
            ExprKind::Object(fs) => {
                if fs.is_empty() {
                    return t("{}");
                }
                let ds: Vec<Doc> = fs
                    .iter()
                    .map(|f| match &f.value {
                        Some(v) => concat(vec![t(&f.name.name), t(": "), self.expr(v)]),
                        None => t(&f.name.name),
                    })
                    .collect();
                group(concat(vec![
                    t("{"),
                    indent(concat(vec![
                        Doc::Line,
                        join(|| concat(vec![t(","), Doc::Line]), ds),
                    ])),
                    if_break(t(","), Doc::Nil),
                    Doc::Line,
                    t("}"),
                ]))
            }
            ExprKind::Unary { op, expr } => {
                let s = match op {
                    UnOp::Neg => "-",
                    UnOp::Not => "!",
                    UnOp::BitNot => "~",
                };
                let space = *op == UnOp::Neg
                    && matches!(
                        &expr.kind,
                        ExprKind::Unary { op: UnOp::Neg, .. }
                            | ExprKind::Update {
                                op: IncDec::Dec,
                                prefix: true,
                                ..
                            }
                    );
                concat(vec![
                    t(s),
                    if space { t(" ") } else { Doc::Nil },
                    self.expr(expr),
                ])
            }
            ExprKind::Assign { op, target, value } => {
                let l = self.expr(target);
                self.assign_like(l, &assign_text(*op), value)
            }
            ExprKind::Update {
                op,
                prefix: true,
                target,
            } => {
                let s = if *op == IncDec::Inc { "++" } else { "--" };
                concat(vec![t(s), self.expr(target)])
            }
            ExprKind::Update { .. }
            | ExprKind::Call { .. }
            | ExprKind::Member { .. }
            | ExprKind::Index { .. }
            | ExprKind::Try(_)
            | ExprKind::Cast { .. }
            | ExprKind::Binary { .. } => self.spine(e),
            ExprKind::New { ty, args } => {
                let ty = self.ty(ty);
                concat(vec![t("new "), ty, self.args(args)])
            }
            ExprKind::Cond { .. } => self.cond_chain(e),
            ExprKind::Arrow(a) => {
                let ps = self.params(&a.params);
                let mut v = vec![ps];
                if let Some(r) = &a.ret {
                    v.push(t(": "));
                    v.push(self.ty(r));
                }
                v.push(t(" =>"));
                match &a.body {
                    ArrowBody::Block(b) => {
                        v.push(t(" "));
                        v.push(self.block(b));
                    }
                    ArrowBody::Expr(x) => {
                        let body = self.expr(x);
                        if matches!(x.kind, ExprKind::Binary { .. } | ExprKind::Cond { .. }) {
                            v.push(group(indent(concat(vec![Doc::Line, body]))));
                        } else {
                            v.push(t(" "));
                            v.push(body);
                        }
                    }
                }
                concat(v)
            }
            ExprKind::Switch(sw) => {
                let sc = self.expr(&sw.scrutinee);
                let spans: Vec<(u32, u32)> =
                    sw.arms.iter().map(|a| (a.span.lo, a.span.hi)).collect();
                let close = e.span.hi.saturating_sub(1);
                let body = self.braced_seq(&spans, close, &mut |b, i| {
                    let arm = &sw.arms[i];
                    let mut v = Vec::new();
                    if arm.is_default {
                        v.push(t("default: "));
                    } else {
                        let ls: Vec<Doc> = arm.labels.iter().map(|l| b.expr(l)).collect();
                        v.push(t("case "));
                        v.push(join(|| t(", "), ls));
                        v.push(t(": "));
                    }
                    v.push(b.expr(&arm.body));
                    v.push(t(","));
                    concat(v)
                });
                concat(vec![t("switch ("), sc, t(") "), body])
            }
            ExprKind::Paren(x) => {
                let inner = self.expr(x);
                if matches!(x.kind, ExprKind::Binary { .. } | ExprKind::Cond { .. }) {
                    group(concat(vec![
                        t("("),
                        indent(concat(vec![Doc::SoftLine, inner])),
                        Doc::SoftLine,
                        t(")"),
                    ]))
                } else {
                    concat(vec![t("("), inner, t(")")])
                }
            }
        }
    }
}

/// Close a run of equal-precedence binary operators into one flat group.
fn finish_run(first: &mut Vec<Doc>, run: Option<(u8, Vec<Doc>)>) -> Doc {
    let head = concat(std::mem::take(first));
    match run {
        Some((_, rest)) => group(concat(vec![head, concat(rest)])),
        None => head,
    }
}

fn stmt_doc_len(s: &Stmt) -> usize {
    match &s.kind {
        StmtKind::Item(it) => it.doc.len(),
        _ => usize::MAX,
    }
}

fn one_discard(d: Doc) -> Doc {
    // an empty statement body prints as `{}`; comments around it (none expected) are kept
    d_filter(d)
}

fn d_filter(d: Doc) -> Doc {
    // the `;` of the discarded empty statement is dropped, other content is kept
    match d {
        Doc::Concat(v, _) => {
            let kept: Vec<Doc> = v
                .into_iter()
                .filter(|x| !matches!(x, Doc::Text(s, _) if s == ";"))
                .map(d_filter)
                .collect();
            concat(kept)
        }
        Doc::Text(s, _) if s == ";" => Doc::Nil,
        other => other,
    }
}
