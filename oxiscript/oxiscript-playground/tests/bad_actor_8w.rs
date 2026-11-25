//! BAD ACTOR 8.W regression tests (originally reproductions; F3 is fixed and its test asserts the safe behaviour).

use oxiscript_playground::{apply_fix_json, check_json, explain_json, FixExpect};
use serde_json::Value;

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap()
}

/// F-STALE (F3, FIXED; test flipped to assert the safe behaviour): the page sends (diag index, fix index) from its last `check`, but `apply_fix_json` re-checks
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
    let exp = FixExpect {
        code: d["code"].as_str().unwrap(),
        start: d["start"].as_f64().unwrap(),
        end: d["end"].as_f64().unwrap(),
        title: d["fixes"][0]["title"].as_str().unwrap(),
    };
    // user types one more `==` line ABOVE before the debounced re-check fires, then clicks the old button
    let src2 = "const z: bool = 2 == 2;\nconst a: i32 = 1;\nconst b: bool = a == 1;\n";
    #[allow(clippy::cast_precision_loss)]
    let r = parse(&apply_fix_json(src2, di as f64, 0.0, &exp));
    // FLIPPED (F3 fixed): used to apply the wrong occurrence; now a clean "stale" refusal.
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(r["kind"], "stale", "{r}");
    assert!(r.get("text").is_none());
    // the positive case: the text did not change, the same button still works and fixes the right line
    #[allow(clippy::cast_precision_loss)]
    let ok = parse(&apply_fix_json(src1, di as f64, 0.0, &exp));
    assert_eq!(ok["ok"], true, "{ok}");
    assert!(ok["text"].as_str().unwrap().contains("a === 1"));
    // the same button after an edit BELOW is also fine only if code+span+title still match (here: they do)
    let src3 = format!("{src1}const c: i32 = 3;\n");
    #[allow(clippy::cast_precision_loss)]
    let ok3 = parse(&apply_fix_json(&src3, di as f64, 0.0, &exp));
    assert_eq!(ok3["ok"], true, "{ok3}");
    // a stale fix title is refused too
    let wrong = FixExpect {
        title: "something else",
        ..exp
    };
    #[allow(clippy::cast_precision_loss)]
    let w = parse(&apply_fix_json(src1, di as f64, 0.0, &wrong));
    assert_eq!(w["kind"], "stale", "{w}");
}

/// Hostile reason-code text (as typed into the explain call) is just an unknown code.
#[test]
fn explain_with_markup_code_is_inert_json() {
    assert_eq!(
        parse(&explain_json("<img src=x onerror=alert(1)>"))["ok"],
        false
    );
}
