//! OxiIR text form: parser. Hand-written, no dependencies. The grammar is
//! specified in `docs/oxiscript/ir.md`; briefly: line-oriented but whitespace
//! insensitive, `;` starts a comment, every value is defined textually before
//! use (any reverse post-order of blocks satisfies this), block labels may be
//! referenced before they are defined.

use crate::types::*;
use std::collections::HashMap;
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: u32,
    pub col: u32,
    pub msg: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "oxiir:{}:{}: {}", self.line, self.col, self.msg)
    }
}

impl std::error::Error for ParseError {}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    /// `%name`
    Val(String),
    /// `@name`
    Sym(String),
    /// `#name`
    Hash(String),
    Int(i128),
    /// Raw float text (contains `.` or an exponent).
    Float(String),
    Str(String),
    Loc(Loc),
    P(&'static str),
    Eof,
}

struct Lexed {
    tok: Tok,
    line: u32,
    col: u32,
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}
fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.'
}

fn lex(src: &str) -> Result<Vec<Lexed>, ParseError> {
    let ch: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    macro_rules! bump {
        () => {{
            let c = ch[i];
            i += 1;
            if c == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            c
        }};
    }
    while i < ch.len() {
        let (sl, sc) = (line, col);
        let err = |msg: String| ParseError {
            line: sl,
            col: sc,
            msg,
        };
        let c = ch[i];
        if c.is_whitespace() {
            bump!();
        } else if c == ';' {
            while i < ch.len() && ch[i] != '\n' {
                bump!();
            }
        } else if is_ident_start(c) {
            let mut s = String::new();
            while i < ch.len() && is_ident_cont(ch[i]) {
                s.push(bump!());
            }
            out.push(Lexed {
                tok: Tok::Ident(s),
                line: sl,
                col: sc,
            });
        } else if c == '%' || c == '@' || c == '#' {
            bump!();
            let mut s = String::new();
            while i < ch.len() && is_ident_cont(ch[i]) {
                s.push(bump!());
            }
            if s.is_empty() {
                return Err(err(format!("expected a name after `{c}`")));
            }
            let tok = match c {
                '%' => Tok::Val(s),
                '@' => Tok::Sym(s),
                _ => Tok::Hash(s),
            };
            out.push(Lexed {
                tok,
                line: sl,
                col: sc,
            });
        } else if c.is_ascii_digit()
            || (c == '-' && ch.get(i + 1).is_some_and(char::is_ascii_digit))
        {
            let mut s = String::new();
            if c == '-' {
                s.push(bump!());
            }
            if ch[i] == '0' && matches!(ch.get(i + 1), Some('x' | 'X')) {
                bump!();
                bump!();
                let mut h = String::new();
                while i < ch.len() && ch[i].is_ascii_hexdigit() {
                    h.push(bump!());
                }
                let v = i128::from_str_radix(&h, 16).map_err(|_| err("bad hex literal".into()))?;
                out.push(Lexed {
                    tok: Tok::Int(if s == "-" { -v } else { v }),
                    line: sl,
                    col: sc,
                });
                continue;
            }
            let mut is_float = false;
            while i < ch.len() {
                let d = ch[i];
                if d.is_ascii_digit() {
                    s.push(bump!());
                } else if d == '.' && ch.get(i + 1).is_some_and(char::is_ascii_digit) && !is_float {
                    is_float = true;
                    s.push(bump!());
                } else if (d == 'e' || d == 'E')
                    && ch
                        .get(i + 1)
                        .is_some_and(|x| x.is_ascii_digit() || *x == '-' || *x == '+')
                {
                    is_float = true;
                    s.push(bump!());
                    s.push(bump!());
                } else {
                    break;
                }
            }
            let tok = if is_float {
                Tok::Float(s)
            } else {
                Tok::Int(
                    s.parse::<i128>()
                        .map_err(|_| err(format!("integer `{s}` out of range")))?,
                )
            };
            out.push(Lexed {
                tok,
                line: sl,
                col: sc,
            });
        } else if c == '-' && ch[i..].starts_with(&['-', 'i', 'n', 'f']) {
            for _ in 0..4 {
                bump!();
            }
            out.push(Lexed {
                tok: Tok::Ident("-inf".into()),
                line: sl,
                col: sc,
            });
        } else if c == '-' && ch.get(i + 1) == Some(&'>') {
            bump!();
            bump!();
            out.push(Lexed {
                tok: Tok::P("->"),
                line: sl,
                col: sc,
            });
        } else if c == ':' && ch.get(i + 1) == Some(&':') {
            bump!();
            bump!();
            out.push(Lexed {
                tok: Tok::P("::"),
                line: sl,
                col: sc,
            });
        } else if c == '"' {
            bump!();
            let mut s = String::new();
            loop {
                if i >= ch.len() || ch[i] == '\n' {
                    return Err(err("unterminated string".into()));
                }
                match bump!() {
                    '"' => break,
                    '\\' => {
                        if i >= ch.len() {
                            return Err(err("unterminated string".into()));
                        }
                        match bump!() {
                            'n' => s.push('\n'),
                            'r' => s.push('\r'),
                            't' => s.push('\t'),
                            '\\' => s.push('\\'),
                            '"' => s.push('"'),
                            'u' => {
                                if i >= ch.len() || bump!() != '{' {
                                    return Err(err("expected `{` after `\\u`".into()));
                                }
                                let mut h = String::new();
                                while i < ch.len() && ch[i] != '}' {
                                    h.push(bump!());
                                }
                                if i >= ch.len() {
                                    return Err(err("unterminated `\\u{`".into()));
                                }
                                bump!();
                                let cp = u32::from_str_radix(&h, 16).ok().and_then(char::from_u32);
                                s.push(cp.ok_or_else(|| err(format!("bad unicode escape `{h}`")))?);
                            }
                            o => return Err(err(format!("unknown escape `\\{o}`"))),
                        }
                    }
                    c => s.push(c),
                }
            }
            out.push(Lexed {
                tok: Tok::Str(s),
                line: sl,
                col: sc,
            });
        } else if c == '!' {
            bump!();
            let mut num = |sep: Option<char>| -> Result<u32, ParseError> {
                let mut s = String::new();
                while i < ch.len() && ch[i].is_ascii_digit() {
                    s.push(bump!());
                }
                if let Some(sep) = sep {
                    if i >= ch.len() || bump!() != sep {
                        return Err(err(format!("expected `{sep}` in a location `!file:lo-hi`")));
                    }
                }
                s.parse::<u32>()
                    .map_err(|_| err("bad location number".into()))
            };
            let file = num(Some(':'))?;
            let lo = num(Some('-'))?;
            let hi = num(None)?;
            out.push(Lexed {
                tok: Tok::Loc(Loc { file, lo, hi }),
                line: sl,
                col: sc,
            });
        } else {
            let p = match c {
                '(' => "(",
                ')' => ")",
                '{' => "{",
                '}' => "}",
                '[' => "[",
                ']' => "]",
                ',' => ",",
                ':' => ":",
                '=' => "=",
                '?' => "?",
                _ => return Err(err(format!("unexpected character `{c}`"))),
            };
            bump!();
            out.push(Lexed {
                tok: Tok::P(p),
                line: sl,
                col: sc,
            });
        }
    }
    out.push(Lexed {
        tok: Tok::Eof,
        line,
        col,
    });
    Ok(out)
}

