//! Layout computation engine mapping DOM structures onto Taffy trees.
//!
//! Synchronizes OxiTerm node styles and parent-child hierarchies into a TaffyTree
//! to calculate precise coordinates using the Flexbox layout algorithm.

use taffy::prelude::*;
use crate::document::THTMLDocument;
use crate::layout::types::{LayoutResult, Rect as OxiRect};
use oxiterm_proto::style::RenderMode;
use oxiterm_proto::dom::NodeId as OxiNodeId;
use std::collections::HashMap;
use std::sync::Arc;
use anyhow::anyhow;
use crate::font::{resolve_fonts, ResolvedFont, ResolvedFonts};
use crate::text::layout::{TextCache, TextLayout};

/// Per-node context handed to Taffy's measure closure for content whose height
/// depends on the final laid-out width — i.e. `wrap:word` text. Only such leaves
/// carry a context; every other node is sized purely from its style.
#[derive(Clone, Debug)]
pub struct MeasureContext {
    /// Raw text of the node, wrapped at measure time against the available width.
    text: String,
    /// The node's resolved font when it is NOT on the cell fast path (GUI only: a proportional
    /// face, another size or line height): the text is then measured by shaping, in px converted
    /// to cells. `None`: whole cells, as always. Part of the reconcile comparison, so a change
    /// of the node's own or an inherited font re-measures it.
    font: Option<ResolvedFont>,
}

/// Measures a `wrap:word` leaf through the shaped text layout (Stage 6.3). Same contract as
/// [`measure_wrap_text`]: min-content is the widest word, max-content the widest paragraph, a
/// definite width resolves to `max.min(avail.max(min))`, and the height comes from the lines at
/// the resolved width. Widths are px / 9.6, heights lines x line height / 20. Returns `None`
/// when shaping is unavailable (the caller falls back to the cell measure).
fn measure_shaped(cache: &mut TextCache, known: Size<Option<f32>>, available: Size<AvailableSpace>, text: &str, font: &ResolvedFont) -> Option<Size<f32>> {
    // `text` is already collapsed like the mono path paints it (see `sync_node`).
    let base = cache.base(text, font)?;
    let (min_c, max_c) = (base.min_content_cells(), base.max_content_cells());
    let width = match known.width {
        Some(w) => w,
        None => match available.width {
            AvailableSpace::MinContent => min_c,
            AvailableSpace::MaxContent => max_c,
            AvailableSpace::Definite(aw) => max_c.min(aw.max(min_c)),
        },
    };
    let height = match known.height {
        Some(h) => h,
        None => cache.at_width(&base, width).height_cells(),
    };
    Some(Size { width, height })
}

/// The Taffy measure closure body.
fn measure_node(cache: &mut TextCache, known: Size<Option<f32>>, available: Size<AvailableSpace>, ctx: &MeasureContext) -> Size<f32> {
    if let Some(font) = &ctx.font {
        if let Some(size) = measure_shaped(cache, known, available, &ctx.text, font) {
            return size;
        }
    }
    measure_wrap_text(known, available, &ctx.text)
}

/// How a node's final shaped layout is found after the solve (see `LayoutEngine::shaped_text`).
enum ShapedRef {
    /// `wrap:none` text or button: no wrapping, the unwrapped layout.
    Fixed(Arc<TextLayout>),
    /// `wrap:word` text: shaped again (a cache hit) at the final content width.
    Word,
}

/// Total display columns of the widest hard line (before any word wrapping).
fn text_intrinsic_width(text: &str) -> u16 {
    text.lines()
        .map(|line| line.chars().map(|c| crate::render::unicode::UnicodeWidthCache::get().width(c) as u16).sum())
        .max()
        .unwrap_or(0)
}

/// Columns of the widest single word — the narrowest a word-wrapped block can get,
/// since words are never broken mid-word. Acts as the min-content width.
fn longest_word_width(text: &str) -> u16 {
    text.split_whitespace()
        .map(|w| w.chars().map(|c| crate::render::unicode::UnicodeWidthCache::get().width(c) as u16).sum())
        .max()
        .unwrap_or(0)
}

/// Number of visual lines produced by word-wrapping `text` to `max_w` columns.
/// Mirrors the draw-time wrapping in `render::renderer` so layout height and the
/// rendered line count always agree.
fn word_wrapped_line_count(text: &str, max_w: u16) -> usize {
    let mut total_lines = 0;
    for line in text.lines() {
        if line.is_empty() {
            total_lines += 1;
            continue;
        }
        let mut current_line_width = 0;
        let mut line_has_words = false;

        for word in line.split_whitespace() {
            let word_w = word.chars()
                .map(|c| crate::render::unicode::UnicodeWidthCache::get().width(c) as u16)
                .sum::<u16>();

            if !line_has_words {
                current_line_width = word_w;
                line_has_words = true;
                total_lines += 1;
            } else {
                let space_w = 1;
                if current_line_width + space_w + word_w <= max_w {
                    current_line_width += space_w + word_w;
                } else {
                    current_line_width = word_w;
                    total_lines += 1;
                }
            }
        }
        if !line_has_words {
            total_lines += 1;
        }
    }
    total_lines
}

/// Taffy measure closure body for a `wrap:word` text leaf: resolves the node's
/// width against the available space (clamped between the longest word and the
/// full intrinsic width) and derives its height from that final width.
fn measure_wrap_text(known: Size<Option<f32>>, available: Size<AvailableSpace>, text: &str) -> Size<f32> {
    let intrinsic = text_intrinsic_width(text) as f32;
    let min_content = longest_word_width(text) as f32;

    let width = match known.width {
        Some(w) => w,
        None => match available.width {
            AvailableSpace::MinContent => min_content,
            AvailableSpace::MaxContent => intrinsic,
            // Shrink toward the available width, but never below the longest word
            // (which cannot be broken) nor above the full intrinsic width.
            AvailableSpace::Definite(aw) => intrinsic.min(aw.max(min_content)),
        },
    };

    let height = match known.height {
        Some(h) => h,
        None => {
            let w_u = width.round().max(0.0) as u16;
            let lines = if w_u >= 1 {
                word_wrapped_line_count(text, w_u)
            } else {
                text.lines().count()
            };
            lines.max(1) as f32
        }
    };

    Size { width, height }
}

/// Phase boundaries reported to the probe of [`LayoutEngine::compute_probed`].
#[doc(hidden)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayoutPhase {
    /// Old Taffy tree dropped and node map cleared.
    Reset,
    /// Taffy tree rebuilt from the DOM (styles mapped, root style set).
    Build,
    /// `compute_layout_with_measure` finished.
    Solve,
    /// Rects flattened, paint order built, `last_layout` stored.
    Finish,
}

/// Engine managing layout calculations and synchronization with the Taffy trees.
pub struct LayoutEngine {
    /// Internal Taffy tree used for layout calculations.
    pub taffy: TaffyTree<MeasureContext>,
    /// Mapping of active DOM node IDs to their corresponding Taffy tree node IDs.
    node_map: HashMap<OxiNodeId, taffy::NodeId>,
    /// The cached result of the last layout calculation run.
    pub last_layout: Option<LayoutResult>,
    /// Whether style lengths and computed rectangles are snapped to whole cells.
    mode: RenderMode,
    /// Viewport height in rows. A root without an explicit height gets this as its
    /// minimum height (it still grows with its content); `None` sizes it to content.
    viewport_rows: Option<u16>,
    /// Debug cross-check: every `compute` is verified against a from-scratch layout of a
    /// clone of the document and panics on any difference. On by default in debug builds.
    cross_check: bool,
    /// Whether the kept tree is reconciled (default) or rebuilt on every call (benchmark
    /// baseline, `set_incremental(false)`).
    incremental: bool,
    /// Available space of the last successful solve on the kept tree (`None`: no valid solve).
    last_avail: Option<Size<AvailableSpace>>,
    /// Frame counters (see [`LayoutStats`]); hidden test/benchmark hook.
    stats: LayoutStats,
    /// Effective fonts of the document, resolved once per `compute` (GUI only; TUI never
    /// shapes, so there it stays uniform).
    fonts: ResolvedFonts,
    /// Shaped text layouts shared by measuring and (from 6.4) painting. Bounded; cleared
    /// between frames only.
    text_cache: TextCache,
    /// Nodes with a non-default font collected by the current `sync_node` pass.
    shaped_refs: Vec<(OxiNodeId, ShapedRef)>,
    /// The final shaped layout of every text leaf and button measured by shaping, at its final
    /// content width, filled after each solve. See [`LayoutEngine::shaped_text`].
    shaped: HashMap<OxiNodeId, Arc<TextLayout>>,
}

/// Counters of how `compute` calls were served (stage 5.1b); hidden test/benchmark hook.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LayoutStats {
    /// Calls that rebuilt the Taffy tree from scratch.
    pub full: u64,
    /// Calls that reconciled the kept tree.
    pub incremental: u64,
    /// Calls verified against a from-scratch layout.
    pub cross_checked: u64,
    /// Of the incremental calls, those that changed nothing in the Taffy tree.
    pub quiet: u64,
}

impl LayoutEngine {
    /// Creates a new layout engine with an empty Taffy tree.
    pub fn new() -> Self {
        Self::with_mode(RenderMode::Tui)
    }

    /// Creates a layout engine for the given render mode.
    ///
    /// `Tui` snaps lengths and rectangles to whole cells; `Gui` keeps them fractional.
    pub fn with_mode(mode: RenderMode) -> Self {
        Self {
            taffy: Self::new_tree(mode),
            node_map: HashMap::new(),
            last_layout: None,
            mode,
            viewport_rows: None,
            cross_check: cfg!(debug_assertions),
            incremental: true,
            last_avail: None,
            stats: LayoutStats::default(),
            fonts: ResolvedFonts::default(),
            text_cache: TextCache::default(),
            shaped_refs: Vec::new(),
            shaped: HashMap::new(),
        }
    }

