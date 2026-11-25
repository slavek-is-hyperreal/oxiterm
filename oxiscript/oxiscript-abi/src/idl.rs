//! The host interface definition language (IDL): a small text format that
//! describes a *capability* (what a host offers a script) and compiles to plain
//! Core Wasm imports. No Component Model dependency; the format is kept simple
//! enough to be mechanically convertible to WIT later.
//!
//! ```text
//! // comment
//! interface Log "host:log" @1 {
//!     fn write(level: u8, msg: str);
//! }
//! interface State "host:state" @1 {
//!     handle Slot;
//!     fn open(key: str) -> result<Slot>;
//!     fn get_i64(slot: Slot) -> result<i64>;
//!     fn read_str(slot: Slot) -> result<str>;
//!     callback OnChange(slot: Slot);
//!     fn watch(slot: Slot, cb: OnChange);
//! }
//! ```
//!
//! Grammar (EBNF; `//` comments and whitespace are skipped):
//! ```text
//! file      = { interface } ;
//! interface = "interface" Ident String "@" Int "{" { member } "}" ;
//! member    = "handle" Ident ";"
//!           | "fn" Ident "(" [ params ] ")" [ "->" ret ] ";"
//!           | "callback" Ident "(" [ params ] ")" [ "->" prim ] ";" ;
//! params    = param { "," param } [ "," ] ;   param = Ident ":" type ;
//! type      = prim | "str" | "bytes" | Ident ;   (* Ident = a handle or callback of this interface *)
//! ret       = prim | Ident (* handle *) | "result" "<" ( "void" | type-without-callback ) ">" ;
//! prim      = "bool" | "i8" | "u8" | "i16" | "u16" | "i32" | "u32" | "i64" | "u64" | "f32" | "f64" ;
//! ```
//!
//! # Mapping to Wasm imports
//!
//! Import module = `"<module>@<version>"` (e.g. `host:log@1`), import name =
//! the function name. Every function takes the instance's capability handle
//! as an implicit first `i32` (`self`): a script can only call what it was
//! handed, and the host validates the handle on every call.
//!
//! | IDL type | Wasm params |
//! |---|---|
//! | `bool`, `i8..i32`, `u8..u32` | one `i32` (canonical sign/zero extension) |
//! | `i64`, `u64` | `i64` |
//! | `f32`, `f64` | `f32`, `f64` |
//! | `str`, `bytes` | `(ptr: i32, len: i32)`, borrowed for the call: UTF-8 bytes (`str`) resp. raw bytes in guest memory |
//! | handle `H` | `i32` host-managed opaque id (generational; the host validates) |
//! | callback `C` | `i32` closure pointer, ownership transferred to the host |
//!
//! Results: `-> T` (prim/handle) = `[T]`; none = `[]`; `result<void>` =
//! `[i32 status]`; `result<T>` = `[i32 status, T]` (Core Wasm multi-value);
//! `result<str|bytes>` = `[i32 status, i32 len]` and two extra trailing params
//! `(out_ptr: i32, out_cap: i32)`: the host writes at most `out_cap` bytes
//! into guest memory and returns the full length; if `len > out_cap` it
//! returns `STATUS_BUFFER_TOO_SMALL` with the needed `len`, **without
//! side effects**, and the guest retries with a bigger buffer. (The host may
//! not call into the guest: see `exports` re-entrancy rule.) Status `0` is
//! success; the value result of a failed call is zero.
//!
//! Callbacks (`callback C(...)`): the module exports `cb:<Interface>.<C>`
//! with signature `(closure: i32, params...) -> ret`; `str`/`bytes` params
//! arrive as `(ptr, len)` into the scratch region written by the host.

use crate::layout::Prim;
use crate::WasmTy;
use std::fmt;

/// Example interface: the `log` capability (also a test fixture).
pub const EXAMPLE_LOG_IDL: &str = include_str!("../idl/log.idl");
/// Example interface: the `state` capability (also a test fixture).
pub const EXAMPLE_STATE_IDL: &str = include_str!("../idl/state.idl");