struct Parser {
    toks: Vec<Lexed>,
    pos: usize,
    /// Current type nesting depth (bounded by [`MAX_TYPE_DEPTH`]).
    depth: u32,
}

type R<T> = Result<T, ParseError>;

impl Parser {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn err<T>(&self, msg: impl Into<String>) -> R<T> {
        let l = &self.toks[self.pos];
        Err(ParseError {
            line: l.line,
            col: l.col,
            msg: msg.into(),
        })
    }
    fn bump(&mut self) -> Tok {
        let t = self.toks[self.pos].tok.clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }
    fn is_p(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::P(q) if *q == p)
    }
    fn eat_p(&mut self, p: &str) -> bool {
        if self.is_p(p) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_p(&mut self, p: &str) -> R<()> {
        if self.eat_p(p) {
            Ok(())
        } else {
            self.err(format!("expected `{p}`, found {}", show(self.peek())))
        }
    }
    fn is_kw(&self, k: &str) -> bool {
        matches!(self.peek(), Tok::Ident(s) if s == k)
    }
    fn eat_kw(&mut self, k: &str) -> bool {
        if self.is_kw(k) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn expect_kw(&mut self, k: &str) -> R<()> {
        if self.eat_kw(k) {
            Ok(())
        } else {
            self.err(format!("expected `{k}`, found {}", show(self.peek())))
        }
    }
    fn ident(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            t => self.err(format!("expected an identifier, found {}", show(&t))),
        }
    }
    fn sym(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Sym(s) => {
                self.bump();
                Ok(s)
            }
            t => self.err(format!("expected `@name`, found {}", show(&t))),
        }
    }
    fn string(&mut self) -> R<String> {
        match self.peek().clone() {
            Tok::Str(s) => {
                self.bump();
                Ok(s)
            }
            t => self.err(format!("expected a string, found {}", show(&t))),
        }
    }
    fn int(&mut self) -> R<i128> {
        match self.peek().clone() {
            Tok::Int(v) => {
                self.bump();
                Ok(v)
            }
            t => self.err(format!("expected an integer, found {}", show(&t))),
        }
    }
}

