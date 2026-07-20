//! Layout computation engine mapping DOM structures onto Taffy trees.
//!
//! Synchronizes OxiTerm node styles and parent-child hierarchies into a TaffyTree
//! to calculate precise coordinates using the Flexbox layout algorithm.

use taffy::prelude::*;
use crate::document::THTMLDocument;
use crate::layout::types::{LayoutResult, Rect as OxiRect};
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
}

impl LayoutEngine {
    /// Creates a new layout engine with an empty Taffy tree.
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            node_map: HashMap::new(),
            last_layout: None,
        }
    }

    /// Resets the engine, discarding the current Taffy tree and node mappings.
    ///
    /// Required after document defragmentation (compaction) to prevent stale ID lookups.
    pub fn reset_nodes(&mut self) {
        self.taffy = TaffyTree::new();
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
        
        let mut root_w = cols;
        let mut root_h = u16::MAX;

        if let Some(root_node) = doc.arena.get(doc.root) {
            let mut style = self.map_style(root_node, state_evaluator);
            if let Some(w) = root_node.style.width {
                root_w = w;
            } else {
                style.size.width = taffy::style::Dimension::Length(cols as f32);
            }
            if let Some(h) = root_node.style.height {
                root_h = h;
            } else {
                style.size.height = taffy::style::Dimension::Length(u16::MAX as f32);
                root_h = u16::MAX;
            }
            self.taffy.set_style(root_taffy_id, style)?;
        }

        let available_space = Size {
            width: AvailableSpace::Definite(root_w as f32),
            height: AvailableSpace::Definite(root_h as f32),
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
        self.flatten_layout_recursive(doc, doc.root, 0, 0, &mut nodes)?;

        doc.clear_dirty();

        let mut max_bottom = 0;
        for (&node_id, rect) in &nodes {
            if node_id == doc.root {
                continue;
            }
            let bottom = rect.y + rect.height;
            if bottom > max_bottom {
                max_bottom = bottom;
            }
        }

        let result = LayoutResult { nodes, total_height: max_bottom };
        self.last_layout = Some(result.clone());
        Ok(result)
    }

    fn flatten_layout_recursive(
        &self,
        doc: &THTMLDocument,
        oxi_id: OxiNodeId,
        parent_x: u16,
        parent_y: u16,
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
                
                let rect_x = layout.location.x.round() as u16;
                let rect_y = layout.location.y.round() as u16;
                let width = layout.size.width.round() as u16;
                let height = layout.size.height.round() as u16;
                
                let abs_x = parent_x + rect_x;
                let abs_y = parent_y + rect_y;
                
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

    /// Queries the computed layout to find which node lies at coordinate (x, y).
    ///
    /// Returns the smallest (deepest nested) node covering the coordinates.
    pub fn hit_test(&self, x: u16, y: u16) -> Option<OxiNodeId> {
        let layout = self.last_layout.as_ref()?;
        let mut best_node = None;
        let mut best_area = u32::MAX;

        for (&id, rect) in &layout.nodes {
            if x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height {
                let area = (rect.width as u32) * (rect.height as u32);
                if area <= best_area {
                    best_area = area;
                    best_node = Some(id);
                }
            }
        }
        best_node
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
        let mut width = style.width;
        let mut height = style.height;
        // Intrinsic glyph width used as a shrink floor for non-wrapping text, set ONLY
        // when the width was auto-derived from text (not when the author gave a width).
        let mut min_width: Option<u16> = None;
        // Whether this node's size is resolved by the Taffy measure closure (wrap:word
        // text). Such nodes keep only their explicit style dimensions here; the closure
        // fills in the rest and must not be overridden by cross-axis stretch.
        let mut measured_text = false;

        if node.tag == oxiterm_proto::dom::NodeTag::Text {
            if node.text.is_some() {
                if style.wrap == oxiterm_proto::style::WrapMode::Word {
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
                        let calculated_width = text_intrinsic_width(text);
                        width = Some(calculated_width);
                        min_width = Some(calculated_width);
                    }
                    if height.is_none() {
                        height = Some((text.lines().count() as u16).max(1));
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

        Style {
            display,
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
            // Measured text must keep its measured cross size: without this, a parent's
            // default `align-items: stretch` would blow an auto-height paragraph up to the
            // container's cross size (e.g. a Row of height MAX). Non-measured nodes inherit
            // the parent's alignment (so centered single-line labels stay centered).
            align_self: if measured_text { Some(AlignItems::Start) } else { None },
            size: Size {
                width: width.map(|w| Dimension::Length(w as f32)).unwrap_or(Dimension::Auto),
                height: height.map(|h| Dimension::Length(h as f32)).unwrap_or(Dimension::Auto),
            },
            min_size: Size {
                width: min_width.map(|w| Dimension::Length(w as f32)).unwrap_or(Dimension::Auto),
                height: Dimension::Auto,
            },
            padding: Rect {
                left: LengthPercentage::Length(style.padding.left as f32),
                right: LengthPercentage::Length(style.padding.right as f32),
                top: LengthPercentage::Length(style.padding.top as f32),
                bottom: LengthPercentage::Length(style.padding.bottom as f32),
            },
            margin: Rect {
                left: LengthPercentage::Length(style.margin.left as f32).into(),
                right: LengthPercentage::Length(style.margin.right as f32).into(),
                top: LengthPercentage::Length(style.margin.top as f32).into(),
                bottom: LengthPercentage::Length(style.margin.bottom as f32).into(),
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
        child.style.width = Some(20);
        child.style.height = Some(10);
        let child_id = doc.arena.alloc(child);
        doc.append_child(doc.root, child_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&child_id).unwrap();
        assert_eq!(rect.width, 20);
        assert_eq!(rect.height, 10);
    }

    #[test]
    fn test_hit_test() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(10);
        child.style.height = Some(10);
        child.style.margin.left = 5;
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
        parent.style.width = Some(30);
        parent.style.height = Some(10);
        parent.style.margin.left = 5;
        parent.style.margin.top = 2;
        let parent_id = doc.arena.alloc(parent);
        doc.append_child(doc.root, parent_id).unwrap();

        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(8);
        child.style.height = Some(3);
        child.style.margin.left = 4;
        child.style.margin.top = 2;
        let child_id = doc.arena.alloc(child);
        doc.append_child(parent_id, child_id).unwrap();

        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let pr = *result.nodes.get(&parent_id).unwrap();
        let cr = *result.nodes.get(&child_id).unwrap();
        assert!(cr.width > 0 && cr.height > 0 && pr.width > 0);

        // Every cell of the (deepest) child maps back to the child.
        for y in cr.y..cr.y + cr.height {
            for x in cr.x..cr.x + cr.width {
                assert_eq!(engine.hit_test(x, y), Some(child_id),
                    "cell ({x},{y}) inside the child must hit-test back to it");
            }
        }
        // Every cell of the parent maps to the child where they overlap, else the parent.
        for y in pr.y..pr.y + pr.height {
            for x in pr.x..pr.x + pr.width {
                let expected = if cr.contains(x, y) { child_id } else { parent_id };
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
        parent.style.width = Some(20);
        parent.style.height = Some(20);
        parent.style.margin.left = 10;
        let parent_id = doc.arena.alloc(parent);
        doc.append_child(doc.root, parent_id).unwrap();

        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(5);
        child.style.height = Some(5);
        child.style.margin.left = 5;
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
        assert_eq!(rect.width, 6);
        assert_eq!(rect.height, 2);
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
        parent.style.width = Some(40);
        parent.style.height = Some(2);
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
        assert!(text_rect.y + text_rect.height <= result.total_height - 1,
            "bottom border row must sit below the last text line");
        assert_eq!(result.total_height, 3, "scroll extent must include the bottom border row");
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
        outer.style.width = Some(40);
        outer.style.height = Some(5);
        let outer_id = doc.arena.alloc(outer);
        doc.append_child(doc.root, outer_id).unwrap();

        let mut inner = Node::new(NodeTag::Box);
        inner.style.width = Some(40);
        inner.style.height = Some(20);
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
        child.style.width = Some(10);
        child.style.height = Some(10);
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
        text_node.style.width = Some(5);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&node_id).unwrap();
        assert_eq!(rect.height, 2);
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
        row.style.width = Some(8); // narrower than spacer(6) + label(6), forces shrink
        row.style.height = Some(1);
        let row_id = doc.arena.alloc(row);
        doc.append_child(doc.root, row_id).unwrap();

        let mut spacer = Node::new(NodeTag::Box);
        spacer.style.width = Some(6);
        spacer.style.height = Some(1);
        let spacer_id = doc.arena.alloc(spacer);
        doc.append_child(row_id, spacer_id).unwrap();

        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("← Back".to_string());
        text_node.style.height = Some(1);
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
        assert_eq!(rect.width, 6, "text node must not shrink below its glyph span");
    }

    #[test]
    fn test_every_column_of_clickable_text_is_hit_testable() {
        // t2: every column across the glyph span resolves to the text node.
        let (engine, text_id) = narrow_row_with_back_label(false);
        let rect = *engine.last_layout.as_ref().unwrap().nodes.get(&text_id).unwrap();
        assert_eq!(rect.width, 6);
        for x in rect.x..rect.x + rect.width {
            assert_eq!(
                engine.hit_test(x, rect.y),
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
        assert_eq!(rect.width, 6);
        assert_eq!(
            engine.hit_test(rect.x + rect.width - 1, rect.y),
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
        container.style.width = Some(container_width);
        container.style.height = Some(container_height);
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
        assert_eq!(rect.width, 10, "must narrow to the container width, not stay at 22");
        assert_eq!(rect.height, 3, "height must follow the wrapped line count, not be 1");
    }

    #[test]
    fn test_wrap_word_height_follows_final_width() {
        // Narrower available width => more wrapped lines. Same text, width 6:
        // "alpha" / "beta" / "gamma" / "delta" => 4 lines (no two 5/4-wide words fit in 6
        // together since 5+1+4=10 > 6).
        let rect = wrapped_paragraph_rect(6, 6, "alpha beta gamma delta");
        assert_eq!(rect.width, 6);
        assert_eq!(rect.height, 4, "height must recompute from the narrower final width");
    }

    #[test]
    fn test_28_wrap_word_single_long_word() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("abcdefg".to_string());
        text_node.style.wrap = oxiterm_proto::style::WrapMode::Word;
        text_node.style.width = Some(5);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();
        
        let result = engine.compute(&mut doc, 80, 0, None).unwrap();
        let rect = result.nodes.get(&node_id).unwrap();
        assert_eq!(rect.height, 1);
    }
}

