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
use anyhow::anyhow;

/// Per-node context handed to Taffy's measure closure for content whose height
/// depends on the final laid-out width — i.e. `wrap:word` text. Only such leaves
/// carry a context; every other node is sized purely from its style.
#[derive(Clone, Debug)]
pub struct MeasureContext {
    /// Raw text of the node, wrapped at measure time against the available width.
    text: String,
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
        }
    }

    /// Sets the viewport height used as the root's minimum height (see `compute`).
    pub fn set_viewport_rows(&mut self, rows: u16) {
        self.viewport_rows = Some(rows);
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
        // Clearing nodes on each run forces rebuild of the Taffy tree.
        // This ensures correctness for elements whose visibility (bind-show)
        // might have toggled since the last frame.
        self.reset_nodes();

        self.ensure_nodes_exist_recursive(doc, doc.root, state_evaluator)?;

        for &oxi_id in &doc.dirty_nodes {
            if let Some(&taffy_id) = self.node_map.get(&oxi_id) {
                if let Some(node) = doc.arena.get(oxi_id) {
                    let style = self.map_style(node, state_evaluator);
                    self.taffy.set_style(taffy_id, style)?;

                    let mut taffy_children = Vec::new();
                    for &child_id in &node.children {
                        if let Some(&t_child) = self.node_map.get(&child_id) {
                            taffy_children.push(t_child);
                        } else {
                            let t_child = self.ensure_nodes_exist_recursive(doc, child_id, state_evaluator)?;
                            taffy_children.push(t_child);
                        }
                    }
                    self.taffy.set_children(taffy_id, &taffy_children)?;
                }
            }
        }

        let root_taffy_id = *self.node_map.get(&doc.root).unwrap();
        
        let mut root_w = cols as f32;
        // Without an explicit root height the available height is unbounded and the root
        // is sized to its content, with the viewport as its minimum height. (A fixed
        // u16::MAX-tall root used to stretch children to 65535 rows, which made the
        // scroll extent meaningless.)
        let mut root_h_available = AvailableSpace::MaxContent;

        if let Some(root_node) = doc.arena.get(doc.root) {
            let mut style = self.map_style(root_node, state_evaluator);
            if let Some(w) = root_node.style.width {
                root_w = self.snap_size(w);
            } else {
                style.size.width = taffy::style::Dimension::Length(cols as f32);
            }
            if let Some(h) = root_node.style.height {
                root_h_available = AvailableSpace::Definite(self.snap_size(h));
            } else {
                style.size.height = taffy::style::Dimension::Auto;
                if let Some(rows) = self.viewport_rows {
                    style.min_size.height = taffy::style::Dimension::Length(f32::from(rows));
                }
            }
            self.taffy.set_style(root_taffy_id, style)?;
        }

        let available_space = Size {
            width: AvailableSpace::Definite(root_w),
            height: root_h_available,
        };

        // Measure closure resolves height-from-width for `wrap:word` text leaves
        // (the only nodes given a MeasureContext); all other nodes fall back to Zero
        // and are sized purely from their style.
        self.taffy.compute_layout_with_measure(
            root_taffy_id,
            available_space,
            |known, avail, _node_id, ctx, _style| match ctx {
                Some(ctx) => measure_wrap_text(known, avail, &ctx.text),
                None => Size::ZERO,
            },
        )?;

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
            total_height: max_bottom.ceil() as u16 };
        self.last_layout = Some(result.clone());
        Ok(result)
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

    fn ensure_nodes_exist_recursive(
        &mut self, 
        doc: &THTMLDocument, 
        oxi_id: OxiNodeId,
        state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>,
    ) -> anyhow::Result<taffy::NodeId> {
        if let Some(&taffy_id) = self.node_map.get(&oxi_id) {
            return Ok(taffy_id);
        }

        let node = doc.arena.get(oxi_id).ok_or_else(|| anyhow::anyhow!("Node not found"))?;
        
        let mut children = Vec::new();
        for &child_id in &node.children {
            children.push(self.ensure_nodes_exist_recursive(doc, child_id, state_evaluator)?);
        }

        let style = self.map_style(node, state_evaluator);
        let taffy_id = self.taffy.new_with_children(style, &children)?;

        // Only `wrap:word` text leaves need width-dependent height resolution, so only
        // they receive a measure context; everything else is sized from its style alone.
        if node.tag == oxiterm_proto::dom::NodeTag::Text
            && node.style.wrap == oxiterm_proto::style::WrapMode::Word
        {
            if let Some(text) = &node.text {
                self.taffy.set_node_context(taffy_id, Some(MeasureContext { text: text.clone() }))?;
            }
        }

        self.node_map.insert(oxi_id, taffy_id);
        Ok(taffy_id)
    }

    fn map_style(&self, node: &oxiterm_proto::dom::Node, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>) -> Style {
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
                        let mut calculated_width = text_intrinsic_width(text) as f32;
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
                        let mut calculated_height = (text.lines().count() as f32).max(1.0);
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
        let style = engine.map_style(&node, None);
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
