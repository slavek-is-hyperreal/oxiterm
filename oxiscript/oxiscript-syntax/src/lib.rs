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
    clippy::naive_bytecount
)]

pub mod ast;
pub mod diag;
pub mod keywords;
pub mod lexer;
pub mod parser;
pub mod visit;

mod fmt;

pub use ast::*;
pub use diag::{apply_edits, Diagnostic, Fix, LineIndex, TextEdit};
pub use fmt::{format, format_with, FormatError, FormatOptions};
pub use lexer::{lex, Comment, CommentKind};
pub use parser::{is_error_ident, parse, parse_module, Parsed, MAX_DEPTH, MAX_HEIGHT};
pub use visit::*;
