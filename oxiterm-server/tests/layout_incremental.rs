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
use oxiterm_proto::style::{AnsiColor, BorderChars, BorderStyle, FlexDirection, Position, RenderMode, WrapMode};
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
    let st = StateManager::new();
    let (mut full, mut inc, mut quiet, mut checked, mut computes) = (0, 0, 0, 0, 0);

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
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD6E8_FEB8_6659_FD93) ^ ((pi as u64 + 1) << 20) ^ (mode as u64 + 1));
            for step in 0..70 {
                // 0 to 3 edits per frame; 0 edits is a quiet frame
                for _ in 0..rng.below(4) {
                    edit(&mut doc, &eval, &mut rng, &mut cols, &mut rows, &mut engine);
                }
                engine.set_viewport_rows(rows);
                engine
                    .compute(&mut doc, cols, 0, Some(&eval))
                    .unwrap_or_else(|e| panic!("{} {mode:?} step {step}: {e}", path.display()));
                computes += 1;
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
    eprintln!("layout_incremental seed {seed}: computes {computes}, full {full}, incremental {inc}, quiet {quiet}");
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
