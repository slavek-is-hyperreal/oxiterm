//! OxiUI stage 5.0: per-frame cost of the render pipeline on the example pages.
//!
//! Run: `cargo bench -p oxiterm-server --bench frame_cost`
//! Stage 5.1b: layout keeps its Taffy tree between frames (a quiet frame reconciles and hits
//! Taffy's cache, a click frame reconciles and re-solves). `FRAME_COST_FULL=1` turns that off
//! (rebuild on every call, the pre-5.1b behaviour) for before/after numbers with one binary.
//! Stage 5.1c: GUI frames skip the snapshot + `diff_grids` when no overlay painted and reuse the
//! ARIA JSON when its inputs are unchanged (`AriaCache`); `FRAME_COST_EAGER=1` restores the old work.
//! Env: `FRAME_COST_FILTER=counter` (path substring), `FRAME_COST_MS=500` (min ms per page/size),
//!      `FRAME_COST_MIN_ITERS=300`, `FRAME_COST_VIDEO=1` (also run pages with `<video`).
//!
//! Mirrors `EventLoop::render_frame` (steady state, page already loaded) minus the parts that
//! are not per-page cost: state sync, animation tick, focus rebuild, media walk, overlays.
//! TUI: compute(Tui) -> render_node -> DiffEngine::diff + encode_ansi (SSH sink encoder), for an
//! unchanged frame and for a full repaint (web epoch path). GUI: compute(Gui) -> render_frame_mode
//! (display list build + CellSink raster + legacy content) -> buffer snapshot -> diff_grids ->
//! wire::encode -> zlib (flate2 fast, as web.rs) -> build_aria_json. Stages are timed inside one
//! iteration; totals are per-iteration sums (not sums of medians).

#![allow(clippy::all, clippy::pedantic)]

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oxiterm_core::event_loop::EventLoop;
use oxiterm_core::state::{StateManager, StateValue};
use oxiterm_proto::dom::NodeId;
use oxiterm_proto::style::{RenderMode, TerminalProfile};
use oxiterm_renderer::render::cell_sink::CellSink;
use oxiterm_renderer::render::wire;
use oxiterm_renderer::{CellBuffer, DiffEngine, LayoutEngine, LayoutPhase, THTMLDocument};
use oxiterm_renderer::render::renderer::Renderer;
use oxiterm_server::loader::load_thtml_file_mode;

const SIZES: [(u16, u16); 3] = [(80, 24), (120, 40), (200, 60)];

fn full_rebuild() -> bool { std::env::var("FRAME_COST_FULL").is_ok() }
/// Stage 5.1c off: eager snapshot clone + `diff_grids` and an uncached `build_aria_json` on every GUI frame.
fn eager_gui() -> bool { std::env::var("FRAME_COST_EAGER").is_ok() }

fn us(d: u128) -> f64 { d as f64 / 1000.0 }

struct Series(Vec<u128>);
impl Series {
    fn new() -> Self { Series(Vec::new()) }
    fn push(&mut self, d: Duration) { self.0.push(d.as_nanos()); }
    fn pct(&self, p: f64) -> f64 {
        let mut v = self.0.clone();
        v.sort_unstable();
        if v.is_empty() { return 0.0; }
        us(v[(((v.len() - 1) as f64) * p).round() as usize])
    }
    fn med(&self) -> f64 { self.pct(0.5) }
    fn p95(&self) -> f64 { self.pct(0.95) }
}

fn sum_series(parts: &[&Series]) -> Series {
    let n = parts[0].0.len();
    Series((0..n).map(|i| parts.iter().map(|s| s.0[i]).sum()).collect())
}

#[derive(Default)]
struct Stages {
    // layout phases (TUI)
    t_reset: Vec<u128>, t_build: Vec<u128>, t_solve: Vec<u128>, t_finish: Vec<u128>,
    // layout phases (GUI)
    g_reset: Vec<u128>, g_build: Vec<u128>, g_solve: Vec<u128>, g_finish: Vec<u128>,
}