fn show(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("`{s}`"),
        Tok::Val(s) => format!("`%{s}`"),
        Tok::Sym(s) => format!("`@{s}`"),
        Tok::Hash(s) => format!("`#{s}`"),
        Tok::Int(v) => format!("`{v}`"),
        Tok::Float(s) => format!("`{s}`"),
        Tok::Str(s) => format!("{s:?}"),
        Tok::Loc(_) => "a location".into(),
        Tok::P(p) => format!("`{p}`"),
        Tok::Eof => "end of input".into(),
    }
}

pub fn parse_module(src: &str) -> Result<Module, ParseError> {
    let mut p = Parser {
        toks: lex(src)?,
        pos: 0,
        depth: 0,
    };
    p.expect_kw("oxiir")?;
    let v = p.int()?;
    if v != crate::print::TEXT_VERSION as i128 {
        return p.err(format!("unsupported text version {v}"));
    }
    p.expect_kw("module")?;
    let mut m = Module {
        name: p.string()?,
        ..Module::default()
    };
    m.attrs = p.attrs()?;
    while *p.peek() != Tok::Eof {
        if p.eat_kw("type") {
            let name = p.ident()?;
            let attrs = p.attrs()?;
            p.expect_p("=")?;
            let kind = p.type_kind()?;
            m.types.push(TypeDecl { name, attrs, kind });
        } else if p.eat_kw("import") {
            let name = p.sym()?;
            p.expect_p("=")?;
            let module = p.string()?;
            let field = p.string()?;
            let sig = p.sig()?;
            m.imports.push(Import {
                name,
                module,
                field,
                sig,
            });
        } else if p.eat_kw("data") {
            let name = p.sym()?;
            p.expect_p("=")?;
            m.data.push(DataDecl {
                name,
                text: p.string()?,
            });
        } else if p.eat_kw("global") {
            let name = p.sym()?;
            p.expect_p(":")?;
            let ty = p.ty()?;
            let init = if p.eat_p("=") {
                Some(p.constant(&ty)?)
            } else {
                None
            };
            m.globals.push(Global { name, ty, init });
        } else if p.is_kw("export") || p.is_kw("func") {
            let export = p.eat_kw("export");
            p.expect_kw("func")?;
            m.funcs.push(p.func(export)?);
        } else {
            return p.err(format!("expected a declaration, found {}", show(p.peek())));
        }
    }
    Ok(m)
}

impl Parser {
    fn attrs(&mut self) -> R<Vec<Attr>> {
        let mut out = Vec::new();
        while let Tok::Hash(name) = self.peek().clone() {
            self.bump();
            let mut args = Vec::new();
            if self.eat_p("(") {
                while !self.eat_p(")") {
                    match self.bump() {
                        Tok::Ident(mut s) => {
                            // `host:log`: identifiers joined by single colons
                            while self.is_p(":")
                                && matches!(self.toks[self.pos + 1].tok, Tok::Ident(_))
                            {
                                self.bump();
                                if let Tok::Ident(next) = self.bump() {
                                    s.push(':');
                                    s.push_str(&next);
                                }
                            }
                            args.push(s);
                        }
                        Tok::Str(s) => args.push(s),
                        Tok::Int(v) => args.push(v.to_string()),
                        t => return self.err(format!("bad attribute argument {}", show(&t))),
                    }
                    if !self.eat_p(",") {
                        self.expect_p(")")?;
                        break;
                    }
                }
            }
            out.push(Attr { name, args });
        }
        Ok(out)
    }

    fn fields(&mut self) -> R<Vec<Field>> {
        self.expect_p("{")?;
        let mut out = Vec::new();
        while !self.eat_p("}") {
            let name = self.ident()?;
            self.expect_p(":")?;
            out.push(Field {
                name,
                ty: self.ty()?,
            });
            if !self.eat_p(",") {
                self.expect_p("}")?;
                break;
            }
        }
        Ok(out)
    }

    fn type_kind(&mut self) -> R<TypeKind> {
        if self.eat_kw("struct") {
            Ok(TypeKind::Struct {
                fields: self.fields()?,
            })
        } else if self.eat_kw("closure") {
            Ok(TypeKind::Closure {
                captures: self.fields()?,
            })
        } else if self.eat_kw("union") {
            self.expect_p("{")?;
            let mut variants = Vec::new();
            while !self.eat_p("}") {
                let name = self.ident()?;
                let fields = if self.is_p("{") {
                    self.fields()?
                } else {
                    vec![]
                };
                variants.push(Variant { name, fields });
                if !self.eat_p(",") {
                    self.expect_p("}")?;
                    break;
                }
            }
            Ok(TypeKind::Union { variants })
        } else {
            self.err("expected `struct`, `closure` or `union`")
        }
    }

    fn ty(&mut self) -> R<Ty> {
        let atom = self.ty_atom()?;
        if self.is_p("?") {
            if !matches!(atom, Ty::Str | Ty::Ref(_) | Ty::Array(_) | Ty::Fn(_)) {
                return self.err("only managed reference types can be nullable");
            }
            self.bump();
            return Ok(Ty::Opt(Box::new(atom)));
        }
        Ok(atom)
    }

