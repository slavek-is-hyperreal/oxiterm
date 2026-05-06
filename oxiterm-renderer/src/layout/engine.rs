use taffy::prelude::*;
use crate::document::THTMLDocument;
use crate::layout::types::{LayoutResult, Rect as OxiRect};
use oxiterm_proto::dom::NodeId as OxiNodeId;
use std::collections::HashMap;

pub struct LayoutEngine {
    pub taffy: TaffyTree<()>,
    /// Persistent mapping from OxiTerm NodeId to Taffy NodeId
    node_map: HashMap<OxiNodeId, taffy::NodeId>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
            node_map: HashMap::new(),
        }
    }
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutEngine {
    pub fn compute(&mut self, doc: &THTMLDocument) -> anyhow::Result<LayoutResult> {
        // Step 1: Ensure all nodes are in the Taffy tree (Incremental Build)
        self.ensure_nodes_exist_recursive(doc, doc.root)?;

        // Step 2: Synchronize dirty nodes
        for &oxi_id in &doc.dirty_nodes {
            if let Some(&taffy_id) = self.node_map.get(&oxi_id) {
                if let Some(node) = doc.arena.get(oxi_id) {
                    // Update style
                    let style = self.map_style(&node.style);
                    self.taffy.set_style(taffy_id, style)?;

                    // Update children
                    let mut taffy_children = Vec::new();
                    for &child_id in &node.children {
                        if let Some(&t_child) = self.node_map.get(&child_id) {
                            taffy_children.push(t_child);
                        } else {
                            // This should not happen if ensure_nodes_exist_recursive works
                            let t_child = self.ensure_nodes_exist_recursive(doc, child_id)?;
                            taffy_children.push(t_child);
                        }
                    }
                    self.taffy.set_children(taffy_id, &taffy_children)?;
                }
            }
        }

        // Step 3: Compute layout
        let root_taffy_id = *self.node_map.get(&doc.root).unwrap();
        let width = doc.arena.get(doc.root).and_then(|n| n.style.width).unwrap_or(80) as f32;
        let height = doc.arena.get(doc.root).and_then(|n| n.style.height).unwrap_or(24) as f32;

        let available_space = Size {
            width: AvailableSpace::Definite(width),
            height: AvailableSpace::Definite(height),
        };

        self.taffy.compute_layout(root_taffy_id, available_space)?;
        
        let mut nodes = HashMap::new();
        for (&oxi_id, &taffy_id) in &self.node_map {
            let layout = self.taffy.layout(taffy_id).unwrap();
            nodes.insert(oxi_id, OxiRect {
                x: layout.location.x.round() as u16,
                y: layout.location.y.round() as u16,
                width: layout.size.width.round() as u16,
                height: layout.size.height.round() as u16,
            });
        }

        Ok(LayoutResult { nodes })
    }

    fn ensure_nodes_exist_recursive(
        &mut self, 
        doc: &THTMLDocument, 
        oxi_id: OxiNodeId
    ) -> anyhow::Result<taffy::NodeId> {
        if let Some(&taffy_id) = self.node_map.get(&oxi_id) {
            return Ok(taffy_id);
        }

        let node = doc.arena.get(oxi_id).ok_or_else(|| anyhow::anyhow!("Node not found"))?;
        
        let mut children = Vec::new();
        for &child_id in &node.children {
            children.push(self.ensure_nodes_exist_recursive(doc, child_id)?);
        }

        let style = self.map_style(&node.style);
        let taffy_id = self.taffy.new_with_children(style, &children)?;
        
        self.node_map.insert(oxi_id, taffy_id);
        Ok(taffy_id)
    }

    fn map_style(&self, style: &oxiterm_proto::style::ComputedStyle) -> Style {
        Style {
            display: Display::Flex,
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
                width: style.width.map(|w| Dimension::Length(w as f32)).unwrap_or(Dimension::Auto),
                height: style.height.map(|h| Dimension::Length(h as f32)).unwrap_or(Dimension::Auto),
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
            ..Default::default()
        }
    }
}
