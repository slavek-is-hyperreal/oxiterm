//! BAD ACTOR: grammar.md self-contradictions / ambiguities (doc-level; no parser exists yet).
fn g() -> String {
    std::fs::read_to_string(format!(
        "{}/../../docs/oxiscript/grammar.md",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

#[test]
fn ba01_dot5_example_vs_literal_rule() {
    let d = g();
    assert!(!(d.contains("`a ?.5 : 1`") && d.contains("`.5` and `5.` are not literals")));
}
#[test]
fn ba02_generic_call_example_contradicts_rule() {
    let d = g();
    // rule: `<TypeList>` followed by `(` is a call; example calls `a < b > (c)` a comparison
    assert!(!(d.contains("`a < b > (c)` are comparisons") && d.contains("next token is `(`")));
}
#[test]
fn ba03_numsuffix_includes_float_suffix_for_int() {
    let d = g();
    let l = d.lines().find(|l| l.starts_with("NUMSUFFIX")).unwrap();
    assert!(
        !l.contains("\"f32\""),
        "`1f32`/`0x1f32` match INT and FLOAT: {l}"
    );
}
#[test]
fn ba04_gt_eq_split_rule() {
    let d = g();
    let l = d.lines().find(|l| l.starts_with("7. **`>>`**")).unwrap();
    assert!(l.contains(">="), "`Array<i32>= x` / `>>=` not covered: {l}");
}
#[test]
fn ba05_arm_label_arrow_ambiguity() {
    let d = g();
    assert!(
        d.contains("not an arrow function"),
        "`case k => v`: `k => v` is an arrow function by 3.4.2"
    );
}
#[test]
fn ba06_ternary_vs_arrow_return_type() {
    let d = g();
    let l = d
        .lines()
        .find(|l| l.starts_with("2. **Arrow function versus"))
        .unwrap();
    assert!(
        l.contains("ternary"),
        "`x ? (y) : z => w` has two parses: {l}"
    );
}
#[test]
fn ba07_as_union_type_vs_bitor() {
    let d = g();
    let l = d.lines().find(|l| l.starts_with("8. **`as`**")).unwrap();
    assert!(
        l.contains('|'),
        "`x as u8 | 1` : union type or bitwise-or: {l}"
    );
}
#[test]
fn ba08_second_whitespace_sensitive_token() {
    let d = g();
    assert!(!(d.contains("exactly one exception") && d.contains("\"as|\"") && d.contains("UnionType   = [ \"|\" ]")),
        "`x as| T` (saturating cast) vs `x as | T` (cast to union with leading bar) differ only by whitespace");
}
#[test]
fn ba09_optional_chain_on_new_line() {
    let d = g();
    assert!(d.contains("line break") || d.contains("newline"), "`foo()\\n  ?.bar` : ` ?.` preceded by whitespace is ternary by 1.5 but `?.` token by the first bullet");
}
