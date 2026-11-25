//! Recursive-descent / Pratt parser for the MVP grammar.
//!
//! Design notes (see also `docs/oxiscript/grammar.md` 3.4):
//!
//! * **Tokens first.** The whole file is lexed up front; the parser walks a
//!   token vector with an index, so lookahead and rewinding are O(1).
//! * **Bounded speculation.** Three places need a speculative parse: a generic
//!   call (`a<T>(x)` versus `a < b`), an arrow function with a return type
//!   (`(x): T => y`), and `>>`/`>=` splitting. Speculation snapshots the cursor,
//!   node-id counter and diagnostic count and rewinds on failure; the total
//!   number of tokens consumed while speculating is capped (`spec_cap`), so no
//!   input can make the parser exponential: past the cap parsing stops with
//!   `OXS-E0012`.
//! * **Depth limits.** Recursion depth is limited ([`MAX_DEPTH`]) and so is the
//!   height of the tree built (iterative loops such as `a + b + c + ...` build
//!   left-deep trees), tracked in a side table, so neither the parser nor any
//!   later recursive consumer (formatter, checker, `Drop`) can overflow the
//!   stack. Exceeding either gives `OXS-E0011` and parsing stops.
//! * **Recovery.** Errors are recorded and parsing continues with a placeholder
//!   (an `Ident` with an empty name, see [`is_error_ident`]); statements and
//!   declarations resynchronise at `;`, a closing `}` or a line that starts
//!   with a declaration keyword. At most one `E0001`/`E0004` is reported per
//!   source position.

use crate::ast::*;
use crate::diag::{mk_span, Diagnostic, Fix, MAX_DIAGS};
use crate::lexer::{lex, Comment, CommentKind, Kw, Lexed, LitData, Tk, Token};
use crate::visit::*;

mod expr;
pub use expr::op_text;
mod item;
mod stmt;
mod types;

/// Default for [`ParseOptions::max_depth`]: maximum nesting of parser recursion
/// (parentheses, blocks, types, ...). Chosen so that the inline parser fits in
/// 256 KB of release stack on native targets and in the default wasm32 stack:
/// see the crate docs for the measured bytes per level.
pub const DEFAULT_MAX_DEPTH: usize = 100;
/// Maximum height of the expression/statement tree (counts chain length). Chains
/// are parsed, formatted and dropped iteratively, so this costs no stack.
pub const MAX_HEIGHT: u32 = 1000;
/// Nesting depth at which an inline attempt is abandoned for a big-stack thread.
const SOFT_DEPTH: u32 = 12;
/// Bracket nesting (from the lexer) above which the thread is used right away.
const SOFT_NEST: u32 = 6;
const BIG_STACK: usize = 128 << 20;

/// Options for [`parse_with`].
#[derive(Clone, Debug)]
pub struct ParseOptions {
    /// Maximum nesting depth (`OXS-E0011` beyond it). Whether a program is
    /// accepted depends only on this number, never on the platform or on
    /// whether a helper thread was available.
    pub max_depth: usize,
    /// Allow a helper thread with a large stack for deeply nested input
    /// (native targets; ignored where threads cannot be created). It adds
    /// headroom for the caller's stack and never changes the result.
    pub use_thread: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        ParseOptions {
            max_depth: DEFAULT_MAX_DEPTH,
            use_thread: true,
        }
    }
}

/// An identifier standing in for a missing or malformed name after a syntax
/// error has been reported. Consumers must not look at a tree whose parse
/// returned diagnostics, but tools that do (LSP) can recognise placeholders.
pub fn is_error_ident(i: &Ident) -> bool {
    i.name.is_empty()
}

/// Everything a tool needs after parsing one file.
#[derive(Debug)]
pub struct Parsed {
    pub module: Module,
    pub diagnostics: Vec<Diagnostic>,
    /// All comments in source order (the formatter and doc tooling use them).
    pub comments: Vec<Comment>,
    /// Height of the tallest expression/statement/block/type subtree.
    pub max_height: u32,
    /// Deepest bracket nesting in the token stream.
    pub max_nest: u32,
}

#[derive(Clone)]
struct Snapshot {
    pos: usize,
    part: u32,
    ids: NodeIdGen,
    ht_len: usize,
    diags_len: usize,
    updates_len: usize,
    last_end: u32,
    depth: u32,
}

