//! OxiScript syntax: AST, visitor, keyword lists (slice 8.C) and the lexer,
//! parser and formatter (slice 8.P). Grammar: `docs/oxiscript/grammar.md`.
//!
//! ```
//! let (module, diagnostics) =
//!     oxiscript_syntax::parse_module("export function f(): i32 { return 1; }");
//! assert!(diagnostics.is_empty());
//! assert_eq!(module.items.len(), 1);
//! assert_eq!(oxiscript_syntax::format("let   x=1 ;").unwrap(), "let x = 1;\n");
//! ```
//!
//! # Stack use and limits
//!
//! Parsing, formatting and dropping never need a thread or a large stack for
//! correctness. The nesting limit is [`ParseOptions::max_depth`] (default
//! [`DEFAULT_MAX_DEPTH`] = 56 depth units; a parenthesis, call argument, array or
//! type level costs 1 unit, a statement or declaration level 2); deeper input
//! is rejected with `OXS-E0011`, the same on every platform. Measured native
//! release stack of the inline parser (`examples/stack_probe`): about 3.2 KB per
//! parenthesis/call/bracket level, 3.5 KB per arrow, 3.6 KB per switch-expression
//! level, 1.6 KB per depth unit of blocks, 0.9 KB per generic type level, so the
//! default limit needs at most about 235 KB and fits a 256 KB stack and the
//! default wasm32 stack. Debug builds use about six times more. Long chains
//! (`a + b + c`, `a.b().c`, `x ? y : z ? ...`, `else if` ladders, up to
//! [`MAX_HEIGHT`] links) are parsed, formatted and dropped iteratively and
//! cost constant stack. When a thread with a large stack can be created
//! (native targets, `use_thread`), nested or large inputs use it as extra
//! headroom; this never changes the result. `Expr` and `Stmt` implement `Drop`
//! (iteratively), so their fields cannot be moved out by destructuring.
//!
//! No dependencies; in particular no dependency on `oxiscript-ir` or on any
//! `oxiterm-*` crate.
#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::match_same_arms,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::wildcard_imports,
    clippy::struct_excessive_bools,
    clippy::large_enum_variant,
    clippy::option_option,
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::self_only_used_in_recursion,
    clippy::unused_self,
    clippy::return_self_not_must_use,
    clippy::doc_markdown,
    clippy::needless_lifetimes,
    clippy::manual_let_else,
    clippy::unnecessary_wraps,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::cognitive_complexity,
    clippy::collapsible_else_if,
    clippy::while_let_loop,
    clippy::naive_bytecount,
    clippy::result_large_err
)]

pub mod ast;
pub mod diag;
pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod visit;

mod drop;
mod fmt;

pub use ast::*;
pub use diag::{apply_edits, Diagnostic, Fix, LineIndex, TextEdit, MAX_DIAGS};
pub use fmt::{format, format_with, FormatError, FormatOptions};
pub use lexer::{lex, Comment, CommentKind};
pub use parser::{
    is_error_ident, parse, parse_module, parse_with, ParseOptions, Parsed, DEFAULT_MAX_DEPTH,
    MAX_HEIGHT,
};
pub use visit::*;
