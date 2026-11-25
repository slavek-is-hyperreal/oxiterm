//! The OxiScript lexer (`docs/oxiscript/grammar.md` section 1).
//!
//! Total over `&str`: it never panics and never allocates more than a small
//! multiple of the input. Whitespace and comments are not tokens; instead every
//! token records whether whitespace/a newline precedes it (`ws`, `nl`), which is
//! all the parser needs for the one whitespace-sensitive rule (`?`), and the
//! comments are returned separately, in source order, for the formatter.
//!
//! Template literals are lexed in one pass with a brace stack: `` `a${ `` is a
//! [`Tk::TemplateHead`], the tokens of the interpolated expression follow, and
//! the `}` that closes the interpolation resumes the string as a
//! [`Tk::TemplateMiddle`] or [`Tk::TemplateTail`]. No feedback from the parser
//! is needed.

use crate::ast::{FileId, NumTy, Span};
use crate::diag::{mk_span, Diagnostic, Fix, MAX_DIAGS};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kw {
    As,
    Break,
    Case,
    Class,
    Const,
    Continue,
    Default,
    Do,
    Else,
    Enum,
    Export,
    Extends,
    False,
    For,
    Function,
    If,
    Implements,
    Import,
    Interface,
    Let,
    New,
    Null,
    Private,
    Readonly,
    Return,
    Static,
    Struct,
    Switch,
    This,
    True,
    While,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tk {
    Eof,
    Ident,
    /// A reserved word (`keywords::RESERVED`); the text is in the source.
    Reserved,
    Kw(Kw),
    Int,
    Float,
    Str,
    /// `` `text` `` without interpolation.
    TemplateFull,
    /// `` `text${ ``
    TemplateHead,
    /// `` }text${ ``
    TemplateMiddle,
    /// `` }text` ``
    TemplateTail,
    // punctuation
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Dot,
    Colon,
    Question,
    QDot,
    QQ,
    At,
    AtAt,
    FatArrow,
    // arithmetic
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    PlusWrap,
    MinusWrap,
    StarWrap,
    PlusSat,
    MinusSat,
    StarSat,
    Amp,
    Pipe,
    Caret,
    Tilde,
    Shl,
    Shr,
    Bang,
    AmpAmp,
    PipePipe,
    EqEqEq,
    NotEqEq,
    /// `==` (not an OxiScript operator: reported with a fix)
    EqEq,
    /// `!=` (not an OxiScript operator: reported with a fix)
    NotEq,
    Lt,
    Le,
    Gt,
    Ge,
    Assign,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    PlusWrapEq,
    MinusWrapEq,
    StarWrapEq,
    PlusSatEq,
    MinusSatEq,
    StarSatEq,
    AmpEq,
    PipeEq,
    CaretEq,
    ShlEq,
    ShrEq,
    PlusPlus,
    MinusMinus,
}

#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub kind: Tk,
    pub lo: u32,
    pub hi: u32,
    /// Index into [`Lexed::lits`] for `Int`, `Float`, `Str` and template chunks.
    pub data: u32,
    /// Whitespace, a comment or the start of the file precedes this token.
    pub ws: bool,
    /// A line break (also inside a block comment) precedes this token.
    pub nl: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum LitData {
    Int { value: u128, suffix: Option<NumTy> },
    Float { text: String, suffix: Option<NumTy> },
    Str(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommentKind {
    /// `// text`
    Line,
    /// `/// text` (attached to the following item or member by the parser)
    Doc,
    /// `/* text */`
    Block,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Comment {
    pub kind: CommentKind,
    /// Byte range of the whole comment including its markers; a line comment
    /// excludes the line terminator.
    pub lo: u32,
    pub hi: u32,
}

#[derive(Debug)]
pub struct Lexed {
    /// Always ends with an `Eof` token.
    pub tokens: Vec<Token>,
    pub lits: Vec<LitData>,
    pub comments: Vec<Comment>,
    pub diags: Vec<Diagnostic>,
    /// For each `(` token index the index of its matching `)`; `u32::MAX` if none.
    pub paren_match: Vec<u32>,
    /// Deepest `(`/`[`/`{`/`${` nesting seen (a lower bound on parser recursion).
    pub max_nest: u32,
}

pub fn keyword(w: &str) -> Option<Kw> {
    Some(match w {
        "as" => Kw::As,
        "break" => Kw::Break,
        "case" => Kw::Case,
        "class" => Kw::Class,
        "const" => Kw::Const,
        "continue" => Kw::Continue,
        "default" => Kw::Default,
        "do" => Kw::Do,
        "else" => Kw::Else,
        "enum" => Kw::Enum,
        "export" => Kw::Export,
        "extends" => Kw::Extends,
        "false" => Kw::False,
        "for" => Kw::For,
        "function" => Kw::Function,
        "if" => Kw::If,
        "implements" => Kw::Implements,
        "import" => Kw::Import,
        "interface" => Kw::Interface,
        "let" => Kw::Let,
        "new" => Kw::New,
        "null" => Kw::Null,
        "private" => Kw::Private,
        "readonly" => Kw::Readonly,
        "return" => Kw::Return,
        "static" => Kw::Static,
        "struct" => Kw::Struct,
        "switch" => Kw::Switch,
        "this" => Kw::This,
        "true" => Kw::True,
        "while" => Kw::While,
        _ => return None,
    })
}

/// Same list as `keywords::RESERVED` (a test keeps them in sync); a `match`
/// because it runs for every identifier.
pub fn is_reserved_word(w: &str) -> bool {
    matches!(
        w,
        "arena"
            | "async"
            | "await"
            | "dyn"
            | "ensures"
            | "impl"
            | "inout"
            | "invariant"
            | "isolate"
            | "kernel"
            | "match"
            | "move"
            | "parallel"
            | "region"
            | "requires"
            | "spawn"
            | "trait"
            | "trusted"
            | "unsafe"
            | "weak"
            | "where"
            | "yield"
            | "any"
            | "debugger"
            | "declare"
            | "delete"
            | "instanceof"
            | "namespace"
            | "protected"
            | "public"
            | "super"
            | "typeof"
            | "undefined"
            | "unknown"
            | "var"
            | "with"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Brace {
    Plain,
    Template,
}

struct Lexer<'a> {
    src: &'a str,
    b: &'a [u8],
    file: FileId,
    pos: usize,
    toks: Vec<Token>,
    lits: Vec<LitData>,
    comments: Vec<Comment>,
    diags: Vec<Diagnostic>,
    braces: Vec<Brace>,
    capped: bool,
}

/// Characters that render as nothing or reorder text (Trojan Source): they must
/// be written as `\u{...}` inside string and template literals.
pub fn is_invisible(c: char) -> bool {
    (c.is_control() && c != '\t')
        || matches!(c,
            '\u{ad}' | '\u{61c}' | '\u{180e}' | '\u{200b}'..='\u{200f}' | '\u{2028}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}'
            | '\u{e0000}'..='\u{e007f}')
}

const fn is_id_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}
const fn is_id_cont(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}
fn digit_of(c: u8, radix: u32) -> bool {
    (c as char).is_digit(radix)
}

/// Lex `src`. `file` is stored in every span.
pub fn lex(src: &str, file: FileId) -> Lexed {
    let mut lx = Lexer {
        src,
        b: src.as_bytes(),
        file,
        pos: 0,
        toks: Vec::with_capacity(src.len() / 4 + 4),
        lits: Vec::new(),
        comments: Vec::new(),
        diags: Vec::new(),
        braces: Vec::new(),
        capped: false,
    };
    if src.len() >= u32::MAX as usize - 1 {
        lx.diags.push(Diagnostic::new(
            "OXS-E0012",
            mk_span(file, 0, 0),
            "the source file is larger than 4 GiB and cannot be parsed",
        ));
        lx.push_eof(false, false);
        return lx.finish();
    }
    if src.starts_with('\u{feff}') {
        lx.pos = 3;
    }
    lx.run();
    lx.finish()
}

impl Lexer<'_> {
    fn finish(self) -> Lexed {
        let mut paren_match = vec![u32::MAX; self.toks.len()];
        let mut stack: Vec<u32> = Vec::new();
        let (mut nest, mut max_nest) = (0u32, 0u32);
        for (i, t) in self.toks.iter().enumerate() {
            match t.kind {
                Tk::LParen | Tk::LBracket | Tk::LBrace | Tk::TemplateHead => {
                    nest += 1;
                    max_nest = max_nest.max(nest);
                }
                Tk::RParen | Tk::RBracket | Tk::RBrace | Tk::TemplateTail => {
                    nest = nest.saturating_sub(1);
                }
                _ => {}
            }
            match t.kind {
                Tk::LParen => stack.push(i as u32),
                Tk::RParen => {
                    if let Some(o) = stack.pop() {
                        paren_match[o as usize] = i as u32;
                    }
                }
                _ => {}
            }
        }
        Lexed {
            tokens: self.toks,
            lits: self.lits,
            comments: self.comments,
            diags: self.diags,
            paren_match,
            max_nest,
        }
    }

    fn span(&self, lo: usize, hi: usize) -> Span {
        mk_span(self.file, lo, hi)
    }

    fn err(&mut self, code: &'static str, lo: usize, hi: usize, msg: String) {
        let s = self.span(lo, hi);
        self.push_diag(Diagnostic::new(code, s, msg));
    }

    /// Record a diagnostic; past [`MAX_DIAGS`] only one `OXS-E0017` is kept.
    fn push_diag(&mut self, d: Diagnostic) {
        if self.diags.len() < MAX_DIAGS {
            self.diags.push(d);
        } else if !self.capped {
            self.capped = true;
            let sp = d.span;
            self.diags.push(Diagnostic::new(
                "OXS-E0017",
                sp,
                format!("more than {MAX_DIAGS} errors; giving up on this file"),
            ));
        }
    }

    fn push_eof(&mut self, ws: bool, nl: bool) {
        let n = u32::try_from(self.src.len()).unwrap_or(u32::MAX);
        self.toks.push(Token {
            kind: Tk::Eof,
            lo: n,
            hi: n,
            data: 0,
            ws,
            nl,
        });
    }

    fn push(&mut self, kind: Tk, lo: usize, hi: usize, data: u32, ws: bool, nl: bool) {
        self.toks.push(Token {
            kind,
            lo: lo as u32,
            hi: hi as u32,
            data,
            ws,
            nl,
        });
    }

    fn add_lit(&mut self, l: LitData) -> u32 {
        self.lits.push(l);
        (self.lits.len() - 1) as u32
    }

    /// Skip whitespace and comments; returns `(saw_whitespace, saw_newline)`.
    fn skip_trivia(&mut self) -> (bool, bool) {
        let b = self.b;
        let n = b.len();
        let mut ws = self.pos == 0 || self.pos == 3 && self.src.starts_with('\u{feff}');
        let mut nl = false;
        loop {
            if self.pos >= n {
                return (ws, nl);
            }
            match b[self.pos] {
                b' ' | b'\t' | 0x0b | 0x0c => {
                    self.pos += 1;
                    ws = true;
                }
                b'\n' | b'\r' => {
                    self.pos += 1;
                    ws = true;
                    nl = true;
                }
                b'/' if self.pos + 1 < n && b[self.pos + 1] == b'/' => {
                    let lo = self.pos;
                    let mut e = lo + 2;
                    while e < n && b[e] != b'\n' && b[e] != b'\r' {
                        e += 1;
                    }
                    let hi = e;
                    let is_doc = b.get(lo + 2) == Some(&b'/') && b.get(lo + 3) != Some(&b'/');
                    self.comments.push(Comment {
                        kind: if is_doc {
                            CommentKind::Doc
                        } else {
                            CommentKind::Line
                        },
                        lo: lo as u32,
                        hi: hi as u32,
                    });
                    self.pos = e;
                    ws = true;
                }
                b'/' if self.pos + 1 < n && b[self.pos + 1] == b'*' => {
                    let lo = self.pos;
                    let mut e = lo + 2;
                    let mut closed = false;
                    while e < n {
                        if b[e] == b'\n' || b[e] == b'\r' {
                            nl = true;
                        }
                        if b[e] == b'*' && e + 1 < n && b[e + 1] == b'/' {
                            e += 2;
                            closed = true;
                            break;
                        }
                        e += 1;
                    }
                    if !closed {
                        self.err(
                            "OXS-E0002",
                            lo,
                            lo + 2,
                            "the block comment starting here is never closed".into(),
                        );
                    }
                    self.comments.push(Comment {
                        kind: CommentKind::Block,
                        lo: lo as u32,
                        hi: e as u32,
                    });
                    self.pos = e;
                    ws = true;
                }
                _ => return (ws, nl),
            }
        }
    }

    fn run(&mut self) {
        loop {
            let (ws, nl) = self.skip_trivia();
            let b = self.b;
            let n = b.len();
            if self.pos >= n {
                self.push_eof(ws, nl);
                return;
            }
            let i = self.pos;
            let c = b[i];
            let at = |k: usize| -> u8 { b.get(i + k).copied().unwrap_or(0) };
            let (kind, len): (Tk, usize) = match c {
                b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                    let mut e = i + 1;
                    while e < n && is_id_cont(b[e]) {
                        e += 1;
                    }
                    let w = &self.src[i..e];
                    let k = if let Some(k) = keyword(w) {
                        Tk::Kw(k)
                    } else if is_reserved_word(w) {
                        Tk::Reserved
                    } else {
                        Tk::Ident
                    };
                    (k, e - i)
                }
                b'0'..=b'9' => {
                    self.lex_number(ws, nl);
                    continue;
                }
                b'.' if at(1).is_ascii_digit() => {
                    self.lex_number(ws, nl);
                    continue;
                }
                b'"' | b'\'' => {
                    self.lex_string(c, ws, nl);
                    continue;
                }
                b'`' => {
                    self.lex_template_chunk(i, i + 1, true, ws, nl);
                    continue;
                }
                b'{' => {
                    self.braces.push(Brace::Plain);
                    (Tk::LBrace, 1)
                }
                b'}' => {
                    if self.braces.last() == Some(&Brace::Template) {
                        self.braces.pop();
                        self.lex_template_chunk(i, i + 1, false, ws, nl);
                        continue;
                    }
                    self.braces.pop();
                    (Tk::RBrace, 1)
                }
                b'(' => (Tk::LParen, 1),
                b')' => (Tk::RParen, 1),
                b'[' => (Tk::LBracket, 1),
                b']' => (Tk::RBracket, 1),
                b';' => (Tk::Semi, 1),
                b',' => (Tk::Comma, 1),
                b'.' => (Tk::Dot, 1),
                b':' => (Tk::Colon, 1),
                b'?' => {
                    if at(1) == b'?' {
                        (Tk::QQ, 2)
                    } else if at(1) == b'.' && !at(2).is_ascii_digit() {
                        (Tk::QDot, 2)
                    } else {
                        (Tk::Question, 1)
                    }
                }
                b'@' => {
                    if at(1) == b'@' {
                        (Tk::AtAt, 2)
                    } else {
                        (Tk::At, 1)
                    }
                }
                b'+' => match (at(1), at(2)) {
                    (b'%', b'=') => (Tk::PlusWrapEq, 3),
                    (b'|', b'=') => (Tk::PlusSatEq, 3),
                    (b'%', _) => (Tk::PlusWrap, 2),
                    (b'|', _) => (Tk::PlusSat, 2),
                    (b'+', _) => (Tk::PlusPlus, 2),
                    (b'=', _) => (Tk::PlusEq, 2),
                    _ => (Tk::Plus, 1),
                },
                b'-' => match (at(1), at(2)) {
                    (b'%', b'=') => (Tk::MinusWrapEq, 3),
                    (b'|', b'=') => (Tk::MinusSatEq, 3),
                    (b'%', _) => (Tk::MinusWrap, 2),
                    (b'|', _) => (Tk::MinusSat, 2),
                    (b'-', _) => (Tk::MinusMinus, 2),
                    (b'=', _) => (Tk::MinusEq, 2),
                    _ => (Tk::Minus, 1),
                },
                b'*' => match (at(1), at(2)) {
                    (b'%', b'=') => (Tk::StarWrapEq, 3),
                    (b'|', b'=') => (Tk::StarSatEq, 3),
                    (b'%', _) => (Tk::StarWrap, 2),
                    (b'|', _) => (Tk::StarSat, 2),
                    (b'=', _) => (Tk::StarEq, 2),
                    _ => (Tk::Star, 1),
                },
                b'/' => {
                    if at(1) == b'=' {
                        (Tk::SlashEq, 2)
                    } else {
                        (Tk::Slash, 1)
                    }
                }
                b'%' => {
                    if at(1) == b'=' {
                        (Tk::PercentEq, 2)
                    } else {
                        (Tk::Percent, 1)
                    }
                }
                b'&' => match at(1) {
                    b'&' => (Tk::AmpAmp, 2),
                    b'=' => (Tk::AmpEq, 2),
                    _ => (Tk::Amp, 1),
                },
                b'|' => match at(1) {
                    b'|' => (Tk::PipePipe, 2),
                    b'=' => (Tk::PipeEq, 2),
                    _ => (Tk::Pipe, 1),
                },
                b'^' => {
                    if at(1) == b'=' {
                        (Tk::CaretEq, 2)
                    } else {
                        (Tk::Caret, 1)
                    }
                }
                b'~' => (Tk::Tilde, 1),
                b'=' => match (at(1), at(2)) {
                    (b'=', b'=') => (Tk::EqEqEq, 3),
                    (b'=', _) => (Tk::EqEq, 2),
                    (b'>', _) => (Tk::FatArrow, 2),
                    _ => (Tk::Assign, 1),
                },
                b'!' => match (at(1), at(2)) {
                    (b'=', b'=') => (Tk::NotEqEq, 3),
                    (b'=', _) => (Tk::NotEq, 2),
                    _ => (Tk::Bang, 1),
                },
                b'<' => match (at(1), at(2)) {
                    (b'<', b'=') => (Tk::ShlEq, 3),
                    (b'<', _) => (Tk::Shl, 2),
                    (b'=', _) => (Tk::Le, 2),
                    _ => (Tk::Lt, 1),
                },
                b'>' => match (at(1), at(2)) {
                    (b'>', b'=') => (Tk::ShrEq, 3),
                    (b'>', _) => (Tk::Shr, 2),
                    (b'=', _) => (Tk::Ge, 2),
                    _ => (Tk::Gt, 1),
                },
                _ => {
                    // not part of the language: skip one whole character
                    let ch = self.src[i..].chars().next().unwrap_or('\u{fffd}');
                    let l = ch.len_utf8();
                    let msg = if c < 0x80 {
                        format!("unexpected character `{}`", ch.escape_debug())
                    } else {
                        format!(
                            "unexpected character `{}` (U+{:04X}); identifiers are ASCII-only, \
                             Unicode is allowed in strings and comments",
                            ch.escape_debug(),
                            ch as u32
                        )
                    };
                    self.err("OXS-E0014", i, i + l, msg);
                    self.pos = i + l;
                    continue;
                }
            };
            self.push(kind, i, i + len, 0, ws, nl);
            self.pos = i + len;
        }
    }

    // ---- numbers -----------------------------------------------------------

    /// Scan digits of `radix` with single `_` separators between digits.
    fn scan_digits(&self, mut i: usize, radix: u32, out: &mut String, bad: &mut bool) -> usize {
        let b = self.b;
        let n = b.len();
        let mut prev_digit = false;
        while i < n {
            let c = b[i];
            if digit_of(c, radix) {
                out.push(c as char);
                prev_digit = true;
                i += 1;
            } else if c == b'_' {
                let next_ok = i + 1 < n && digit_of(b[i + 1], radix);
                if !(prev_digit && next_ok) {
                    *bad = true;
                }
                prev_digit = false;
                i += 1;
            } else {
                break;
            }
        }
        i
    }

    fn lex_number(&mut self, ws: bool, nl: bool) {
        let b = self.b;
        let n = b.len();
        let start = self.pos;
        let mut i = start;
        let mut bad = false;
        let mut reason = String::new();
        let mut text = String::new();
        let mut radix = 10u32;
        let mut is_float = false;
        let mut dot_fix: Option<String> = None;
        let mut float_hint = false;

        if b[i] == b'.' {
            is_float = true;
            text.push('.');
            i += 1;
            i = self.scan_digits(i, 10, &mut text, &mut bad);
        } else {
            if b[i] == b'0' && i + 1 < n && matches!(b[i + 1], b'x' | b'b' | b'o') {
                radix = match b[i + 1] {
                    b'x' => 16,
                    b'b' => 2,
                    _ => 8,
                };
                i += 2;
                let before = text.len();
                i = self.scan_digits(i, radix, &mut text, &mut bad);
                if text.len() == before {
                    bad = true;
                    reason = "no digits after the radix prefix".into();
                }
                // a decimal digit that is not valid in this radix (`0b12`)
                if radix != 16 && i < n && b[i].is_ascii_digit() {
                    bad = true;
                    reason = format!("digit `{}` is not valid here", b[i] as char);
                    while i < n && is_id_cont(b[i]) {
                        i += 1;
                    }
                }
            } else {
                i = self.scan_digits(i, 10, &mut text, &mut bad);
                if i < n && b[i] == b'.' {
                    let nx = b.get(i + 1).copied().unwrap_or(0);
                    if nx.is_ascii_digit() {
                        is_float = true;
                        text.push('.');
                        i += 1;
                        i = self.scan_digits(i, 10, &mut text, &mut bad);
                    } else if !is_id_start(nx) && nx != b'.' {
                        // `5.` is not a literal
                        let mut t = text.clone();
                        t.push('.');
                        dot_fix = Some(format!("{t}0"));
                        float_hint = true;
                        i += 1;
                        bad = true;
                        reason = "a digit must follow the decimal point".into();
                    }
                }
            }
        }
        if radix == 10 && dot_fix.is_none() && i < n && (b[i] == b'e' || b[i] == b'E') {
            let sign = matches!(b.get(i + 1), Some(b'+' | b'-'));
            let d = b.get(i + 1 + usize::from(sign)).copied().unwrap_or(0);
            if d.is_ascii_digit() {
                is_float = true;
                text.push('e');
                if sign {
                    text.push(b[i + 1] as char);
                }
                i += 1 + usize::from(sign);
                i = self.scan_digits(i, 10, &mut text, &mut bad);
            }
        }

        // suffix
        let mut suffix: Option<NumTy> = None;
        if i < n && is_id_start(b[i]) {
            let s0 = i;
            while i < n && is_id_cont(b[i]) {
                i += 1;
            }
            let w = &self.src[s0..i];
            match NumTy::from_name(w) {
                Some(t) if t.is_float() == is_float => suffix = Some(t),
                Some(t) if t.is_float() => {
                    bad = true;
                    reason = format!(
                        "an integer literal cannot take the float suffix `{w}`; write `{}.0{w}`",
                        &self.src[start..s0]
                    );
                    dot_fix = Some(format!("{}.0{w}", &self.src[start..s0]));
                }
                Some(_) => {
                    bad = true;
                    reason = format!("a float literal cannot take the integer suffix `{w}`");
                }
                None => {
                    bad = true;
                    if w.starts_with(['e', 'E']) && !is_float {
                        float_hint = true;
                        reason = "an exponent needs at least one digit".into();
                    } else {
                        reason = format!("`{w}` is not a numeric type suffix");
                    }
                }
            }
        }
        // `1.5.x` / `.5.x` are left to the parser. Value.
        let tok_text = &self.src[start..i];
        let lit = if is_float {
            LitData::Float { text, suffix }
        } else {
            let mut v: u128 = 0;
            let mut over = false;
            for ch in text.bytes() {
                let d = u128::from((ch as char).to_digit(radix).unwrap_or(0));
                match v
                    .checked_mul(u128::from(radix))
                    .and_then(|x| x.checked_add(d))
                {
                    Some(x) => v = x,
                    None => {
                        over = true;
                        break;
                    }
                }
            }
            if over {
                bad = true;
                reason = "the value does not fit in 128 bits".into();
            }
            LitData::Int { value: v, suffix }
        };
        if bad {
            let kind = if is_float || float_hint {
                "float"
            } else {
                "integer"
            };
            let msg = if reason.is_empty() {
                format!(
                    "`{}` is not a valid {kind} literal (digit separators `_` go between digits)",
                    clip(tok_text)
                )
            } else {
                format!(
                    "`{}` is not a valid {kind} literal: {reason}",
                    clip(tok_text)
                )
            };
            let sp = self.span(start, i);
            let mut d = Diagnostic::new("OXS-E0003", sp, msg);
            if let Some(f) = dot_fix {
                d = d.with_fix(Fix::replace(format!("write `{f}`"), sp, f));
            }
            self.push_diag(d);
        }
        let idx = self.add_lit(lit);
        self.push(
            if is_float { Tk::Float } else { Tk::Int },
            start,
            i,
            idx,
            ws,
            nl,
        );
        self.pos = i;
    }

    // ---- strings and templates ---------------------------------------------

    /// Append source text `lo..hi` of a literal, warning about invisible characters.
    fn copy_run(&mut self, out: &mut String, lo: usize, hi: usize, template: bool) {
        let text = &self.src[lo..hi];
        out.push_str(text);
        if text.is_ascii() && !text.bytes().any(|b| b < 0x20 && b != b'\t' || b == 0x7f) {
            return;
        }
        for (k, c) in text.char_indices() {
            if is_invisible(c) && !(template && matches!(c, '\n' | '\r')) {
                let at = lo + k;
                let sp = self.span(at, at + c.len_utf8());
                let esc = format!("\\u{{{:x}}}", c as u32);
                let d = Diagnostic::new(
                    "OXS-W0003",
                    sp,
                    format!(
                        "invisible or direction-changing character U+{:04X} in a literal; write it as `{esc}`",
                        c as u32
                    ),
                )
                .with_fix(Fix::replace(format!("write `{esc}`"), sp, esc));
                self.push_diag(d);
            }
        }
    }

    /// Decode one escape starting at the backslash at `i`; returns the index
    /// after it. `out` receives the decoded character (nothing on error).
    fn escape(&mut self, i: usize, out: &mut String) -> usize {
        let b = self.b;
        let n = b.len();
        let c = b.get(i + 1).copied();
        let simple = match c {
            Some(b'n') => Some('\n'),
            Some(b'r') => Some('\r'),
            Some(b't') => Some('\t'),
            Some(b'0') => Some('\0'),
            Some(b'\\') => Some('\\'),
            Some(b'"') => Some('"'),
            Some(b'\'') => Some('\''),
            Some(b'`') => Some('`'),
            Some(b'$') => Some('$'),
            _ => None,
        };
        if let Some(ch) = simple {
            out.push(ch);
            return i + 2;
        }
        match c {
            Some(b'x') => {
                let h = |k: usize| b.get(i + k).and_then(|&x| (x as char).to_digit(16));
                if let (Some(a), Some(d)) = (h(2), h(3)) {
                    let v = a * 16 + d;
                    if v <= 0x7f {
                        out.push(char::from(v as u8));
                    } else {
                        self.err(
                            "OXS-E0013",
                            i,
                            i + 4,
                            format!("`\\x{a:x}{d:x}` is above 0x7F; use `\\u{{...}}` for non-ASCII characters"),
                        );
                    }
                    i + 4
                } else {
                    self.err(
                        "OXS-E0013",
                        i,
                        (i + 2).min(n),
                        "`\\x` needs exactly two hexadecimal digits".into(),
                    );
                    i + 2
                }
            }
            Some(b'u') => {
                if b.get(i + 2) == Some(&b'{') {
                    let mut j = i + 3;
                    let mut v: u32 = 0;
                    let mut cnt = 0;
                    while j < n && digit_of(b[j], 16) && cnt < 7 {
                        v = v
                            .saturating_mul(16)
                            .saturating_add((b[j] as char).to_digit(16).unwrap_or(0));
                        j += 1;
                        cnt += 1;
                    }
                    if j < n && b[j] == b'}' && (1..=6).contains(&cnt) {
                        if let Some(ch) = char::from_u32(v) {
                            out.push(ch);
                        } else {
                            self.err(
                                "OXS-E0013",
                                i,
                                j + 1,
                                "`\\u{...}` must be a Unicode scalar value (no surrogates, at most 10FFFF)"
                                    .into(),
                            );
                        }
                        return j + 1;
                    }
                    self.err(
                        "OXS-E0013",
                        i,
                        j.min(n),
                        "`\\u{` needs 1 to 6 hexadecimal digits and a closing `}`".into(),
                    );
                    j
                } else {
                    self.err(
                        "OXS-E0013",
                        i,
                        (i + 2).min(n),
                        "`\\u` must be written `\\u{...}`".into(),
                    );
                    i + 2
                }
            }
            Some(_) => {
                let ch = self.src[i + 1..].chars().next().unwrap_or('?');
                let l = ch.len_utf8();
                self.err(
                    "OXS-E0013",
                    i,
                    i + 1 + l,
                    format!("unknown escape `\\{}`", ch.escape_debug()),
                );
                i + 1 + l
            }
            None => {
                self.err(
                    "OXS-E0013",
                    i,
                    i + 1,
                    "a backslash at the end of the file".into(),
                );
                i + 1
            }
        }
    }

    fn lex_string(&mut self, q: u8, ws: bool, nl: bool) {
        let b = self.b;
        let n = b.len();
        let start = self.pos;
        let mut i = start + 1;
        let mut out = String::new();
        let mut run = i;
        loop {
            if i >= n || b[i] == b'\n' || b[i] == b'\r' {
                self.copy_run(&mut out, run, i, false);
                self.err(
                    "OXS-E0002",
                    start,
                    start + 1,
                    "the string starting here is never closed".into(),
                );
                break;
            }
            let c = b[i];
            if c == q {
                self.copy_run(&mut out, run, i, false);
                i += 1;
                break;
            }
            if c == b'\\' {
                self.copy_run(&mut out, run, i, false);
                i = self.escape(i, &mut out);
                run = i;
                continue;
            }
            i += 1;
        }
        let idx = self.add_lit(LitData::Str(out));
        self.push(Tk::Str, start, i, idx, ws, nl);
        self.pos = i;
    }

    /// Scan template text starting at `from` (after the opening backtick or
    /// the closing brace of an interpolation). `tok_lo` is the token start.
    fn lex_template_chunk(
        &mut self,
        tok_lo: usize,
        from: usize,
        opening: bool,
        ws: bool,
        nl: bool,
    ) {
        let b = self.b;
        let n = b.len();
        let mut i = from;
        let mut out = String::new();
        let mut run = i;
        let (kind, closed);
        loop {
            if i >= n {
                self.copy_run(&mut out, run, i, true);
                self.err(
                    "OXS-E0002",
                    tok_lo,
                    tok_lo + 1,
                    "the template literal starting here is never closed".into(),
                );
                closed = false;
                kind = if opening {
                    Tk::TemplateFull
                } else {
                    Tk::TemplateTail
                };
                break;
            }
            let c = b[i];
            if c == b'`' {
                self.copy_run(&mut out, run, i, true);
                i += 1;
                closed = true;
                kind = if opening {
                    Tk::TemplateFull
                } else {
                    Tk::TemplateTail
                };
                break;
            }
            if c == b'$' && b.get(i + 1) == Some(&b'{') {
                self.copy_run(&mut out, run, i, true);
                i += 2;
                self.braces.push(Brace::Template);
                closed = true;
                kind = if opening {
                    Tk::TemplateHead
                } else {
                    Tk::TemplateMiddle
                };
                break;
            }
            if c == b'\\' {
                self.copy_run(&mut out, run, i, true);
                i = self.escape(i, &mut out);
                run = i;
                continue;
            }
            if c == b'\r' {
                // CRLF and lone CR are normalised to LF in the cooked value
                self.copy_run(&mut out, run, i, true);
                out.push('\n');
                i += 1;
                if b.get(i) == Some(&b'\n') {
                    i += 1;
                }
                run = i;
                continue;
            }
            i += 1;
        }
        let _ = closed;
        let idx = self.add_lit(LitData::Str(out));
        self.push(kind, tok_lo, i, idx, ws, nl);
        self.pos = i;
    }
}

fn clip(s: &str) -> String {
    if s.chars().count() > 40 {
        let t: String = s.chars().take(40).collect();
        format!("{t}...")
    } else {
        s.to_string()
    }
}
