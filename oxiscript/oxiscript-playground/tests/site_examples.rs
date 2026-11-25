//! CI gate for the website: every `sites/oxiscript/examples/*.oxs` passes `check` (no errors) and is
//! already in canonical format, except the files listed in `MUST_FAIL`, which must produce errors
//! and offer at least one fix. Examples shown on any page come from this directory, so the pages
//! cannot show code the compiler rejects.

use oxiscript_playground::{apply_fix_json, check_json, format_json, FixExpect};
use serde_json::Value;
use std::path::PathBuf;

const MUST_FAIL: &[&str] = &["mistakes.oxs"];

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sites/oxiscript/examples")
}

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

#[test]
fn all_site_examples() {
    let mut n = 0;
    for entry in std::fs::read_dir(dir()).expect("examples dir") {
        let p = entry.unwrap().path();
        if p.extension().and_then(|e| e.to_str()) != Some("oxs") {
            continue;
        }
        n += 1;
        let name = p.file_name().unwrap().to_str().unwrap().to_string();
        let src = std::fs::read_to_string(&p).unwrap();
        let v = parse(&check_json(&src));
        if MUST_FAIL.contains(&name.as_str()) {
            assert_eq!(v["ok"], false, "{name} must have an error");
            let diags = v["diagnostics"].as_array().unwrap();
            let (di, d) = diags
                .iter()
                .enumerate()
                .find(|(_, d)| !d["fixes"].as_array().unwrap().is_empty())
                .unwrap_or_else(|| panic!("{name}: no fix offered"));
            let exp = FixExpect {
                code: d["code"].as_str().unwrap(),
                start: d["start"].as_f64().unwrap(),
                end: d["end"].as_f64().unwrap(),
                title: d["fixes"][0]["title"].as_str().unwrap(),
            };
            let fixed = parse(&apply_fix_json(&src, di as f64, 0.0, &exp));
            assert_eq!(fixed["ok"], true, "{name}: {fixed}");
            let again = parse(&check_json(fixed["text"].as_str().unwrap()));
            assert_eq!(again["ok"], true, "{name} after the fix: {again}");
            continue;
        }
        assert_eq!(v["ok"], true, "{name} does not check: {v}");
        let f = parse(&format_json(&src));
        assert_eq!(f["ok"], true, "{name} does not format: {f}");
        if std::env::var_os("OXS_REFORMAT").is_some() {
            std::fs::write(&p, f["text"].as_str().unwrap()).unwrap();
            continue;
        }
        assert_eq!(
            f["text"].as_str().unwrap(),
            src,
            "{name} is not in canonical format (run with OXS_REFORMAT=1)"
        );
    }
    assert!(n >= 5, "found only {n} examples");
}