/// Host call succeeded.
pub const STATUS_OK: i32 = 0;
/// `result<str|bytes>`: output buffer too small; `len` holds the needed size.
pub const STATUS_BUFFER_TOO_SMALL: i32 = 1;
/// Capability or handle invalid, revoked or not granted.
pub const STATUS_DENIED: i32 = 2;
/// First status code a host may define for its own errors.
pub const STATUS_HOST_ERROR_BASE: i32 = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdlTy {
    Prim(Prim),
    Str,
    Bytes,
    /// A handle type declared in the same interface.
    Handle(String),
    /// A callback type declared in the same interface (parameters only).
    Callback(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdlRet {
    Void,
    /// Direct value: a primitive or a handle.
    Value(IdlTy),
    /// `result<void>` is `Result(None)`.
    Result(Option<IdlTy>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdlParam {
    pub name: String,
    pub ty: IdlTy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdlFn {
    pub name: String,
    pub params: Vec<IdlParam>,
    pub ret: IdlRet,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdlCallback {
    pub name: String,
    pub params: Vec<IdlParam>,
    /// `None` = void.
    pub ret: Option<Prim>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Interface {
    /// The type name scripts see (`import { Log } from "host:log"`).
    pub name: String,
    /// Wasm import namespace without version, `host:<name>`.
    pub module: String,
    pub version: u32,
    pub handles: Vec<String>,
    pub fns: Vec<IdlFn>,
    pub callbacks: Vec<IdlCallback>,
}

/// A plain Wasm import produced from an IDL function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmImport {
    /// `host:log@1`
    pub module: String,
    pub name: String,
    pub params: Vec<WasmTy>,
    pub results: Vec<WasmTy>,
}

/// A callback trampoline export produced from an IDL callback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WasmCallbackExport {
    pub name: String,
    pub params: Vec<WasmTy>,
    pub results: Vec<WasmTy>,
}

impl Interface {
    /// `host:log@1`
    pub fn import_module(&self) -> String {
        format!("{}@{}", self.module, self.version)
    }

    pub fn wasm_imports(&self) -> Vec<WasmImport> {
        self.fns
            .iter()
            .map(|f| {
                let (params, results) = flatten_fn(f);
                WasmImport {
                    module: self.import_module(),
                    name: f.name.clone(),
                    params,
                    results,
                }
            })
            .collect()
    }

    pub fn wasm_callback_exports(&self) -> Vec<WasmCallbackExport> {
        self.callbacks
            .iter()
            .map(|c| {
                let mut params = vec![WasmTy::I32];
                for p in &c.params {
                    flatten_ty(&p.ty, &mut params);
                }
                WasmCallbackExport {
                    name: crate::exports::callback_export_name(
                        &self.module,
                        self.version,
                        &self.name,
                        &c.name,
                    ),
                    params,
                    results: c.ret.iter().map(|p| p.wasm()).collect(),
                }
            })
            .collect()
    }

    pub fn find_fn(&self, name: &str) -> Option<&IdlFn> {
        self.fns.iter().find(|f| f.name == name)
    }
}

fn flatten_ty(t: &IdlTy, out: &mut Vec<WasmTy>) {
    match t {
        IdlTy::Prim(p) => out.push(p.wasm()),
        IdlTy::Str | IdlTy::Bytes => {
            out.push(WasmTy::I32);
            out.push(WasmTy::I32);
        }
        IdlTy::Handle(_) | IdlTy::Callback(_) => out.push(WasmTy::I32),
    }
}

fn flatten_fn(f: &IdlFn) -> (Vec<WasmTy>, Vec<WasmTy>) {
    let mut params = vec![WasmTy::I32]; // implicit capability handle
    for p in &f.params {
        flatten_ty(&p.ty, &mut params);
    }
    let results = match &f.ret {
        IdlRet::Void => vec![],
        IdlRet::Value(t) => {
            let mut v = vec![];
            flatten_ty(t, &mut v);
            v
        }
        IdlRet::Result(None) => vec![WasmTy::I32],
        IdlRet::Result(Some(IdlTy::Str | IdlTy::Bytes)) => {
            params.push(WasmTy::I32); // out_ptr
            params.push(WasmTy::I32); // out_cap
            vec![WasmTy::I32, WasmTy::I32]
        }
        IdlRet::Result(Some(t)) => {
            let mut v = vec![WasmTy::I32];
            flatten_ty(t, &mut v);
            v
        }
    };
    (params, results)
}

// ---- errors -------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdlError {
    pub line: u32,
    pub col: u32,
    pub msg: String,
}

impl fmt::Display for IdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "idl:{}:{}: {}", self.line, self.col, self.msg)
    }
}

impl std::error::Error for IdlError {}

// ---- lexer --------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Int(u64),
    Str(String),
    Sym(&'static str),
    Eof,
}

struct Lexed {
    tok: Tok,
    line: u32,
    col: u32,
}

fn lex(src: &str) -> Result<Vec<Lexed>, IdlError> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let (mut i, mut line, mut col) = (0usize, 1u32, 1u32);
    let err = |line, col, msg: &str| IdlError {
        line,
        col,
        msg: msg.to_string(),
    };
    while i < chars.len() {
        let c = chars[i];
        let (sl, sc) = (line, col);
        let adv = |i: &mut usize, line: &mut u32, col: &mut u32, c: char| {
            *i += 1;
            if c == '\n' {
                *line += 1;
                *col = 1;
            } else {
                *col += 1;
            }
        };
        if c.is_whitespace() {
            adv(&mut i, &mut line, &mut col, c);
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                let ch = chars[i];
                adv(&mut i, &mut line, &mut col, ch);
            }
        } else if c.is_ascii_alphabetic() || c == '_' {
            let mut s = String::new();
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                s.push(chars[i]);
                let ch = chars[i];
                adv(&mut i, &mut line, &mut col, ch);
            }
            out.push(Lexed {
                tok: Tok::Ident(s),
                line: sl,
                col: sc,
            });
        } else if c.is_ascii_digit() {
            let mut n: u64 = 0;
            while i < chars.len() && chars[i].is_ascii_digit() {
                n = n
                    .checked_mul(10)
                    .and_then(|n| n.checked_add(chars[i] as u64 - '0' as u64))
                    .ok_or_else(|| err(sl, sc, "integer too large"))?;
                let ch = chars[i];
                adv(&mut i, &mut line, &mut col, ch);
            }
            out.push(Lexed {
                tok: Tok::Int(n),
                line: sl,
                col: sc,
            });
        } else if c == '"' {
            adv(&mut i, &mut line, &mut col, c);
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None | Some('\n') => return Err(err(sl, sc, "unterminated string")),
                    Some('"') => {
                        adv(&mut i, &mut line, &mut col, '"');
                        break;
                    }
                    Some(&ch) => {
                        s.push(ch);
                        adv(&mut i, &mut line, &mut col, ch);
                    }
                }
            }
            out.push(Lexed {
                tok: Tok::Str(s),
                line: sl,
                col: sc,
            });
        } else if c == '-' && chars.get(i + 1) == Some(&'>') {
            adv(&mut i, &mut line, &mut col, '-');
            adv(&mut i, &mut line, &mut col, '>');
            out.push(Lexed {
                tok: Tok::Sym("->"),
                line: sl,
                col: sc,
            });
        } else {
            let sym = match c {
                '{' => "{",
                '}' => "}",
                '(' => "(",
                ')' => ")",
                ';' => ";",
                ':' => ":",
                ',' => ",",
                '@' => "@",
                '<' => "<",
                '>' => ">",
                _ => return Err(err(sl, sc, &format!("unexpected character `{c}`"))),
            };
            adv(&mut i, &mut line, &mut col, c);
            out.push(Lexed {
                tok: Tok::Sym(sym),
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

// ---- parser -------------------------------------------------------------

fn prim_of(name: &str) -> Option<Prim> {
    Some(match name {
        "bool" => Prim::Bool,
        "i8" => Prim::I8,
        "u8" => Prim::U8,
        "i16" => Prim::I16,
        "u16" => Prim::U16,
        "i32" => Prim::I32,
        "u32" => Prim::U32,
        "i64" => Prim::I64,
        "u64" => Prim::U64,
        "f32" => Prim::F32,
        "f64" => Prim::F64,
        _ => return None,
    })
}

struct P {
    toks: Vec<Lexed>,
    pos: usize,
}

impl P {
    fn peek(&self) -> &Tok {
        &self.toks[self.pos].tok
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, IdlError> {
        let l = &self.toks[self.pos];
        Err(IdlError {
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
    fn sym(&mut self, s: &'static str) -> Result<(), IdlError> {
        if *self.peek() == Tok::Sym(s) {
            self.bump();
            Ok(())
        } else {
            self.err(format!("expected `{s}`, found {}", show(self.peek())))
        }
    }
    fn eat_sym(&mut self, s: &'static str) -> bool {
        if *self.peek() == Tok::Sym(s) {
            self.bump();
            true
        } else {
            false
        }
    }
    fn ident(&mut self) -> Result<String, IdlError> {
        match self.peek().clone() {
            Tok::Ident(s) => {
                self.bump();
                Ok(s)
            }
            t => self.err(format!("expected an identifier, found {}", show(&t))),
        }
    }
    fn kw(&mut self, k: &str) -> Result<(), IdlError> {
        match self.peek() {
            Tok::Ident(s) if s == k => {
                self.bump();
                Ok(())
            }
            t => self.err(format!("expected `{k}`, found {}", show(t))),
        }
    }
}

fn show(t: &Tok) -> String {
    match t {
        Tok::Ident(s) => format!("`{s}`"),
        Tok::Int(n) => format!("`{n}`"),
        Tok::Str(s) => format!("\"{s}\""),
        Tok::Sym(s) => format!("`{s}`"),
        Tok::Eof => "end of file".into(),
    }
}

/// Parse an IDL file into interfaces, then validate it.
pub fn parse(src: &str) -> Result<Vec<Interface>, IdlError> {
    let mut p = P {
        toks: lex(src)?,
        pos: 0,
    };
    let mut out = Vec::new();
    while *p.peek() != Tok::Eof {
        out.push(parse_interface(&mut p)?);
    }
    for i in &out {
        validate(i).map_err(|msg| IdlError {
            line: 0,
            col: 0,
            msg,
        })?;
    }
    let mut seen = std::collections::HashSet::new();
    for i in &out {
        if !seen.insert((i.module.clone(), i.version)) {
            return Err(IdlError {
                line: 0,
                col: 0,
                msg: format!("duplicate module {}@{}", i.module, i.version),
            });
        }
    }
    Ok(out)
}

fn parse_interface(p: &mut P) -> Result<Interface, IdlError> {
    p.kw("interface")?;
    let name = p.ident()?;
    let module = match p.bump() {
        Tok::Str(s) => s,
        t => {
            return p.err(format!(
                "expected the module name string, found {}",
                show(&t)
            ))
        }
    };
    if !module.starts_with(crate::exports::HOST_NAMESPACE_PREFIX)
        || module.len() <= 5
        || module.contains('@')
    {
        return p.err(format!(
            "module name `{module}` must look like `host:<name>` (no version)"
        ));
    }
    p.sym("@")?;
    let version = match p.bump() {
        Tok::Int(n) if n >= 1 => match u32::try_from(n) {
            Ok(v) => v,
            Err(_) => return p.err("version too large"),
        },
        t => {
            return p.err(format!(
                "expected a version number >= 1, found {}",
                show(&t)
            ))
        }
    };
    p.sym("{")?;
    let mut itf = Interface {
        name,
        module,
        version,
        handles: vec![],
        fns: vec![],
        callbacks: vec![],
    };
    while !p.eat_sym("}") {
        match p.peek().clone() {
            Tok::Ident(k) if k == "handle" => {
                p.bump();
                itf.handles.push(p.ident()?);
                p.sym(";")?;
            }
            Tok::Ident(k) if k == "fn" => {
                p.bump();
                let name = p.ident()?;
                let params = parse_params(p, &itf)?;
                let ret = if p.eat_sym("->") {
                    parse_ret(p, &itf)?
                } else {
                    IdlRet::Void
                };
                p.sym(";")?;
                itf.fns.push(IdlFn { name, params, ret });
            }
            Tok::Ident(k) if k == "callback" => {
                p.bump();
                let name = p.ident()?;
                let params = parse_params(p, &itf)?;
                let ret = if p.eat_sym("->") {
                    let id = p.ident()?;
                    match prim_of(&id) {
                        Some(pr) => Some(pr),
                        None => return p.err("callback results must be a primitive"),
                    }
                } else {
                    None
                };
                p.sym(";")?;
                itf.callbacks.push(IdlCallback { name, params, ret });
            }
            t => {
                return p.err(format!(
                    "expected `handle`, `fn`, `callback` or `}}`, found {}",
                    show(&t)
                ))
            }
        }
    }
    Ok(itf)
}

fn parse_params(p: &mut P, itf: &Interface) -> Result<Vec<IdlParam>, IdlError> {
    p.sym("(")?;
    let mut out = Vec::new();
    while !p.eat_sym(")") {
        let name = p.ident()?;
        p.sym(":")?;
        let ty = parse_ty(p, itf)?;
        out.push(IdlParam { name, ty });
        if !p.eat_sym(",") {
            p.sym(")")?;
            break;
        }
    }
    Ok(out)
}

fn parse_ty(p: &mut P, itf: &Interface) -> Result<IdlTy, IdlError> {
    let id = p.ident()?;
    if let Some(pr) = prim_of(&id) {
        return Ok(IdlTy::Prim(pr));
    }
    match id.as_str() {
        "str" => Ok(IdlTy::Str),
        "bytes" => Ok(IdlTy::Bytes),
        _ => {
            if itf.handles.contains(&id) {
                Ok(IdlTy::Handle(id))
            } else if itf.callbacks.iter().any(|c| c.name == id) {
                Ok(IdlTy::Callback(id))
            } else {
                p.err(format!(
                    "unknown type `{id}` (handles and callbacks must be declared before use)"
                ))
            }
        }
    }
}

fn parse_ret(p: &mut P, itf: &Interface) -> Result<IdlRet, IdlError> {
    if matches!(p.peek(), Tok::Ident(s) if s == "result") {
        p.bump();
        p.sym("<")?;
        let r = if matches!(p.peek(), Tok::Ident(s) if s == "void") {
            p.bump();
            IdlRet::Result(None)
        } else {
            let t = parse_ty(p, itf)?;
            if matches!(t, IdlTy::Callback(_)) {
                return p.err("a function cannot return a callback");
            }
            IdlRet::Result(Some(t))
        };
        p.sym(">")?;
        return Ok(r);
    }
    let t = parse_ty(p, itf)?;
    match t {
        IdlTy::Str | IdlTy::Bytes => p.err("`str`/`bytes` results must be written `result<str>`/`result<bytes>` (they use an out buffer)"),
        IdlTy::Callback(_) => p.err("a function cannot return a callback"),
        t => Ok(IdlRet::Value(t)),
    }
}

fn validate(i: &Interface) -> Result<(), String> {
    let mut names = std::collections::HashSet::new();
    for n in i
        .handles
        .iter()
        .map(String::as_str)
        .chain(i.fns.iter().map(|f| f.name.as_str()))
        .chain(i.callbacks.iter().map(|c| c.name.as_str()))
    {
        if !names.insert(n) {
            return Err(format!("duplicate name `{n}` in interface {}", i.name));
        }
    }
    for n in i.handles.iter().chain(i.callbacks.iter().map(|c| &c.name)) {
        if prim_of(n).is_some()
            || matches!(
                n.as_str(),
                "str" | "bytes" | "result" | "void" | "fn" | "handle" | "callback" | "interface"
            )
        {
            return Err(format!(
                "`{n}` is a reserved IDL word and cannot name a handle or callback"
            ));
        }
    }
    let params_of = i
        .fns
        .iter()
        .map(|f| (f.name.as_str(), &f.params))
        .chain(i.callbacks.iter().map(|c| (c.name.as_str(), &c.params)));
    for (fname, params) in params_of {
        let mut ps = std::collections::HashSet::new();
        for p in params {
            if !ps.insert(&p.name) {
                return Err(format!(
                    "duplicate parameter `{}` in {}.{}",
                    p.name, i.name, fname
                ));
            }
        }
    }
    // Host to guest data arrives through the single scratch region, so a
    // callback has at most one str/bytes parameter (at most HOST_PAYLOAD_MAX
    // bytes, delivered at oxi_scratch_ptr). See abi.md 5.4.
    for c in &i.callbacks {
        let blobs = c
            .params
            .iter()
            .filter(|p| matches!(p.ty, IdlTy::Str | IdlTy::Bytes))
            .count();
        if blobs > 1 {
            return Err(format!(
                "callback {}.{} has {blobs} str/bytes parameters; at most one is allowed (single scratch region)",
                i.name, c.name
            ));
        }
        if c.params.iter().any(|p| matches!(p.ty, IdlTy::Callback(_))) {
            return Err(format!(
                "callback {}.{} cannot take a callback parameter",
                i.name, c.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use WasmTy::{F64, I32, I64};
    const LOG: &str = EXAMPLE_LOG_IDL;
    const STATE: &str = EXAMPLE_STATE_IDL;

    #[test]
    fn log_example_maps_to_plain_imports() {
        let ifs = parse(LOG).unwrap();
        assert_eq!(ifs.len(), 1);
        let log = &ifs[0];
        assert_eq!(log.import_module(), "host:log@1");
        let imports = log.wasm_imports();
        // write(self, level: u8, msg: str)
        assert_eq!(imports[0].name, "write");
        assert_eq!(imports[0].params, vec![I32, I32, I32, I32]);
        assert!(imports[0].results.is_empty());
    }

    #[test]
    fn state_example_covers_results_handles_callbacks() {
        let ifs = parse(STATE).unwrap();
        let st = &ifs[0];
        assert_eq!(st.import_module(), "host:state@1");
        let by = |n: &str| st.wasm_imports().into_iter().find(|i| i.name == n).unwrap();
        // open(self, key: str) -> result<Slot>  => (i32,i32,i32) -> (i32 status, i32 handle)
        assert_eq!(by("open").params, vec![I32, I32, I32]);
        assert_eq!(by("open").results, vec![I32, I32]);
        // get_i64(self, slot) -> result<i64> => (i32,i32) -> (i32, i64)
        assert_eq!(by("get_i64").results, vec![I32, I64]);
        // read_str(self, slot) -> result<str> => (self, slot, out_ptr, out_cap) -> (status, len)
        assert_eq!(by("read_str").params, vec![I32, I32, I32, I32]);
        assert_eq!(by("read_str").results, vec![I32, I32]);
        // set_f64(self, slot, v: f64) -> result<void>
        assert_eq!(by("set_f64").params, vec![I32, I32, F64]);
        assert_eq!(by("set_f64").results, vec![I32]);
        // watch(self, slot, cb) : callback = i32
        assert_eq!(by("watch").params, vec![I32, I32, I32]);
        let cbs = st.wasm_callback_exports();
        assert_eq!(cbs[0].name, "cb:host:state@1.State.OnChange");
        assert_eq!(cbs[0].params, vec![I32, I32, I32, I32]); // closure, slot, key (ptr, len)
    }

    #[test]
    fn errors_have_positions() {
        let e = parse("interface X \"host:x\" @1 { fn f(a: nope); }").unwrap_err();
        assert!(e.msg.contains("unknown type"), "{e}");
        assert_eq!(e.line, 1);
        assert!(parse("interface X \"x\" @1 {}").is_err());
        assert!(parse("interface X \"host:x\" @0 {}").is_err());
        assert!(parse("interface X \"host:x\" @1 { fn f() -> str; }").is_err());
        assert!(parse("interface X \"host:x\" @1 { fn f(); fn f(); }").is_err());
        assert!(parse("interface X \"host:x\" @1 { fn f() }").is_err());
        assert!(parse("interface X \"host:x\" @1 { fn f(a: i32, a: i32); }").is_err());
        assert!(parse("interface X \"host:x\" @1 { } interface Y \"host:x\" @1 { }").is_err());
    }

    #[test]
    fn empty_and_comments() {
        assert_eq!(parse("// nothing\n").unwrap(), vec![]);
    }
}
