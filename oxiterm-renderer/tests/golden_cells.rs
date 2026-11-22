//! OxiUI Phase 0: golden cell snapshots.
//!
//! Renders every `examples/**/*.thtml` page at fixed terminal sizes and compares the
//! resulting `CellBuffer` (glyphs, colours, modifiers, wide-char skip cells) with the
//! committed baseline in `tests/golden/`. This pins the TUI output so the renderer can be
//! refactored (DisplayList, f32 geometry, see `docs/oxi-ui.md`) without visual regressions.
//!
//! Rendering is static, as in the web playground: no App Server state is injected, so
//! `bind-*` values are their defaults. Media runs in web mode (`is_web`), which draws
//! placeholders/overlay slots instead of decoding pixels, keeping snapshots deterministic.
//!
//! To accept an intentional change, regenerate the baseline and review the diff:
//!
//! ```text
//! OXI_BLESS=1 cargo test -p oxiterm-renderer --test golden_cells
//! git diff oxiterm-renderer/tests/golden/
//! ```

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use oxiterm_proto::style::{AnsiColor, TerminalProfile};
use oxiterm_renderer::layout::engine::LayoutEngine;
use oxiterm_renderer::parser::THTMLParser;
use oxiterm_renderer::render::buffer::Cell;
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_renderer::CellBuffer;

const SIZES: &[(u16, u16)] = &[(80, 24), (120, 40)];

fn thtml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            thtml_files(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("thtml") {
            out.push(path);
        }
    }
}

fn color(c: AnsiColor) -> String {
    match c {
        AnsiColor::TrueColor(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        AnsiColor::Color256(i) => format!("c{i}"),
        AnsiColor::Reset => "reset".to_string(),
    }
}

fn style_key(cell: &Cell) -> String {
    let mut s = format!("fg={} bg={}", color(cell.fg), color(cell.bg));
    if cell.bold {
        s.push_str(" bold");
    }
    if cell.underline {
        s.push_str(" underline");
    }
    if cell.italic {
        s.push_str(" italic");
    }
    s
}

/// Serializes a buffer to a stable, diff-friendly text form:
/// a glyph grid (skip cells shown as nothing, so wide glyphs line up visually),
/// then per-row style runs, then the number of graphics payloads.
fn snapshot(buf: &CellBuffer) -> String {
    let mut out = String::new();
    let w = buf.width as usize;
    writeln!(out, "size {}x{}", buf.width, buf.height).unwrap();
    writeln!(out, "== glyphs").unwrap();
    for y in 0..buf.height as usize {
        let row = &buf.cells[y * w..(y + 1) * w];
        let line: String = row.iter().filter(|c| !c.skip).map(|c| c.ch).collect();
        writeln!(out, "|{}|", line.trim_end()).unwrap();
    }
    writeln!(out, "== skip").unwrap();
    for y in 0..buf.height as usize {
        let row = &buf.cells[y * w..(y + 1) * w];
        let skips: Vec<String> = row
            .iter()
            .enumerate()
            .filter(|(_, c)| c.skip)
            .map(|(x, _)| x.to_string())
            .collect();
        if !skips.is_empty() {
            writeln!(out, "{y}: {}", skips.join(",")).unwrap();
        }
    }
    writeln!(out, "== styles").unwrap();
    for y in 0..buf.height as usize {
        let row = &buf.cells[y * w..(y + 1) * w];
        let mut runs = Vec::new();
        let mut start = 0;
        while start < w {
            let key = style_key(&row[start]);
            let mut end = start + 1;
            while end < w && style_key(&row[end]) == key {
                end += 1;
            }
            runs.push(format!("{start}-{} {key}", end - 1));
            start = end;
        }
        writeln!(out, "{y}: {}", runs.join(" | ")).unwrap();
    }
    writeln!(out, "== graphics {}", buf.graphics.len()).unwrap();
    out
}

fn render(path: &Path, examples: &Path, width: u16, height: u16) -> Result<String, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut doc = THTMLParser::parse(&content).map_err(|e| format!("parse error: {e}"))?;
    let mut engine = LayoutEngine::new();
    let layout = engine.compute(&mut doc, width, 0, None).map_err(|e| format!("layout error: {e}"))?;
    let mut buf = CellBuffer::new(width, height);
    let mut profile = TerminalProfile::default();
    profile.is_web = true;
    Renderer::render_node(&doc, &layout, &mut buf, &profile, path.parent(), Some(examples), 0);
    Ok(snapshot(&buf))
}

fn golden_name(rel: &Path, width: u16, height: u16) -> String {
    let stem = rel.with_extension("").to_string_lossy().replace(['/', '\\'], "__");
    format!("{stem}@{width}x{height}.txt")
}

#[test]
fn golden_cell_snapshots() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let examples = root.join("../examples").canonicalize().unwrap();
    let golden_dir = root.join("tests/golden");
    let bless = std::env::var_os("OXI_BLESS").is_some();

    let mut files = Vec::new();
    thtml_files(&examples, &mut files);
    files.sort();
    assert!(!files.is_empty(), "no example pages found at {:?}", examples);

    if bless {
        std::fs::create_dir_all(&golden_dir).unwrap();
    }

    let mut expected_names = Vec::new();
    let mut failures = Vec::new();
    for file in &files {
        let rel = file.strip_prefix(&examples).unwrap();
        for &(w, h) in SIZES {
            let name = golden_name(rel, w, h);
            expected_names.push(name.clone());
            let actual = match render(file, &examples, w, h) {
                Ok(s) => s,
                Err(e) => format!("error {e}\n"),
            };
            let golden_path = golden_dir.join(&name);
            if bless {
                std::fs::write(&golden_path, &actual).unwrap();
                continue;
            }
            match std::fs::read_to_string(&golden_path) {
                Ok(expected) if expected == actual => {}
                Ok(expected) => {
                    let line = expected
                        .lines()
                        .zip(actual.lines())
                        .position(|(a, b)| a != b)
                        .map(|i| i + 1)
                        .unwrap_or_else(|| expected.lines().count().min(actual.lines().count()) + 1);
                    failures.push(format!("{name}: differs from baseline (first difference at line {line})"));
                }
                Err(_) => failures.push(format!("{name}: missing baseline")),
            }
        }
    }

    // Baselines for pages that no longer exist would silently stop guarding anything.
    if let Ok(entries) = std::fs::read_dir(&golden_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !expected_names.contains(&name) {
                if bless {
                    std::fs::remove_file(entry.path()).unwrap();
                } else {
                    failures.push(format!("{name}: stale baseline (no matching example page)"));
                }
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} golden snapshot(s) failed; if intended, rerun with OXI_BLESS=1 and review `git diff`:\n  {}",
        failures.len(),
        failures.join("\n  ")
    );
}