pub(crate) struct Parser<'a> {
    src: &'a str,
    file: FileId,
    toks: &'a [Token],
    lits: &'a [LitData],
    pm: &'a [u32],
    comments: &'a [Comment],
    max_depth: u32,
    diag_budget: usize,
    allow_restart: bool,
    pos: usize,
    /// Bytes already consumed from the current token (`>>` split into `>` `>`).
    part: u32,
    ids: NodeIdGen,
    /// Tree height per `NodeId` (only meaningful for expr/stmt/block/item ids).
    ht: Vec<u32>,
    diags: Vec<Diagnostic>,
    fatal: Vec<Diagnostic>,
    depth: u32,
    spec: u32,
    spec_fail: bool,
    bail: bool,
    stop: bool,
    spec_work: u64,
    spec_cap: u64,
    /// `case` labels: `x => y` is not an arrow function there.
    no_arrow: bool,
    /// Inside the then-branch of a `?:` (arrow return-type rule, 3.4 rule 2).
    then_ctx: bool,
    /// Every `++`/`--` built, to reject uses as a value.
    updates: Vec<(NodeId, Span)>,
    last_end: u32,
    /// Running on the large-stack thread: no restart.
    big: bool,
    /// Nesting reached `SOFT_DEPTH` on the small stack: discard this attempt
    /// and parse again once on a big-stack thread.
    restart: bool,
}

struct Attempt {
    module: Module,
    diags: Vec<Diagnostic>,
    max_height: u32,
    restart: bool,
}

struct Inputs<'a> {
    src: &'a str,
    file: FileId,
    toks: &'a [Token],
    lits: &'a [LitData],
    pm: &'a [u32],
    comments: &'a [Comment],
    max_depth: u32,
    diag_budget: usize,
    allow_restart: bool,
}

fn attempt(i: &Inputs<'_>, big: bool) -> Attempt {
    let n = i.toks.len() as u64;
    let mut p = Parser {
        src: i.src,
        file: i.file,
        toks: i.toks,
        lits: i.lits,
        pm: i.pm,
        comments: i.comments,
        max_depth: i.max_depth,
        diag_budget: i.diag_budget,
        allow_restart: i.allow_restart,
        pos: 0,
        part: 0,
        ids: NodeIdGen::new(),
        ht: Vec::new(),
        diags: Vec::new(),
        fatal: Vec::new(),
        depth: 0,
        spec: 0,
        spec_fail: false,
        bail: false,
        stop: false,
        spec_work: 0,
        spec_cap: n * 24 + 20_000,
        no_arrow: false,
        then_ctx: false,
        updates: Vec::new(),
        last_end: 0,
        big,
        restart: false,
    };
    let module = p.parse_module();
    let mut diags = std::mem::take(&mut p.diags);
    diags.append(&mut p.fatal);
    let max_height = p.ht.iter().copied().max().unwrap_or(0);
    Attempt {
        module,
        diags,
        max_height,
        restart: p.restart,
    }
}

/// Run one attempt on a helper thread with a large stack; `None` if no thread
/// can be created (wasm, tiny RTOS, resource limits).
fn on_big_stack(i: &Inputs<'_>) -> Option<Attempt> {
    let spawned = std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(BIG_STACK)
            .spawn_scoped(s, || attempt(i, true))
            .map(std::thread::ScopedJoinHandle::join)
    });
    match spawned {
        Ok(Ok(r)) => Some(r),
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => None,
    }
}

/// Parse a module with default options. `file` is recorded in every span.
pub fn parse(src: &str, file: FileId) -> Parsed {
    parse_with(src, file, &ParseOptions::default())
}

