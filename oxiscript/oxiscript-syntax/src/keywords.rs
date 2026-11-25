//! The frozen word lists. Adding a *reserved* word later breaks programs, so
//! the future-reserved list is deliberately generous and fixed now; the
//! lexer (slice 8.P) and the formatter take their word lists from here.

/// Strict keywords: never identifiers.
pub const KEYWORDS: &[&str] = &[
    "as",
    "break",
    "case",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "interface",
    "let",
    "new",
    "null",
    "private",
    "readonly",
    "return",
    "static",
    "struct",
    "switch",
    "this",
    "true",
    "while",
];

/// Contextual keywords: keywords only in a specific position, ordinary
/// identifiers elsewhere (so `from`, `of`, `type`, `law` stay usable names).
pub const CONTEXTUAL: &[&str] = &[
    "constructor",
    "from",
    "law",
    "mutating",
    "of",
    "type",
    "when",
];

/// Reserved for future language features or rejected on purpose (TypeScript
/// words with no meaning here). Using one as an identifier is `OXS-E0900`
/// (with a specific hint where one exists, e.g. `any`, `undefined`, `var`).
/// Never shrink this list; moving a word out of it is a breaking change.
pub const RESERVED: &[&str] = &[
    // future features
    "arena",
    "async",
    "await",
    "dyn",
    "ensures",
    "impl",
    "inout",
    "invariant",
    "isolate",
    "kernel",
    "match",
    "move",
    "parallel",
    "region",
    "requires",
    "spawn",
    "trait",
    "trusted",
    "unsafe",
    "weak",
    "where",
    "yield",
    // TypeScript words that do not exist here
    "any",
    "debugger",
    "declare",
    "delete",
    "instanceof",
    "namespace",
    "protected",
    "public",
    "super",
    "typeof",
    "undefined",
    "unknown",
    "var",
    "with",
];

/// Builtin type names (ordinary identifiers resolved by the checker; listed
/// so the formatter and editor tooling can highlight them).
pub const BUILTIN_TYPES: &[&str] = &[
    "bool", "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "string", "void",
    "never", "Array", "Result",
];

pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&s)
}

pub fn is_reserved(s: &str) -> bool {
    RESERVED.contains(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn lists_are_disjoint_and_sorted_within_groups() {
        let mut all = HashSet::new();
        for w in KEYWORDS.iter().chain(CONTEXTUAL).chain(RESERVED) {
            assert!(all.insert(*w), "`{w}` appears twice");
        }
        for list in [KEYWORDS, CONTEXTUAL] {
            let mut sorted = list.to_vec();
            sorted.sort_unstable();
            assert_eq!(sorted, list);
        }
    }

    #[test]
    fn builtin_types_are_not_keywords() {
        for t in BUILTIN_TYPES {
            assert!(!is_keyword(t) && !is_reserved(t), "{t}");
        }
    }
}
