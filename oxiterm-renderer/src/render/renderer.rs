use crate::document::THTMLDocument;
use crate::layout::types::LayoutResult;
use crate::render::buffer::{CellBuffer, Cell};
use oxiterm_proto::dom::{NodeTag, NodeId};

pub struct Renderer;

impl Renderer {
    pub fn render_node(doc: &THTMLDocument, layout: &LayoutResult, buffer: &mut CellBuffer) {
        // 1. Całkowite czyszczenie bufora do spacji (zapobiega duszkom jak "PROUALNA")
        for y in 0..buffer.height {
            for x in 0..buffer.width {
                buffer.set(x, y, Cell {
                    ch: ' ',
                    fg: oxiterm_proto::style::AnsiColor::Color256(15),
                    bg: oxiterm_proto::style::AnsiColor::Color256(0),
                    ..Default::default()
                });
            }
        }
        
        // 2. Rekurencyjne rysowanie drzewa DOM
        Self::render_recursive(doc, layout, buffer, doc.root, 0, 0);
    }

    fn render_recursive(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        node_id: NodeId,
        parent_x: u16,
        parent_y: u16,
    ) {
        if let Some(node) = doc.arena.get(node_id) {
            let rect = layout.nodes.get(&node_id).copied().unwrap_or_default();
            let abs_x = parent_x + rect.x;
            let abs_y = parent_y + rect.y;

            let bg = node.style.bg;
            let fg = node.style.fg;

            for y in 0..rect.height {
                for x in 0..rect.width {
                    buffer.set(abs_x + x, abs_y + y, Cell {
                        ch: ' ',
                        fg,
                        bg,
                        ..Default::default()
                    });
                }
            }

            match &node.tag {
                NodeTag::Text => {
                    if let Some(text) = &node.text_content {
                        let mut cx = 0;
                        let mut cy = 0;
                        for ch in text.chars() {
                            if ch == '\n' {
                                cx = 0;
                                cy += 1;
                            } else {
                                if cx < rect.width && cy < rect.height {
                                    buffer.set(abs_x + cx, abs_y + cy, Cell {
                                        ch,
                                        fg,
                                        bg,
                                        ..Default::default()
                                    });
                                }
                                cx += 1;
                            }
                        }
                    }
                }
                NodeTag::Input => {
                    for x in 0..rect.width {
                        buffer.set(abs_x + x, abs_y, Cell {
                            ch: '_',
                            fg,
                            bg,
                            ..Default::default()
                        });
                    }
                }
                _ => {}
            }

            for &child_id in &node.children {
                Self::render_recursive(doc, layout, buffer, child_id, abs_x, abs_y);
            }
        }
    }
}
