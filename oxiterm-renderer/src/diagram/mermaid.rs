//! Parser for Mermaid flowchart subset in OxiTerm diagrams.

use anyhow::{anyhow, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    TD,
    LR,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramNode {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub direction: Direction,
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
}

impl Graph {
    pub fn get_node(&self, id: &str) -> Option<&DiagramNode> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn in_degree(&self, id: &str) -> usize {
        self.edges.iter().filter(|e| e.to == id).count()
    }
}

/// Parses a subset of Mermaid flowchart syntax into a `Graph`.
pub fn parse_mermaid(source: &str) -> Result<Graph> {
    if source.len() > 64 * 1024 {
        return Err(anyhow!("Diagram source exceeds maximum size limit of 64 KiB"));
    }

    let mut direction = Direction::TD;
    let mut nodes: Vec<DiagramNode> = Vec::new();
    let mut edges: Vec<DiagramEdge> = Vec::new();
    let mut header_found = false;

    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.startswith("%%") {
            continue;
        }

        if !header_found {
            if line.starts_with("flowchart") || line.starts_with("graph") {
                header_found = true;
                if line.contains("LR") {
                    direction = Direction::LR;
                } else {
                    direction = Direction::TD;
                }
                continue;
            } else {
                // Check for unsupported diagram types
                let first_word = line.split_whitespace().next().unwrap_or(line);
                let unsupported_types = [
                    "sequenceDiagram", "gantt", "classDiagram", "erDiagram",
                    "pie", "stateDiagram", "stateDiagram-v2", "gitGraph", "C4Context"
                ];
                if unsupported_types.iter().any(|&t| first_word == t || line.starts_with(t)) {
                    return Err(anyhow!("Unsupported diagram type: {}", first_word));
                } else {
                    return Err(anyhow!("Unsupported diagram type: {}", first_word));
                }
            }
        }

        // Parse line for edges or nodes
        if line.contains("-->") || line.contains("->") {
            parse_edge_line(line, &mut nodes, &mut edges)?;
        } else {
            parse_node_line(line, &mut nodes)?;
        }

        if nodes.len() > 256 {
            return Err(anyhow!("Diagram exceeds maximum node limit of 256 nodes"));
        }
        if edges.len() > 512 {
            return Err(anyhow!("Diagram exceeds maximum edge limit of 512 edges"));
        }
    }

    if !header_found {
        return Err(anyhow!("Unsupported diagram type: missing flowchart header"));
    }

    Ok(Graph {
        direction,
        nodes,
        edges,
    })
}

fn parse_node_spec(s: &str) -> (String, String) {
    let s = s.trim();
    if let Some(pos) = s.find('[').or_else(|| s.find('(')) {
        let id = s[..pos].trim().to_string();
        let rest = &s[pos + 1..];
        let end_char = if s.as_bytes()[pos] == b'[' { ']' } else { ')' };
        let label = if let Some(end_pos) = rest.rfind(end_char) {
            rest[..end_pos].trim_matches('"').to_string()
        } else {
            rest.trim_matches('"').to_string()
        };
        (id, label)
    } else {
        (s.to_string(), s.to_string())
    }
}

fn add_or_update_node(nodes: &mut Vec<DiagramNode>, id: String, label: String) {
    if let Some(existing) = nodes.iter_mut().find(|n| n.id == id) {
        if existing.label == existing.id && label != id {
            existing.label = label;
        }
    } else {
        nodes.push(DiagramNode { id, label });
    }
}

fn parse_edge_line(line: &str, nodes: &mut Vec<DiagramNode>, edges: &mut Vec<DiagramEdge>) -> Result<()> {
    // Syntax examples:
    // A --> B
    // A -->|label| B
    // A -- label --> B
    let (left_part, right_part, edge_label) = if let Some(pos) = line.find("-->") {
        let left = &line[..pos];
        let right_raw = &line[pos + 3..];
        if let Some(bar_start) = right_raw.find('|') {
            if let Some(bar_end) = right_raw[bar_start + 1..].find('|') {
                let lbl = &right_raw[bar_start + 1..bar_start + 1 + bar_end];
                let right = &right_raw[bar_start + 1 + bar_end + 1..];
                (left, right, Some(lbl.trim().to_string()))
            } else {
                (left, right_raw, None)
            }
        } else {
            (left, right_raw, None)
        }
    } else if let Some(pos) = line.find("->") {
        let left = &line[..pos];
        let right_raw = &line[pos + 2..];
        (left, right_raw, None)
    } else {
        return Ok(());
    };

    let (from_id, from_label) = parse_node_spec(left_part);
    let (to_id, to_label) = parse_node_spec(right_part);

    add_or_update_node(nodes, from_id.clone(), from_label);
    add_or_update_node(nodes, to_id.clone(), to_label);

    edges.push(DiagramEdge {
        from: from_id,
        to: to_id,
        label: edge_label,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_t8_flowchart_td_three_nodes() {
        let src = r#"
            flowchart TD
            A[Start] --> B[Process]
            B --> C[End]
        "#;
        let graph = parse_mermaid(src).unwrap();
        assert_eq!(graph.direction, Direction::TD);
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.nodes[0].id, "A");
        assert_eq!(graph.nodes[0].label, "Start");
        assert_eq!(graph.edges[0].from, "A");
        assert_eq!(graph.edges[0].to, "B");
    }

    #[test]
    fn test_t9_sequence_diagram_unsupported() {
        let src = "sequenceDiagram\nAlice->>Bob: Hello";
        let res = parse_mermaid(src);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("sequenceDiagram"), "Error message must name sequenceDiagram, got: {}", err_msg);
    }

    #[test]
    fn test_t10_node_limit_exceeded() {
        let mut src = String::from("flowchart TD\n");
        for i in 0..257 {
            src.push_str(&format!("N{} --> N{}\n", i, i + 1));
        }
        let res = parse_mermaid(&src);
        assert!(res.is_err());
        let err_msg = res.unwrap_err().to_string();
        assert!(err_msg.contains("256"), "Error message must mention limit 256, got: {}", err_msg);
    }
}
