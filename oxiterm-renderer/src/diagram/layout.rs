//! Layered graph layout algorithm for OxiTerm diagram engine.

use std::collections::HashMap;
use super::mermaid::{Graph, Direction};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodePosition {
    pub id: String,
    pub col: usize,
    pub row: usize,
    pub width: usize,
    pub height: usize,
    pub layer: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphLayout {
    pub nodes: Vec<NodePosition>,
    pub total_cols: usize,
    pub total_rows: usize,
}

impl GraphLayout {
    pub fn get_node(&self, id: &str) -> Option<&NodePosition> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

/// Computes grid-based coordinates (columns and rows) for a `Graph`.
pub fn compute_layout(graph: &Graph) -> GraphLayout {
    if graph.nodes.is_empty() {
        return GraphLayout {
            nodes: Vec::new(),
            total_cols: 1,
            total_rows: 1,
        };
    }

    // 1. Layer Assignment (Longest Path)
    let mut layers: HashMap<String, usize> = HashMap::new();

    // Initialize root nodes (in-degree 0) to layer 0
    for node in &graph.nodes {
        if graph.in_degree(&node.id) == 0 {
            layers.insert(node.id.clone(), 0);
        }
    }

    // Assign layers iteratively along directed edges
    let mut changed = true;
    let mut pass = 0;
    while changed && pass < graph.nodes.len() + 1 {
        changed = false;
        pass += 1;
        for edge in &graph.edges {
            let from_layer = *layers.get(&edge.from).unwrap_or(&0);
            let to_layer = layers.entry(edge.to.clone()).or_insert(0);
            if *to_layer < from_layer + 1 {
                *to_layer = from_layer + 1;
                changed = true;
            }
        }
    }

    // Group nodes by layer
    let max_layer = layers.values().cloned().max().unwrap_or(0);
    let mut layer_groups: Vec<Vec<String>> = vec![Vec::new(); max_layer + 1];
    for node in &graph.nodes {
        let l = *layers.get(&node.id).unwrap_or(&0);
        layer_groups[l].push(node.id.clone());
    }

    // 2. Barycenter Ordering within layers
    for l in 1..=max_layer {
        let prev_positions: HashMap<String, usize> = layer_groups[l - 1]
            .iter()
            .enumerate()
            .map(|(idx, id)| (id.clone(), idx))
            .collect();

        layer_groups[l].sort_by_key(|id| {
            let in_edges: Vec<&String> = graph
                .edges
                .iter()
                .filter(|e| e.to == *id)
                .map(|e| &e.from)
                .collect();
            if in_edges.is_empty() {
                0
            } else {
                let sum: usize = in_edges
                    .iter()
                    .map(|src| *prev_positions.get(*src).unwrap_or(&0))
                    .sum();
                sum / in_edges.len()
            }
        });
    }

    // 3. Coordinate Allocation
    let mut node_positions = Vec::new();
    let mut max_col = 0usize;
    let mut max_row = 0usize;

    match graph.direction {
        Direction::TD => {
            let row_height_step = 5; // 3 cell height + 2 gap
            for (layer_idx, group) in layer_groups.iter().enumerate() {
                let row = layer_idx * row_height_step + 1;
                let mut current_col = 2;
                for id in group {
                    let node = graph.get_node(id).unwrap();
                    let width = (node.label.chars().count() + 4).max(8);
                    let height = 3;

                    node_positions.push(NodePosition {
                        id: id.clone(),
                        col: current_col,
                        row,
                        width,
                        height,
                        layer: layer_idx,
                    });

                    max_col = max_col.max(current_col + width + 2);
                    max_row = max_row.max(row + height + 1);

                    current_col += width + 4; // gap between nodes in same layer
                }
            }
        }
        Direction::LR => {
            let col_width_step = 20; // box width + gap
            for (layer_idx, group) in layer_groups.iter().enumerate() {
                let col = layer_idx * col_width_step + 2;
                let mut current_row = 1;
                for id in group {
                    let node = graph.get_node(id).unwrap();
                    let width = (node.label.chars().count() + 4).max(8);
                    let height = 3;

                    node_positions.push(NodePosition {
                        id: id.clone(),
                        col,
                        row: current_row,
                        width,
                        height,
                        layer: layer_idx,
                    });

                    max_col = max_col.max(col + width + 2);
                    max_row = max_row.max(current_row + height + 1);

                    current_row += height + 2;
                }
            }
        }
    }

    GraphLayout {
        nodes: node_positions,
        total_cols: max_col.max(20),
        total_rows: max_row.max(10),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::mermaid::{parse_mermaid, Graph};

    #[test]
    fn test_t11_linear_graph_td_three_layers() {
        let src = "flowchart TD\nA --> B\nB --> C";
        let graph = parse_mermaid(src).unwrap();
        let layout = compute_layout(&graph);

        let pos_a = layout.get_node("A").unwrap();
        let pos_b = layout.get_node("B").unwrap();
        let pos_c = layout.get_node("C").unwrap();

        assert_eq!(pos_a.layer, 0);
        assert_eq!(pos_b.layer, 1);
        assert_eq!(pos_c.layer, 2);

        assert!(pos_a.row < pos_b.row);
        assert!(pos_b.row < pos_c.row);
    }

    #[test]
    fn test_t12_branched_graph_same_layer() {
        let src = "flowchart TD\nA --> B\nA --> C";
        let graph = parse_mermaid(src).unwrap();
        let layout = compute_layout(&graph);

        let pos_b = layout.get_node("B").unwrap();
        let pos_c = layout.get_node("C").unwrap();

        assert_eq!(pos_b.layer, 1);
        assert_eq!(pos_c.layer, 1);
        assert_eq!(pos_b.row, pos_c.row);
        assert_ne!(pos_b.col, pos_c.col);
    }
}