/// Parse a module. Parsing runs on the caller's stack unless the input nests
/// deeply, in which case (and only if [`ParseOptions::use_thread`]) it runs once
/// on one helper thread with a large stack; without a thread the same parse runs
/// inline, bounded by `max_depth`. At most one thread is spawned per call.
pub fn parse_with(src: &str, file: FileId, opts: &ParseOptions) -> Parsed {
    let Lexed {
        tokens,
        lits,
        comments,
        diags: lex_diags,
        paren_match,
        max_nest,
    } = lex(src, file);
    let max_depth = u32::try_from(opts.max_depth.min(10_000)).unwrap_or(10_000);
    let inputs = Inputs {
        src,
        file,
        toks: &tokens,
        lits: &lits,
        pm: &paren_match,
        comments: &comments,
        max_depth,
        diag_budget: MAX_DIAGS.saturating_sub(lex_diags.len()),
        allow_restart: opts.use_thread,
    };
    let mut a = if opts.use_thread && max_nest > SOFT_NEST {
        on_big_stack(&inputs).unwrap_or_else(|| attempt(&inputs, true))
    } else {
        attempt(&inputs, false)
    };
    if a.restart {
        a = on_big_stack(&inputs).unwrap_or_else(|| attempt(&inputs, true));
    }
    let mut all = lex_diags;
    all.append(&mut a.diags);
    all.sort_by_key(|d| (d.span.lo, d.span.hi));
    Parsed {
        max_height: a.max_height,
        max_nest,
        module: a.module,
        diagnostics: all,
        comments,
    }
}

/// Parse a module from source text (file id 0).
pub fn parse_module(src: &str) -> (Module, Vec<Diagnostic>) {
    let p = parse(src, FileId(0));
    (p.module, p.diagnostics)
}

#[derive(Default)]
struct HMax<'t> {
    ht: &'t [u32],
    m: u32,
}