struct PageResult {
    name: String,
    media: bool,
    parse_tui: f64, parse_gui: f64,
    t_layout: Series, t_render: Series, t_diff_same: Series, t_diff_full: Series,
    t_tot_same: Series, t_tot_full: Series,
    g_layout: Series, g_render: Series, g_snap: Series, g_enc: Series, g_zlib: Series, g_aria: Series, g_snap_row: Series,
    g_tot_unch: Series, g_tot_chg: Series,
    // layout phase medians (us): reset, build, solve, finish for TUI and GUI
    t_ph: [f64; 4], g_ph: [f64; 4],
    ansi_full: usize, wire_len: usize, zlib_len: usize, iters: usize,
}

fn med_u(v: &[u128]) -> f64 { let mut s = v.to_vec(); s.sort_unstable(); if s.is_empty() { 0.0 } else { us(s[s.len() / 2]) } }

fn timed<R>(f: impl FnOnce() -> R) -> (R, Duration) {
    let t = Instant::now();
    let r = f();
    (r, t.elapsed())
}

/// Lays out through the probe and returns (layout, [reset, build, solve, finish]).
fn layout_probed(eng: &mut LayoutEngine, doc: &mut THTMLDocument, cols: u16, st: &StateManager) -> (oxiterm_renderer::LayoutResult, [Duration; 4]) {
    let mut ts = [Duration::ZERO; 4];
    let mut last = Instant::now();
    let r = eng.compute_probed(doc, cols, Some(st), |ph| {
        let now = Instant::now();
        let i = match ph { LayoutPhase::Reset => 0, LayoutPhase::Build => 1, LayoutPhase::Solve => 2, LayoutPhase::Finish => 3 };
        ts[i] = now - last;
        last = now;
    }).expect("layout");
    (r, ts)
}

fn load(path: &Path, mode: RenderMode, st: &StateManager) -> THTMLDocument {
    let mut d = load_thtml_file_mode(path, mode).expect("parse");
    EventLoop::inject_initial_state(&mut d, st);
    d
}

