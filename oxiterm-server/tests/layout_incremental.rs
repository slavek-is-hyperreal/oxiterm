//! OxiUI stage 5.1b: incremental layout equals a full rebuild.
//!
//! One `LayoutEngine` is kept per page and mode while a deterministic pseudo-random sequence
//! of edits (text, every layout-relevant style field, `bind-show` flips, structural changes,
//! arena id reuse and `compact`, resizes) is applied to the document. The engine's built-in
//! cross-check (`set_cross_check(true)`) lays the document out from scratch after every
//! `compute` and panics, naming the node, unless the result is bit-identical. The edits are
//! made WITHOUT `mark_dirty`, as most real mutations are.
//!
//! The test also asserts that the incremental and quiet paths were really taken, so the
//! check cannot pass vacuously through full rebuilds.

use std::cell::Cell;
use std::path::{Path, PathBuf};

use oxiterm_core::event_loop::EventLoop;
use oxiterm_core::state::StateManager;
use oxiterm_proto::dom::{Node, NodeId, NodeTag, StateEvaluator};
use oxiterm_proto::style::{
    AnsiColor, BorderChars, BorderStyle, FlexDirection, FontFace, FontSize, FontWeight, LineHeight, Position, RenderMode, WrapMode,
};
use oxiterm_renderer::{LayoutEngine, THTMLDocument};
use oxiterm_server::loader::load_thtml_file_mode;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// `bind-show` evaluator whose answers change when `epoch` is bumped.
struct Eval {
    epoch: Cell<u64>,
}
impl StateEvaluator for Eval {
    fn evaluate_bind_show(&self, cond: &str) -> bool {
        let h = cond.bytes().fold(7u64, |a, b| a.wrapping_mul(31).wrapping_add(u64::from(b)));
        (h.wrapping_add(self.epoch.get())) % 3 != 0
    }
}

fn collect_pages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    es.sort();
    for p in es {
        if p.is_dir() {
            collect_pages(&p, out);
        } else if p.extension().map_or(false, |e| e == "thtml") {
            out.push(p);
        }
    }
}

const TEXTS: &[&str] = &["", "x", "Hello world", "a much longer line of words that should wrap somewhere", "line one\nline two", "zażółć gęślą jaźń", "日本語のテキスト", "9999999", "← Back"];

