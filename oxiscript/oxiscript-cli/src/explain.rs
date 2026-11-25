//! `oxi explain <code>`: meaning, message template, fix hint and (for the
//! common codes) a failing example with its fixed form. Source of truth for
//! everything but the examples is the registry (`oxiscript-abi/registry/codes.txt`).

use oxiscript_abi::codes::{self, CodeDef, Severity};

use crate::examples::EXAMPLES;

fn phase_note(phase: &str) -> &'static str {
    match phase {
        "lex" | "parse" => "Reported by the lexer/parser: `oxi check` and `oxi fmt` stop at syntax errors, and type checking is skipped until they are fixed.",
        "check" => "Reported by the type checker (`oxi check`).",
        "lower" | "link" | "runtime" => "Reported when a program is compiled or run (`oxi build` / `oxi run`, after 8.L), not by `oxi check`.",
        "place" => "A placement/performance note from the compiler (8.S4); `@expect` can assert some of them.",
        "law" => "A law outcome (`oxi test`, 8.S4).",
        _ => "",
    }
}

pub fn render(d: &CodeDef) -> String {
    let sev = match d.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Note => "note",
        Severity::Internal => "internal error",
    };
    let mut s = format!("{}: {}\n{} (phase: {})\n\nMessage template:\n  {}\n", d.code, d.title, sev, d.phase, d.message);
    if let Some(f) = &d.fix {
        s.push_str(&format!("\nHow to fix:\n  {f}\n"));
    }
    if let Some((_, bad, good)) = EXAMPLES.iter().find(|(c, _, _)| *c == d.code) {
        s.push_str("\nExample that fails:\n");
        for l in bad.lines() {
            s.push_str(&format!("  {l}\n"));
        }
        s.push_str("\nFixed:\n");
        for l in good.lines() {
            s.push_str(&format!("  {l}\n"));
        }
    } else {
        let note = phase_note(&d.phase);
        if !note.is_empty() {
            s.push_str(&format!("\n{note}\n"));
        }
    }
    s.push_str("\nSpec: docs/oxiscript/grammar.md (section 6 lists the differences from TypeScript).\n");
    s
}

pub fn run(code: &str) -> i32 {
    let want = code.trim().to_ascii_uppercase();
    let want = if want.starts_with("OXS-") { want } else { format!("OXS-{want}") };
    match codes::lookup(&want) {
        Some(d) => {
            print!("{}", render(&d));
            0
        }
        None => {
            eprintln!("error: `{code}` is not a known code (expected e.g. OXS-E0102; the full list is oxiscript-abi/registry/codes.txt)");
            2
        }
    }
}