fn bench_page(path: &Path, (cols, rows): (u16, u16), budget: Duration, min_iters: usize) -> PageResult {
    let st = StateManager::new();
    let profile = TerminalProfile::default();
    let base_dir = path.parent();

    // parse (reported once per page; measured here per size run but identical)
    let parse = |mode| {
        let mut v = Vec::new();
        for _ in 0..15 { let (d, t) = timed(|| load_thtml_file_mode(path, mode).unwrap()); black_box(d); v.push(t.as_nanos()); }
        med_u(&v)
    };
    let parse_tui = parse(RenderMode::Tui);
    let parse_gui = parse(RenderMode::Gui);

    let mut tdoc = load(path, RenderMode::Tui, &st);
    let mut gdoc = load(path, RenderMode::Gui, &st);
    let mut teng = LayoutEngine::with_mode(RenderMode::Tui);
    let mut geng = LayoutEngine::with_mode(RenderMode::Gui);
    teng.set_viewport_rows(rows);
    geng.set_viewport_rows(rows);
    teng.set_cross_check(false);
    geng.set_cross_check(false);
    if full_rebuild() { teng.set_incremental(false); geng.set_incremental(false); }

    let mut tback = CellBuffer::new(cols, rows);
    let mut gback = CellBuffer::new(cols, rows);
    let mut tfront: Option<CellBuffer> = None;
    let mut last_list: Option<Vec<u8>> = None;
    let mut aria_cache = oxiterm_a11y::AriaCache::new();
    let eager = eager_gui();

    let mut r = PageResult {
        name: String::new(), media: false, parse_tui, parse_gui,
        t_layout: Series::new(), t_render: Series::new(), t_diff_same: Series::new(), t_diff_full: Series::new(),
        t_tot_same: Series::new(), t_tot_full: Series::new(),
        g_layout: Series::new(), g_render: Series::new(), g_snap: Series::new(), g_enc: Series::new(), g_zlib: Series::new(), g_aria: Series::new(), g_snap_row: Series::new(),
        g_tot_unch: Series::new(), g_tot_chg: Series::new(),
        t_ph: [0.0; 4], g_ph: [0.0; 4], ansi_full: 0, wire_len: 0, zlib_len: 0, iters: 0,
    };
    let mut stg = Stages::default();

    let started = Instant::now();
    let mut i = 0usize;
    loop {
        let warm = i < 20;
        // ---- TUI frame ----
        let ((layout, ph), _) = timed(|| layout_probed(&mut teng, &mut tdoc, cols, &st));
        let t_lay: Duration = ph.iter().sum();
        let (_, t_ren) = timed(|| {
            tback.clear();
            Renderer::render_node(&tdoc, &layout, &mut tback, &profile, base_dir, None, 0);
        });
        let front = tfront.get_or_insert_with(|| CellBuffer { cells: tback.cells.clone(), width: cols, height: rows, graphics: Vec::new() });
        // unchanged: previous frame equals this one
        let ((cmds, bytes), t_same) = timed(|| { let c = DiffEngine::diff(front, &tback); let b = DiffEngine::encode_ansi(&c); (c, b) });
        if i == 0 { assert!(cmds.is_empty(), "unchanged TUI diff must be empty"); }
        black_box((&cmds, &bytes));
        // full repaint (epoch path in web.rs: blank previous, force_dirty)
        let ((fcmds, fbytes), t_full) = timed(|| {
            let mut prev = CellBuffer::new(tback.width, tback.height);
            prev.force_dirty();
            let c = DiffEngine::diff(&prev, &tback);
            let b = DiffEngine::encode_ansi(&c);
            (c, b)
        });
        black_box(&fcmds);
        let ansi_full = fbytes.len();

        // ---- GUI frame ----
        let ((glayout, gph), _) = timed(|| layout_probed(&mut geng, &mut gdoc, cols, &st));
        let g_lay: Duration = gph.iter().sum();
        gback.clear();
        let (list, g_ren) = timed(|| Renderer::render_frame_mode(&gdoc, &glayout, &mut gback, &profile, base_dir, None, 0, RenderMode::Gui));
        // 5.1c: a frame with no overlay needs neither the snapshot nor the diff (the bench frames
        // carry no overlays). EAGER restores the pre-5.1c work.
        let (_, g_snap) = timed(|| {
            if eager {
                let s = CellBuffer { cells: gback.cells.clone(), width: gback.width, height: gback.height, graphics: Vec::new() };
                let e = CellSink::diff_grids(&s, &gback);
                black_box((&s, &e));
            }
        });
        // A frame with the focus ring painted: save one row, then diff that row only.
        let (_, g_snap_row) = timed(|| {
            let w = gback.width as usize;
            let y = (gback.height / 2) as usize;
            let saved = vec![(y as u16, gback.cells[y * w..(y + 1) * w].to_vec())];
            black_box(CellSink::diff_grids_rows(&saved, &gback));
        });
        let (encoded, g_enc) = timed(|| { let e = wire::encode_for(&list, wire::WIRE_VERSION_3); let same = last_list.as_deref() == Some(e.as_slice()); black_box(same); e });
        let (zl, g_zlib) = timed(|| {
            let mut enc = flate2::write::ZlibEncoder::new(vec![0x36u8], flate2::Compression::fast());
            std::io::Write::write_all(&mut enc, &encoded).unwrap();
            enc.finish().unwrap()
        });
        let (json_len, g_aria) = timed(|| if eager {
            oxiterm_a11y::build_aria_json(&gdoc, &glayout, None, (0, 0), (0, 0)).len()
        } else {
            aria_cache.json(&gdoc, &glayout, None, (0, 0), (0, 0)).len()
        });
        black_box(json_len);
        let (wire_len, zlib_len) = (encoded.len(), zl.len());
        last_list = Some(encoded);

        if !warm {
            r.t_layout.push(t_lay); r.t_render.push(t_ren); r.t_diff_same.push(t_same); r.t_diff_full.push(t_full);
            r.t_tot_same.push(t_lay + t_ren + t_same); r.t_tot_full.push(t_lay + t_ren + t_full);
            r.g_layout.push(g_lay); r.g_render.push(g_ren); r.g_snap.push(g_snap); r.g_enc.push(g_enc); r.g_zlib.push(g_zlib); r.g_aria.push(g_aria); r.g_snap_row.push(g_snap_row);
            let base = g_lay + g_ren + g_snap + g_enc + g_aria;
            r.g_tot_unch.push(base); r.g_tot_chg.push(base + g_zlib);
            stg.t_reset.push(ph[0].as_nanos()); stg.t_build.push(ph[1].as_nanos()); stg.t_solve.push(ph[2].as_nanos()); stg.t_finish.push(ph[3].as_nanos());
            stg.g_reset.push(gph[0].as_nanos()); stg.g_build.push(gph[1].as_nanos()); stg.g_solve.push(gph[2].as_nanos()); stg.g_finish.push(gph[3].as_nanos());
            r.ansi_full = ansi_full; r.wire_len = wire_len; r.zlib_len = zlib_len;
        }
        i += 1;
        if i >= 20 + min_iters && started.elapsed() >= budget { break; }
        if i > 20_000 { break; }
    }
    r.iters = i - 20;
    r.t_ph = [med_u(&stg.t_reset), med_u(&stg.t_build), med_u(&stg.t_solve), med_u(&stg.t_finish)];
    r.g_ph = [med_u(&stg.g_reset), med_u(&stg.g_build), med_u(&stg.g_solve), med_u(&stg.g_finish)];
    r
}