    fn ty_atom(&mut self) -> R<Ty> {
        self.depth += 1;
        if self.depth > MAX_TYPE_DEPTH {
            return self.err(format!("types nest deeper than {MAX_TYPE_DEPTH} levels"));
        }
        let r = self.ty_atom_inner();
        self.depth -= 1;
        r
    }

    fn ty_atom_inner(&mut self) -> R<Ty> {
        if self.eat_p("[") {
            let e = self.ty()?;
            self.expect_p("]")?;
            return Ok(Ty::Array(Box::new(e)));
        }
        if self.eat_p("(") {
            let t = self.ty()?;
            self.expect_p(")")?;
            return Ok(t);
        }
        let name = self.ident()?;
        Ok(match name.as_str() {
            "bool" => Ty::Bool,
            "i8" => Ty::I8,
            "i16" => Ty::I16,
            "i32" => Ty::I32,
            "i64" => Ty::I64,
            "u8" => Ty::U8,
            "u16" => Ty::U16,
            "u32" => Ty::U32,
            "u64" => Ty::U64,
            "f32" => Ty::F32,
            "f64" => Ty::F64,
            "ptr" => Ty::Ptr,
            "str" => Ty::Str,
            "ref" => Ty::Ref(self.ident()?),
            "fn" => {
                let sig = self.sig()?;
                if sig.rets.len() > 1 {
                    return self.err("closure types have at most one result");
                }
                Ty::Fn(Box::new(sig))
            }
            other => {
                return self.err(format!(
                    "unknown type `{other}` (managed references are written `ref Name`)"
                ))
            }
        })
    }

    fn sig(&mut self) -> R<Sig> {
        self.expect_p("(")?;
        let mut params = Vec::new();
        while !self.eat_p(")") {
            params.push(self.ty()?);
            if !self.eat_p(",") {
                self.expect_p(")")?;
                break;
            }
        }
        self.expect_p("->")?;
        Ok(Sig {
            params,
            rets: self.rets()?,
        })
    }

    fn rets(&mut self) -> R<Vec<Ty>> {
        if self.eat_kw("void") {
            return Ok(vec![]);
        }
        if self.eat_p("(") {
            let mut list = Vec::new();
            while !self.eat_p(")") {
                list.push(self.ty()?);
                if !self.eat_p(",") {
                    self.expect_p(")")?;
                    break;
                }
            }
            if list.len() == 1 && self.is_p("?") {
                self.bump();
                let t = list.pop().unwrap();
                if !matches!(t, Ty::Str | Ty::Ref(_) | Ty::Array(_) | Ty::Fn(_)) {
                    return self.err("only managed reference types can be nullable");
                }
                return Ok(vec![Ty::Opt(Box::new(t))]);
            }
            if list.is_empty() {
                return self.err("empty result list; write `void`");
            }
            return Ok(list);
        }
        Ok(vec![self.ty()?])
    }

    fn constant(&mut self, ty: &Ty) -> R<Const> {
        match self.peek().clone() {
            Tok::Ident(s) if s == "true" || s == "false" => {
                self.bump();
                Ok(Const::Bool(s == "true"))
            }
            Tok::Int(v) => {
                self.bump();
                Ok(Const::Int(v))
            }
            Tok::Float(raw) => {
                self.bump();
                match ty {
                    Ty::F32 => match raw.parse::<f32>() {
                        Ok(v) if v.is_finite() => Ok(Const::F32(v.to_bits())),
                        _ => self.err(format!(
                            "`{raw}` is out of range for f32 (write `inf` explicitly)"
                        )),
                    },
                    Ty::F64 => match raw.parse::<f64>() {
                        Ok(v) if v.is_finite() => Ok(Const::F64(v.to_bits())),
                        _ => self.err(format!(
                            "`{raw}` is out of range for f64 (write `inf` explicitly)"
                        )),
                    },
                    _ => self.err("float literal for a non-float type"),
                }
            }
            Tok::Ident(s) if s == "inf" || s == "-inf" => {
                self.bump();
                let neg = s == "-inf";
                match ty {
                    Ty::F32 => Ok(Const::F32(
                        if neg {
                            f32::NEG_INFINITY
                        } else {
                            f32::INFINITY
                        }
                        .to_bits(),
                    )),
                    Ty::F64 => Ok(Const::F64(
                        if neg {
                            f64::NEG_INFINITY
                        } else {
                            f64::INFINITY
                        }
                        .to_bits(),
                    )),
                    _ => self.err("`inf` for a non-float type"),
                }
            }
            Tok::Ident(s) if s == "nan" => {
                self.bump();
                self.expect_p(":")?;
                let bits = self.int()?;
                match ty {
                    Ty::F32 => match u32::try_from(bits) {
                        Ok(b) if f32::from_bits(b).is_nan() => Ok(Const::F32(b)),
                        _ => self.err("`nan:` bits must be a 32-bit NaN pattern for f32"),
                    },
                    Ty::F64 => match u64::try_from(bits) {
                        Ok(b) if f64::from_bits(b).is_nan() => Ok(Const::F64(b)),
                        _ => self.err("`nan:` bits must be a 64-bit NaN pattern for f64"),
                    },
                    _ => self.err("`nan` for a non-float type"),
                }
            }
            t => self.err(format!("expected a constant, found {}", show(&t))),
        }
    }

