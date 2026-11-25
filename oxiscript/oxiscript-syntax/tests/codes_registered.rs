//! Every reason code the syntax crate can emit exists in the registry
//! (`oxiscript-abi/registry/codes.txt`; this crate does not depend on the abi crate).

fn read(rel: &str) -> String {
    std::fs::read_to_string(format!("{}/../../{rel}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

#[test]
fn every_code_in_the_syntax_sources_is_registered() {
    let registry = read("oxiscript/oxiscript-abi/registry/codes.txt");
    let mut found = std::collections::BTreeSet::new();
    for dir in ["src", "src/parser", "src/fmt"] {
        for e in std::fs::read_dir(format!("{}/{dir}", env!("CARGO_MANIFEST_DIR"))).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_none_or(|x| x != "rs") {
                continue;
            }
            let text = std::fs::read_to_string(&p).unwrap();
            let mut i = 0;
            while let Some(at) = text[i..].find("\"OXS-") {
                let s = i + at + 1;
                let code = &text[s..(s + 9).min(text.len())];
                assert!(
                    registry.contains(&format!("code {code}\n")),
                    "{} uses unregistered {code}",
                    p.display()
                );
                found.insert(code.to_string());
                i = s + 4;
            }
        }
    }
    // the syntax errors, the reserved-syntax code and the literal warning
    for c in [
        "OXS-E0001",
        "OXS-E0002",
        "OXS-E0003",
        "OXS-E0004",
        "OXS-E0005",
        "OXS-E0006",
        "OXS-E0007",
        "OXS-E0008",
        "OXS-E0009",
        "OXS-E0010",
        "OXS-E0011",
        "OXS-E0012",
        "OXS-E0013",
        "OXS-E0014",
        "OXS-E0015",
        "OXS-E0016",
        "OXS-E0017",
        "OXS-E0900",
        "OXS-W0003",
    ] {
        assert!(found.contains(c), "{c} is registered but never emitted");
    }
}