fn collect_pages(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut es: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
    es.sort();
    for p in es {
        if p.is_dir() { collect_pages(&p, out); } else if p.extension().map_or(false, |e| e == "thtml") { out.push(p); }
    }
}

fn median_of(mut v: Vec<f64>) -> f64 { v.sort_by(|a, b| a.partial_cmp(b).unwrap()); if v.is_empty() { 0.0 } else { v[v.len() / 2] } }

fn print_size(size: (u16, u16), rs: &[PageResult]) {
    println!("\n=== {}x{}  (median us per frame; p95 in totals) ===", size.0, size.1);
    println!("{:<30} | {:>7} {:>7} {:>7} {:>7} {:>8} {:>8} | {:>7} {:>7} {:>6} {:>6} {:>7} {:>6} {:>8} {:>8} | {:>6} {:>6} {:>6}",
        "page", "t.lay", "t.rend", "t.dif=", "t.difF", "t.tot=", "t.totF", "g.lay", "g.rend", "g.snap", "g.enc", "g.zlib", "g.aria", "g.tot=", "g.totZ", "ansiF", "wire", "zlib");
    for r in rs {
        println!("{:<30} | {:>7.0} {:>7.0} {:>7.0} {:>7.0} {:>8.0} {:>8.0} | {:>7.0} {:>7.0} {:>6.0} {:>6.0} {:>7.0} {:>6.0} {:>8.0} {:>8.0} | {:>6} {:>6} {:>6}{}",
            r.name, r.t_layout.med(), r.t_render.med(), r.t_diff_same.med(), r.t_diff_full.med(), r.t_tot_same.med(), r.t_tot_full.med(),
            r.g_layout.med(), r.g_render.med(), r.g_snap.med(), r.g_enc.med(), r.g_zlib.med(), r.g_aria.med(), r.g_tot_unch.med(), r.g_tot_chg.med(),
            r.ansi_full, r.wire_len, r.zlib_len, if r.media { "  [media]" } else { "" });
    }
}

