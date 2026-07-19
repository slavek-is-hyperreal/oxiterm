//! Helper types for spatial layout metrics and mouse interaction tests.
//!
//! Exposes structures representing bounding boxes, collected document layouts,
//! and hit-testers checking interaction boundaries.

use std::collections::HashMap;
use oxiterm_proto::dom::NodeId;
use serde::{Deserialize, Serialize};

/// Bounding rectangle of an element on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Rect {
    /// The X coordinate of the top-left corner.
    pub x: u16,
    /// The Y coordinate of the top-left corner.
    pub y: u16,
    /// The width of the rectangle.
    pub width: u16,
    /// The height of the rectangle.
    pub height: u16,
}

impl Rect {
    /// Checks if a given (x, y) coordinate is within the boundaries of the rectangle.
    pub fn contains(&self, x: u16, y: u16) -> bool {
        x >= self.x && x < self.x + self.width &&
        y >= self.y && y < self.y + self.height
    }
}

/// The aggregated layout results of a THTML document.
#[derive(Debug, Clone)]
pub struct LayoutResult {
    /// Map of active node IDs to their computed bounding rectangles.
    pub nodes: HashMap<NodeId, Rect>,
    /// The total height of the scrollable content.
    pub total_height: u16,
}

impl LayoutResult {
    /// Computes offset values needed to center the content of the root node on the screen.
    ///
    /// Horizontal centering tracks the first child's width. Vertical centering tracks the
    /// *total content extent* ([`Self::total_height`]) — NOT the first child's height — so
    /// that overflowing content (taller than the viewport) gets `oy = 0`. Centering on the
    /// first child's height would shift a tall document downward and steal exactly that many
    /// rows from the scroll range, hiding the bottom of the document from the scroll clamp.
    pub fn get_centering_offset(&self, doc: &crate::document::THTMLDocument, view_w: u16, view_h: u16) -> (u16, u16) {
        let target_node_id = if let Some(root_node) = doc.get_node(doc.root) {
            root_node.children.first().copied().unwrap_or(doc.root)
        } else {
            doc.root
        };

        let ox = if let Some(rect) = self.nodes.get(&target_node_id) {
            if view_w > rect.width { (view_w - rect.width) / 2 } else { 0 }
        } else {
            0
        };
        let oy = if view_h > self.total_height { (view_h - self.total_height) / 2 } else { 0 };
        (ox, oy)
    }
}

/// Hit tester for mapping cursor coordinates onto layout nodes.
pub struct HitTester<'a> {
    /// Reference to the active layout result.
    pub result: &'a LayoutResult,
}

impl<'a> HitTester<'a> {
    /// Creates a new hit tester referencing the given layout result.
    pub fn new(result: &'a LayoutResult) -> Self {
        Self { result }
    }

    /// Finds the deepest child node covering the specified coordinates (col, row).
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

    /// Determines if a node responds to user input (e.g. is a Button, Input, or has an HTMX event).
    pub fn is_interactive(&self, id: NodeId, doc: &crate::document::THTMLDocument) -> bool {
        if let Some(node) = doc.arena.get(id) {
            matches!(node.tag, oxiterm_proto::dom::NodeTag::Button | oxiterm_proto::dom::NodeTag::Input) || node.attrs.event_htmx.is_some()
        } else {
            false
        }
    }
}