impl<'a> Visitor<'a> for HMax<'_> {
    fn visit_expr(&mut self, e: &'a Expr) {
        self.m = self.m.max(self.ht[e.id.0 as usize]);
    }
    fn visit_stmt(&mut self, s: &'a Stmt) {
        self.m = self.m.max(self.ht[s.id.0 as usize]);
    }
    fn visit_block(&mut self, b: &'a Block) {
        self.m = self.m.max(self.ht[b.id.0 as usize]);
    }
    fn visit_item(&mut self, i: &'a Item) {
        self.m = self.m.max(self.ht[i.id.0 as usize]);
    }
    fn visit_type(&mut self, _t: &'a TypeExpr) {}
    fn visit_pattern(&mut self, _p: &'a Pattern) {}
}

fn split_kind(k: Tk, part: u32) -> Tk {
    match (k, part) {
        (Tk::Shr, 1) => Tk::Gt,
        (Tk::Ge | Tk::ShrEq, 1) => {
            if k == Tk::Ge {
                Tk::Assign
            } else {
                Tk::Ge
            }
        }
        (Tk::ShrEq, 2) => Tk::Assign,
        _ => Tk::Eof,
    }
}

impl<'a> Parser<'a> {
    // ---- cursor ------------------------------------------------------------

    fn kind(&self) -> Tk {
        if self.stop {
            Tk::Eof
        } else if self.part == 0 {
            self.toks[self.pos].kind
        } else {
            split_kind(self.toks[self.pos].kind, self.part)
        }
    }

    /// Kind of the raw token `n` positions ahead (ignores a split in progress).
    fn nth(&self, n: usize) -> Tk {
        if self.stop {
            return Tk::Eof;
        }
        let i = (self.pos + n).min(self.toks.len() - 1);
        self.toks[i].kind
    }

    fn nth_tok(&self, n: usize) -> &Token {
        let i = (self.pos + n).min(self.toks.len() - 1);
        &self.toks[i]
    }

    fn at(&self, k: Tk) -> bool {
        self.kind() == k
    }

    fn at_kw(&self, k: Kw) -> bool {
        self.kind() == Tk::Kw(k)
    }

    fn cur_lo(&self) -> u32 {
        if self.stop {
            return self.toks[self.toks.len() - 1].lo;
        }
        self.toks[self.pos].lo + self.part
    }

    fn cur_hi(&self) -> u32 {
        if self.stop {
            return self.toks[self.toks.len() - 1].hi;
        }
        self.toks[self.pos].hi
    }

    fn cur_ws(&self) -> bool {
        self.toks[self.pos].ws || self.toks[self.pos].nl
    }

    fn cur_nl(&self) -> bool {
        self.toks[self.pos].nl
    }

    fn cur_span(&self) -> Span {
        mk_span(self.file, self.cur_lo() as usize, self.cur_hi() as usize)
    }

    /// Source text of the current token (the unconsumed part of a split token).
    fn cur_text(&self) -> &'a str {
        if self.stop {
            return "";
        }
        &self.src[self.cur_lo() as usize..self.cur_hi() as usize]
    }

    fn tok_text(&self, t: &Token) -> &'a str {
        &self.src[t.lo as usize..t.hi as usize]
    }

    fn at_ident_word(&self, w: &str) -> bool {
        self.kind() == Tk::Ident && self.cur_text() == w
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos];
        if self.stop {
            return t;
        }
        if self.spec > 0 {
            self.spec_work += 1;
            if self.spec_work > self.spec_cap {
                self.blow_budget();
            }
        }
        self.last_end = t.hi;
        self.part = 0;
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, k: Tk) -> bool {
        if self.kind() == k {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_kw(&mut self, k: Kw) -> bool {
        self.eat(Tk::Kw(k))
    }

    /// Consume one `>` at the cursor, splitting `>>`, `>=` and `>>=`.
    fn eat_gt(&mut self) -> bool {
        match self.kind() {
            Tk::Gt => {
                self.bump();
                true
            }
            Tk::Shr | Tk::Ge | Tk::ShrEq => {
                if self.spec > 0 {
                    self.spec_work += 1;
                }
                self.part += 1;
                self.last_end = self.toks[self.pos].lo + self.part;
                true
            }
            _ => false,
        }
    }

    fn span_from(&self, lo: u32) -> Span {
        mk_span(self.file, lo as usize, self.last_end.max(lo) as usize)
    }

    fn prev_end_span(&self) -> Span {
        mk_span(self.file, self.last_end as usize, self.last_end as usize)
    }

    fn found_desc(&self) -> String {
        match self.kind() {
            Tk::Eof => "end of file".to_string(),
            Tk::Ident => format!("identifier `{}`", clip(self.cur_text())),
            Tk::Int | Tk::Float | Tk::Str => format!("literal `{}`", clip(self.cur_text())),
            Tk::TemplateFull | Tk::TemplateHead | Tk::TemplateMiddle | Tk::TemplateTail => {
                "a template string".to_string()
            }
            _ => format!("`{}`", clip(self.cur_text())),
        }
    }

    // ---- ids, nodes, heights ----------------------------------------------

    fn nid(&mut self) -> NodeId {
        self.ht.push(0);
        self.ids.next_id()
    }

    fn mk_ident(&mut self, name: String, span: Span) -> Ident {
        Ident {
            id: self.nid(),
            span,
            name,
        }
    }

    fn error_ident(&mut self) -> Ident {
        let s = mk_span(self.file, self.cur_lo() as usize, self.cur_lo() as usize);
        self.mk_ident(String::new(), s)
    }

    fn mk_expr(&mut self, kind: ExprKind, lo: u32) -> Expr {
        let span = self.span_from(lo);
        self.mk_expr_span(kind, span)
    }

    fn mk_expr_span(&mut self, kind: ExprKind, span: Span) -> Expr {
        let e = Expr {
            id: self.nid(),
            span,
            kind,
        };
        let m = {
            let mut h = HMax { ht: &self.ht, m: 0 };
            walk_expr(&mut h, &e);
            h.m
        };
        self.set_height(e.id, m + 1);
        e
    }

    fn mk_stmt(&mut self, kind: StmtKind, attrs: Vec<Attribute>, lo: u32) -> Stmt {
        let span = self.span_from(lo);
        let s = Stmt {
            id: self.nid(),
            span,
            attrs,
            kind,
        };
        let m = {
            let mut h = HMax { ht: &self.ht, m: 0 };
            walk_stmt(&mut h, &s);
            h.m
        };
        self.set_height(s.id, m + 1);
        s
    }

    fn mk_block(&mut self, stmts: Vec<Stmt>, tail: Option<Box<Expr>>, lo: u32) -> Block {
        let span = self.span_from(lo);
        let b = Block {
            id: self.nid(),
            span,
            stmts,
            tail,
        };
        let m = {
            let mut h = HMax { ht: &self.ht, m: 0 };
            walk_block(&mut h, &b);
            h.m
        };
        self.set_height(b.id, m + 1);
        b
    }

    fn mk_item(
        &mut self,
        lo: u32,
        doc: Vec<String>,
        attrs: Vec<Attribute>,
        export: bool,
        kind: ItemKind,
    ) -> Item {
        let span = self.span_from(lo);
        let it = Item {
            id: self.nid(),
            span,
            doc,
            attrs,
            export,
            kind,
        };
        let m = {
            let mut h = HMax { ht: &self.ht, m: 0 };
            walk_item(&mut h, &it);
            h.m
        };
        self.set_height(it.id, m + 1);
        it
    }

    fn set_height(&mut self, id: NodeId, h: u32) {
        self.ht[id.0 as usize] = h;
        if h > MAX_HEIGHT {
            let s = self.cur_span();
            self.fatal_diag(Diagnostic::new(
                "OXS-E0011",
                s,
                format!(
                    "this construct is nested or chained more than {MAX_HEIGHT} levels deep; \
                     split it into several statements or helper functions"
                ),
            ));
        }
    }

    // ---- diagnostics and failure ------------------------------------------

    fn fail_spec(&mut self) {
        self.spec_fail = true;
        self.stop = true;
    }

    fn blow_budget(&mut self) {
        if !self.bail {
            let s = mk_span(self.file, self.last_end as usize, self.last_end as usize);
            self.fatal.push(Diagnostic::new(
                "OXS-E0012",
                s,
                "this input is too ambiguous to parse in reasonable time (too much backtracking); \
                 add parentheses or split the expression"
                    .to_string(),
            ));
        }
        self.bail = true;
        self.stop = true;
    }

    /// Report an error and stop parsing (depth and size limits).
    fn fatal_diag(&mut self, d: Diagnostic) {
        if self.spec > 0 {
            self.fail_spec();
            return;
        }
        if !self.bail {
            self.fatal.push(d);
        }
        self.bail = true;
        self.stop = true;
    }

    fn diag(&mut self, d: Diagnostic) {
        if self.spec > 0 {
            self.fail_spec();
            return;
        }
        if self.bail {
            return;
        }
        if matches!(d.code, "OXS-E0001" | "OXS-E0004") {
            if let Some(last) = self.diags.last() {
                if last.span.lo == d.span.lo && matches!(last.code, "OXS-E0001" | "OXS-E0004") {
                    return;
                }
            }
        }
        if self.diags.len() >= self.diag_budget {
            if self.diag_budget == 0 {
                self.bail = true;
                self.stop = true;
                return;
            }
            let s = mk_span(self.file, self.last_end as usize, self.last_end as usize);
            self.fatal_diag(Diagnostic::new(
                "OXS-E0017",
                s,
                format!("more than {MAX_DIAGS} errors; giving up on this file"),
            ));
            return;
        }
        self.diags.push(d);
    }

    fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) {
        self.diag(Diagnostic::new(code, span, msg));
    }

    /// `expected {what}, found {found}` at the current token.
    fn expected(&mut self, what: &str) {
        let span = if self.kind() == Tk::Eof {
            self.prev_end_span()
        } else {
            self.cur_span()
        };
        let msg = format!("expected {what}, found {}", self.found_desc());
        self.error("OXS-E0001", span, msg);
    }

    fn expect(&mut self, k: Tk, what: &str) -> bool {
        if self.eat(k) {
            true
        } else {
            self.expected(what);
            false
        }
    }

    /// Statement terminator; a missing one is `OXS-E0004` with an insert fix.
    fn expect_semi(&mut self) {
        if self.eat(Tk::Semi) {
            return;
        }
        let at = self.prev_end_span();
        let d = Diagnostic::new("OXS-E0004", at, "statements end with `;`").with_fix(Fix::insert(
            "insert `;`",
            at,
            ";",
        ));
        self.diag(d);
    }

    fn enter(&mut self) -> bool {
        self.depth += 1;
        if self.depth > SOFT_DEPTH && self.allow_restart && !self.big && !self.restart {
            self.restart = true;
            self.bail = true;
            self.stop = true;
            return false;
        }
        if self.depth > self.max_depth {
            let s = self.cur_span();
            self.fatal_diag(Diagnostic::new(
                "OXS-E0011",
                s,
                format!(
                    "nesting deeper than {} levels; split the code into smaller pieces",
                    self.max_depth
                ),
            ));
            return false;
        }
        true
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    // ---- speculation -------------------------------------------------------

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            pos: self.pos,
            part: self.part,
            ids: self.ids.clone(),
            ht_len: self.ht.len(),
            diags_len: self.diags.len(),
            updates_len: self.updates.len(),
            last_end: self.last_end,
            depth: self.depth,
        }
    }

    fn restore(&mut self, s: &Snapshot) {
        self.pos = s.pos;
        self.part = s.part;
        self.ids = s.ids.clone();
        self.ht.truncate(s.ht_len);
        self.diags.truncate(s.diags_len);
        self.updates.truncate(s.updates_len);
        self.last_end = s.last_end;
        self.depth = s.depth;
        self.spec_fail = false;
        self.stop = self.bail;
    }

    // ---- literals ----------------------------------------------------------

    fn lit_from(&mut self, t: &Token) -> Lit {
        let kind = match (t.kind, self.lits.get(t.data as usize)) {
            (Tk::Int, Some(LitData::Int { value, suffix })) => LitKind::Int {
                value: *value,
                suffix: *suffix,
            },
            (Tk::Float, Some(LitData::Float { text, suffix })) => LitKind::Float {
                text: text.clone(),
                suffix: *suffix,
            },
            (Tk::Str, Some(LitData::Str(s))) => LitKind::Str(s.clone()),
            (Tk::Kw(Kw::True), _) => LitKind::Bool(true),
            (Tk::Kw(Kw::False), _) => LitKind::Bool(false),
            _ => LitKind::Null,
        };
        Lit {
            id: self.nid(),
            span: mk_span(self.file, t.lo as usize, t.hi as usize),
            kind,
        }
    }

    fn str_data(&self, t: &Token) -> String {
        match self.lits.get(t.data as usize) {
            Some(LitData::Str(s)) => s.clone(),
            _ => String::new(),
        }
    }

    // ---- identifiers -------------------------------------------------------

    /// An identifier. Reserved words are reported (`OXS-E0900`) and accepted so
    /// that parsing can continue; other tokens are an error and not consumed
    /// (keywords on the same line are consumed as the offending name).
    fn parse_ident(&mut self, what: &str) -> Ident {
        match self.kind() {
            Tk::Ident => {
                let t = self.bump();
                let name = self.tok_text(&t).to_string();
                self.mk_ident(name, mk_span(self.file, t.lo as usize, t.hi as usize))
            }
            Tk::Reserved => {
                let t = self.bump();
                let name = self.tok_text(&t).to_string();
                let sp = mk_span(self.file, t.lo as usize, t.hi as usize);
                self.reserved_error(&name, sp);
                self.mk_ident(name, sp)
            }
            Tk::Kw(_) if !self.cur_nl() => {
                let t = self.bump();
                let sp = mk_span(self.file, t.lo as usize, t.hi as usize);
                let msg = format!(
                    "expected {what}, found keyword `{}` (keywords cannot be used as names)",
                    self.tok_text(&t)
                );
                self.error("OXS-E0001", sp, msg);
                self.mk_ident(String::new(), sp)
            }
            _ => {
                self.expected(what);
                self.error_ident()
            }
        }
    }

    fn reserved_error(&mut self, word: &str, span: Span) {
        let hint = reserved_hint(word);
        self.error(
            "OXS-E0900",
            span,
            format!("`{word}` is reserved for a future version of OxiScript{hint}"),
        );
    }

    // ---- comments ----------------------------------------------------------

    /// Doc comments (`///`) directly before the token at `lo`: the trailing run
    /// of doc comments between the previous token and `lo`.
    fn docs_before(&self, lo: u32) -> Vec<String> {
        let end = self.comments.partition_point(|c| c.hi <= lo);
        let mut start = end;
        while start > 0 {
            let c = &self.comments[start - 1];
            if c.kind != CommentKind::Doc || c.lo < self.last_end {
                break;
            }
            start -= 1;
        }
        self.comments[start..end]
            .iter()
            .map(|c| {
                let t = &self.src[c.lo as usize + 3..c.hi as usize];
                let t = t.strip_prefix(' ').unwrap_or(t);
                t.trim_end().to_string()
            })
            .collect()
    }

    // ---- module ------------------------------------------------------------

    fn parse_module(&mut self) -> Module {
        let lo = 0u32;
        let mut attrs = Vec::new();
        let mut items = Vec::new();
        while self.at(Tk::AtAt) {
            let a = self.parse_attribute();
            self.expect_semi();
            attrs.push(a);
        }
        while !self.at(Tk::Eof) {
            let before = self.pos;
            let part_before = self.part;
            if let Some(it) = self.parse_item(false) {
                items.push(it);
            } else {
                self.sync_item();
            }
            if self.pos == before && self.part == part_before && !self.at(Tk::Eof) {
                self.bump();
            }
        }
        let id = self.nid();
        let hi = u32::try_from(self.src.len()).unwrap_or(u32::MAX);
        Module {
            id,
            span: Span::new(self.file, lo, hi),
            attrs,
            items,
        }
    }

    /// Skip to the next plausible declaration start after a failed item.
    fn sync_item(&mut self) {
        self.sync(true);
    }

    /// Skip tokens until a boundary: after a `;` at nesting 0, after the `}`
    /// that closes a block opened while skipping, before a `}` that closes an
    /// enclosing block, or before a line that starts a declaration/statement.
    fn sync(&mut self, items_only: bool) {
        let mut nest = 0i32;
        let mut first = true;
        loop {
            let k = self.kind();
            if k == Tk::Eof {
                return;
            }
            if !first && nest == 0 && self.cur_nl() && self.starts_decl(items_only) {
                return;
            }
            first = false;
            match k {
                Tk::LBrace | Tk::LParen | Tk::LBracket => nest += 1,
                Tk::RBrace => {
                    if nest == 0 {
                        return;
                    }
                    nest -= 1;
                    self.bump();
                    if nest == 0 {
                        return;
                    }
                    continue;
                }
                Tk::RParen | Tk::RBracket => nest = (nest - 1).max(0),
                Tk::Semi if nest == 0 => {
                    self.bump();
                    return;
                }
                _ => {}
            }
            self.bump();
        }
    }

    fn starts_decl(&self, items_only: bool) -> bool {
        match self.kind() {
            Tk::Kw(
                Kw::Export
                | Kw::Import
                | Kw::Function
                | Kw::Struct
                | Kw::Class
                | Kw::Interface
                | Kw::Enum
                | Kw::Let
                | Kw::Const,
            )
            | Tk::At
            | Tk::AtAt => true,
            Tk::Kw(
                Kw::If
                | Kw::While
                | Kw::For
                | Kw::Do
                | Kw::Return
                | Kw::Switch
                | Kw::Break
                | Kw::Continue,
            ) => !items_only,
            _ => false,
        }
    }
}

