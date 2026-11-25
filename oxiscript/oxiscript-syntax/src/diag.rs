//! Diagnostics produced by the lexer, parser and formatter.
//!
//! Every diagnostic carries a stable reason code from
//! `oxiscript-abi/registry/codes.txt` (this crate has no dependency on the abi
//! crate; `tests/codes_registered.rs` checks every code used here exists in the
//! registry), a span, a ready-to-print message and zero or more fixes that an
//! editor (or an AI agent) can apply mechanically.

use crate::ast::{FileId, Span};
use std::fmt;

/// Replace the bytes of `span` by `replacement` (an insertion has an empty span).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub span: Span,
    pub replacement: String,
}

/// One mechanical repair: a title for humans and the edits to apply together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub title: String,
    pub edits: Vec<TextEdit>,
}

impl Fix {
    pub fn insert(title: impl Into<String>, at: Span, text: impl Into<String>) -> Fix {
        Fix {
            title: title.into(),
            edits: vec![TextEdit {
                span: Span::new(at.file, at.lo, at.lo),
                replacement: text.into(),
            }],
        }
    }
    pub fn replace(title: impl Into<String>, span: Span, text: impl Into<String>) -> Fix {
        Fix {
            title: title.into(),
            edits: vec![TextEdit {
                span,
                replacement: text.into(),
            }],
        }
    }
    /// Wrap `span` in parentheses.
    pub fn parenthesise(title: impl Into<String>, span: Span) -> Fix {
        Fix {
            title: title.into(),
            edits: vec![
                TextEdit {
                    span: Span::new(span.file, span.lo, span.lo),
                    replacement: "(".into(),
                },
                TextEdit {
                    span: Span::new(span.file, span.hi, span.hi),
                    replacement: ")".into(),
                },
            ],
        }
    }
}

/// Most diagnostics kept per file (lexer and parser share this limit); the
/// last one is `OXS-E0017`.
pub const MAX_DIAGS: usize = 200;

/// A syntax diagnostic (an error, or a warning such as `OXS-W0003`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Stable reason code, e.g. `OXS-E0004`.
    pub code: &'static str,
    pub span: Span,
    pub message: String,
    pub fixes: Vec<Fix>,
}

impl Diagnostic {
    /// `true` for error codes (`OXS-E....`), `false` for warnings.
    pub fn is_error(&self) -> bool {
        self.code.as_bytes().get(4) == Some(&b'E')
    }
    pub fn new(code: &'static str, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            span,
            message: message.into(),
            fixes: Vec::new(),
        }
    }
    #[must_use]
    pub fn with_fix(mut self, fix: Fix) -> Diagnostic {
        self.fixes.push(fix);
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} (at {}..{})",
            self.code, self.message, self.span.lo, self.span.hi
        )
    }
}

/// Apply `edits` to `src` (edits must not overlap). Returns `None` when an edit
/// is out of range or not on a character boundary. Used by tests and by tools
/// that auto-fix.
pub fn apply_edits(src: &str, edits: &[TextEdit]) -> Option<String> {
    let mut sorted: Vec<&TextEdit> = edits.iter().collect();
    sorted.sort_by_key(|e| (e.span.lo, e.span.hi));
    let mut out = String::with_capacity(src.len() + 16);
    let mut at = 0usize;
    for e in sorted {
        let (lo, hi) = (e.span.lo as usize, e.span.hi as usize);
        if lo < at
            || hi < lo
            || hi > src.len()
            || !src.is_char_boundary(lo)
            || !src.is_char_boundary(hi)
        {
            return None;
        }
        out.push_str(&src[at..lo]);
        out.push_str(&e.replacement);
        at = hi;
    }
    out.push_str(&src[at..]);
    Some(out)
}

/// Byte offset to 1-based line/column (column counted in characters).
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<u32>,
}

impl LineIndex {
    pub fn new(src: &str) -> LineIndex {
        let mut starts = vec![0u32];
        let bytes = src.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            let lone_cr = b == b'\r' && bytes.get(i + 1) != Some(&b'\n');
            if b == b'\n' || lone_cr {
                starts.push(u32::try_from(i + 1).unwrap_or(u32::MAX));
            }
        }
        LineIndex { starts }
    }
    /// 1-based line of a byte offset.
    pub fn line(&self, offset: u32) -> u32 {
        let idx = self.starts.partition_point(|&s| s <= offset);
        u32::try_from(idx).unwrap_or(u32::MAX).max(1)
    }
    /// 1-based line and column.
    pub fn line_col(&self, src: &str, offset: u32) -> (u32, u32) {
        let line = self.line(offset);
        let start = self.starts[(line - 1) as usize] as usize;
        let end = (offset as usize).min(src.len());
        let col = src.get(start..end).map_or(0, |s| s.chars().count());
        (line, u32::try_from(col + 1).unwrap_or(u32::MAX))
    }
}

pub(crate) fn mk_span(file: FileId, lo: usize, hi: usize) -> Span {
    Span {
        file,
        lo: u32::try_from(lo).unwrap_or(u32::MAX),
        hi: u32::try_from(hi).unwrap_or(u32::MAX),
    }
}
