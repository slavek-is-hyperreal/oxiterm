//! Regression guard: no clickable label in any shipped example page may contain an
//! East-Asian *Ambiguous*-width glyph (e.g. `←`, `→`, `×`). Such glyphs render 1 cell
//! on some terminals and 2 on others, so the visible label drifts off its hit box and
//! clicks miss. Non-clickable/decorative text is exempt.
//!
//! If this fails, replace the flagged glyph in the clickable label with an unambiguous
//! one (e.g. `<` for `←`, `x` for `×`).

use oxiterm_renderer::parser::THTMLParser;

fn thtml_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            thtml_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("thtml") {
            out.push(path);
        }
    }
}

#[test]
fn no_ambiguous_width_in_clickable_labels() {
    let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut files = Vec::new();
    thtml_files(&examples, &mut files);
    assert!(!files.is_empty(), "no example pages found at {:?}", examples);

    let mut violations = Vec::new();
    for file in &files {
        let content = std::fs::read_to_string(file).unwrap();
        let doc = match THTMLParser::parse(&content) {
            Ok(d) => d,
            Err(_) => continue, // parse errors are a different test's concern
        };
        for (_, text, bad) in doc.clickable_ambiguous_width() {
            violations.push(format!(
                "{}: clickable {:?} has ambiguous-width {:?}",
                file.file_name().unwrap().to_string_lossy(),
                text,
                bad
            ));
        }
    }

    assert!(
        violations.is_empty(),
        "clickable labels contain ambiguous-width glyphs (use unambiguous ones):\n{}",
        violations.join("\n")
    );
}