    fn func(&mut self, export: bool) -> R<Func> {
        let name = self.sym()?;
        let mut f = Func {
            name,
            export,
            attrs: vec![],
            params: vec![],
            rets: vec![],
            values: vec![],
            blocks: vec![],
        };
        let names = self.prescan()?;
        f.values = (0..names.len())
            .map(|_| ValueInfo {
                name: String::new(),
                ty: Ty::Bool,
            })
            .collect();
        self.expect_p("(")?;
        while !self.eat_p(")") {
            let vname = match self.bump() {
                Tok::Val(v) => v,
                t => return self.err(format!("expected a parameter `%name`, found {}", show(&t))),
            };
            self.expect_p(":")?;
            let own = self.eat_kw("own");
            let ty = self.ty()?;
            let id = self.define(&mut f, &names, vname, ty)?;
            f.params.push(Param { value: id, own });
            if !self.eat_p(",") {
                self.expect_p(")")?;
                break;
            }
        }
        self.expect_p("->")?;
        f.rets = self.rets()?;
        f.attrs = self.attrs()?;
        self.expect_p("{")?;

        // labels may be referenced before definition: intern on first mention.
        let mut label_ids: HashMap<String, u32> = HashMap::new();
        let mut label_order: Vec<String> = Vec::new();
        let mut defined: HashMap<String, usize> = HashMap::new();
        while !self.eat_p("}") {
            let label = self.ident()?;
            if defined.contains_key(&label) {
                return self.err(format!("block `{label}` defined twice"));
            }
            defined.insert(label.clone(), f.blocks.len());
            let mut bparams = Vec::new();
            if self.eat_p("(") {
                while !self.eat_p(")") {
                    let vname = match self.bump() {
                        Tok::Val(v) => v,
                        t => {
                            return self.err(format!(
                                "expected a block parameter `%name`, found {}",
                                show(&t)
                            ))
                        }
                    };
                    self.expect_p(":")?;
                    let ty = self.ty()?;
                    bparams.push(self.define(&mut f, &names, vname, ty)?);
                    if !self.eat_p(",") {
                        self.expect_p(")")?;
                        break;
                    }
                }
            }
            self.expect_p(":")?;
            let mut insts = Vec::new();
            let (term, term_loc) = loop {
                if let Some(t) =
                    self.try_terminator(&f, &names, &mut label_ids, &mut label_order)?
                {
                    let loc = self.opt_loc();
                    break (t, loc);
                }
                insts.push(self.inst(&mut f, &names)?);
            };
            f.blocks.push(Block {
                label,
                params: bparams,
                insts,
                term,
                term_loc,
            });
        }
        if f.blocks.is_empty() {
            return self.err(format!("function @{} has no blocks", f.name));
        }
        // Resolve label ids (first-mention numbering) to block positions.
        let mut remap = HashMap::new();
        for (id, label) in label_order.iter().enumerate() {
            match defined.get(label) {
                Some(pos) => {
                    remap.insert(id as u32, *pos as u32);
                }
                None => {
                    return self.err(format!(
                        "branch to undefined block `{label}` in @{}",
                        f.name
                    ))
                }
            }
        }
        for b in &mut f.blocks {
            let fix = |t: &mut Target| t.block = BlockId(remap[&t.block.0]);
            match &mut b.term {
                Terminator::Br(t) => fix(t),
                Terminator::BrIf { then, els, .. } => {
                    fix(then);
                    fix(els);
                }
                Terminator::Switch { cases, default, .. } => {
                    for (_, t) in cases.iter_mut() {
                        fix(t);
                    }
                    fix(default);
                }
                _ => {}
            }
        }
        Ok(f)
    }

    /// Ids are assigned by [`Self::prescan`] in textual definition order, so
    /// values may be used before their definition (the verifier checks dominance).
    fn define(
        &self,
        f: &mut Func,
        names: &HashMap<String, ValueId>,
        name: String,
        ty: Ty,
    ) -> R<ValueId> {
        let Some(id) = names.get(&name).copied() else {
            return self.err(format!("internal: `%{name}` was not found by the pre-scan"));
        };
        f.values[id.0 as usize] = ValueInfo { name, ty };
        Ok(id)
    }

