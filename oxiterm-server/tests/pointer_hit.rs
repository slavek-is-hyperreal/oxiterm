//! OxiUI Phase 6.5: hit-testing at fractional pointer coordinates.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::RenderMode;
use oxiterm_renderer::document::THTMLDocument;
use oxiterm_renderer::{LayoutEngine, LayoutResult};

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn manifest() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).to_path_buf()
}

fn layout_of(path: &Path, mode: RenderMode) -> (THTMLDocument, LayoutEngine, LayoutResult) {
    let mut doc = oxiterm_server::loader::load_thtml_file_mode(path, mode).unwrap();
    let mut engine = LayoutEngine::with_mode(mode);
    engine.set_viewport_rows(ROWS);
    let layout = engine.compute(&mut doc, COLS, 0, None).unwrap();
    (doc, engine, layout)
}

fn thtml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            thtml_files(&p, out);
        } else if p.extension().map_or(false, |x| x == "thtml") {
            out.push(p);
        }
    }
}

fn parents(doc: &THTMLDocument) -> HashMap<NodeId, NodeId> {
    let mut m = HashMap::new();
    for (id, n) in doc.arena.iter() {
        for &c in &n.children {
            m.insert(c, id);
        }
    }
    m
}

fn is_interactive(doc: &THTMLDocument, id: NodeId) -> bool {
    let n = doc.get_node(id).unwrap();
    n.attrs.event_htmx.is_some() || matches!(n.tag, oxiterm_proto::dom::NodeTag::Button | oxiterm_proto::dom::NodeTag::Input)
}

/// The interactive node a hit on `id` belongs to (the node itself or its closest ancestor).
fn interactive_of(doc: &THTMLDocument, parents: &HashMap<NodeId, NodeId>, mut id: NodeId) -> Option<NodeId> {
    loop {
        if is_interactive(doc, id) {
            return Some(id);
        }
        id = *parents.get(&id)?;
    }
}

/// On a TUI (whole-cell) layout the fractional hit test at a cell centre is today's cell
/// hit test: the top-most node in paint order whose cell rect covers the cell. Checked for
/// every cell of every example page, which includes every interactive node.
#[test]
fn cell_centres_equal_the_cell_hit_test_on_every_example_page() {
    let mut files = Vec::new();
    thtml_files(&manifest().join("../examples"), &mut files);
    files.sort();
    assert!(files.len() > 10);
    let mut interactive_cells = 0usize;
    for path in files {
        let Ok(mut doc) = oxiterm_server::loader::load_thtml_file_mode(&path, RenderMode::Tui) else { continue };
        let mut engine = LayoutEngine::with_mode(RenderMode::Tui);
        engine.set_viewport_rows(ROWS);
        let Ok(layout) = engine.compute(&mut doc, COLS, 0, None) else { continue };
        let par = parents(&doc);
        for row in 0..ROWS + 4 {
            for col in 0..COLS + 4 {
                // Reference: integer containment on the snapped rects, last painted wins.
                let reference = layout.paint_order.iter().rev().copied().find(|id| {
                    layout.nodes.get(id).map_or(false, |r| {
                        let c = r.cells();
                        col >= c.x && col < c.x + c.width && row >= c.y && row < c.y + c.height
                    })
                });
                let by_cell = engine.hit_test(col, row);
                let by_point = engine.hit_test_point(f32::from(col) + 0.5, f32::from(row) + 0.5);
                assert_eq!(by_point, by_cell, "{} cell ({col},{row})", path.display());
                if !layout.paint_order.is_empty() {
                    assert_eq!(by_point, reference, "{} cell ({col},{row})", path.display());
                }
                if let Some(h) = by_point {
                    if interactive_of(&doc, &par, h).is_some() {
                        interactive_cells += 1;
                    }
                }
            }
        }
    }
    assert!(interactive_cells > 500, "the example pages must exercise interactive nodes ({interactive_cells})");
}