    /// Sets the viewport height used as the root's minimum height (see `compute`).
    pub fn set_viewport_rows(&mut self, rows: u16) {
        self.viewport_rows = Some(rows);
    }

    /// Enables or disables the from-scratch cross-check of every `compute` (default: on in
    /// debug builds, off in release).
    #[doc(hidden)]
    pub fn set_cross_check(&mut self, on: bool) {
        self.cross_check = on;
    }

    /// Turns the kept-tree reconcile off (`false`: every `compute` rebuilds, the pre-5.1b
    /// behaviour) or on. Benchmark baseline and escape hatch.
    #[doc(hidden)]
    pub fn set_incremental(&mut self, on: bool) {
        self.incremental = on;
    }

    /// How `compute` calls were served so far.
    #[doc(hidden)]
    pub fn stats(&self) -> LayoutStats {
        self.stats
    }

    /// The mode this engine lays out in.
    pub fn mode(&self) -> RenderMode {
        self.mode
    }

    /// Builds a Taffy tree for `mode`. Taffy's own pixel rounding stays on for TUI (it
    /// rounds by absolute position, which the cell snapshots rely on) and is disabled
    /// for GUI so fractional sizes survive.
    fn new_tree(mode: RenderMode) -> TaffyTree<MeasureContext> {
        let mut tree = TaffyTree::new();
        if mode == RenderMode::Gui {
            tree.disable_rounding();
        }
        tree
    }

    /// The one place style lengths are snapped: in TUI mode a style length becomes a whole
    /// number of cells (`f32::round`); in GUI mode it is passed through unchanged.
    fn snap(&self, v: f32) -> f32 {
        if self.mode == RenderMode::Tui { v.round() } else { v }
    }

    /// [`Self::snap`] for lengths that cannot be negative (sizes, padding, margin).
    fn snap_size(&self, v: f32) -> f32 {
        self.snap(v).max(0.0)
    }

    /// Resets the engine, discarding the current Taffy tree and node mappings.
    ///
    /// Required after document defragmentation (compaction) to prevent stale ID lookups.
    pub fn reset_nodes(&mut self) {
        self.taffy = Self::new_tree(self.mode);
        self.node_map.clear();
        self.last_layout = None;
        self.last_avail = None;
        self.shaped.clear();
        self.shaped_refs.clear();
    }

    /// The shaped layout of a text leaf or button at its final width, for nodes whose font is
    /// not on the cell fast path (`None` for every other node, and in TUI mode). It is the very
    /// layout the node was measured with: the display-list builder (6.4) must paint from it and
    /// never shape again from the rect. Valid until the next `compute`.
    pub fn shaped_text(&self, id: OxiNodeId) -> Option<&Arc<TextLayout>> {
        self.shaped.get(&id)
    }

    /// Number of nodes that currently have a shaped layout (hidden test hook).
    #[doc(hidden)]
    pub fn shaped_count(&self) -> usize {
        self.shaped.len()
    }

    /// (layouts, glyphs) held by the text cache (hidden test/diagnostics hook).
    #[doc(hidden)]
    #[cfg(feature = "text-shaping")]
    pub fn text_cache_stats(&self) -> (usize, usize) {
        (self.text_cache.len(), self.text_cache.glyph_count())
    }

    /// Builds `shaped` from the nodes collected during the sync, after the solve.
    fn collect_shaped(&mut self) {
        self.shaped.clear();
        for (oxi, r) in std::mem::take(&mut self.shaped_refs) {
            let layout = match r {
                ShapedRef::Fixed(l) => Some(l),
                ShapedRef::Word => self.final_word_layout(oxi),
            };
            if let Some(l) = layout {
                self.shaped.insert(oxi, l);
            }
        }
    }

