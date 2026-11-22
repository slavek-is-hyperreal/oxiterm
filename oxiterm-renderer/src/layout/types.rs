//! Helper types for spatial layout metrics and mouse interaction tests.
//!
//! Exposes structures representing bounding boxes, collected document layouts,
//! and hit-testers checking interaction boundaries.

use std::collections::HashMap;
use oxiterm_proto::dom::NodeId;
use serde::{Deserialize, Serialize};

/// Converts a style length or layout coordinate in cell units to whole cells.
///
/// The single TUI rounding rule for consumers that work in cells: `round()` then clamp
/// at zero. (A bare `as u16` would truncate, disagreeing with layout's rounding.)
pub fn to_cells(v: f32) -> u16 {
    v.round().max(0.0) as u16
}

/// Bounding rectangle of an element on the screen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Rect {
    /// The X coordinate of the top-left corner.
    pub x: f32,
    /// The Y coordinate of the top-left corner.
    pub y: f32,
    /// The width of the rectangle.
    pub width: f32,
    /// The height of the rectangle.
    pub height: f32,
}

/// A [`Rect`] snapped to whole terminal cells (see [`to_cells`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellRect {
    /// Left column.
    pub x: u16,
    /// Top row.
    pub y: u16,
    /// Width in columns.
    pub width: u16,
    /// Height in rows.
    pub height: u16,
}

impl Rect {
    /// Snaps this rectangle to whole cells for cell-based consumers (cursor, scroll, etc.).
    pub fn cells(&self) -> CellRect {
        CellRect {
            x: to_cells(self.x),
            y: to_cells(self.y),
            width: to_cells(self.width),
            height: to_cells(self.height),
        }
    }

    /// Checks if a given (x, y) point (cell units; a cell centre is `col + 0.5`) is within
    /// the boundaries of the rectangle.
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width &&
        y >= self.y && y < self.y + self.height
    }
}

/// The aggregated layout results of a THTML document.
#[derive(Debug, Clone, Default)]
pub struct LayoutResult {
    /// Map of active node IDs to their computed bounding rectangles.
    pub nodes: HashMap<NodeId, Rect>,
    /// Sorted list of node IDs determining drawing and hit-testing order (bottom-most to top-most).
    pub paint_order: Vec<NodeId>,
    /// The total height of the scrollable content, in whole cells (scroll is still cell-based).
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
            if f32::from(view_w) > rect.width { ((f32::from(view_w) - rect.width) / 2.0) as u16 } else { 0 }
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

    /// Finds the top-most visible child node covering the specified point (cell units).
    pub fn find_node(&self, col: f32, row: f32) -> Option<NodeId> {
        if !self.result.paint_order.is_empty() {
            for &id in self.result.paint_order.iter().rev() {
                if let Some(rect) = self.result.nodes.get(&id) {
                    if rect.contains(col, row) {
                        return Some(id);
                    }
                }
            }
            return None;
        }

        let mut best_node = None;
        let mut best_area = f32::MAX;

        for (id, rect) in &self.result.nodes {
            if rect.contains(col, row) {
                let area = rect.width * rect.height;
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
