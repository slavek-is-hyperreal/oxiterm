use std::collections::HashMap;
use oxiterm_proto::dom::NodeId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
        // Find the node under (col, row). 
        // In a real implementation, we would respect Z-index and nesting.
        // For now, we take the one that covers the smallest area (usually the most specific child).
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
}
