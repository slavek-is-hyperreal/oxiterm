//! BAD ACTOR 8.W: reproducing tests (report only; these assert the *observed* behaviour that is a defect).

use oxiscript_playground::{apply_fix_json, check_json, explain_json};
use serde_json::Value;

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

/// F-STALE: the page sends (diag index, fix index) from its last `check`, but `apply_fix_json` re-checks
/// whatever the textarea holds now. After an edit that adds an EARLIER diagnostic (within the 250 ms
/// debounce, or while the check has not run yet) the button labelled "== -> ===" applies a different fix.
#[test]
fn stale_fix_index_applies_a_different_fix() {
    let src1 = "const a: i32 = 1;\nconst b: bool = a == 1;\n";
    let v = parse(&check_json(src1));
    let (di, d) = v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .find(|(_, d)| !d["fixes"].as_array().unwrap().is_empty())
        .unwrap();
    let label = d["fixes"][0]["title"].as_str().unwrap().to_string();
    // user types one more `==` line ABOVE before the debounced re-check fires, then clicks the old button
    let src2 = "const z: bool = 2 == 2;\nconst a: i32 = 1;\nconst b: bool = a == 1;\n";
    let r = parse(&apply_fix_json(src2, di, 0));
    println!("button label: {label}; result: {r}");
    // The first `==` (a different diagnostic) was rewritten, not the one the user clicked on.
    let text = r["text"].as_str().unwrap();
    // NOTE: asserts the CURRENT defective behaviour (F3); whoever fixes F3 should flip this assertion.
    assert!(
        text.contains("2 === 2") && text.contains("a == 1"),
        "expected the wrong occurrence to be fixed, got {text}"
    );
}

/// Hostile reason-code text (as typed into the explain call) is just an unknown code.
#[test]
fn explain_with_markup_code_is_inert_json() {
    assert_eq!(
        parse(&explain_json("<img src=x onerror=alert(1)>"))["ok"],
        false
    );
}