    /// Scan the tokens of one function (from the parameter list to the closing
    /// brace of its body) and number every value definition (`%name :`).
    fn prescan(&self) -> R<HashMap<String, ValueId>> {
        let mut names = HashMap::new();
        let (mut depth, mut seen_open) = (0u32, false);
        let mut i = self.pos;
        while i < self.toks.len() {
            match &self.toks[i].tok {
                Tok::P("{") => {
                    depth += 1;
                    seen_open = true;
                }
                Tok::P("}") => {
                    depth = depth.saturating_sub(1);
                    if seen_open && depth == 0 {
                        break;
                    }
                }
                Tok::Eof => break,
                Tok::Val(n)
                    if matches!(self.toks.get(i + 1).map(|t| &t.tok), Some(Tok::P(":"))) =>
                {
                    let id = ValueId(names.len() as u32);
                    if names.insert(n.clone(), id).is_some() {
                        return Err(ParseError {
                            line: self.toks[i].line,
                            col: self.toks[i].col,
                            msg: format!("value `%{n}` defined twice"),
                        });
                    }
                }
                _ => {}
            }
            i += 1;
        }
        Ok(names)
    }

    fn opt_loc(&mut self) -> Option<Loc> {
        if let Tok::Loc(l) = self.peek().clone() {
            self.bump();
            Some(l)
        } else {
            None
        }
    }

    fn value(&mut self, names: &HashMap<String, ValueId>) -> R<ValueId> {
        match self.bump() {
            Tok::Val(n) => {
                if let Some(id) = names.get(&n) {
                    Ok(*id)
                } else {
                    self.pos = self.pos.saturating_sub(1);
                    self.err(format!(
                        "`%{n}` is not defined (values must be defined before use in text order)"
                    ))
                }
            }
            t => {
                self.pos = self.pos.saturating_sub(1);
                self.err(format!("expected a value `%name`, found {}", show(&t)))
            }
        }
    }

    fn values_until_close(&mut self, names: &HashMap<String, ValueId>) -> R<Vec<ValueId>> {
        // after '(' has been consumed
        let mut out = Vec::new();
        while !self.eat_p(")") {
            out.push(self.value(names)?);
            if !self.eat_p(",") {
                self.expect_p(")")?;
                break;
            }
        }
        Ok(out)
    }

    fn target(
        &mut self,
        names: &HashMap<String, ValueId>,
        ids: &mut HashMap<String, u32>,
        order: &mut Vec<String>,
    ) -> R<Target> {
        let label = self.ident()?;
        let next = ids.len() as u32;
        let id = *ids.entry(label.clone()).or_insert_with(|| {
            order.push(label.clone());
            next
        });
        self.expect_p("(")?;
        let args = self.values_until_close(names)?;
        Ok(Target {
            block: BlockId(id),
            args,
        })
    }

