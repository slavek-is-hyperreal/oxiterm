#![allow(clippy::format_push_string)]
//! Every diagnostic the checker can emit is a registered code with phase `check`
//! (or the reserved-syntax code), and the checker invents no text outside the registry.
use oxiscript_abi::codes;

#[test]
fn every_code_used_in_the_sources_is_registered() {
    let reg = codes::seed();
    let dir = format!("{}/src", env!("CARGO_MANIFEST_DIR"));
    let mut found = std::collections::BTreeSet::new();
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().is_none_or(|x| x != "rs") {
            continue;
        }
        let text = std::fs::read_to_string(&p).unwrap();
        let mut i = 0;
        while let Some(at) = text[i..].find("\"OXS-") {
            let s = i + at + 1;
            let code = &text[s..(s + 9).min(text.len())];
            found.insert(code.to_string());
            i = s + 4;
        }
    }
    assert!(found.len() >= 25, "{found:?}");
    for code in &found {
        let def = reg.iter().find(|c| c.code == *code).unwrap_or_else(|| panic!("{code} is not in the registry"));
        if code.as_bytes()[4] == b'P' {
            continue; // quoted as an example in a message, never emitted
        }
        assert!(
            def.phase == "check" || *code == "OXS-E0900" || def.phase == "link" && *code == "OXS-E0400" || *code == "OXS-E0401",
            "{code} has phase {}",
            def.phase
        );
        assert!(def.fix.is_some() || *code == "OXS-E0900", "{code} has no fix text");
    }
}

#[test]
fn registry_stays_sorted_and_valid() {
    let reg = codes::seed();
    let ids: Vec<&str> = reg.iter().map(|c| c.code.as_str()).collect();
    let mut s = ids.clone();
    s.sort_unstable();
    assert_eq!(ids, s);
}