fn print_aggregate(size: (u16, u16), rs: &[PageResult]) {
    let nm: Vec<&PageResult> = rs.iter().filter(|r| !r.media).collect();
    if nm.is_empty() { return; }
    let m = |f: &dyn Fn(&PageResult) -> f64| median_of(nm.iter().map(|r| f(r)).collect());
    println!("\n--- {}x{} aggregate over {} non-media pages: median-of-page-medians (p95 of totals: median of page p95) ---", size.0, size.1, nm.len());
    let rows: Vec<(&str, f64, Option<f64>)> = vec![
        ("TUI layout", m(&|r| r.t_layout.med()), None),
        ("  reset (full: drop tree; kept: reconcile)", m(&|r| r.t_ph[0]), None),
        ("  build (full: map_style + tree; kept: root setup)", m(&|r| r.t_ph[1]), None),
        ("  solve (taffy)", m(&|r| r.t_ph[2]), None),
        ("  finish (flatten+order+clone)", m(&|r| r.t_ph[3]), None),
        ("TUI render_node (clear+paint)", m(&|r| r.t_render.med()), None),
        ("TUI diff+ansi, unchanged", m(&|r| r.t_diff_same.med()), None),
        ("TUI diff+ansi, full repaint", m(&|r| r.t_diff_full.med()), None),
        ("TUI total, unchanged", m(&|r| r.t_tot_same.med()), Some(m(&|r| r.t_tot_same.p95()))),
        ("TUI total, full repaint", m(&|r| r.t_tot_full.med()), Some(m(&|r| r.t_tot_full.p95()))),
        ("GUI layout", m(&|r| r.g_layout.med()), None),
        ("  reset (full: drop tree; kept: reconcile)", m(&|r| r.g_ph[0]), None),
        ("  build (full: map_style + tree; kept: root setup)", m(&|r| r.g_ph[1]), None),
        ("  solve (taffy)", m(&|r| r.g_ph[2]), None),
        ("  finish (flatten+order+clone)", m(&|r| r.g_ph[3]), None),
        ("GUI render_frame_mode (list+raster)", m(&|r| r.g_render.med()), None),
        ("GUI snapshot clone + diff_grids", m(&|r| r.g_snap.med()), None),
        ("GUI wire::encode (+cmp)", m(&|r| r.g_enc.med()), None),
        ("GUI zlib fast", m(&|r| r.g_zlib.med()), None),
        ("GUI build_aria_json (cached key+hit)", m(&|r| r.g_aria.med()), None),
        ("  (info) one overlay row: save+diff", m(&|r| r.g_snap_row.med()), None),
        ("GUI total, unchanged (no zlib)", m(&|r| r.g_tot_unch.med()), Some(m(&|r| r.g_tot_unch.p95()))),
        ("GUI total, changed (with zlib)", m(&|r| r.g_tot_chg.med()), Some(m(&|r| r.g_tot_chg.p95()))),
        ("parse TUI (once per page)", m(&|r| r.parse_tui), None),
        ("parse GUI (once per page)", m(&|r| r.parse_gui), None),
    ];
    for (n, med, p95) in rows {
        match p95 { Some(p) => println!("{:<40} {:>9.0} us   p95 {:>9.0} us", n, med, p), None => println!("{:<40} {:>9.0} us", n, med) }
    }
    let mut heavy: Vec<&&PageResult> = nm.iter().collect();
    heavy.sort_by(|a, b| b.g_tot_chg.med().partial_cmp(&a.g_tot_chg.med()).unwrap());
    println!("5 heaviest by GUI total (changed): {}", heavy.iter().take(5).map(|r| format!("{} {:.0}us", r.name, r.g_tot_chg.med())).collect::<Vec<_>>().join(", "));
    let mut heavy: Vec<&&PageResult> = nm.iter().collect();
    heavy.sort_by(|a, b| b.t_tot_full.med().partial_cmp(&a.t_tot_full.med()).unwrap());
    println!("5 heaviest by TUI total (full):    {}", heavy.iter().take(5).map(|r| format!("{} {:.0}us", r.name, r.t_tot_full.med())).collect::<Vec<_>>().join(", "));
    let _ = sum_series;
}

