//! Extra guidance for the mistakes a TypeScript developer (or a model that
//! writes TypeScript) makes most often, where the checker's own diagnostic is
//! generic. The checker is untouched: these hints are added to the diagnostic
//! after the fact, from the code, the message and the source text under the
//! span. They never change a code or a position.

use crate::project::SourceFile;
use crate::report::{Diag, EditOut, FixOut, ADVICE, MAYBE_INCORRECT};

fn advice(title: &str) -> FixOut {
    FixOut {
        title: title.to_string(),
        applicability: ADVICE,
        edits: Vec::new(),
    }
}

fn replace(d: &Diag, title: &str, new_text: &str) -> FixOut {
    FixOut {
        title: title.to_string(),
        applicability: MAYBE_INCORRECT,
        edits: vec![EditOut {
            file: d.file.clone(),
            range: d.range.clone(),
            new_text: new_text.to_string(),
        }],
    }
}

/// Text of `d`'s line before the span start.
fn line_before<'a>(f: &'a SourceFile, d: &Diag) -> &'a str {
    let line = f.line_text(d.range.start.line);
    let cut = (d.range.start.col as usize).saturating_sub(1);
    let end = line.char_indices().nth(cut).map_or(line.len(), |(i, _)| i);
    line[..end].trim_start()
}

pub fn apply(d: &mut Diag, f: &SourceFile) {
    let lo = d.range.start.byte as usize;
    let hi = (d.range.end.byte as usize).min(f.src.len());
    let span_text = f.src.get(lo..hi).unwrap_or("");
    match d.code.as_str() {
        "OXS-E0300" => match span_text {
            "number" => {
                d.fixes = vec![
                    replace(d, "OxiScript has no `number`: use `i32` for whole numbers", "i32"),
                    replace(d, "OxiScript has no `number`: use `f64` for fractions", "f64"),
                ];
            }
            "console" => d.fixes = vec![advice("There is no `console`: take a `log: Log` parameter in `main` (`import { Log } from \"host:log\";`) and call `log.write(2, \"text\")`.")],
            "Math" => d.fixes = vec![advice("There is no `Math` global in this edition: write the arithmetic with `f64` operators.")],
            "undefined" | "null" => {}
            _ => {}
        },
        "OXS-E0108" if span_text == "length" && d.message.contains("type `string`") => {
            d.fixes = vec![
                replace(d, "A string has a byte length in UTF-8: use `.byteLength`", "byteLength"),
                advice("For the number of characters use `s.chars().length`."),
            ];
        }
        "OXS-E0004" => {
            // `throw`, `try`, `catch`, `finally` anywhere before the error on its line (`} catch (e) {`,
            // `if (x) throw "y";`): the `;` fix would only paper over a construct that does not exist.
            let before = line_before(f, d);
            let exceptions = before
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .any(|w| matches!(w, "throw" | "try" | "catch" | "finally"));
            if exceptions {
                for fx in &mut d.fixes {
                    fx.applicability = MAYBE_INCORRECT;
                }
                d.fixes.push(advice("OxiScript has no exceptions (`throw`, `try`, `catch`): return `Err(...)` from a function that returns `Result<T, E>` and propagate with `?`."));
            }
        }
        "OXS-E0110" if d.message.contains("operator `+`") && d.message.contains("`string`") => {
            d.fixes = vec![advice("`+` does not join strings and numbers: use a template literal, for example `n=${a}`.")];
        }
        "OXS-E0128" if d.message.contains("type `string`") => {
            d.fixes = vec![advice("A string has no O(1) character index: use `s.bytes()[i]` for a byte or `s.chars()[i]` for a code point.")];
        }
        _ => {}
    }
}