    /// The layout of a `wrap:word` leaf at the content width Taffy gave it (rect minus padding
    /// and border), through the same guard function the measure used.
    fn final_word_layout(&mut self, oxi: OxiNodeId) -> Option<Arc<TextLayout>> {
        let t = *self.node_map.get(&oxi)?;
        let style = self.taffy.style(t).ok()?;
        if style.display == Display::None {
            return None;
        }
        let len = |l: LengthPercentage| match l {
            LengthPercentage::Length(v) => v,
            _ => 0.0,
        };
        let (p, b) = (style.padding, style.border);
        let extra = len(p.left) + len(p.right) + len(b.left) + len(b.right);
        let w = self.taffy.layout(t).ok()?.size.width - extra;
        let ctx = self.taffy.get_node_context(t)?;
        let font = ctx.font?;
        let base = self.text_cache.base(&ctx.text, &font)?; // ctx.text is already collapsed
        Some(self.text_cache.at_width(&base, w.max(0.0)))
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutEngine {
    /// Computes positions and dimensions of all visible elements in the document.
    ///
    /// The `_scroll_offset` parameter is kept for signature compatibility but layout
    /// coordinates remain independent of view scrolling (which is resolved at draw time).
    ///
    /// # Errors
    ///
    /// Returns an error if Taffy fails to build or calculate styles inside the tree.
    pub fn compute(&mut self, doc: &mut THTMLDocument, cols: u16, _scroll_offset: u16, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>) -> anyhow::Result<LayoutResult> {
        self.compute_probed(doc, cols, state_evaluator, |_| {})
    }

    /// [`Self::compute`] with a phase probe, for the frame-cost benchmark (stage 5.0).
    ///
    /// `probe` is called at the end of each [`LayoutPhase`]; the caller reads its own clock
    /// there (this crate never touches `Instant`, it must build for wasm32). `compute` passes
    /// an empty closure, so after monomorphization the probe calls vanish: zero cost and no
    /// behaviour change.
    #[doc(hidden)]
    pub fn compute_probed<P: FnMut(LayoutPhase)>(&mut self, doc: &mut THTMLDocument, cols: u16, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>, mut probe: P) -> anyhow::Result<LayoutResult> {
        // Stage 5.1b: the Taffy tree is kept between calls and reconciled against the document.
        // The reconcile compares EVERY node's inputs to Taffy (mapped style, measure context,
        // child list) with what Taffy already holds and touches only what differs, so it does
        // not depend on the document's dirty list (many mutations, e.g. bound text and
        // animated styles, never reach it). Whatever it cannot prove (an unreachable stale
        // node, an error) falls back to a full rebuild.
        let mut changed = false;
        let mut incremental = false;
        self.text_cache.begin_frame();
        // Fonts only matter in GUI mode; TUI measures whole cells and skips the pass entirely.
        self.fonts = if self.mode == RenderMode::Gui { resolve_fonts(doc, self.mode) } else { ResolvedFonts::default() };
        self.shaped_refs.clear();
        if self.incremental && !self.node_map.is_empty() {
            let mut visited = 0usize;
            let mut bail = false;
            match self.sync_node(doc, doc.root, cols, state_evaluator, &mut visited, &mut changed, &mut bail) {
                Ok(_) if !bail && visited == self.node_map.len() => incremental = true,
                _ => {}
            }
        }
        if incremental {
            self.stats.incremental += 1;
            probe(LayoutPhase::Reset);
        } else {
            self.stats.full += 1;
            self.reset_nodes();
            self.shaped_refs.clear();
            probe(LayoutPhase::Reset);
            let mut visited = 0usize;
            let mut bail = false;
            self.sync_node(doc, doc.root, cols, state_evaluator, &mut visited, &mut changed, &mut bail)?;
        }

        let root_taffy_id = *self.node_map.get(&doc.root).unwrap();

        let mut root_w = cols as f32;
        let mut root_h_available = AvailableSpace::MaxContent;
        if let Some(root_node) = doc.arena.get(doc.root) {
            if let Some(w) = root_node.style.width {
                root_w = self.snap_size(w);
            }
            if let Some(h) = root_node.style.height {
                root_h_available = AvailableSpace::Definite(self.snap_size(h));
            }
        }

        probe(LayoutPhase::Build);

        let available_space = Size {
            width: AvailableSpace::Definite(root_w),
            height: root_h_available,
        };

        // A kept Taffy cache is NOT equivalent to a rebuild. Observed (fuzz, examples/index
        // in GUI mode, after a viewport height change, a flex-direction flip and an append):
        // every style, child list, parent link and measure context matched a fresh tree,
        // marking every node dirty and re-solving gave the rebuild's result, yet the
        // incrementally dirtied solve left stale positions. Whether the cause is Taffy's
        // size-matching cache heuristic or a dirty-propagation gap was not isolated. So the
        // kept tree is solved from an empty cache whenever anything changed (a rebuild minus
        // the node allocations), and the cache is kept only when NOTHING changed since the
        // last solve, where a hit returns that very result.
        let avail_same = self.last_avail.as_ref() == Some(&available_space);
        self.last_avail = None;
        if incremental && !changed && avail_same {
            self.stats.quiet += 1;
        }
        if incremental && (changed || !avail_same) {
            let leaves: Vec<taffy::NodeId> = self.node_map.values().copied().filter(|&t| self.taffy.child_count(t) == 0).collect();
            for t in leaves {
                self.taffy.mark_dirty(t)?;
            }
        }

        // Measure closure resolves height-from-width for `wrap:word` text leaves
        // (the only nodes given a MeasureContext); all other nodes fall back to Zero
        // and are sized purely from their style.
        let cache = &mut self.text_cache;
        self.taffy.compute_layout_with_measure(
            root_taffy_id,
            available_space,
            |known, avail, _node_id, ctx, _style| match ctx {
                Some(ctx) => measure_node(cache, known, avail, ctx),
                None => Size::ZERO,
            },
        )?;
        self.collect_shaped();

        self.last_avail = Some(available_space);

        probe(LayoutPhase::Solve);

        let mut nodes = HashMap::new();
        self.flatten_layout_recursive(doc, doc.root, 0.0, 0.0, &mut nodes)?;

        doc.clear_dirty();

        let mut max_bottom = 0.0f32;
        for (&node_id, rect) in &nodes {
            if node_id == doc.root {
                continue;
            }
            if let Some(node) = doc.arena.get(node_id) {
                if node.style.position == oxiterm_proto::style::Position::Fixed {
                    continue;
                }
            }
            let bottom = rect.y + rect.height;
            if bottom > max_bottom {
                max_bottom = bottom;
            }
        }

        let mut paint_order = Vec::with_capacity(nodes.len());
        collect_paint_order_recursive(doc, doc.root, &nodes, &mut paint_order);

        let result = LayoutResult { nodes, paint_order, // Scroll is still cell-based, so the extent stays whole cells.
            total_height: max_bottom.ceil() as u16, shaped: self.shaped.clone() };
        self.last_layout = Some(result.clone());
        probe(LayoutPhase::Finish);
        if self.cross_check {
            self.stats.cross_checked += 1;
            self.verify_against_rebuild(doc, cols, state_evaluator, &result);
        }
        Ok(result)
    }

    /// Re-runs only the Taffy solve on the tree built by the last `compute` (benchmark hook,
    /// stage 5.0). Measures the incremental-layout best case: no rebuild, Taffy's own cache
    /// decides how much work is left. `dirty_text_leaf` optionally dirties one node first.
    /// Returns false if there is no tree yet.
    #[doc(hidden)]
    pub fn resolve_only(&mut self, doc: &THTMLDocument, cols: u16, dirty: Option<OxiNodeId>) -> bool {
        let Some(&root) = self.node_map.get(&doc.root) else { return false };
        self.last_avail = None;
        if let Some(d) = dirty.and_then(|d| self.node_map.get(&d).copied()) {
            let _ = self.taffy.mark_dirty(d);
        }
        // Same root constraints as `compute`.
        let mut root_w = cols as f32;
        let mut root_h = AvailableSpace::MaxContent;
        if let Some(n) = doc.arena.get(doc.root) {
            if let Some(w) = n.style.width { root_w = self.snap_size(w); }
            if let Some(h) = n.style.height { root_h = AvailableSpace::Definite(self.snap_size(h)); }
        }
        let avail = Size { width: AvailableSpace::Definite(root_w), height: root_h };
        let cache = &mut self.text_cache;
        self.taffy.compute_layout_with_measure(root, avail, |known, avail, _n, ctx, _s| match ctx {
            Some(ctx) => measure_node(cache, known, avail, ctx),
            None => Size::ZERO,
        }).is_ok()
    }

    fn flatten_layout_recursive(
        &self,
        doc: &THTMLDocument,
        oxi_id: OxiNodeId,
        parent_x: f32,
        parent_y: f32,
        nodes: &mut HashMap<OxiNodeId, OxiRect>,
    ) -> anyhow::Result<()> {
        if let Some(&taffy_id) = self.node_map.get(&oxi_id) {
            let is_visible = if let Ok(style) = self.taffy.style(taffy_id) {
                style.display != Display::None
            } else {
                true
            };

            if is_visible {
                let layout = self.taffy.layout(taffy_id)
                    .map_err(|e| anyhow!("Taffy layout missing for node {oxi_id:?}: {e:?}"))?;
                
                // TUI snaps to whole cells here exactly as before; GUI keeps fractions
                // (sizes are still clamped to be non-negative).
                let (rect_x, rect_y) = if self.mode == RenderMode::Tui {
                    (layout.location.x.round().max(0.0), layout.location.y.round().max(0.0))
                } else {
                    (layout.location.x, layout.location.y)
                };
                let (width, height) = if self.mode == RenderMode::Tui {
                    (layout.size.width.round().max(0.0), layout.size.height.round().max(0.0))
                } else {
                    (layout.size.width.max(0.0), layout.size.height.max(0.0))
                };
                
                let (abs_x, abs_y) = if let Some(node) = doc.arena.get(oxi_id) {
                    if node.style.position == oxiterm_proto::style::Position::Fixed {
                        (rect_x, rect_y)
                    } else {
                        (parent_x + rect_x, parent_y + rect_y)
                    }
                } else {
                    (parent_x + rect_x, parent_y + rect_y)
                };
                
                nodes.insert(oxi_id, OxiRect {
                    x: abs_x,
                    y: abs_y,
                    width,
                    height,
                });

                if let Some(node) = doc.arena.get(oxi_id) {
                    for &child_id in &node.children {
                        self.flatten_layout_recursive(doc, child_id, abs_x, abs_y, nodes)?;
                    }
                }
            }
        }
        Ok(())
    }
}

fn collect_paint_order_recursive(
    doc: &THTMLDocument,
    node_id: OxiNodeId,
    nodes: &HashMap<OxiNodeId, OxiRect>,
    paint_order: &mut Vec<OxiNodeId>,
) {
    if !nodes.contains_key(&node_id) {
        return;
    }

    paint_order.push(node_id);

    if let Some(node) = doc.arena.get(node_id) {
        let mut children_sorted = node.children.clone();
        children_sorted.sort_by(|&a_id, &b_id| {
            let a_z = doc.arena.get(a_id).and_then(|n| n.style.z_index).unwrap_or(0);
            let b_z = doc.arena.get(b_id).and_then(|n| n.style.z_index).unwrap_or(0);
            a_z.cmp(&b_z)
        });

        for child_id in children_sorted {
            collect_paint_order_recursive(doc, child_id, nodes, paint_order);
        }
    }
}

impl LayoutEngine {

    /// Queries the computed layout to find which node lies at coordinate (x, y).
    ///
    /// Respects stacking contexts and visual paint order (`z-index`), selecting
    /// the top-most visible element covering the coordinates.
    pub fn hit_test(&self, x: u16, y: u16) -> Option<OxiNodeId> {
        let layout = self.last_layout.as_ref()?;
        // Cell (col, row) is probed at its centre.
        crate::layout::types::HitTester::new(layout).find_node(x as f32 + 0.5, y as f32 + 0.5)
    }

    /// The Taffy style of `node`: `map_style`, plus the root-only overrides (explicit width
    /// or the viewport columns, auto height with the viewport rows as minimum).
    fn style_for(&self, doc: &THTMLDocument, id: OxiNodeId, node: &oxiterm_proto::dom::Node, cols: u16, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>, shaped: Option<(f32, f32)>) -> Style {
        let mut style = self.map_style(node, state_evaluator, shaped);
        if id == doc.root {
            if node.style.width.is_none() {
                style.size.width = taffy::style::Dimension::Length(cols as f32);
            }
            if node.style.height.is_none() {
                style.size.height = taffy::style::Dimension::Auto;
                if let Some(rows) = self.viewport_rows {
                    style.min_size.height = taffy::style::Dimension::Length(f32::from(rows));
                }
            }
        }
        style
    }

    /// The measure-context text of `node` (only `wrap:word` text leaves have one).
    fn measure_text(node: &oxiterm_proto::dom::Node) -> Option<&str> {
        if node.tag == oxiterm_proto::dom::NodeTag::Text && node.style.wrap == oxiterm_proto::style::WrapMode::Word {
            node.text.as_deref()
        } else {
            None
        }
    }

    /// Brings the Taffy node for `oxi_id` (and its subtree) in line with the document,
    /// creating it when unknown and otherwise updating only what differs. Sets `changed`
    /// when anything in the Taffy tree was touched. `visited` counts reached nodes.
    fn sync_node(
        &mut self,
        doc: &THTMLDocument,
        oxi_id: OxiNodeId,
        cols: u16,
        state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>,
        visited: &mut usize,
        changed: &mut bool,
        bail: &mut bool,
    ) -> anyhow::Result<taffy::NodeId> {
        let node = doc.arena.get(oxi_id).ok_or_else(|| anyhow::anyhow!("Node not found"))?;
        let existing = self.node_map.get(&oxi_id).copied();
        if existing.is_some() {
            // A node reached twice (shared child) cannot be reconciled twice: force a rebuild.
            *visited += 1;
        }

        let mut child_ids: Vec<taffy::NodeId> = Vec::with_capacity(node.children.len());
        for &child_id in &node.children {
            child_ids.push(self.sync_node(doc, child_id, cols, state_evaluator, visited, changed, bail)?);
        }

        // Stage 6.3: a node whose font is not the root default (GUI only) is measured by shaping.
        let font = self.fonts.get(oxi_id);
        let shaped_font = self.mode == RenderMode::Gui && !font.uses_cell_metrics();
        // A shaped `wrap:word` node keeps the text the mono path would paint (whitespace runs
        // collapsed, once per frame here instead of on every measure call) in its context.
        let collapsed;
        let ctx_text = match Self::measure_text(node) {
            Some(t) if shaped_font => {
                collapsed = crate::text::collapse_word_whitespace(t);
                Some(&*collapsed)
            }
            other => other,
        };
        let ctx_font = if shaped_font && ctx_text.is_some() { Some(font) } else { None };
        if ctx_font.is_some() {
            self.shaped_refs.push((oxi_id, ShapedRef::Word));
        }
        // `wrap:none` text and buttons pin their size in the style: the shaped max-content width
        // and line count replace the cell intrinsic size (the style compare then sees a change).
        let mut shaped_dims = None;
        if shaped_font
            && (node.tag == oxiterm_proto::dom::NodeTag::Button || (node.tag == oxiterm_proto::dom::NodeTag::Text && ctx_text.is_none()))
        {
            if let Some(text) = node.text.as_deref() {
                if let Some(base) = self.text_cache.base(text, &font) {
                    shaped_dims = Some((base.max_content_cells(), base.height_cells()));
                    self.shaped_refs.push((oxi_id, ShapedRef::Fixed(base)));
                }
            }
        }
        let style = self.style_for(doc, oxi_id, node, cols, state_evaluator, shaped_dims);

        // Taffy's `set_children`/`new_with_children` rewrite parent links unconditionally, so a
        // node adopted by a new parent while its old parent still lists it (moved subtree,
        // reused arena id) would lose its dirty propagation. Never reconcile that: rebuild.
        if child_ids.iter().any(|&c| self.taffy.parent(c).is_some()) {
            let adopter = existing;
            if child_ids.iter().any(|&c| self.taffy.parent(c).is_some_and(|p| Some(p) != adopter)) {
                *bail = true;
            }
        }

        let Some(taffy_id) = existing else {
            let taffy_id = self.taffy.new_with_children(style, &child_ids)?;
            if let Some(text) = ctx_text {
                self.taffy.set_node_context(taffy_id, Some(MeasureContext { text: text.to_owned(), font: ctx_font }))?;
            }
            self.node_map.insert(oxi_id, taffy_id);
            *visited += 1;
            *changed = true;
            return Ok(taffy_id);
        };

        if *self.taffy.style(taffy_id)? != style {
            self.taffy.set_style(taffy_id, style)?;
            *changed = true;
        }
        let same_ctx = match (self.taffy.get_node_context(taffy_id), ctx_text) {
            (None, None) => true,
            (Some(c), Some(t)) => c.text == t && c.font == ctx_font,
            _ => false,
        };
        if !same_ctx {
            self.taffy.set_node_context(taffy_id, ctx_text.map(|t| MeasureContext { text: t.to_owned(), font: ctx_font }))?;
            *changed = true;
        }
        let same_children = self.taffy.child_count(taffy_id) == child_ids.len()
            && child_ids.iter().enumerate().all(|(i, &c)| self.taffy.child_at_index(taffy_id, i).ok() == Some(c));
        if !same_children {
            self.taffy.set_children(taffy_id, &child_ids)?;
            *changed = true;
        }
        Ok(taffy_id)
    }

    /// Debug cross-check: lays `doc` out from scratch on a fresh engine and panics unless the
    /// result is bit-identical to `got`, naming the first differing node.
    fn verify_against_rebuild(&self, doc: &THTMLDocument, cols: u16, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>, got: &LayoutResult) {
        let mut fresh = LayoutEngine::with_mode(self.mode);
        fresh.cross_check = false;
        fresh.viewport_rows = self.viewport_rows;
        let mut scratch = doc.clone();
        let want = fresh.compute(&mut scratch, cols, 0, state_evaluator).expect("cross-check rebuild failed");
        let bits = |r: &OxiRect| [r.x.to_bits(), r.y.to_bits(), r.width.to_bits(), r.height.to_bits()];
        for (id, w) in &want.nodes {
            match got.nodes.get(id) {
                Some(g) if bits(g) == bits(w) => {}
                other => panic!("incremental layout differs from rebuild at node {id:?}: incremental {other:?}, rebuild {w:?}"),
            }
        }
        if let Some(id) = got.nodes.keys().find(|id| !want.nodes.contains_key(id)) {
            panic!("incremental layout has node {id:?} that the rebuild does not");
        }
        assert_eq!(self.shaped.len(), fresh.shaped.len(), "incremental shaped-text node set differs from rebuild");
        for (id, w) in &fresh.shaped {
            match self.shaped.get(id) {
                Some(g) if **g == **w => {}
                other => panic!("incremental shaped text differs from rebuild at node {id:?}: incremental {other:?}, rebuild {w:?}"),
            }
        }
        assert_eq!(got.paint_order, want.paint_order, "incremental paint order differs from rebuild");
        assert_eq!(got.total_height, want.total_height, "incremental total_height differs from rebuild");
    }

    /// `shaped`: for a `wrap:none` text or button in a non-default font, the shaped
    /// (max-content width, height) in cells; it replaces the cell intrinsic size.
    fn map_style(&self, node: &oxiterm_proto::dom::Node, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>, shaped: Option<(f32, f32)>) -> Style {
        let style = &node.style;
        let mut width = style.width.map(|w| self.snap_size(w));
        let mut height = style.height.map(|h| self.snap_size(h));
        // Intrinsic glyph width used as a shrink floor for non-wrapping text, set ONLY
        // when the width was auto-derived from text (not when the author gave a width).
        let mut min_width: Option<f32> = None;
        // Whether this node's size is resolved by the Taffy measure closure (wrap:word
        // text). Such nodes keep only their explicit style dimensions here; the closure
        // fills in the rest and must not be overridden by cross-axis stretch.
        let mut measured_text = false;

        if node.tag == oxiterm_proto::dom::NodeTag::Text || node.tag == oxiterm_proto::dom::NodeTag::Button {
            if node.text.is_some() {
                if style.wrap == oxiterm_proto::style::WrapMode::Word && node.tag == oxiterm_proto::dom::NodeTag::Text {
                    // Height depends on the FINAL wrapped width, which is only known
                    // after flex resolution — so defer both dimensions to the measure
                    // closure (see `measure_wrap_text`). Explicit author width/height,
                    // if any, still flow through `size` below as known constraints.
                    measured_text = true;
                } else {
                    // wrap:none — content-sized. Derive the intrinsic width and pin it as
                    // a min-width floor so a narrow flex row (default flex_shrink=1) cannot
                    // compress the node below its glyph span, which would leave only the
                    // first column hit-testable/clickable (e.g. the "← Back" link).
                    let text = node.text.as_deref().unwrap_or("");
                    if width.is_none() {
                        let mut calculated_width = match shaped {
                            Some((w, _)) => w,
                            None => text_intrinsic_width(text) as f32,
                        };
                        if node.tag == oxiterm_proto::dom::NodeTag::Button {
                            if style.border.is_some() {
                                calculated_width += 2.0;
                            }
                            calculated_width += self.snap_size(style.padding.left) + self.snap_size(style.padding.right);
                        }
                        width = Some(calculated_width);
                        min_width = Some(calculated_width);
                    }
                    if height.is_none() {
                        let mut calculated_height = match shaped {
                            Some((_, h)) => h,
                            None => (text.lines().count() as f32).max(1.0),
                        };
                        if node.tag == oxiterm_proto::dom::NodeTag::Button {
                            if style.border.is_some() {
                                calculated_height += 2.0;
                            }
                            calculated_height += self.snap_size(style.padding.top) + self.snap_size(style.padding.bottom);
                        }
                        height = Some(calculated_height);
                    }
                }
            }
        }

        let mut display = Display::Flex;
        if let Some(cond) = &node.attrs.bind_show {
            if let Some(eval) = state_evaluator {
                if !eval.evaluate_bind_show(cond) {
                    display = Display::None;
                }
            }
            // Keep visible during static previews/playground evaluations if state evaluator is absent.
        }

        let (flex_grow, flex_shrink, flex_basis) = match style.flex {
            Some(val) => (val, 1.0, Dimension::Length(0.0)),
            None => (0.0, 1.0, Dimension::Auto),
        };

        let min_height: Dimension = if node.tag == oxiterm_proto::dom::NodeTag::Img || node.tag == oxiterm_proto::dom::NodeTag::Video {
            height.map(|h| Dimension::Length(h)).unwrap_or(Dimension::Auto)
        } else {
            Dimension::Auto
        };

        let final_min_width: Dimension = if node.tag == oxiterm_proto::dom::NodeTag::Img || node.tag == oxiterm_proto::dom::NodeTag::Video {
            width.map(|w| Dimension::Length(w)).unwrap_or(Dimension::Auto)
        } else {
            min_width.map(|w| Dimension::Length(w)).unwrap_or(Dimension::Auto)
        };

        Style {
            display,
            flex_grow,
            flex_shrink,
            flex_basis,
            flex_direction: match style.flex_direction {
                oxiterm_proto::style::FlexDirection::Row => FlexDirection::Row,
                oxiterm_proto::style::FlexDirection::Column => FlexDirection::Column,
            },
            align_items: Some(match style.align_items {
                oxiterm_proto::style::AlignItems::FlexStart => AlignItems::FlexStart,
                oxiterm_proto::style::AlignItems::FlexEnd => AlignItems::FlexEnd,
                oxiterm_proto::style::AlignItems::Center => AlignItems::Center,
                oxiterm_proto::style::AlignItems::Stretch => AlignItems::Stretch,
            }),
            justify_content: Some(match style.justify_content {
                oxiterm_proto::style::JustifyContent::FlexStart => JustifyContent::FlexStart,
                oxiterm_proto::style::JustifyContent::FlexEnd => JustifyContent::FlexEnd,
                oxiterm_proto::style::JustifyContent::Center => JustifyContent::Center,
                oxiterm_proto::style::JustifyContent::SpaceBetween => JustifyContent::SpaceBetween,
                oxiterm_proto::style::JustifyContent::SpaceAround => JustifyContent::SpaceAround,
            }),
            align_self: if measured_text { Some(AlignItems::Start) } else { None },
            size: Size {
                width: width.map(|w| Dimension::Length(w)).unwrap_or(Dimension::Auto),
                height: height.map(|h| Dimension::Length(h)).unwrap_or(Dimension::Auto),
            },
            min_size: Size {
                width: final_min_width,
                height: min_height,
            },
            padding: Rect {
                left: LengthPercentage::Length(self.snap_size(style.padding.left)),
                right: LengthPercentage::Length(self.snap_size(style.padding.right)),
                top: LengthPercentage::Length(self.snap_size(style.padding.top)),
                bottom: LengthPercentage::Length(self.snap_size(style.padding.bottom)),
            },
            margin: Rect {
                left: LengthPercentage::Length(self.snap_size(style.margin.left)).into(),
                right: LengthPercentage::Length(self.snap_size(style.margin.right)).into(),
                top: LengthPercentage::Length(self.snap_size(style.margin.top)).into(),
                bottom: LengthPercentage::Length(self.snap_size(style.margin.bottom)).into(),
            },
            border: if style.border.is_some() {
                Rect {
                    left: LengthPercentage::Length(1.0),
                    right: LengthPercentage::Length(1.0),
                    top: LengthPercentage::Length(1.0),
                    bottom: LengthPercentage::Length(1.0),
                }
            } else {
                Rect {
                    left: LengthPercentage::Length(0.0),
                    right: LengthPercentage::Length(0.0),
                    top: LengthPercentage::Length(0.0),
                    bottom: LengthPercentage::Length(0.0),
                }
            },
            position: match style.position {
                oxiterm_proto::style::Position::Absolute => Position::Absolute,
                _ => Position::Relative,
            },
            inset: Rect {
                left: style.left.map(|l| LengthPercentageAuto::Length(self.snap(l))).unwrap_or(LengthPercentageAuto::Auto),
                right: style.right.map(|r| LengthPercentageAuto::Length(self.snap(r))).unwrap_or(LengthPercentageAuto::Auto),
                top: style.top.map(|t| LengthPercentageAuto::Length(self.snap(t))).unwrap_or(LengthPercentageAuto::Auto),
                bottom: style.bottom.map(|b| LengthPercentageAuto::Length(self.snap(b))).unwrap_or(LengthPercentageAuto::Auto),
            },
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::THTMLDocument;
    use oxiterm_proto::dom::{Node, NodeTag, StateEvaluator};

    #[test]
    fn test_basic_layout() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(20.0);
        child.style.height = Some(10.0);
        let child_id = doc.arena.alloc(child);
        doc.append_child(doc.root, child_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&child_id).unwrap();
        assert_eq!(rect.width, 20.0);
        assert_eq!(rect.height, 10.0);
    }

    #[test]
    fn test_hit_test() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(10.0);
        child.style.height = Some(10.0);
        child.style.margin.left = 5.0;
        let child_id = doc.arena.alloc(child);
        doc.append_child(doc.root, child_id).unwrap();
        
        engine.compute(&mut doc, 80, 0, None).unwrap();
        
        assert_eq!(engine.hit_test(6, 1), Some(child_id));
        assert_eq!(engine.hit_test(1, 1), Some(doc.root));
    }

    #[test]
    fn test_hit_test_roundtrip_every_cell_maps_back() {
        // Round-trip invariant: every cell a node occupies in the computed layout must
        // hit-test back to that node (the deepest one covering the cell). This locks the
        // layout↔hit-test geometry so a coordinate/offset regression can't slip through.
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        let mut parent = Node::new(NodeTag::Box);
        parent.style.width = Some(30.0);
        parent.style.height = Some(10.0);
        parent.style.margin.left = 5.0;
        parent.style.margin.top = 2.0;
        let parent_id = doc.arena.alloc(parent);
        doc.append_child(doc.root, parent_id).unwrap();

        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(8.0);
        child.style.height = Some(3.0);
        child.style.margin.left = 4.0;
        child.style.margin.top = 2.0;
        let child_id = doc.arena.alloc(child);
        doc.append_child(parent_id, child_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let pr = *result.nodes.get(&parent_id).unwrap();
        let cr = *result.nodes.get(&child_id).unwrap();
        let (pc, cc) = (pr.cells(), cr.cells());
        assert!(cr.width > 0.0 && cr.height > 0.0 && pr.width > 0.0);

        // Every cell of the (deepest) child maps back to the child.
        for y in cc.y..cc.y + cc.height {
            for x in cc.x..cc.x + cc.width {
                assert_eq!(engine.hit_test(x, y), Some(child_id),
                    "cell ({x},{y}) inside the child must hit-test back to it");
            }
        }
        // Every cell of the parent maps to the child where they overlap, else the parent.
        for y in pc.y..pc.y + pc.height {
            for x in pc.x..pc.x + pc.width {
                let expected = if cr.contains(x as f32 + 0.5, y as f32 + 0.5) { child_id } else { parent_id };
                assert_eq!(engine.hit_test(x, y), Some(expected),
                    "cell ({x},{y}) must hit-test to the deepest covering node");
            }
        }
        // A cell outside the parent falls through to the root, never the child.
        assert_ne!(engine.hit_test(0, 0), Some(child_id));
    }

    #[test]
    fn test_hit_test_nested() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut parent = Node::new(NodeTag::Box);
        parent.style.width = Some(20.0);
        parent.style.height = Some(20.0);
        parent.style.margin.left = 10.0;
        let parent_id = doc.arena.alloc(parent);
        doc.append_child(doc.root, parent_id).unwrap();

        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(5.0);
        child.style.height = Some(5.0);
        child.style.margin.left = 5.0;
        let child_id = doc.arena.alloc(child);
        doc.append_child(parent_id, child_id).unwrap();
        
        engine.compute(&mut doc, 80, 0, None).unwrap();
        
        assert_eq!(engine.hit_test(16, 1), Some(child_id));
        assert_eq!(engine.hit_test(11, 1), Some(parent_id));
    }

    #[test]
    fn test_text_node_intrinsic_size() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("Hello\nWorld!".to_string());
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&node_id).unwrap();
        assert_eq!(rect.width, 6.0);
        assert_eq!(rect.height, 2.0);
    }

