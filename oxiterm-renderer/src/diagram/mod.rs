//! Diagram rendering sub-engine for OxiTerm.

pub mod mermaid;
pub mod layout;
pub mod grid;

pub use mermaid::{parse_mermaid, Graph, DiagramNode, DiagramEdge, Direction};
pub use layout::{compute_layout, GraphLayout, NodePosition};
pub use grid::CellGrid;
