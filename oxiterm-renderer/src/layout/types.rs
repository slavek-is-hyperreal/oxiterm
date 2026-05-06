use std::collections::HashMap;
use oxiterm_proto::dom::NodeId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.width &&
        y >= self.y && y < self.y + self.height
    }
}

pub struct LayoutResult {
    pub nodes: HashMap<NodeId, Rect>,
}

pub struct HitTester<'a> {
    pub result: &'a LayoutResult,
}

impl<'a> HitTester<'a> {
    pub fn new(result: &'a LayoutResult) -> Self {
        Self { result }
    }

    pub fn find_node(&self, col: u16, row: u16) -> Option<NodeId> {
        let mut best_node = None;
        let mut best_area = u32::MAX;

        for (id, rect) in &self.result.nodes {
            if rect.contains(col, row) {
                let area = u32::from(rect.width) * u32::from(rect.height);
                if area <= best_area {
                    best_area = area;
                    best_node = Some(*id);
                }
            }
        }

        best_node
    }

    pub fn is_interactive(&self, id: NodeId, doc: &crate::document::THTMLDocument) -> bool {
        if let Some(node) = doc.arena.get(id) {
            matches!(node.tag, oxiterm_proto::dom::NodeTag::Button | oxiterm_proto::dom::NodeTag::Input) || node.attrs.event_htmx.is_some()
        } else {
            false
        }
    }
}