/// Counter page: change one bound value (like a click) and time a full frame on each path,
/// splitting layout into its phases; also the best case of incremental layout (solve only).
fn counter_scenario(root: &Path, size: (u16, u16), n: usize) {
    let path = root.join("demos/counter.thtml");
    if !path.exists() { return; }
    let (cols, rows) = size;
    let profile = TerminalProfile::default();
    let base_dir = path.parent();
    let mut st = StateManager::new();
    st.set("counter".into(), StateValue::Int(0));

    for (label, mode) in [("TUI", RenderMode::Tui), ("GUI", RenderMode::Gui)] {
        let mut doc = load(&path, mode, &st);
        let mut subs: Vec<NodeId> = Vec::new();
        for (id, node) in doc.arena.iter() { if node.attrs.bind_state.as_deref() == Some("counter") { subs.push(id); } }
        st.clear_subscriptions();
        for &id in &subs { st.subscribe("counter".into(), id); }
        let mut eng = LayoutEngine::with_mode(mode);
        eng.set_viewport_rows(rows);
        eng.set_cross_check(false);
        if full_rebuild() { eng.set_incremental(false); }
        let mut back = CellBuffer::new(cols, rows);
        let mut prev = CellBuffer::new(cols, rows);
        let mut aria_cache = oxiterm_a11y::AriaCache::new();
        let _ = layout_probed(&mut eng, &mut doc, cols, &st);

        let mut full = Series::new(); let mut lay = Series::new(); let mut rest = Series::new();
        let mut ph = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        let mut resolve_hit = Vec::new(); let mut resolve_dirty = Vec::new(); let mut quiet_v = Vec::new();
        let (mut full_s, mut lay_s, mut rest_s) = (Series::new(), Series::new(), Series::new());
        for i in 0..(n + 20) {
            // like sync_dirty_state: set text + mark dirty
            st.set("counter".into(), StateValue::Int(i as i64 + 1));
            let t0 = Instant::now();
            for id in st.get_dirty_nodes() {
                if let Some(node) = doc.arena.get_mut(id) { if let Some(k) = &node.attrs.bind_state { if let Some(v) = st.get(k) { node.text = Some(v.to_string()); } } }
                doc.mark_dirty(id);
            }
            let (layout, p) = layout_probed(&mut eng, &mut doc, cols, &st);
            let t_l = t0.elapsed();
            let (_, t_rest) = timed(|| {
                back.clear();
                match mode {
                    RenderMode::Tui => {
                        Renderer::render_node(&doc, &layout, &mut back, &profile, base_dir, None, 0);
                        let c = DiffEngine::diff(&prev, &back); // against the previous frame, like the sink
                        black_box(DiffEngine::encode_ansi(&c));
                        prev.cells.copy_from_slice(&back.cells);
                    }
                    RenderMode::Gui => {
                        let list = Renderer::render_frame_mode(&doc, &layout, &mut back, &profile, base_dir, None, 0, RenderMode::Gui);
                        if eager_gui() {
                            let snap = CellBuffer { cells: back.cells.clone(), width: back.width, height: back.height, graphics: Vec::new() };
                            black_box(CellSink::diff_grids(&snap, &back));
                        }
                        let enc = wire::encode_for(&list, wire::WIRE_VERSION_3);
                        let mut z = flate2::write::ZlibEncoder::new(vec![0x36u8], flate2::Compression::fast());
                        std::io::Write::write_all(&mut z, &enc).unwrap();
                        black_box(z.finish().unwrap());
                        if eager_gui() {
                            black_box(oxiterm_a11y::build_aria_json(&doc, &layout, None, (0, 0), (0, 0)));
                        } else {
                            black_box(aria_cache.json(&doc, &layout, None, (0, 0), (0, 0)).len());
                        }
                    }
                }
            });
            // a quiet frame right after the click frame: nothing changed (kept tree: cache hit)
            let (_, t_quiet) = timed(|| layout_probed(&mut eng, &mut doc, cols, &st));
            // incremental best case on the tree just built
            let (_, t_hit) = timed(|| eng.resolve_only(&doc, cols, None));
            let (_, t_dirty) = timed(|| eng.resolve_only(&doc, cols, subs.first().copied()));
            if i >= 20 {
                full_s.push(t_l + t_rest); lay_s.push(t_l); rest_s.push(t_rest);
                for k in 0..4 { ph[k].push(p[k].as_nanos()); }
                quiet_v.push(t_quiet.as_nanos()); resolve_hit.push(t_hit.as_nanos()); resolve_dirty.push(t_dirty.as_nanos());
            }
        }
        let _ = (&mut full, &mut lay, &mut rest);
        let (m, l, r) = (full_s.med(), lay_s.med(), rest_s.med());
        println!("{label} {cols}x{rows} counter click, full frame: median {m:.0} us (p95 {:.0}); layout {l:.0} us ({:.0}%), rest {r:.0} us", full_s.p95(), 100.0 * l / m);
        println!("    layout phases: reset {:.0}, build {:.0}, solve {:.0}, finish {:.0} us;  solve-only on kept tree: cache hit {:.1} us, after mark_dirty(counter text) {:.1} us",
            med_u(&ph[0]), med_u(&ph[1]), med_u(&ph[2]), med_u(&ph[3]), med_u(&resolve_hit), med_u(&resolve_dirty));
        println!("    quiet layout frame right after a click frame: {:.1} us", med_u(&quiet_v));
        println!("    => rebuild (reset+build+finish) vs incremental solve: {:.0} us vs {:.1} us", med_u(&ph[0]) + med_u(&ph[1]) + med_u(&ph[3]), med_u(&resolve_dirty));
    }
}

