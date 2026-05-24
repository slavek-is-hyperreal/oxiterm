use taffy::prelude::*;
use crate::document::THTMLDocument;
use crate::layout::types::{LayoutResult, Rect as OxiRect};
use oxiterm_proto::dom::NodeId as OxiNodeId;
use std::collections::HashMap;
use anyhow::anyhow;

pub struct LayoutEngine {
    pub taffy: TaffyTree<()>,
    /// Persistent mapping from OxiTerm NodeId to Taffy NodeId
    node_map: HashMap<OxiNodeId, taffy::NodeId>,
    pub last_layout: Option<LayoutResult>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            node_map: HashMap::new(),
            last_layout: None,
        }
    }

    /// BUG-COMPACT-01: Clear the node map to ensure consistency after document compaction.
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
    pub fn compute(&mut self, doc: &mut THTMLDocument, cols: u16, _scroll_offset: u16, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>) -> anyhow::Result<LayoutResult> {
        // Step 1: Ensure all nodes are in the Taffy tree (Incremental Build)
        self.ensure_nodes_exist_recursive(doc, doc.root, state_evaluator)?;

        // Step 2: Synchronize dirty nodes
        for &oxi_id in &doc.dirty_nodes {
            if let Some(&taffy_id) = self.node_map.get(&oxi_id) {
                if let Some(node) = doc.arena.get(oxi_id) {
                    // Update style
                    let style = self.map_style(node, state_evaluator);
                    self.taffy.set_style(taffy_id, style)?;

                    // Update children
                    let mut taffy_children = Vec::new();
                    for &child_id in &node.children {
                        if let Some(&t_child) = self.node_map.get(&child_id) {
                            taffy_children.push(t_child);
                        } else {
                            // This should not happen if ensure_nodes_exist_recursive works
                            let t_child = self.ensure_nodes_exist_recursive(doc, child_id, state_evaluator)?;
                            taffy_children.push(t_child);
                        }
                    }
                    self.taffy.set_children(taffy_id, &taffy_children)?;
                }
            }
        }

        // Step 3: Compute layout
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
                // SPRINT SPECIFICATIONS: unconstrained height (u16::MAX) to allow measuring full layout height
                style.size.height = taffy::style::Dimension::Length(u16::MAX as f32);
                root_h = u16::MAX;
            }
            self.taffy.set_style(root_taffy_id, style)?;
        }

        let available_space = Size {
            width: AvailableSpace::Definite(root_w as f32),
            height: AvailableSpace::Definite(root_h as f32),
        };

        self.taffy.compute_layout(root_taffy_id, available_space)?;
        
        let mut nodes = HashMap::new();
        self.flatten_layout_recursive(doc, doc.root, 0, 0, &mut nodes)?;

        // BUG-H05: Clear dirty nodes after computation
        doc.clear_dirty();

        let mut max_bottom = 0;
        for rect in nodes.values() {
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

    pub fn hit_test(&self, x: u16, y: u16) -> Option<OxiNodeId> {
        let layout = self.last_layout.as_ref()?;
        let mut best_node = None;
        let mut best_area = u32::MAX;

        for (&id, rect) in &layout.nodes {
            if x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height {
                // We want the smallest (deepest) node
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
        
        self.node_map.insert(oxi_id, taffy_id);
        Ok(taffy_id)
    }

    fn map_style(&self, node: &oxiterm_proto::dom::Node, state_evaluator: Option<&dyn oxiterm_proto::dom::StateEvaluator>) -> Style {
        let style = &node.style;
        let mut width = style.width;
        let mut height = style.height;

        if node.tag == oxiterm_proto::dom::NodeTag::Text {
            if let Some(text) = &node.text {
                if width.is_none() {
                    let calculated_width = text.lines()
                        .map(|line| line.chars().map(|c| crate::render::unicode::UnicodeWidthCache::get().width(c) as u16).sum())
                        .max()
                        .unwrap_or(0);
                    width = Some(calculated_width);
                }
                if height.is_none() {
                    let calculated_height = text.lines().count() as u16;
                    height = Some(calculated_height.max(1));
                }
            }
        }

        let mut display = Display::Flex;
        if let Some(cond) = &node.attrs.bind_show {
            if let Some(eval) = state_evaluator {
                if !eval.evaluate_bind_show(cond) {
                    display = Display::None;
                }
            } else {
                display = Display::None;
            }
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
            size: Size {
                width: width.map(|w| Dimension::Length(w as f32)).unwrap_or(Dimension::Auto),
                height: height.map(|h| Dimension::Length(h as f32)).unwrap_or(Dimension::Auto),
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
        
        // Inside
        assert_eq!(engine.hit_test(6, 1), Some(child_id));
        // Outside
        assert_eq!(engine.hit_test(1, 1), Some(doc.root));
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
        
        // Child absolute coordinate: parent.left (10) + child.left (5) = 15.
        // Inside nested child (e.g. x = 16, y = 1)
        assert_eq!(engine.hit_test(16, 1), Some(child_id));
        // Outside child but inside parent (e.g. x = 11, y = 1)
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
    fn test_bind_show_layout() {
        let mut engine = LayoutEngine::new();
        let mut doc = THTMLDocument::new();
        
        let mut child = Node::new(NodeTag::Box);
        child.style.width = Some(10);
        child.style.height = Some(10);
        child.attrs.bind_show = Some("some_cond".to_string());
        let child_id = doc.arena.alloc(child);
        doc.append_child(doc.root, child_id).unwrap();

        // 1. Evaluator says false -> node is hidden (absent from nodes)
        let eval_false = MockEvaluator { val: false };
        let result_hidden = engine.compute(&mut doc, 80, 0, Some(&eval_false)).unwrap();
        assert!(result_hidden.nodes.get(&child_id).is_none());

        // 2. Evaluator says true -> node is visible
        let eval_true = MockEvaluator { val: true };
        engine.taffy = taffy::TaffyTree::new();
        engine.node_map.clear();
        let result_visible = engine.compute(&mut doc, 80, 0, Some(&eval_true)).unwrap();
        assert!(result_visible.nodes.get(&child_id).is_some());
    }
}
