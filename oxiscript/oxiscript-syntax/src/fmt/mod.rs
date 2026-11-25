//! The canonical formatter: 2-space indent, TypeScript-like layout, line width
//! 100, comments preserved. `format(format(x)) == format(x)`, and for every
//! valid `x` the output parses to the same AST (modulo spans, and modulo the
//! braces added around single-statement bodies; see [`FormatOptions::braces`]).
//!
//! The layout is a pure function of the AST, the comments and the blank lines
//! between statements (at most one is kept); nothing else of the original
//! layout survives, which is what makes the result canonical.

use crate::ast::*;
use crate::diag::Diagnostic;
use crate::lexer::CommentKind;
use crate::parser::{parse_with, ParseOptions};
use std::fmt;

mod build;
mod doc;

/// Formatting options. The default is the canonical style.
#[derive(Clone, Debug)]
pub struct FormatOptions {
    /// Maximum line width (default 100).
    pub width: usize,
    /// Wrap single-statement bodies of `if`/`while`/`for` in braces (default
    /// `true`, the canonical style). With `false` the statement structure of
    /// the input is kept exactly, so `parse(format(x))` equals `parse(x)`
    /// modulo spans; used by the round-trip tests.
    pub braces: bool,
    /// Options for the parse step (nesting limit, helper thread).
    pub parse: ParseOptions,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            width: 100,
            braces: true,
            parse: ParseOptions::default(),
        }
    }
}

/// The source has syntax errors and cannot be formatted.
#[derive(Debug)]
pub struct FormatError {
    pub diagnostics: Vec<Diagnostic>,
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "cannot format: {} syntax error(s), first: {}",
            self.diagnostics.len(),
            self.diagnostics
                .first()
                .map_or_else(String::new, ToString::to_string)
        )
    }
}

impl std::error::Error for FormatError {}

/// Format OxiScript source in the canonical style.
pub fn format(src: &str) -> Result<String, FormatError> {
    format_with(src, &FormatOptions::default())
}

/// Format with explicit options. Fails only if the source has syntax *errors*
/// (warnings, such as invisible characters in literals, do not block formatting).
pub fn format_with(src: &str, opts: &FormatOptions) -> Result<String, FormatError> {
    let parsed = parse_with(src, FileId(0), &opts.parse);
    if parsed.diagnostics.iter().any(Diagnostic::is_error) {
        return Err(FormatError {
            diagnostics: parsed.diagnostics,
        });
    }
    let run = || {
        let mut f = build::Builder::new(src, &parsed.comments, opts);
        let d = f.module(&parsed.module);
        let mut out = doc::render(&d, opts.width);
        while out.ends_with('\n') || out.ends_with(' ') {
            out.pop();
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out
    };
    // Optional headroom: deeply nested input is formatted on a helper thread
    // with a large stack when one can be created. The result never depends on
    // it: the nesting limit applies to the parse, and chains are iterative.
    if opts.parse.use_thread && parsed.max_nest > 6 {
        let spawned = std::thread::scope(|s| {
            std::thread::Builder::new()
                .stack_size(128 << 20)
                .spawn_scoped(s, run)
                .map(std::thread::ScopedJoinHandle::join)
        });
        match spawned {
            Ok(Ok(out)) => return Ok(out),
            Ok(Err(payload)) => std::panic::resume_unwind(payload),
            Err(_) => {}
        }
    }
    Ok(run())
}

pub(crate) fn comment_kind_is_line(k: CommentKind) -> bool {
    !matches!(k, CommentKind::Block)
}