    struct MockEvaluator {
        val: bool,
    }
    impl StateEvaluator for MockEvaluator {
        fn evaluate_bind_show(&self, _condition: &str) -> bool {
            self.val
        }
    }

    #[test]
    fn test_total_height_includes_border_below_last_text_line() {
        // t1: a bordered box whose bottom border sits a row BELOW its last text line.
        // total_height (the scroll extent) must include that border row, not stop at the
        // text. Box = top border (1) + text line (1) + bottom border (1) = 3 rows.
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        // Fixed-height parent (like a real page's top box) that the bordered box overflows,
        // so the box's bottom border — not the parent — is the document's lowest row.
        let mut parent = Node::new(NodeTag::Box);
        parent.style.flex_direction = oxiterm_proto::style::FlexDirection::Column;
        parent.style.width = Some(40.0);
        parent.style.height = Some(2.0);
        let parent_id = doc.arena.alloc(parent);
        doc.append_child(doc.root, parent_id).unwrap();

        let mut boxed = Node::new(NodeTag::Box);
        boxed.style.border = Some(oxiterm_proto::style::BorderStyle {
            fg: oxiterm_proto::style::AnsiColor::Reset,
            chars: oxiterm_proto::style::BorderChars::single(),
        });
        let box_id = doc.arena.alloc(boxed);
        doc.append_child(parent_id, box_id).unwrap();

        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("hi".to_string());
        let text_id = doc.arena.alloc(text_node);
        doc.append_child(box_id, text_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let text_rect = result.nodes.get(&text_id).unwrap();
        // Box = top border (1) + text (1) + bottom border (1) = 3 rows, overflowing the
        // height-2 parent. The bottom border row sits strictly below the last text line.
        assert!(text_rect.y + text_rect.height <= f32::from(result.total_height) - 1.0,
            "bottom border row must sit below the last text line");
        assert_eq!(result.total_height, 3, "scroll extent must include the bottom border row");
    }

    #[test]
    fn test_root_child_without_height_fills_viewport_not_u16_max() {
        // Regression: a root without an explicit height used to be 65535 rows tall, so a
        // root child without a height stretched to it and the scroll extent read 65510+.
        fn page(inner_h: f32) -> THTMLDocument {
            let mut doc = THTMLDocument::new();
            let outer_id = doc.arena.alloc(Node::new(NodeTag::Box)); // no height
            doc.append_child(doc.root, outer_id).unwrap();
            let mut inner = Node::new(NodeTag::Box);
            inner.style.width = Some(10.0);
            inner.style.height = Some(inner_h);
            let inner_id = doc.arena.alloc(inner);
            doc.append_child(outer_id, inner_id).unwrap();
            doc
        }

        // Short content: the page fills the viewport, no further.
        let mut engine = LayoutEngine::new();
        engine.set_viewport_rows(24);
        let mut doc = page(3.0);
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        assert!(result.total_height <= 24, "extent must not exceed the viewport, got {}", result.total_height);

        // Tall content: the page grows with it.
        let mut engine = LayoutEngine::new();
        engine.set_viewport_rows(24);
        let mut doc = page(30.0);
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(result.total_height, 30, "extent follows content taller than the viewport");

        // No viewport known: sized to content.
        let mut engine = LayoutEngine::new();
        let mut doc = page(3.0);
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(result.total_height, 3);
    }

    #[test]
    fn test_centering_offset_tracks_total_extent_not_first_child() {
        // Regression for the "borders cut off" scroll bug: a first child that is SHORTER
        // than the overflowing content must not seed a vertical centering offset, which
        // would shift the document down and steal that many rows from the scroll range.
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        // First child is only 5 tall, but holds a 20-tall child that overflows it.
        let mut outer = Node::new(NodeTag::Box);
        outer.style.width = Some(40.0);
        outer.style.height = Some(5.0);
        let outer_id = doc.arena.alloc(outer);
        doc.append_child(doc.root, outer_id).unwrap();

        let mut inner = Node::new(NodeTag::Box);
        inner.style.width = Some(40.0);
        inner.style.height = Some(20.0);
        let inner_id = doc.arena.alloc(inner);
        doc.append_child(outer_id, inner_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(result.total_height, 20, "extent is the overflowing child, not the 5-tall first child");

        // Viewport shorter than the content: NO vertical offset (was (10-5)/2 = 2 before).
        let (ox, oy) = result.get_centering_offset(&doc, 80, 10);
        assert_eq!(oy, 0, "overflowing content must not be pushed down");
        assert_eq!(ox, 20, "horizontal centering still tracks the first child width");

        // Viewport taller than the content: center on the TOTAL extent (20), not the child.
        let (_, oy_fits) = result.get_centering_offset(&doc, 80, 30);
        assert_eq!(oy_fits, (30 - 20) / 2);
    }

    #[test]
    fn test_bind_show_layout() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(10.0);
        child.style.height = Some(10.0);
        child.attrs.bind_show = Some("some_cond".to_string());
        let child_id = doc.arena.alloc(child);
        doc.append_child(doc.root, child_id).unwrap();

        let eval_false = MockEvaluator { val: false };
        let result_hidden = engine.compute(&mut doc, 80, 0, Some(&eval_false)).unwrap();
        assert!(result_hidden.nodes.get(&child_id).is_none());

        let eval_true = MockEvaluator { val: true };
        let result_visible = engine.compute(&mut doc, 80, 0, Some(&eval_true)).unwrap();
        assert!(result_visible.nodes.get(&child_id).is_some());
    }

    #[test]
    fn test_27_wrap_word_height() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("aa bb cc".to_string());
        text_node.style.wrap = oxiterm_proto::style::WrapMode::Word;
        text_node.style.width = Some(5.0);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&node_id).unwrap();
        assert_eq!(rect.height, 2.0);
    }

    // Builds a flex Row (width 8) holding a shrinkable spacer Box followed by the
    // "← Back" label. The combined ideal width (6 + 6) exceeds the row, so Taffy must
    // shrink something; the label must stay at its 6-wide glyph span while the spacer
    // absorbs the deficit. Returns (engine, text_id) with layout already computed.
    fn narrow_row_with_back_label(interactive: bool) -> (LayoutEngine, oxiterm_proto::dom::NodeId) {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        let mut row = Node::new(NodeTag::Box);
        row.style.flex_direction = oxiterm_proto::style::FlexDirection::Row;
        row.style.width = Some(8.0); // narrower than spacer(6) + label(6), forces shrink
        row.style.height = Some(1.0);
        let row_id = doc.arena.alloc(row);
        doc.append_child(doc.root, row_id).unwrap();

        let mut spacer = Node::new(NodeTag::Box);
        spacer.style.width = Some(6.0);
        spacer.style.height = Some(1.0);
        let spacer_id = doc.arena.alloc(spacer);
        doc.append_child(row_id, spacer_id).unwrap();

        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("← Back".to_string());
        text_node.style.height = Some(1.0);
        if interactive {
            text_node.attrs.event_htmx = Some("/back".to_string());
        }
        let text_id = doc.arena.alloc(text_node);
        doc.append_child(row_id, text_id).unwrap();

        engine.compute(&mut doc, 80, 0, None).unwrap();
        (engine, text_id)
    }

    #[test]
    fn test_text_intrinsic_width_survives_narrow_flex_row() {
        // t1: "← Back" (auto width) inside a Row narrower than its ideal content must
        // keep its 6-wide glyph span, not be shrunk by Taffy's default flex_shrink=1.
        let (engine, text_id) = narrow_row_with_back_label(false);
        let rect = engine.last_layout.as_ref().unwrap().nodes.get(&text_id).unwrap();
        assert_eq!(rect.width, 6.0, "text node must not shrink below its glyph span");
    }

    #[test]
    fn test_every_column_of_clickable_text_is_hit_testable() {
        // t2: every column across the glyph span resolves to the text node.
        let (engine, text_id) = narrow_row_with_back_label(false);
        let rect = *engine.last_layout.as_ref().unwrap().nodes.get(&text_id).unwrap();
        assert_eq!(rect.width, 6.0);
        for x in rect.cells().x..rect.cells().x + rect.cells().width {
            assert_eq!(
                engine.hit_test(x, rect.cells().y),
                Some(text_id),
                "column {x} should hit the text node"
            );
        }
    }

    #[test]
    fn test_last_column_of_event_text_activates_node() {
        // t3: the last glyph column of an interactive (event-htmx) text node resolves
        // to that node, so a click on the last letter reaches the activation wiring.
        let (engine, text_id) = narrow_row_with_back_label(true);
        let rect = *engine.last_layout.as_ref().unwrap().nodes.get(&text_id).unwrap();
        assert_eq!(rect.width, 6.0);
        assert_eq!(
            engine.hit_test(rect.cells().x + rect.cells().width - 1, rect.cells().y),
            Some(text_id),
            "last column of the link must resolve to the interactive node"
        );
    }

    // Places an auto-width (no explicit width) wrap:word paragraph inside a Box of the
    // given inner width and returns the paragraph's computed rect. The container is a
    // Row (default) with a definite height, so it also exercises that align-items:stretch
    // does NOT blow the measured height up to the container height.
    fn wrapped_paragraph_rect(container_width: u16, container_height: u16, text: &str) -> OxiRect {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        let mut container = Node::new(NodeTag::Box);
        container.style.width = Some(f32::from(container_width));
        container.style.height = Some(f32::from(container_height));
        let container_id = doc.arena.alloc(container);
        doc.append_child(doc.root, container_id).unwrap();

        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some(text.to_string());
        text_node.style.wrap = oxiterm_proto::style::WrapMode::Word;
        let text_id = doc.arena.alloc(text_node);
        doc.append_child(container_id, text_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        *result.nodes.get(&text_id).unwrap()
    }

    #[test]
    fn test_wrap_word_auto_width_wraps_to_container_and_grows_tall() {
        // THE bug: an auto-width wrap:word paragraph used to derive height from its full
        // unwrapped width and freeze at height=1, never wrapping. It must now narrow to
        // the container and grow to the wrapped line count.
        // "alpha beta gamma delta" at width 10 wraps to: "alpha beta" / "gamma" / "delta".
        let rect = wrapped_paragraph_rect(10, 4, "alpha beta gamma delta");
        assert_eq!(rect.width, 10.0, "must narrow to the container width, not stay at 22");
        assert_eq!(rect.height, 3.0, "height must follow the wrapped line count, not be 1");
    }

    #[test]
    fn test_wrap_word_height_follows_final_width() {
        // Narrower available width => more wrapped lines. Same text, width 6:
        // "alpha" / "beta" / "gamma" / "delta" => 4 lines (no two 5/4-wide words fit in 6
        // together since 5+1+4=10 > 6).
        let rect = wrapped_paragraph_rect(6, 6, "alpha beta gamma delta");
        assert_eq!(rect.width, 6.0);
        assert_eq!(rect.height, 4.0, "height must recompute from the narrower final width");
    }

    #[test]
    fn test_28_wrap_word_single_long_word() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("abcdefg".to_string());
        text_node.style.wrap = oxiterm_proto::style::WrapMode::Word;
        text_node.style.width = Some(5.0);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&node_id).unwrap();
        assert_eq!(rect.height, 1.0);
    }

    #[test]
    fn test_flex_style_mapping_t8() {
        let engine = LayoutEngine::new();
        let mut node = Node::new(NodeTag::Box);
        node.style.flex = Some(1.0);
        let style = engine.map_style(&node, None, None);
        assert_eq!(style.flex_grow, 1.0);
        assert_eq!(style.flex_shrink, 1.0);
        assert_eq!(style.flex_basis, taffy::style::Dimension::Length(0.0));
    }

    #[test]
    fn test_flex_layout_t9() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut col = Node::new(NodeTag::Box);
        col.style.flex_direction = oxiterm_proto::style::FlexDirection::Column;
        col.style.height = Some(10.0);
        let col_id = doc.arena.alloc(col);
        doc.append_child(doc.root, col_id).unwrap();

        let mut child1 = Node::new(NodeTag::Box);
        child1.style.height = Some(3.0);
        let c1_id = doc.arena.alloc(child1);
        doc.append_child(col_id, c1_id).unwrap();

        let mut child2 = Node::new(NodeTag::Box);
        child2.style.flex = Some(1.0);
        let c2_id = doc.arena.alloc(child2);
        doc.append_child(col_id, c2_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect1 = result.nodes.get(&c1_id).unwrap();
        let rect2 = result.nodes.get(&c2_id).unwrap();
        assert_eq!(rect1.height, 3.0);
        assert_eq!(rect2.height, 7.0);
    }

    #[test]
    fn test_flex_layout_t10a_and_t10b() {
        // T-10a: column h=10, 2 children flex: 1, both with 1 line content -> 5 rows each
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        let mut col = Node::new(NodeTag::Box);
        col.style.flex_direction = oxiterm_proto::style::FlexDirection::Column;
        col.style.height = Some(10.0);
        let col_id = doc.arena.alloc(col.clone());
        doc.append_child(doc.root, col_id).unwrap();

        let mut child1 = Node::new(NodeTag::Text);
        child1.text = Some("Line 1".to_string());
        child1.style.flex = Some(1.0);
        let c1_id = doc.arena.alloc(child1);
        doc.append_child(col_id, c1_id).unwrap();

        let mut child2 = Node::new(NodeTag::Text);
        child2.text = Some("Line 1".to_string());
        child2.style.flex = Some(1.0);
        let c2_id = doc.arena.alloc(child2);
        doc.append_child(col_id, c2_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(result.nodes.get(&c1_id).unwrap().height, 5.0);
        assert_eq!(result.nodes.get(&c2_id).unwrap().height, 5.0);

        // T-10b: column h=10, 2 children flex: 1, 1st with 1 line content, 2nd with 3 lines -> 5 rows each
        let mut doc2 = THTMLDocument::new();
        let col2_id = doc2.arena.alloc(col);
        doc2.append_child(doc2.root, col2_id).unwrap();

        let mut c1_b = Node::new(NodeTag::Text);
        c1_b.text = Some("Line 1".to_string());
        c1_b.style.flex = Some(1.0);
        let c1_b_id = doc2.arena.alloc(c1_b);
        doc2.append_child(col2_id, c1_b_id).unwrap();

        let mut c2_b = Node::new(NodeTag::Text);
        c2_b.text = Some("Line 1\nLine 2\nLine 3".to_string());
        c2_b.style.flex = Some(1.0);
        let c2_b_id = doc2.arena.alloc(c2_b);
        doc2.append_child(col2_id, c2_b_id).unwrap();

        let result2 = engine.compute(&mut doc2, 80, 0, None).unwrap();
        assert_eq!(result2.nodes.get(&c1_b_id).unwrap().height, 5.0);
        assert_eq!(result2.nodes.get(&c2_b_id).unwrap().height, 5.0);
    }

    #[test]
    fn test_flex_media_t18_t18b() {
        // T-18 & T-18b: column h=10, rigid child h=12, child flex: 1 containing <img width=16 height=8>
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();

        let mut col = Node::new(NodeTag::Box);
        col.style.flex_direction = oxiterm_proto::style::FlexDirection::Column;
        col.style.height = Some(10.0);
        let col_id = doc.arena.alloc(col);
        doc.append_child(doc.root, col_id).unwrap();

        let mut rigid = Node::new(NodeTag::Box);
        rigid.style.height = Some(12.0);
        let rigid_id = doc.arena.alloc(rigid);
        doc.append_child(col_id, rigid_id).unwrap();

        let mut flex_container = Node::new(NodeTag::Box);
        flex_container.style.flex = Some(1.0);
        let flex_id = doc.arena.alloc(flex_container);
        doc.append_child(col_id, flex_id).unwrap();

        let mut img = Node::new(NodeTag::Img);
        img.style.width = Some(16.0);
        img.style.height = Some(8.0);
        let img_id = doc.arena.alloc(img);
        doc.append_child(flex_id, img_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let flex_rect = result.nodes.get(&flex_id).unwrap();
        let img_rect = result.nodes.get(&img_id).unwrap();

        // T-18: container rect has height >= 8
        assert!(flex_rect.height >= 8.0, "flex container height must be >= 8, got {}", flex_rect.height);
        // T-18b: img rect has 16x8
        assert_eq!(img_rect.width, 16.0);
        assert_eq!(img_rect.height, 8.0);
    }

    #[test]
    fn test_examples_video_mobile_t19() {
        // T-19: examples/demos/video_mobile.thtml (width 48) -> <video> rect has width 38 and height 9
        let path = if std::path::Path::new("examples/demos/video_mobile.thtml").exists() {
            "examples/demos/video_mobile.thtml"
        } else {
            "../examples/demos/video_mobile.thtml"
        };
        let template = std::fs::read_to_string(path).unwrap();
        let mut doc = crate::parser::THTMLParser::parse(&template).unwrap();
        let stylesheet = crate::parser::tcss::parse_tcss(&template).unwrap();
        crate::parser::tcss::apply_styles(&mut doc, &stylesheet);

        let mut engine = LayoutEngine::new();
        let result = engine.compute(&mut doc, 48, 0, None).unwrap();

        let mut found_video = None;
        for (&id, rect) in &result.nodes {
            if let Some(node) = doc.arena.get(id) {
                if node.tag == NodeTag::Video {
                    found_video = Some(rect.clone());
                    break;
                }
            }
        }
        let video_rect = found_video.expect("video node must be in layout");
        assert_eq!(video_rect.width, 38.0);
        assert_eq!(video_rect.height, 9.0);
    }

    #[test]
    fn test_examples_index_t20() {
        // T-20: examples/index.thtml (width 80) -> <img> rect keeps 16x8 after enabling flex
        let path = if std::path::Path::new("examples/index.thtml").exists() {
            "examples/index.thtml"
        } else {
            "../examples/index.thtml"
        };
        let template = std::fs::read_to_string(path).unwrap();
        let mut doc = crate::parser::THTMLParser::parse(&template).unwrap();
        let stylesheet = crate::parser::tcss::parse_tcss(&template).unwrap();
        crate::parser::tcss::apply_styles(&mut doc, &stylesheet);

        let mut engine = LayoutEngine::new();
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();

        let mut found_img = None;
        for (&id, rect) in &result.nodes {
            if let Some(node) = doc.arena.get(id) {
                if node.tag == NodeTag::Img {
                    found_img = Some(rect.clone());
                    break;
                }
            }
        }
        let img_rect = found_img.expect("img node must be in layout");
        assert_eq!(img_rect.width, 16.0);
        assert_eq!(img_rect.height, 8.0);
    }

    // Two flex:1 children in a 5-wide row: 2.5 each exactly.
    fn two_flex_children(engine: &mut LayoutEngine) -> (LayoutResult, OxiNodeId, OxiNodeId) {
        let mut doc = THTMLDocument::new();
        let mut row = Node::new(NodeTag::Box);
        row.style.width = Some(5.0);
        row.style.height = Some(1.0);
        let row_id = doc.arena.alloc(row);
        doc.append_child(doc.root, row_id).unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let mut c = Node::new(NodeTag::Box);
            c.style.flex = Some(1.0);
            let id = doc.arena.alloc(c);
            doc.append_child(row_id, id).unwrap();
            ids.push(id);
        }
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        (result, ids[0], ids[1])
    }

    #[test]
    fn test_gui_mode_keeps_fractional_sizes() {
        let mut engine = LayoutEngine::with_mode(RenderMode::Gui);
        let (result, a, b) = two_flex_children(&mut engine);
        assert_eq!(result.nodes[&a].width, 2.5);
        assert_eq!(result.nodes[&b].width, 2.5);
        assert_eq!(result.nodes[&b].x, 2.5);
    }

    #[test]
    fn test_tui_mode_snaps_to_whole_cells() {
        let mut engine = LayoutEngine::new();
        let (result, a, b) = two_flex_children(&mut engine);
        let (wa, wb) = (result.nodes[&a].width, result.nodes[&b].width);
        assert_eq!(wa.fract(), 0.0);
        assert_eq!(wb.fract(), 0.0);
        assert_eq!(wa + wb, 5.0);
        assert_eq!(result.nodes[&b].x.fract(), 0.0);
    }

    #[test]
    fn test_tui_rounds_fractional_style_lengths() {
        let mut doc = THTMLDocument::new();
        let mut n = Node::new(NodeTag::Box);
        n.style.width = Some(10.4);
        n.style.height = Some(3.6);
        let id = doc.arena.alloc(n);
        doc.append_child(doc.root, id).unwrap();
        let tui = LayoutEngine::new().compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!((tui.nodes[&id].width, tui.nodes[&id].height), (10.0, 4.0));
        doc.mark_dirty(id);
        let gui = LayoutEngine::with_mode(RenderMode::Gui).compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!((gui.nodes[&id].width, gui.nodes[&id].height), (10.4, 3.6));
    }

    #[test]
    fn test_hit_test_cell_centre() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        let mut n = Node::new(NodeTag::Box);
        n.style.width = Some(4.0);
        n.style.height = Some(2.0);
        n.style.margin.left = 3.0;
        let id = doc.arena.alloc(n);
        doc.append_child(doc.root, id).unwrap();
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let tester = crate::layout::types::HitTester::new(&result);
        assert_eq!(tester.find_node(3.5, 0.5), Some(id));
        assert_eq!(tester.find_node(6.5, 1.5), Some(id));
        assert_ne!(tester.find_node(2.5, 0.5), Some(id));
        assert_ne!(tester.find_node(7.5, 0.5), Some(id));
        assert_eq!(engine.hit_test(3, 0), Some(id));
    }
}