fn edit(doc: &mut THTMLDocument, eval: &Eval, rng: &mut Rng, cols: &mut u16, rows: &mut u16, engine: &mut LayoutEngine) {
    let ids: Vec<NodeId> = doc.arena.iter().map(|(i, _)| i).collect();
    let pick = |rng: &mut Rng| ids[rng.below(ids.len() as u64) as usize];
    let op = rng.below(18);
    match op {
        0 => {
            let id = pick(rng);
            let t = TEXTS[rng.below(TEXTS.len() as u64) as usize].to_string();
            let n = doc.arena.get_mut(id).unwrap();
            if n.text.is_some() || n.tag == NodeTag::Text {
                n.text = Some(t);
            }
        }
        1 => { let v = rng.below(60) as f32; let id = pick(rng); let n = doc.arena.get_mut(id).unwrap(); n.style.width = if v < 8.0 { None } else { Some(v) }; }
        2 => { let v = rng.below(30) as f32; let id = pick(rng); let n = doc.arena.get_mut(id).unwrap(); n.style.height = if v < 5.0 { None } else { Some(v) }; }
        3 => { let v = rng.below(4) as f32; let id = pick(rng); let n = doc.arena.get_mut(id).unwrap(); n.style.padding.left = v; n.style.padding.top = v; }
        4 => { let v = rng.below(4) as f32; let id = pick(rng); let n = doc.arena.get_mut(id).unwrap(); n.style.margin.top = v; n.style.margin.right = v; }
        5 => {
            let id = pick(rng);
            let n = doc.arena.get_mut(id).unwrap();
            n.style.border = if n.style.border.is_some() { None } else { Some(BorderStyle { fg: AnsiColor::Reset, chars: BorderChars::single() }) };
        }
        6 => {
            let id = pick(rng);
            let n = doc.arena.get_mut(id).unwrap();
            n.style.flex_direction = if n.style.flex_direction == FlexDirection::Row { FlexDirection::Column } else { FlexDirection::Row };
        }
        7 => {
            let id = pick(rng);
            let n = doc.arena.get_mut(id).unwrap();
            n.style.wrap = if n.style.wrap == WrapMode::Word { WrapMode::None } else { WrapMode::Word };
        }
        8 => {
            let id = pick(rng);
            let n = doc.arena.get_mut(id).unwrap();
            n.style.position = [Position::Static, Position::Relative, Position::Absolute, Position::Fixed][rng.below(4) as usize];
            n.style.left = Some(rng.below(10) as f32);
            n.style.top = Some(rng.below(5) as f32);
        }
        9 => { let id = pick(rng); let z = rng.below(4) as i32 - 1; doc.arena.get_mut(id).unwrap().style.z_index = Some(z); }
        10 => { let id = pick(rng); let f = rng.below(3) as f32; doc.arena.get_mut(id).unwrap().style.flex = if f == 0.0 { None } else { Some(f) }; }
        11 => { eval.epoch.set(eval.epoch.get() + 1); } // flips bind-show nodes without touching the document
        12 => { *cols = 20 + rng.below(180) as u16; }
        13 => { *rows = 5 + rng.below(60) as u16; }
        14 => {
            // append a new subtree (a box holding a text leaf) somewhere
            let parent = pick(rng);
            let mut b = Node::new(NodeTag::Box);
            b.style.width = Some(rng.below(30) as f32 + 1.0);
            let bid = doc.arena.alloc(b);
            let mut t = Node::new(NodeTag::Text);
            t.text = Some(TEXTS[rng.below(TEXTS.len() as u64) as usize].to_string());
            t.style.wrap = if rng.below(2) == 0 { WrapMode::Word } else { WrapMode::None };
            let tid = doc.arena.alloc(t);
            doc.append_child(bid, tid).unwrap();
            let _ = doc.append_child(parent, bid);
        }
        15 => {
            // detach a child and free its slot (arena id reuse on the next alloc)
            let parent = pick(rng);
            let kids = doc.arena.get(parent).unwrap().children.clone();
            if !kids.is_empty() {
                let c = kids[rng.below(kids.len() as u64) as usize];
                if doc.detach_child(parent, c).is_ok() {
                    doc.arena.remove(c);
                }
            }
        }
        16 => {
            // reorder children
            let parent = pick(rng);
            let n = doc.arena.get_mut(parent).unwrap();
            if n.children.len() > 1 {
                n.children.reverse();
            }
        }
        _ => {
            // defragment: ids change under the kept engine (EventLoop resets it; the content
            // comparison must not need that)
            doc.compact();
            let _ = engine;
        }
    }
}

/// Stage 6.3 font mutations, drawn from their own generator so the sequence of the layout edits
/// above (and with it the seed-12 regression) is exactly the one it always was. Includes the
/// parent-only case: a node with children gets a new family or size while its descendants
/// declare nothing, so only the inherited value changes under them.
fn edit_font(doc: &mut THTMLDocument, rng: &mut Rng) {
    let ids: Vec<NodeId> = doc.arena.iter().map(|(i, _)| i).collect();
    let id = ids[rng.below(ids.len() as u64) as usize];
    let parents: Vec<NodeId> = ids.iter().copied().filter(|&i| !doc.arena.get(i).unwrap().children.is_empty()).collect();
    let n = doc.arena.get_mut(id).unwrap();
    match rng.below(8) {
        0 => n.style.font.family = [None, Some(FontFace::Mono), Some(FontFace::Sans)][rng.below(3) as usize],
        1 => n.style.font.size = [None, Some(FontSize::Px(12.0)), Some(FontSize::Px(16.0)), Some(FontSize::Px(24.0)), Some(FontSize::Em(1.5))][rng.below(5) as usize],
        2 => n.style.font.line_height = [None, Some(LineHeight::Px(30.0)), Some(LineHeight::Multiplier(1.25)), Some(LineHeight::Multiplier(2.0))][rng.below(4) as usize],
        3 => n.style.font.weight = [None, Some(FontWeight::Absolute(700)), Some(FontWeight::Absolute(400))][rng.below(3) as usize],
        4 => n.style.font.italic = [None, Some(true), Some(false)][rng.below(3) as usize],
        5 => n.style.font = Default::default(),
        6 => {
            // parent-only family change
            if !parents.is_empty() {
                let p = parents[rng.below(parents.len() as u64) as usize];
                let n = doc.arena.get_mut(p).unwrap();
                n.style.font.family = Some(if n.style.font.family == Some(FontFace::Sans) { FontFace::Mono } else { FontFace::Sans });
            }
        }
        _ => {
            // parent-only size change
            if !parents.is_empty() {
                let p = parents[rng.below(parents.len() as u64) as usize];
                let n = doc.arena.get_mut(p).unwrap();
                n.style.font.size = Some(FontSize::Px(12.0 + rng.below(20) as f32));
            }
        }
    }
}

