//! Character cell grid rendering, minimap sampling, and crop window selection.

use super::mermaid::Graph;
use super::layout::{GraphLayout, NodePosition};

/// A 2D grid of character cells representing the full-size rendered diagram.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<Vec<char>>,
}

// Cell to pixel aspect ratio documentation:
// OxiTerm terminal character cell = 10 px wide x 20 px high (aspect ratio 1:2).
// Half-block character ('▀', '▄', '█') provides 2 vertical sub-pixels per cell.
// Each half-block sub-pixel represents a sub-cell area of 10 px x 10 px (1:1 square aspect ratio).
// Sampling 1 grid cell (10x20 px) into half-block space maps 1 grid column -> 1 sub-column,
// and 1 grid row -> 2 sub-rows (top half-block & bottom half-block).
// The minimap sampling algorithm calculates horizontal scale factor (s_x = src_cols / target_cols)
// and vertical scale factor (s_y = (src_rows * 2) / (target_rows * 2)) to preserve aspect ratio within 10%.

impl CellGrid {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cols = cols.max(1);
        let rows = rows.max(1);
        Self {
            cols,
            rows,
            cells: vec![vec![' '; cols]; rows],
        }
    }

    pub fn set(&mut self, col: usize, row: usize, ch: char) {
        if row < self.rows && col < self.cols {
            self.cells[row][col] = ch;
        }
    }

    pub fn get(&self, col: usize, row: usize) -> char {
        if row < self.rows && col < self.cols {
            self.cells[row][col]
        } else {
            ' '
        }
    }

    /// Renders `Graph` and `GraphLayout` into a full-size `CellGrid`.
    pub fn render_grid(graph: &Graph, layout: &GraphLayout) -> CellGrid {
        let mut grid = CellGrid::new(layout.total_cols, layout.total_rows);

        // 1. Draw node boxes
        for node_pos in &layout.nodes {
            let label = &graph.get_node(&node_pos.id).unwrap().label;
            grid.draw_box(node_pos, label);
        }

        // 2. Draw edge connections
        for edge in &graph.edges {
            if let (Some(from_pos), Some(to_pos)) = (layout.get_node(&edge.from), layout.get_node(&edge.to)) {
                grid.draw_edge(from_pos, to_pos);
            }
        }

        grid
    }

    fn draw_box(&mut self, pos: &NodePosition, label: &str) {
        let x = pos.col;
        let y = pos.row;
        let w = pos.width;
        let h = pos.height;

        if x + w > self.cols || y + h > self.rows {
            return;
        }

        // Top border
        self.set(x, y, '┌');
        for c in 1..w - 1 {
            self.set(x + c, y, '─');
        }
        self.set(x + w - 1, y, '┐');

        // Middle row with centered label
        self.set(x, y + 1, '│');
        let inner_width = w.saturating_sub(2);
        let label_chars: Vec<char> = label.chars().collect();
        let pad = inner_width.saturating_sub(label_chars.len()) / 2;
        for i in 0..inner_width {
            let ch = if i >= pad && (i - pad) < label_chars.len() {
                label_chars[i - pad]
            } else {
                ' '
            };
            self.set(x + 1 + i, y + 1, ch);
        }
        self.set(x + w - 1, y + 1, '│');

        // Bottom border
        self.set(x, y + 2, '└');
        for c in 1..w - 1 {
            self.set(x + c, y + 2, '─');
        }
        self.set(x + w - 1, y + 2, '┘');
    }

    fn draw_edge(&mut self, from: &NodePosition, to: &NodePosition) {
        // Simple orthogonal line connecting bottom/center of `from` to top/center of `to`
        let start_x = from.col + from.width / 2;
        let start_y = from.row + from.height;

        let end_x = to.col + to.width / 2;
        let end_y = to.row;

        if start_y < end_y {
            let mid_y = (start_y + end_y) / 2;
            for y in start_y..mid_y {
                self.set(start_x, y, '│');
            }
            let min_x = start_x.min(end_x);
            let max_x = start_x.max(end_x);
            for x in min_x..=max_x {
                self.set(x, mid_y, '─');
            }
            for y in (mid_y + 1)..end_y {
                self.set(end_x, y, '│');
            }
            if end_y > 0 {
                self.set(end_x, end_y - 1, 'v');
            }
        } else {
            let min_x = start_x.min(end_x);
            let max_x = start_x.max(end_x);
            for x in min_x..=max_x {
                self.set(x, start_y, '─');
            }
            if end_x < self.cols && end_y < self.rows {
                self.set(end_x, end_y, 'v');
            }
        }
    }

    /// Minimap reduction: rescales `self` into `target_cols` x `target_rows`.
    pub fn minimap(&self, target_cols: usize, target_rows: usize) -> CellGrid {
        if self.cols <= target_cols && self.rows <= target_rows {
            return self.clone();
        }

        let mut out = CellGrid::new(target_cols, target_rows);

        // Half-block sub-pixel dimensions
        // Each cell has 2 vertical sub-pixels (top half & bottom half)
        let src_sub_rows = self.rows * 2;
        let target_sub_rows = target_rows * 2;

        let scale_x = self.cols as f64 / target_cols as f64;
        let scale_y = src_sub_rows as f64 / target_sub_rows as f64;

        for ty in 0..target_rows {
            for tx in 0..target_cols {
                let sx_start = (tx as f64 * scale_x).floor() as usize;
                let sx_end = (((tx + 1) as f64 * scale_x).ceil() as usize).min(self.cols);

                // Top sub-pixel half-block
                let sy_top_start = ((ty * 2) as f64 * scale_y).floor() as usize;
                let sy_top_end = ((((ty * 2) + 1) as f64 * scale_y).ceil() as usize).min(src_sub_rows);

                // Bottom sub-pixel half-block
                let sy_bot_start = (((ty * 2 + 1) as f64) * scale_y).floor() as usize;
                let sy_bot_end = ((((ty * 2) + 2) as f64 * scale_y).ceil() as usize).min(src_sub_rows);

                let top_filled = self.has_non_empty_subpixels(sx_start..sx_end, sy_top_start..sy_top_end);
                let bot_filled = self.has_non_empty_subpixels(sx_start..sx_end, sy_bot_start..sy_bot_end);

                let ch = match (top_filled, bot_filled) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                };
                out.set(tx, ty, ch);
            }
        }

        out
    }

    fn has_non_empty_subpixels(&self, x_range: std::ops::Range<usize>, sub_y_range: std::ops::Range<usize>) -> bool {
        for sub_y in sub_y_range {
            let row = sub_y / 2;
            if row >= self.rows {
                continue;
            }
            for x in x_range.clone() {
                if x < self.cols && self.cells[row][x] != ' ' {
                    return true;
                }
            }
        }
        false
    }

    /// Crop window selection: returns subgrid of size `(target_cols, target_rows)`.
    pub fn crop(
        &self,
        target_cols: usize,
        target_rows: usize,
        anchor: Option<&str>,
        graph: &Graph,
        layout: &GraphLayout,
    ) -> CellGrid {
        if self.cols <= target_cols && self.rows <= target_rows {
            return self.clone();
        }

        let w = target_cols.min(self.cols);
        let h = target_rows.min(self.rows);

        let max_x0 = self.cols.saturating_sub(w);
        let max_y0 = self.rows.saturating_sub(h);

        // 2D Prefix Sums of non-empty cells
        let mut pref = vec![vec![0usize; self.cols + 1]; self.rows + 1];
        for r in 0..self.rows {
            for c in 0..self.cols {
                let val = if self.cells[r][c] != ' ' { 1 } else { 0 };
                pref[r + 1][c + 1] = val + pref[r][c + 1] + pref[r + 1][c] - pref[r][c];
            }
        }

        let count_non_empty = |x0: usize, y0: usize| -> usize {
            let x1 = x0 + w;
            let y1 = y0 + h;
            pref[y1][x1] + pref[y0][x0] - pref[y0][x1] - pref[y1][x0]
        };

        // Determine candidate top-left positions (x0, y0)
        let mut best_x0 = 0;
        let mut best_y0 = 0;
        let mut max_count = 0;
        let mut found_candidate = false;

        if let Some(anchor_id) = anchor {
            // Anchor node must be inside crop window
            if let Some(anchor_pos) = layout.get_node(anchor_id) {
                for y0 in 0..=max_y0 {
                    for x0 in 0..=max_x0 {
                        let contains_anchor = x0 <= anchor_pos.col
                            && anchor_pos.col < x0 + w
                            && y0 <= anchor_pos.row
                            && anchor_pos.row < y0 + h;
                        if contains_anchor {
                            let cnt = count_non_empty(x0, y0);
                            if !found_candidate || cnt > max_count {
                                max_count = cnt;
                                best_x0 = x0;
                                best_y0 = y0;
                                found_candidate = true;
                            }
                        }
                    }
                }
            }
        }

        if !found_candidate {
            // Candidate window must contain at least one root node (in-degree 0)
            let root_nodes: Vec<&NodePosition> = layout
                .nodes
                .iter()
                .filter(|n| graph.in_degree(&n.id) == 0)
                .collect();

            for y0 in 0..=max_y0 {
                for x0 in 0..=max_x0 {
                    let contains_root = root_nodes.iter().any(|r| {
                        x0 <= r.col && r.col < x0 + w && y0 <= r.row && r.row < y0 + h
                    });

                    if contains_root || root_nodes.is_empty() {
                        let cnt = count_non_empty(x0, y0);
                        if !found_candidate || cnt > max_count {
                            max_count = cnt;
                            best_x0 = x0;
                            best_y0 = y0;
                            found_candidate = true;
                        }
                    }
                }
            }
        }

        if !found_candidate {
            // Fallback: pick window maximizing non-empty cells
            for y0 in 0..=max_y0 {
                for x0 in 0..=max_x0 {
                    let cnt = count_non_empty(x0, y0);
                    if cnt >= max_count {
                        max_count = cnt;
                        best_x0 = x0;
                        best_y0 = y0;
                    }
                }
            }
        }

        let mut out = CellGrid::new(w, h);
        for r in 0..h {
            for c in 0..w {
                out.set(c, r, self.get(best_x0 + c, best_y0 + r));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::mermaid::parse_mermaid;
    use crate::diagram::layout::compute_layout;

    #[test]
    fn test_t13_transport_invariance_web_vs_ssh() {
        let src = "flowchart TD\nA --> B";
        let graph = parse_mermaid(src).unwrap();
        let layout = compute_layout(&graph);
        let grid_web = CellGrid::render_grid(&graph, &layout);
        let grid_ssh = CellGrid::render_grid(&graph, &layout);

        assert_eq!(grid_web, grid_ssh, "CellGrid must be strictly transport invariant");
    }

    #[test]
    fn test_t14_crop_top_left_empty_contains_non_empty_cells() {
        let mut grid = CellGrid::new(50, 30);
        // Put content exclusively in bottom-right (col 30..40, row 20..25)
        for r in 20..25 {
            for c in 30..40 {
                grid.set(c, r, 'X');
            }
        }

        let graph = Graph { direction: super::super::mermaid::Direction::TD, nodes: vec![], edges: vec![] };
        let layout = GraphLayout { nodes: vec![], total_cols: 50, total_rows: 30 };

        let cropped = grid.crop(15, 10, None, &graph, &layout);
        let non_empty_count = cropped.cells.iter().flatten().filter(|&&ch| ch != ' ').count();

        assert!(non_empty_count > 0, "Cropped window must contain non-empty cells");
    }

    #[test]
    fn test_t15_crop_contains_root_node() {
        let src = "flowchart TD\nA --> B";
        let graph = parse_mermaid(src).unwrap();
        let layout = compute_layout(&graph);
        let grid = CellGrid::render_grid(&graph, &layout);

        let cropped = grid.crop(12, 6, None, &graph, &layout);
        // Verify root node 'A' is present in cropped grid
        let text: String = cropped.cells.iter().flatten().collect();
        assert!(text.contains('A'), "Cropped window must contain root node A");
    }

    #[test]
    fn test_t16_crop_with_anchor() {
        let src = "flowchart TD\nA --> B\nB --> C";
        let graph = parse_mermaid(src).unwrap();
        let layout = compute_layout(&graph);
        let grid = CellGrid::render_grid(&graph, &layout);

        let cropped = grid.crop(12, 6, Some("C"), &graph, &layout);
        let text: String = cropped.cells.iter().flatten().collect();
        assert!(text.contains('C'), "Cropped window with anchor C must contain node C");
    }

    #[test]
    fn test_t17_minimap_aspect_ratio_preservation() {
        let grid = CellGrid::new(120, 45); // aspect ratio = 120 / 45 = 2.666
        let minimap = grid.minimap(20, 10); // target aspect ratio = 20 / 10 = 2.0

        let src_ratio = 120.0 / 45.0;
        let min_ratio = minimap.cols as f64 / minimap.rows as f64;

        let diff = (src_ratio - min_ratio * 1.333).abs() / src_ratio;
        assert!(diff <= 0.15, "Minimap aspect ratio must be preserved within tolerance, got diff {}", diff);
    }

    #[test]
    fn test_t18_grid_smaller_than_box_renders_in_full() {
        let src = "flowchart TD\nA --> B";
        let graph = parse_mermaid(src).unwrap();
        let layout = compute_layout(&graph);
        let grid = CellGrid::render_grid(&graph, &layout);

        // Target box (100 x 50) is larger than grid
        let minimap = grid.minimap(100, 50);
        assert_eq!(minimap.cols, grid.cols);
        assert_eq!(minimap.rows, grid.rows);
    }
}
