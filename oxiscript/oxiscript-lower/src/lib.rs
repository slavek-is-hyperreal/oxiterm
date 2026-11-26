//! OxiScript lowering (slice 8.L): checked AST -> verified OxiIR, and the
//! convenience pipeline `.oxs` -> check -> lower -> verify -> Wasm.
//!
//! # Conventions (see `docs/oxiscript/contracts.md`, 8.L section)
//!
//! * **RC discipline: own everything.** Every loaded value of mutable state
//!   (field, array element, global, closure environment) is retained at the
//!   load, so a borrow never lives across a call or a store (convention 10).
//!   Immutable locals and borrowed parameters are used as borrowed operands.
//!   Elision is the job of a later pass.
//! * **Locals are SSA values** carried through joins and loops as block
//!   parameters (only the variables assigned inside the construct).
//!   Variables that are captured by a closure *and* mutated live in a
//!   generated `Cell` struct.
//! * **Structs** have value semantics through copy-on-write
//!   (`is_unique`/`clone`); classes are plain references.
//! * Generics are monomorphised on demand (worklist keyed by definition and
//!   concrete type arguments) with a budget.
#![forbid(unsafe_code)]
#![allow(
    clippy::needless_raw_string_hashes,
    clippy::elidable_lifetime_names,
    clippy::assigning_clones,
    clippy::let_and_return,
    clippy::self_only_used_in_recursion,
    clippy::only_used_in_recursion,
    clippy::wrong_self_convention,
    clippy::unnecessary_get_then_check,
    clippy::ref_option,
    clippy::option_option,
    clippy::question_mark,
    clippy::if_same_then_else,
    clippy::too_many_lines,
    clippy::too_many_arguments,
    clippy::many_single_char_names,
    clippy::similar_names,
    clippy::wildcard_imports,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::doc_markdown,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::type_complexity,
    clippy::match_same_arms,
    clippy::manual_let_else,
    clippy::single_match_else,
    clippy::items_after_statements,
    clippy::struct_excessive_bools,
    clippy::unnecessary_wraps,
    clippy::if_not_else,
    clippy::collapsible_else_if,
    clippy::cognitive_complexity,
    clippy::uninlined_format_args,
    clippy::format_push_string,
    clippy::map_unwrap_or,
    clippy::unused_self,
    clippy::large_enum_variant,
    clippy::enum_glob_use,
    clippy::needless_lifetimes,
    clippy::collapsible_if,
    clippy::bool_to_int_with_if,
    clippy::single_match,
    clippy::redundant_closure_for_method_calls,
    clippy::return_self_not_must_use,
    clippy::explicit_iter_loop,
    clippy::manual_assert,
    clippy::semicolon_if_nothing_returned,
    clippy::match_wildcard_for_single_variants,
    clippy::ptr_arg,
    clippy::needless_range_loop,
    clippy::option_if_let_else,
    clippy::implicit_hasher
)]

mod call;
mod ctx;
mod driver;
mod expr;
mod fb;
mod fx;
mod helpers;
mod place;
mod stmt;
mod switch;
mod types;

use oxiscript_syntax::Span;
use std::fmt;

pub use driver::{compile_program, compile_source, compile_source_with_notes, lower, lower_full, lower_with, lower_with_budget, CompileOutput, Lowered, Options};

/// A lowering (or pipeline) diagnostic. `code` is a registry code (`OXS-E07xx` for the
/// lowering itself; check and code generator codes are passed through).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
}

impl Diagnostic {
    pub(crate) fn new(code: &str, message: impl Into<String>, span: Option<Span>) -> Diagnostic {
        Diagnostic {
            code: code.to_string(),
            message: message.into(),
            span,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)?;
        if let Some(s) = self.span {
            write!(f, " (file {}, bytes {}..{})", s.file.0, s.lo, s.hi)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostics(pub Vec<Diagnostic>);

impl fmt::Display for Diagnostics {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, d) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "{d}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Diagnostics {}

/// Maximum number of generic instantiations (functions and types) per program (`OXS-E0701`).
pub const MAX_INSTANCES: usize = 5000;
