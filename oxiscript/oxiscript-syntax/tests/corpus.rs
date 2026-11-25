#![allow(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    clippy::match_same_arms,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::wildcard_imports,
    clippy::struct_excessive_bools,
    clippy::large_enum_variant,
    clippy::option_option,
    clippy::ref_option,
    clippy::trivially_copy_pass_by_ref,
    clippy::redundant_closure_for_method_calls,
    clippy::single_match_else,
    clippy::format_push_string,
    clippy::similar_names,
    clippy::items_after_statements,
    clippy::needless_pass_by_value,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc,
    clippy::self_only_used_in_recursion,
    clippy::unused_self,
    clippy::return_self_not_must_use,
    clippy::doc_markdown,
    clippy::needless_lifetimes,
    clippy::manual_let_else,
    clippy::unnecessary_wraps,
    clippy::if_not_else,
    clippy::map_unwrap_or,
    clippy::cognitive_complexity,
    clippy::collapsible_else_if,
    clippy::while_let_loop,
    clippy::naive_bytecount
)]
mod common;
use common::*;
use oxiscript_syntax::*;

fn check_valid(name: &str, src: &str) {
    let (m, d) = parse_module(src);
    assert!(d.is_empty(), "{name}: unexpected diagnostics {d:#?}");
    let once = format(src).unwrap_or_else(|e| panic!("{name}: {e}"));
    let twice = format(&once).unwrap_or_else(|e| panic!("{name}: reformat failed: {e}\n{once}"));
    assert_eq!(once, twice, "{name}: formatter is not idempotent");
    // structure-preserving mode: same AST modulo spans
    let o = FormatOptions {
        braces: false,
        ..FormatOptions::default()
    };
    let nb = format_with(src, &o).unwrap();
    let (m2, d2) = parse_module(&nb);
    assert!(
        d2.is_empty(),
        "{name}: formatted output does not parse: {d2:#?}\n{nb}"
    );
    assert_eq!(
        shape(&m),
        shape(&m2),
        "{name}: AST changed by formatting\n{nb}"
    );
    let nb2 = format_with(&nb, &o).unwrap();
    assert_eq!(nb, nb2, "{name}: not idempotent without braces");
}

#[test]
fn valid_corpus_parses_formats_and_roundtrips() {
    let files = corpus("valid");
    assert!(
        files.len() >= 40,
        "need at least 40 valid programs, have {}",
        files.len()
    );
    for (name, src) in &files {
        check_valid(name, src);
    }
}

#[test]
fn invalid_corpus_reports_expected_diagnostics() {
    let files = corpus("invalid");
    assert!(
        files.len() >= 30,
        "need at least 30 invalid programs, have {}",
        files.len()
    );
    for (name, src) in &files {
        let exp_path = format!(
            "{}/tests/corpus/invalid/{}.expected",
            env!("CARGO_MANIFEST_DIR"),
            name.trim_end_matches(".oxs")
        );
        let exp =
            std::fs::read_to_string(&exp_path).unwrap_or_else(|_| panic!("missing {exp_path}"));
        let mut want: Vec<(String, u32)> = exp
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let mut it = l.split_whitespace();
                (
                    it.next().unwrap().to_string(),
                    it.next().unwrap().parse().unwrap(),
                )
            })
            .collect();
        let (_, d) = parse_module(src);
        let li = LineIndex::new(src);
        let mut got: Vec<(String, u32)> = d
            .iter()
            .map(|x| (x.code.to_string(), li.line(x.span.lo)))
            .collect();
        want.sort();
        got.sort();
        assert_eq!(got, want, "{name}: diagnostics differ: {d:#?}");
        assert!(
            format(src).is_err(),
            "{name}: invalid input must not format"
        );
        for x in &d {
            for fx in &x.fixes {
                assert!(
                    apply_edits(src, &fx.edits).is_some(),
                    "{name}: fix `{}` does not apply",
                    fx.title
                );
            }
        }
    }
}