/// The sans fixture of the frame-cost bench, so the fuzzer also starts from a proportional page.
fn sans_fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/pages/sans.thtml")
}

/// Seed 12 is the one that exposed Taffy's history dependent cache (see `LayoutEngine::compute`):
/// with the cache kept across changed frames it produced a layout that differed from a rebuild.
#[test]
fn incremental_layout_equals_rebuild_on_all_examples_under_random_edits() {
    let seeds: Vec<u64> = match std::env::var("LI_SEED").ok().and_then(|s| s.parse().ok()) {
        Some(s) => vec![s],
        None => vec![0, 12],
    };
    for seed in seeds {
        run_seed(seed);
    }
}

fn run_seed(seed: u64) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut pages = Vec::new();
    collect_pages(&root, &mut pages);
    assert!(pages.len() >= 30, "example pages not found");
    pages.push(sans_fixture());
    let st = StateManager::new();
    let (mut full, mut inc, mut quiet, mut checked, mut computes, mut shaped) = (0, 0, 0, 0, 0, 0usize);

    for (pi, path) in pages.iter().enumerate() {
        let src = std::fs::read_to_string(path).unwrap_or_default();
        if src.contains("<video") {
            continue;
        }
        for mode in [RenderMode::Tui, RenderMode::Gui] {
            let mut doc = load_thtml_file_mode(path, mode).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            EventLoop::inject_initial_state(&mut doc, &st);
            let eval = Eval { epoch: Cell::new(pi as u64) };
            let mut engine = LayoutEngine::with_mode(mode);
            engine.set_cross_check(true);
            let (mut cols, mut rows) = (80u16, 24u16);
            let mut frng = Rng(0xC2B2_AE3D_27D4_EB4F ^ seed.wrapping_mul(0x1656_67B1_9E37_79F9) ^ ((pi as u64 + 1) << 12) ^ (mode as u64 + 1));
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ ((pi as u64 + 1) << 20) ^ (mode as u64 + 1));
            for step in 0..70 {
                // 0 to 3 edits per frame; 0 edits is a quiet frame
                for _ in 0..rng.below(4) {
                    edit(&mut doc, &eval, &mut rng, &mut cols, &mut rows, &mut engine);
                }
                for _ in 0..frng.below(3) {
                    edit_font(&mut doc, &mut frng);
                }
                engine.set_viewport_rows(rows);
                engine
                    .compute(&mut doc, cols, 0, Some(&eval))
                    .unwrap_or_else(|e| panic!("{} {mode:?} step {step}: {e}", path.display()));
                computes += 1;
                shaped += engine.shaped_count();
            }
            let s = engine.stats();
            full += s.full;
            inc += s.incremental;
            quiet += s.quiet;
            checked += s.cross_checked;
        }
    }
    assert_eq!(checked, computes, "every compute must have been cross-checked");
    assert!(inc > full, "incremental path must dominate (full {full}, incremental {inc})");
    assert!(quiet > 0, "quiet frames must occur (quiet {quiet})");
    assert!(shaped > 1000, "font edits must put many nodes on the shaped path (shaped node-frames {shaped})");
    eprintln!("layout_incremental seed {seed}: computes {computes}, shaped node-frames {shaped}, full {full}, incremental {inc}, quiet {quiet}");
}

