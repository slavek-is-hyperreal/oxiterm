//! Doc-consistency checks for `docs/oxiscript/*.md` (no parser exists yet, so
//! the grammar document itself is the artefact under test).
#![allow(clippy::doc_markdown)]

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn docs() -> Vec<(String, String)> {
    ["grammar.md", "ir.md", "abi.md", "contracts.md"]
        .iter()
        .map(|n| ((*n).to_string(), read(&format!("docs/oxiscript/{n}"))))
        .collect()
}

/// Every `OXS-<class><4 digits>` mentioned in the docs exists in the registry.
#[test]
fn every_documented_code_is_registered() {
    let registry = read("oxiscript/oxiscript-abi/registry/codes.txt");
    for (name, text) in docs() {
        let b = text.as_bytes();
        let mut i = 0;
        while let Some(p) = text[i..].find("OXS-") {
            let at = i + p;
            let code = &text[at..(at + 9).min(text.len())];
            let well_formed = code.len() == 9
                && matches!(b[at + 4], b'E' | b'W' | b'P' | b'L' | b'I')
                && b[at + 5..at + 9].iter().all(u8::is_ascii_digit);
            if well_formed {
                assert!(
                    registry.contains(&format!("code {code}\n")),
                    "{name}: {code} is not in registry/codes.txt"
                );
            }
            i = at + 4;
        }
    }
}

/// The glued cast spellings were removed (they differed from `as` + operator only by whitespace).
#[test]
fn removed_cast_spellings_do_not_reappear() {
    for (name, text) in docs() {
        for bad in ["as%", "as|", "as?"] {
            assert!(!text.contains(bad), "{name} still mentions `{bad}`");
        }
    }
}

/// Switch-expression arms never use `=>` (arrow-function collision), in the EBNF or the examples.
#[test]
fn switch_arms_do_not_use_fat_arrow() {
    let g = read("docs/oxiscript/grammar.md");
    let arm = g.lines().find(|l| l.starts_with("Arm        =")).unwrap();
    assert!(!arm.contains("=>"), "{arm}");
}

/// Only one whitespace-sensitive rule is claimed, and it is the `?` rule.
#[test]
fn single_whitespace_sensitive_rule() {
    let g = read("docs/oxiscript/grammar.md");
    assert!(g.contains("the `?` rule in section 1.5"));
    assert!(g.contains("**only** whitespace-sensitive rule"));
}

/// The precedence table and the must-parenthesise codes agree with the registry titles' topics.
#[test]
fn must_parenthesise_codes_documented() {
    let g = read("docs/oxiscript/grammar.md");
    for code in ["OXS-E0007", "OXS-E0008", "OXS-E0009", "OXS-E0010"] {
        assert!(
            g.contains(code),
            "{code} missing from the must-parenthesise table"
        );
    }
}

/// The literal rules are disjoint: no float suffix on the integer suffix line, `.5` is a literal.
#[test]
fn literal_rules_are_consistent() {
    let g = read("docs/oxiscript/grammar.md");
    let int_suffix = g.lines().find(|l| l.starts_with("NUMSUFFIX")).unwrap();
    assert!(!int_suffix.contains("f32") && !int_suffix.contains("f64"));
    assert!(g.contains("`.5` is a literal"));
}