    fn try_terminator(
        &mut self,
        f: &Func,
        names: &HashMap<String, ValueId>,
        ids: &mut HashMap<String, u32>,
        order: &mut Vec<String>,
    ) -> R<Option<Terminator>> {
        let _ = f;
        let Tok::Ident(w) = self.peek().clone() else {
            return Ok(None);
        };
        Ok(Some(match w.as_str() {
            "br" => {
                self.bump();
                Terminator::Br(self.target(names, ids, order)?)
            }
            "br_if" => {
                self.bump();
                let cond = self.value(names)?;
                self.expect_p(",")?;
                let then = self.target(names, ids, order)?;
                self.expect_p(",")?;
                let els = self.target(names, ids, order)?;
                Terminator::BrIf { cond, then, els }
            }
            "switch" => {
                self.bump();
                let value = self.value(names)?;
                self.expect_p(",")?;
                self.expect_p("[")?;
                let mut cases = Vec::new();
                while !self.eat_p("]") {
                    let k = self.int()?;
                    let k =
                        i64::try_from(k).or_else(|_| self.err("switch case out of i64 range"))?;
                    self.expect_p(":")?;
                    cases.push((k, self.target(names, ids, order)?));
                    if !self.eat_p(",") {
                        self.expect_p("]")?;
                        break;
                    }
                }
                self.expect_p(",")?;
                self.expect_kw("default")?;
                let default = self.target(names, ids, order)?;
                Terminator::Switch {
                    value,
                    cases,
                    default,
                }
            }
            "ret" => {
                self.bump();
                let mut vs = Vec::new();
                if matches!(self.peek(), Tok::Val(_)) {
                    loop {
                        vs.push(self.value(names)?);
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                }
                Terminator::Ret(vs)
            }
            "trap" => {
                self.bump();
                let k = self.ident()?;
                match TrapKind::from_name(&k) {
                    Some(k) => Terminator::Trap(k),
                    None => return self.err(format!("unknown trap kind `{k}`")),
                }
            }
            _ => return Ok(None),
        }))
    }

    fn inst(&mut self, f: &mut Func, names: &HashMap<String, ValueId>) -> R<Inst> {
        // results: `%a: ty, %b: ty =`
        let mut pending: Vec<(String, Ty)> = Vec::new();
        while let Tok::Val(n) = self.peek().clone() {
            self.bump();
            self.expect_p(":")?;
            let ty = self.ty()?;
            pending.push((n, ty));
            if !self.eat_p(",") {
                break;
            }
        }
        if !pending.is_empty() {
            self.expect_p("=")?;
        }
        let res_tys: Vec<Ty> = pending.iter().map(|(_, t)| t.clone()).collect();
        // The operands may only reference values defined *before* this instruction.
        let op = self.op(names, &res_tys)?;
        let loc = self.opt_loc();
        let mut results = Vec::new();
        for (n, t) in pending {
            results.push(self.define(f, names, n, t)?);
        }
        Ok(Inst { results, op, loc })
    }

    fn type_ref(&mut self) -> R<TypeRef> {
        let ty = self.ident()?;
        let variant = if self.eat_p("::") {
            Some(self.ident()?)
        } else {
            None
        };
        Ok(TypeRef { ty, variant })
    }

    fn field_ref(&mut self) -> R<FieldRef> {
        let first = self.ident()?;
        if self.eat_p("::") {
            let rest = self.ident()?;
            match rest.rsplit_once('.') {
                Some((v, fld)) => Ok(FieldRef {
                    ty: first,
                    variant: Some(v.into()),
                    field: fld.into(),
                }),
                None => self.err("expected `Type::Variant.field`"),
            }
        } else {
            match first.rsplit_once('.') {
                Some((t, fld)) => Ok(FieldRef {
                    ty: t.into(),
                    variant: None,
                    field: fld.into(),
                }),
                None => self.err("expected `Type.field`"),
            }
        }
    }

    fn op(&mut self, names: &HashMap<String, ValueId>, res: &[Ty]) -> R<Op> {
        let word = self.ident()?;
        let (base, ovf) = if let Some(b) = word.strip_suffix(".wrap") {
            (b.to_string(), Ovf::Wrap)
        } else if let Some(b) = word.strip_suffix(".sat") {
            (b.to_string(), Ovf::Sat)
        } else {
            (word.clone(), Ovf::Trap)
        };
        let two = |p: &mut Parser| -> R<(ValueId, ValueId)> {
            let a = p.value(names)?;
            p.expect_p(",")?;
            Ok((a, p.value(names)?))
        };
        if let Some(op) = bin_of(&base) {
            let (a, b) = two(self)?;
            return Ok(Op::Bin { op, ovf, a, b });
        }
        if let Some(op) = un_of(&base) {
            let a = self.value(names)?;
            return Ok(Op::Un { op, ovf, a });
        }
        if let Some(op) = cmp_of(&base) {
            if ovf != Ovf::Trap {
                return self.err("comparisons take no overflow suffix");
            }
            let (a, b) = two(self)?;
            return Ok(Op::Cmp { op, a, b });
        }
        let plain = |this: &Parser| -> R<()> {
            if ovf != Ovf::Trap {
                return this.err(format!("`{word}` takes no overflow suffix"));
            }
            Ok(())
        };
        Ok(match base.as_str() {
            "conv" => Op::Conv {
                ovf,
                a: self.value(names)?,
            },
            "const" => {
                plain(self)?;
                let ty = res.first().cloned().unwrap_or(Ty::I32);
                Op::Const(self.constant(&ty)?)
            }
            "null" => Op::Null,
            "bitcast" => Op::Bitcast {
                a: self.value(names)?,
            },
            "select" => {
                let cond = self.value(names)?;
                self.expect_p(",")?;
                let (a, b) = two(self)?;
                Op::Select { cond, a, b }
            }
            "alloc" => Op::Alloc(self.type_ref()?),
            "getfield" => {
                let field = self.field_ref()?;
                Op::GetField {
                    field,
                    obj: self.value(names)?,
                }
            }
            "setfield" | "setfield.init" => {
                let field = self.field_ref()?;
                let obj = self.value(names)?;
                self.expect_p(",")?;
                Op::SetField {
                    field,
                    obj,
                    val: self.value(names)?,
                    init: base == "setfield.init",
                }
            }
            "tag" => Op::Tag {
                obj: self.value(names)?,
            },
            "is_unique" => Op::IsUnique {
                obj: self.value(names)?,
            },
            "clone" => Op::Clone {
                obj: self.value(names)?,
            },
            "array.new" => Op::ArrayNew {
                len: self.value(names)?,
            },
            "array.len" => Op::ArrayLen {
                arr: self.value(names)?,
            },
            "array.get" => {
                let (arr, idx) = two(self)?;
                Op::ArrayGet { arr, idx }
            }
            "array.set" => {
                let (arr, idx) = two(self)?;
                self.expect_p(",")?;
                Op::ArraySet {
                    arr,
                    idx,
                    val: self.value(names)?,
                }
            }
            "array.push" => {
                let (arr, val) = two(self)?;
                Op::ArrayPush { arr, val }
            }
            "str.const" => Op::StrConst { data: self.sym()? },
            "str.len" => Op::StrLen {
                s: self.value(names)?,
            },
            "str.from_utf8" => {
                let (bytes, len) = two(self)?;
                Op::StrFromUtf8 { bytes, len }
            }
            "str.concat" => {
                let (a, b) = two(self)?;
                Op::StrConcat { a, b }
            }
            "str.eq" => {
                let (a, b) = two(self)?;
                Op::StrEq { a, b }
            }
            "str.cmp" => {
                let (a, b) = two(self)?;
                Op::StrCmp { a, b }
            }
            "str.slice" => {
                let (s0, lo) = two(self)?;
                self.expect_p(",")?;
                Op::StrSlice {
                    s: s0,
                    lo,
                    hi: self.value(names)?,
                }
            }
            "str.byte" => {
                let (s0, idx) = two(self)?;
                Op::StrByte { s: s0, idx }
            }
            "str.from_f64" => Op::StrFromF64 {
                x: self.value(names)?,
            },
            "array.empty" => Op::ArrayEmpty,
            "array.pop" => Op::ArrayPop {
                arr: self.value(names)?,
            },
            "is_null" => Op::IsNull {
                val: self.value(names)?,
            },
            "opt" => Op::Opt {
                val: self.value(names)?,
            },
            "unopt" => Op::Unopt {
                val: self.value(names)?,
            },
            "closure.new" => {
                let env = self.ident()?;
                let func = self.sym()?;
                self.expect_p("(")?;
                Op::ClosureNew {
                    env,
                    func,
                    captures: self.values_until_close(names)?,
                }
            }
            "call" => {
                let callee = self.sym()?;
                self.expect_p("(")?;
                Op::Call {
                    callee,
                    args: self.values_until_close(names)?,
                }
            }
            "call_closure" => {
                let callee = self.value(names)?;
                self.expect_p("(")?;
                Op::CallClosure {
                    callee,
                    args: self.values_until_close(names)?,
                }
            }
            "intrinsic" => {
                let name = self.ident()?;
                self.expect_p("(")?;
                Op::Intrinsic {
                    name,
                    args: self.values_until_close(names)?,
                }
            }
            "retain" => Op::Retain {
                val: self.value(names)?,
            },
            "release" => Op::Release {
                val: self.value(names)?,
            },
            "global.get" => Op::GlobalGet { name: self.sym()? },
            "global.set" => {
                let name = self.sym()?;
                self.expect_p(",")?;
                Op::GlobalSet {
                    name,
                    val: self.value(names)?,
                }
            }
            "load" => {
                let ptr = self.value(names)?;
                self.expect_p(",")?;
                Op::Load {
                    ptr,
                    offset: self.offset()?,
                }
            }
            "store" => {
                let ptr = self.value(names)?;
                self.expect_p(",")?;
                let offset = self.offset()?;
                self.expect_p(",")?;
                Op::Store {
                    ptr,
                    offset,
                    val: self.value(names)?,
                }
            }
            _ => {
                self.pos = self.pos.saturating_sub(1);
                return self.err(format!("unknown instruction `{word}`"));
            }
        })
    }

    fn offset(&mut self) -> R<u32> {
        let v = self.int()?;
        u32::try_from(v).or_else(|_| self.err("offset out of range"))
    }
}

fn bin_of(s: &str) -> Option<BinOp> {
    Some(match s {
        "add" => BinOp::Add,
        "sub" => BinOp::Sub,
        "mul" => BinOp::Mul,
        "div" => BinOp::Div,
        "rem" => BinOp::Rem,
        "and" => BinOp::And,
        "or" => BinOp::Or,
        "xor" => BinOp::Xor,
        "shl" => BinOp::Shl,
        "shr" => BinOp::Shr,
        "min" => BinOp::Min,
        "max" => BinOp::Max,
        _ => return None,
    })
}

fn un_of(s: &str) -> Option<UnOp> {
    Some(match s {
        "neg" => UnOp::Neg,
        "not" => UnOp::Not,
        "abs" => UnOp::Abs,
        "sqrt" => UnOp::Sqrt,
        "floor" => UnOp::Floor,
        "ceil" => UnOp::Ceil,
        "trunc" => UnOp::Trunc,
        "nearest" => UnOp::Nearest,
        "clz" => UnOp::Clz,
        "ctz" => UnOp::Ctz,
        "popcnt" => UnOp::Popcnt,
        _ => return None,
    })
}

fn cmp_of(s: &str) -> Option<CmpOp> {
    Some(match s {
        "eq" => CmpOp::Eq,
        "ne" => CmpOp::Ne,
        "lt" => CmpOp::Lt,
        "le" => CmpOp::Le,
        "gt" => CmpOp::Gt,
        "ge" => CmpOp::Ge,
        _ => return None,
    })
}