fn clip(s: &str) -> String {
    if s.chars().count() > 24 {
        let t: String = s.chars().take(24).collect();
        format!("{t}...")
    } else {
        s.to_string()
    }
}

/// Pointer to the replacement for a reserved TypeScript word, where one exists.
pub(crate) fn reserved_hint(word: &str) -> &'static str {
    match word {
        "any" | "unknown" => {
            "; OxiScript has no `any`: use a concrete type or a discriminated union"
        }
        "undefined" => "; there is no `undefined`, use `null`",
        "var" => "; use `let` or `const`",
        "typeof" => "; there is no `typeof`: compare a union's `kind` field instead",
        "instanceof" => "; use a discriminated union and `switch (x.kind)`, or `x.downcast<T>()`",
        "delete" => "; remove the entry by building a new value",
        "public" | "protected" => "; members are public by default, use `private` to hide",
        "super" => "; classes cannot extend other classes, use interfaces and composition",
        "declare" | "namespace" => "; use modules (`import` / `export`)",
        "with" | "debugger" => "; this statement does not exist",
        "async" | "await" | "spawn" | "isolate" | "yield" => {
            "; asynchronous code is not part of the MVP"
        }
        "match" => "; use `switch` (it is an expression when used as a value)",
        "kernel" | "parallel" => "; the kernel/parallel profile is not available yet",
        "inout" => "; mutable value parameters are not available yet",
        "requires" | "ensures" | "invariant" => "; use a `law` declaration for properties",
        _ => "",
    }
}