/// Stage 6.3: shaped text layout through the engine.
#[cfg(all(test, feature = "text-shaping"))]
mod shaped_tests {
    use super::*;
    use crate::font::{CELL_H_PX, CELL_W_PX};
    use crate::parser::THTMLParser;
    use oxiterm_proto::dom::NodeTag;

    fn gui(src: &str) -> THTMLDocument {
        THTMLParser::parse_with_mode(src, RenderMode::Gui).unwrap()
    }

    /// Ids of all nodes with `tag`, in arena order.
    fn nodes(doc: &THTMLDocument, tag: NodeTag) -> Vec<OxiNodeId> {
        let mut v: Vec<OxiNodeId> = doc.arena.iter().filter(|(_, n)| n.tag == tag).map(|(i, _)| i).collect();
        v.sort_by_key(|i| format!("{i:?}"));
        v
    }

    fn engine() -> LayoutEngine {
        let mut e = LayoutEngine::with_mode(RenderMode::Gui);
        e.set_cross_check(true);
        e
    }

    const PARA: &str = "The quick brown fox jumps over the lazy dog and then keeps on running far away";

    fn page(style: &str, width: u32) -> String {
        format!(r#"<box style="width: {width}; flex-direction: column;"><text style="wrap: word; {style}">{PARA}</text></box>"#)
    }

    #[test]
    fn default_font_stays_on_the_cell_path_with_no_shaping() {
        // mono 16 px / 20 px, also bold and italic: today's numbers, nothing shaped or cached
        for style in ["", "font-weight: bold;", "font-style: italic; font-weight: 700;", "font-family: monospace;"] {
            let mut doc = gui(&page(style, 30));
            let mut e = engine();
            let r = e.compute(&mut doc, 80, 0, None).unwrap();
            let t = nodes(&doc, NodeTag::Text)[0];
            let lines = word_wrapped_line_count(PARA, 30);
            assert_eq!(r.nodes[&t].height, lines as f32, "{style}");
            assert_eq!(e.shaped_count(), 0, "{style}");
            assert_eq!(e.text_cache.len(), 0, "{style}");
        }
    }

    #[test]
    fn mono_at_another_size_or_line_height_is_shaped() {
        let mut doc = gui(r#"<box><text style="font-size: 24px;">abcdef</text></box>"#);
        let mut e = engine();
        let r = e.compute(&mut doc, 80, 0, None).unwrap();
        let t = nodes(&doc, NodeTag::Text)[0];
        assert_eq!(e.shaped_count(), 1);
        // 6 chars x 14.4 px = 86.4 px = 9 cells; line height 30 px = 1.5 cells
        assert!((r.nodes[&t].width - 9.0).abs() < 1e-3, "{}", r.nodes[&t].width);
        assert!((r.nodes[&t].height - 1.5).abs() < 1e-4);
        let mut doc = gui(r#"<box><text style="line-height: 40px;">abc</text></box>"#);
        let mut e = engine();
        let r = e.compute(&mut doc, 80, 0, None).unwrap();
        let t = nodes(&doc, NodeTag::Text)[0];
        assert_eq!(e.shaped_count(), 1, "non-default line height leaves the fast path");
        assert!((r.nodes[&t].width - 3.0).abs() < 1e-3);
        assert_eq!(r.nodes[&t].height, 2.0);
    }

    #[test]
    fn sans_wrap_widths_and_stored_layout_agree_with_the_box() {
        for size in [14.0f32, 16.0, 20.0, 24.0] {
            let mut last_lines = usize::MAX;
            for width in [14u32, 20, 28, 40, 60, 120] {
                let mut doc = gui(&page(&format!("font-family: sans-serif; font-size: {size}px;"), width));
                let mut e = engine();
                let r = e.compute(&mut doc, 160, 0, None).unwrap();
                let t = nodes(&doc, NodeTag::Text)[0];
                let rect = r.nodes[&t];
                let l = e.shaped_text(t).expect("shaped layout stored");
                // the stored layout is the one the box was measured with
                let lh = size * 1.25;
                assert_eq!(rect.height, l.lines.len() as f32 * lh / CELL_H_PX, "size {size} width {width}");
                assert_eq!(rect.height, l.height_cells());
                assert!(rect.width <= f32::max(width as f32, l.min_content_cells()) + 1e-3);
                let widest = l.lines.iter().map(|x| x.width).fold(0.0, f32::max);
                assert!(widest <= rect.width * CELL_W_PX + 0.02, "no line wider than its box (size {size} width {width}): {widest} vs {}", rect.width * CELL_W_PX);
                if size == 16.0 {
                    assert!(l.lines.len() <= last_lines, "wider box, no more lines");
                    last_lines = l.lines.len();
                }
                // all words survive, none split
                let words: Vec<&str> = l.lines.iter().flat_map(|x| l.text[x.start as usize..x.end as usize].split_whitespace()).collect();
                assert_eq!(words.join(" "), PARA);
            }
        }
    }

    #[test]
    fn stored_layout_uses_the_content_box_of_a_padded_bordered_leaf() {
        // padding 1 and a single border on the text leaf itself: the content box is the rect
        // minus 2 columns of padding and 2 of border horizontally, 2 + 2 rows vertically.
        for width in [18u32, 26, 34, 50, 90] {
            let src = format!(r#"<box style="width: {width}; flex-direction: column;"><text style="wrap: word; font-family: sans-serif; font-size: 20px; padding: 1; border-style: single;">{PARA}</text></box>"#);
            let mut doc = gui(&src);
            let mut e = engine();
            let r = e.compute(&mut doc, 160, 0, None).unwrap();
            let t = nodes(&doc, NodeTag::Text)[0];
            let (rect, l) = (r.nodes[&t], e.shaped_text(t).unwrap().clone());
            assert_eq!(rect.height, l.height_cells() + 4.0, "width {width}");
            let content_w = rect.width - 4.0;
            let widest = l.lines.iter().map(|x| x.width).fold(0.0, f32::max);
            assert!(widest <= content_w * CELL_W_PX + 0.02, "width {width}: widest {widest} px in a {} px content box", content_w * CELL_W_PX);
            assert!(l.max_width.is_none_or(|m| (m - content_w * CELL_W_PX).abs() < 0.05), "wrapped at the content width, not the border box");
        }
    }

    #[test]
    fn wide_box_gets_one_line_and_exact_max_content() {
        let mut doc = gui(&page("font-family: sans-serif;", 200));
        let mut e = engine();
        let r = e.compute(&mut doc, 200, 0, None).unwrap();
        let t = nodes(&doc, NodeTag::Text)[0];
        let l = e.shaped_text(t).unwrap();
        assert_eq!(l.lines.len(), 1, "float guard: the last word never wraps on rounding");
        assert_eq!(r.nodes[&t].width, l.max_content_cells());
        assert_eq!(r.nodes[&t].height, 1.0);
    }

    #[test]
    fn nowrap_text_and_buttons_use_the_shaped_width() {
        let src = r#"<box style="flex-direction: row;">
            <text style="font-family: sans-serif;">Hello proportional world</text>
            <button style="font-family: sans-serif; border-style: single; padding-left: 2; padding-right: 2;">Resume</button>
        </box>"#;
        let mut doc = gui(src);
        let mut e = engine();
        let r = e.compute(&mut doc, 80, 0, None).unwrap();
        let t = nodes(&doc, NodeTag::Text)[0];
        let b = nodes(&doc, NodeTag::Button)[0];
        let tl = e.shaped_text(t).unwrap().clone();
        let bl = e.shaped_text(b).unwrap().clone();
        assert_eq!(r.nodes[&t].width, tl.max_content_cells());
        assert_ne!(r.nodes[&t].width, 24.0, "not the cell width of 24 chars");
        assert_eq!(r.nodes[&b].width, bl.max_content_cells() + 2.0 + 4.0, "text + border + padding");
        assert_eq!(r.nodes[&b].height, bl.height_cells() + 2.0);
        // the floor: a narrow row cannot squeeze the text below its shaped width
        let mut doc = gui(&format!(r#"<box style="width: 5; flex-direction: row;"><text style="font-family: sans-serif;">{PARA}</text></box>"#));
        let mut e = engine();
        let r = e.compute(&mut doc, 80, 0, None).unwrap();
        let t = nodes(&doc, NodeTag::Text)[0];
        assert_eq!(r.nodes[&t].width, e.shaped_text(t).unwrap().max_content_cells());
    }

    #[test]
    fn word_text_collapses_whitespace_like_the_mono_path_and_nowrap_keeps_it() {
        let shape = |wrap: &str, txt: &str| {
            let mut doc = gui(&format!(r#"<box style="width: 60; flex-direction: column;"><text style="font-family: sans-serif; wrap: {wrap};">{txt}</text></box>"#));
            let mut e = engine();
            e.compute(&mut doc, 80, 0, None).unwrap();
            let t = nodes(&doc, NodeTag::Text)[0];
            e.shaped_text(t).unwrap().clone()
        };
        let spaced = shape("word", "  alpha   beta \t gamma  ");
        let plain = shape("word", "alpha beta gamma");
        assert_eq!(&*spaced.text, "alpha beta gamma");
        assert_eq!(spaced.max_content, plain.max_content, "same measure as the collapsed text");
        // wrap:none is verbatim (the mono path paints `text.lines()` untouched), spaces included
        let kept = shape("none", "alpha   beta");
        assert_eq!(&*kept.text, "alpha   beta");
        assert!(kept.max_content > shape("none", "alpha beta").max_content);
    }

    #[test]
    fn missing_glyphs_have_a_font_independent_advance() {
        for (txt, cells) in [("日本語", 6.0f32), ("😀", 2.0)] {
            let mut doc = gui(&format!(r#"<box><text style="font-family: sans-serif;">{txt}</text></box>"#));
            let mut e = engine();
            let r = e.compute(&mut doc, 80, 0, None).unwrap();
            let t = nodes(&doc, NodeTag::Text)[0];
            assert!((r.nodes[&t].width - cells).abs() < 1e-3, "{txt}: {}", r.nodes[&t].width);
            assert!(e.shaped_text(t).unwrap().lines[0].glyphs.iter().all(|g| g.missing));
        }
    }

    #[test]
    fn tui_never_shapes() {
        let mut doc = THTMLParser::parse_with_mode(&page("font-family: sans-serif; font-size: 30px;", 30), RenderMode::Tui).unwrap();
        let mut e = LayoutEngine::with_mode(RenderMode::Tui);
        e.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(e.shaped_count(), 0);
        assert_eq!(e.text_cache.len(), 0);
    }

    #[test]
    fn font_changes_re_measure_through_the_incremental_reconcile() {
        let src = format!(r#"<box style="width: 40; flex-direction: column;"><box style="flex-direction: column;"><text style="wrap: word;">{PARA}</text><button style="border-style: single;">Go on</button></box></box>"#);
        let mut doc = gui(&src);
        let mut e = engine(); // cross-check on: every compute is compared with a rebuild
        let t = nodes(&doc, NodeTag::Text)[0];
        let b = nodes(&doc, NodeTag::Button)[0];
        let boxes = nodes(&doc, NodeTag::Box);
        let inner = boxes.into_iter().find(|&i| doc.arena.get(i).unwrap().children.contains(&t)).unwrap();
        let r0 = e.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(e.shaped_count(), 0);
        let h0 = r0.nodes[&t].height;
        // only the PARENT's font changes: descendants must re-measure
        doc.arena.get_mut(inner).unwrap().style.font.family = Some(oxiterm_proto::style::FontFace::Sans);
        let r1 = e.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(e.shaped_count(), 2, "text and button are shaped now");
        assert_eq!(r1.nodes[&t].height, e.shaped_text(t).unwrap().height_cells());
        let w1 = r1.nodes[&b].width;
        doc.arena.get_mut(inner).unwrap().style.font.size = Some(oxiterm_proto::style::FontSize::Px(24.0));
        let r2 = e.compute(&mut doc, 80, 0, None).unwrap();
        assert!(r2.nodes[&b].width > w1, "bigger font, wider button");
        assert!(r2.nodes[&t].height > r1.nodes[&t].height, "bigger font, taller text");
        // back to the default: shaped state is gone and the old numbers return
        doc.arena.get_mut(inner).unwrap().style.font = Default::default();
        let r3 = e.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(e.shaped_count(), 0);
        assert_eq!(r3.nodes[&t].height, h0);
        let s = e.stats();
        assert!(s.incremental >= 3 && s.full == 1, "{s:?}");
    }

    #[test]
    fn bold_only_change_does_not_dirty_a_fast_path_node() {
        let mut doc = gui(&format!(r#"<box style="width: 30;"><text id="t" style="wrap: word;">{PARA}</text></box>"#));
        let mut e = engine();
        e.compute(&mut doc, 80, 0, None).unwrap();
        let t = nodes(&doc, NodeTag::Text)[0];
        doc.arena.get_mut(t).unwrap().style.font.weight = Some(oxiterm_proto::style::FontWeight::Absolute(700));
        e.compute(&mut doc, 80, 0, None).unwrap();
        assert_eq!(e.stats().quiet, 1, "bold alone changes nothing the layout reads");
    }
}