fn main() {
    // `cargo test --all-targets` runs bench binaries in debug without --bench: do nothing.
    if !std::env::args().any(|a| a == "--bench") { return; }
    if cfg!(debug_assertions) { eprintln!("warning: debug build, numbers are meaningless; use `cargo bench`"); }
    let env_u = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let budget = Duration::from_millis(env_u("FRAME_COST_MS", 500));
    let min_iters = env_u("FRAME_COST_MIN_ITERS", 300) as usize;
    let filter = std::env::var("FRAME_COST_FILTER").ok();
    let with_video = std::env::var("FRAME_COST_VIDEO").is_ok();

    // The frozen monospace copy of `examples/` (the demo pages are proportional in GUI mode now):
    // keeps the aggregate comparable with the numbers in docs/oxi-ui.md. The live pages are
    // reported as fixtures below.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/traces/pages/examples_mono");
    let live = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut pages = Vec::new();
    collect_pages(&root, &mut pages);
    if let Some(f) = &filter { pages.retain(|p| p.to_string_lossy().contains(f.as_str())); }
    let mut fixtures = Vec::new();
    collect_pages(&Path::new(env!("CARGO_MANIFEST_DIR")).join("benches/pages"), &mut fixtures);
    for rel in ["index.thtml", "demos/counter.thtml"] {
        fixtures.push(live.join(rel));
    }
    if let Some(f) = &filter { fixtures.retain(|p| p.to_string_lossy().contains(f.as_str())); }

    for size in SIZES {
        let mut results = Vec::new();
        for p in &pages {
            let src = std::fs::read_to_string(p).unwrap_or_default();
            let has_video = src.contains("<video");
            if has_video && !with_video { continue; }
            let mut r = bench_page(p, size, budget, min_iters);
            r.name = p.strip_prefix(&root).unwrap_or(p).to_string_lossy().into_owned();
            r.media = src.contains("<img") || has_video;
            results.push(r);
        }
        print_size(size, &results);
        print_aggregate(size, &results);
        // Stage 6.3 fixtures (benches/pages, e.g. a proportional page): reported on their own, not
        // part of the aggregate above, so the mono numbers stay comparable across stages.
        let mut fx = Vec::new();
        for p in &fixtures {
            let mut r = bench_page(p, size, budget, min_iters);
            let shown = p.strip_prefix(&live).unwrap_or(p);
            r.name = format!("[fixture] {}", if shown == p.as_path() { p.file_name().unwrap_or_default().to_string_lossy().into_owned() } else { format!("examples/{}", shown.display()) });
            fx.push(r);
        }
        if !fx.is_empty() { print_size(size, &fx); }
    }
    println!("\n=== counter click scenario ===");
    for size in SIZES { counter_scenario(&root, size, 400); }
}
