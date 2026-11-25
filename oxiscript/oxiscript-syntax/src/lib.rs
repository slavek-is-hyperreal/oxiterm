//! OxiScript syntax: AST, visitor and the frozen keyword lists (Phase 8,
//! slice 8.C). The lexer, parser and formatter arrive in slice 8.P and build
//! exactly these types. Grammar: `docs/oxiscript/grammar.md`.
//!
//! No dependencies; in particular no dependency on `oxiscript-ir` or on any
//! `oxiterm-*` crate.
#![allow(clippy::doc_markdown)]

pub mod ast;
pub mod keywords;
pub mod visit;

pub use ast::*;
pub use visit::*;