/// The case the stage exists for: adjacent narrow proportional buttons whose edges fall inside
/// a cell.
#[test]
fn sweeping_across_adjacent_proportional_buttons_hits_the_right_one() {
    let page = manifest().join("tests/traces/pages/pointer_sans.thtml");
    let (doc, engine, layout) = layout_of(&page, RenderMode::Gui);
    let par = parents(&doc);
    // The buttons in document order (left to right).
    let mut buttons: Vec<(NodeId, oxiterm_renderer::Rect)> = doc
        .arena
        .iter()
        .filter(|(_, n)| n.attrs.event_htmx.is_some())
        .map(|(id, _)| (id, *layout.nodes.get(&id).unwrap()))
        .collect();
    buttons.sort_by(|a, b| a.1.x.partial_cmp(&b.1.x).unwrap());
    assert_eq!(buttons.len(), 4);
    let y = buttons[0].1.y + buttons[0].1.height / 2.0;
    for (_, r) in &buttons {
        assert_eq!(r.y, buttons[0].1.y);
    }
    // Fixture sanity: the first three touch each other at fractional x (a cell is shared),
    // the last one is separated by a one cell gap.
    for w in buttons.windows(2).take(2) {
        let edge = w[0].1.x + w[0].1.width;
        assert!((edge - w[1].1.x).abs() < 1e-4, "adjacent: {:?} {:?}", w[0].1, w[1].1);
        assert!(edge.fract() > 0.01 && edge.fract() < 0.99, "edge {edge} must fall inside a cell");
    }
    let gap = buttons[3].1.x - (buttons[2].1.x + buttons[2].1.width);
    assert!(gap > 0.9, "gap {gap}");

    let x_start = buttons[0].1.x.floor() - 2.0;
    let x_end = buttons[3].1.x + buttons[3].1.width + 2.0;
    let boundaries: Vec<f32> = buttons.iter().flat_map(|(_, r)| [r.x, r.x + r.width]).collect();
    let steps = ((x_end - x_start) / 0.1) as usize;
    let mut per_button = [0usize; 4];
    let mut gap_points = 0usize;
    for i in 0..=steps {
        let x = x_start + i as f32 * 0.1;
        // A point within float noise of an edge is not a meaningful probe.
        if boundaries.iter().any(|b| (b - x).abs() < 1e-3) {
            continue;
        }
        let got = engine.hit_test_point(x, y).and_then(|h| interactive_of(&doc, &par, h));
        let expect = buttons.iter().position(|(_, r)| r.contains(x, y));
        match expect {
            Some(k) => {
                assert_eq!(got, Some(buttons[k].0), "x={x} must hit button {k} {:?}", buttons[k].1);
                per_button[k] += 1;
            }
            None => {
                assert_eq!(got, None, "x={x} is in no button (gap or outside) and must hit nothing interactive");
                if x > buttons[2].1.x + buttons[2].1.width && x < buttons[3].1.x {
                    gap_points += 1;
                }
            }
        }
    }
    assert!(per_button.iter().all(|&n| n >= 3), "every button is probed {per_button:?}");
    assert!(gap_points >= 5, "the gap is probed ({gap_points})");

    // What the old cell-centre path got wrong: a cell containing the shared edge of A and B
    // is resolved by its centre alone, so every pixel of the other side is lost. Find such a
    // cell and count the pointer positions it would have sent to the wrong button. With the
    // current bundled sans face the edges are at x = 5.43 (cell 5, centre 5.5 is in the right
    // button: its left 0.43 is lost) and x = 12.55 (cell 12, centre 12.5 is in the left
    // button: its right 0.45 is lost): 9 of the 20 probes.
    let mut mis_hits = 0usize;
    for w in buttons.windows(2).take(2) {
        let edge = w[0].1.x + w[0].1.width;
        let col = edge.floor() as u16;
        let row = y.floor() as u16;
        let cell_hit = engine.hit_test(col, row).and_then(|h| interactive_of(&doc, &par, h));
        // Positions inside this cell, 0.1 apart, resolved exactly versus by the cell.
        for k in 0..10 {
            let x = f32::from(col) + 0.05 + k as f32 * 0.1;
            let exact = engine.hit_test_point(x, y).and_then(|h| interactive_of(&doc, &par, h));
            assert!(exact.is_some());
            if exact != cell_hit {
                mis_hits += 1;
            }
        }
    }
    assert!(mis_hits > 0, "the old cell-centre path must be wrong somewhere in the shared cells");
}