/// A parent's font-family or font-size alone changes (its descendants declare nothing): every
/// descendant must re-measure, and the kept tree must equal a rebuild (the engine's cross-check
/// panics otherwise, naming the node).
#[test]
fn parent_only_font_changes_re_measure_descendants() {
    let mut doc = load_thtml_file_mode(&sans_fixture(), RenderMode::Gui).unwrap();
    let mut engine = LayoutEngine::with_mode(RenderMode::Gui);
    engine.set_cross_check(true);
    engine.set_viewport_rows(24);
    let r0 = engine.compute(&mut doc, 80, 0, None).unwrap();
    let shaped0 = engine.shaped_count();
    assert!(shaped0 >= 8, "the sans page is shaped ({shaped0})");
    let root_kid = doc.arena.get(doc.root).unwrap().children[0];
    let paragraph = doc.arena.iter().find(|(_, n)| n.tag == NodeTag::Text && n.style.wrap == WrapMode::Word).map(|(i, _)| i).unwrap();
    let words: Vec<NodeId> = doc.arena.iter().filter(|(_, n)| n.tag == NodeTag::Text && n.style.wrap == WrapMode::Word).map(|(i, _)| i).collect();
    let total = |r: &oxiterm_renderer::LayoutResult| words.iter().map(|i| r.nodes[i].height).sum::<f32>();
    let h0 = total(&r0);
    // The page's own declared font sits on `.page`; replace it on that node only.
    let declared = doc.arena.get(root_kid).unwrap().style.font;
    assert_eq!(declared.family, Some(FontFace::Sans));

    // family only: sans -> mono; paragraphs fall back to the cell path
    doc.arena.get_mut(root_kid).unwrap().style.font.family = Some(FontFace::Mono);
    let r1 = engine.compute(&mut doc, 80, 0, None).unwrap();
    assert!(engine.shaped_count() < shaped0, "descendants left the shaper ({} vs {shaped0})", engine.shaped_count());
    assert_ne!(total(&r1), h0, "the paragraphs re-wrapped under the new family");

    // size only: mono at 22 px; descendants are shaped again, at the new size
    doc.arena.get_mut(root_kid).unwrap().style.font.size = Some(FontSize::Px(22.0));
    let r2 = engine.compute(&mut doc, 80, 0, None).unwrap();
    assert!(engine.shaped_count() >= shaped0 - 1);
    assert!(engine.shaped_text(paragraph).unwrap().font.size_px == 22.0);
    assert_ne!(total(&r2), total(&r1), "and again under the new size");

    // back to the original declaration: bit-identical to the first frame
    doc.arena.get_mut(root_kid).unwrap().style.font = declared;
    let r3 = engine.compute(&mut doc, 80, 0, None).unwrap();
    assert_eq!(engine.shaped_count(), shaped0);
    for (id, r) in &r0.nodes {
        let g = &r3.nodes[id];
        assert_eq!([r.x.to_bits(), r.y.to_bits(), r.width.to_bits(), r.height.to_bits()], [g.x.to_bits(), g.y.to_bits(), g.width.to_bits(), g.height.to_bits()]);
    }
    let s = engine.stats();
    assert_eq!(s.cross_checked, 4);
    assert!(s.incremental >= 3, "{s:?}");
}

/// Edits that go through the document API with `mark_dirty`, a kept engine, no `bind-show`
/// evaluator at first (static preview) and then one: the evaluator's presence flips display.
#[test]
fn incremental_layout_handles_evaluator_presence_and_dirty_marks() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/demos/counter.thtml");
    let mut doc = load_thtml_file_mode(&path, RenderMode::Tui).unwrap();
    let mut engine = LayoutEngine::with_mode(RenderMode::Tui);
    engine.set_cross_check(true);
    engine.set_viewport_rows(24);
    let eval = Eval { epoch: Cell::new(0) };
    for i in 0..30u64 {
        for (id, n) in doc.arena.iter_mut().map(|(i, n)| (i, n)).collect::<Vec<_>>() {
            if n.attrs.bind_state.is_some() {
                n.text = Some((i * 7919).to_string());
                let _ = id;
            }
        }
        let ids: Vec<NodeId> = doc.arena.iter().filter(|(_, n)| n.attrs.bind_state.is_some()).map(|(i, _)| i).collect();
        for id in ids {
            doc.mark_dirty(id);
        }
        eval.epoch.set(i / 3);
        let ev: Option<&dyn StateEvaluator> = if i % 2 == 0 { Some(&eval) } else { None };
        engine.compute(&mut doc, 80, 0, ev).unwrap();
    }
    let s = engine.stats();
    assert_eq!(s.cross_checked, 30);
    assert!(s.incremental >= 25, "{s:?}");
}
