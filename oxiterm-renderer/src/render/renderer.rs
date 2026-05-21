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
        
        // 2. Rekurencyjne rysowanie drzewa DOM z centrowaniem korzenia, jeśli ma sztywną mniejszą wielkość
        let (offset_x, offset_y) = layout.get_centering_offset(doc, buffer.width, buffer.height);

        Self::render_recursive(
            doc,
            layout,
            buffer,
            doc.root,
            offset_x,
            offset_y,
            oxiterm_proto::style::AnsiColor::Color256(15),
            oxiterm_proto::style::AnsiColor::Color256(0),
        );
    }

    fn render_recursive(
        doc: &THTMLDocument,
        layout: &LayoutResult,
        buffer: &mut CellBuffer,
        node_id: NodeId,
        parent_x: u16,
        parent_y: u16,
        inherited_fg: oxiterm_proto::style::AnsiColor,
        inherited_bg: oxiterm_proto::style::AnsiColor,
    ) {
        if let Some(node) = doc.arena.get(node_id) {
            let rect = layout.nodes.get(&node_id).copied().unwrap_or_default();
            let abs_x = parent_x + rect.x;
            let abs_y = parent_y + rect.y;

            let resolved_fg = match node.style.fg {
                oxiterm_proto::style::AnsiColor::Reset => inherited_fg,
                c => c,
            };
            let resolved_bg = match node.style.bg {
                oxiterm_proto::style::AnsiColor::Reset => inherited_bg,
                c => c,
            };

            // Draw background
            for y in 0..rect.height {
                for x in 0..rect.width {
                    buffer.set(abs_x + x, abs_y + y, Cell {
                        ch: ' ',
                        fg: resolved_fg,
                        bg: resolved_bg,
                        ..Default::default()
                    });
                }
            }

            // Draw border if defined
            if let Some(border) = &node.style.border {
                let border_fg = match border.fg {
                    oxiterm_proto::style::AnsiColor::Reset => resolved_fg,
                    c => c,
                };
                
                if rect.width > 0 && rect.height > 0 {
                    // Corners
                    buffer.set(abs_x, abs_y, Cell {
                        ch: border.chars.top_left,
                        fg: border_fg,
                        bg: resolved_bg,
                        ..Default::default()
                    });
                    if rect.width > 1 {
                        buffer.set(abs_x + rect.width - 1, abs_y, Cell {
                            ch: border.chars.top_right,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                    if rect.height > 1 {
                        buffer.set(abs_x, abs_y + rect.height - 1, Cell {
                            ch: border.chars.bot_left,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                    if rect.width > 1 && rect.height > 1 {
                        buffer.set(abs_x + rect.width - 1, abs_y + rect.height - 1, Cell {
                            ch: border.chars.bot_right,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }

                    // Horizontal borders
                    for x in 1..rect.width.saturating_sub(1) {
                        buffer.set(abs_x + x, abs_y, Cell {
                            ch: border.chars.top,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                        if rect.height > 1 {
                            buffer.set(abs_x + x, abs_y + rect.height - 1, Cell {
                                ch: border.chars.bot,
                                fg: border_fg,
                                bg: resolved_bg,
                                ..Default::default()
                            });
                        }
                    }

                    // Vertical borders
                    for y in 1..rect.height.saturating_sub(1) {
                        buffer.set(abs_x, abs_y + y, Cell {
                            ch: border.chars.left,
                            fg: border_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                        if rect.width > 1 {
                            buffer.set(abs_x + rect.width - 1, abs_y + y, Cell {
                                ch: border.chars.right,
                                fg: border_fg,
                                bg: resolved_bg,
                                ..Default::default()
                            });
                        }
                    }
                }
            }

            let has_border = node.style.border.is_some();
            let content_x = if has_border { abs_x + 1 } else { abs_x };
            let content_y = if has_border { abs_y + 1 } else { abs_y };
            let content_w = if has_border { rect.width.saturating_sub(2) } else { rect.width };
            let content_h = if has_border { rect.height.saturating_sub(2) } else { rect.height };

            match &node.tag {
                NodeTag::Text => {
                    if let Some(text) = &node.text {
                        let mut cx = 0;
                        let mut cy = 0;
                        for ch in text.chars() {
                            if ch == '\n' {
                                cx = 0;
                                cy += 1;
                            } else {
                                let char_w = crate::render::unicode::UnicodeWidthCache::get().width(ch) as u16;
                                if char_w > 0 {
                                    if cx < content_w && cy < content_h {
                                        buffer.set(content_x + cx, content_y + cy, Cell {
                                            ch,
                                            fg: resolved_fg,
                                            bg: resolved_bg,
                                            ..Default::default()
                                        });
                                        // Fill continuation cells with styled spaces
                                        for i in 1..char_w {
                                            if cx + i < content_w {
                                                buffer.set(content_x + cx + i, content_y + cy, Cell {
                                                    ch: ' ',
                                                    fg: resolved_fg,
                                                    bg: resolved_bg,
                                                    ..Default::default()
                                                });
                                            }
                                        }
                                    }
                                    cx += char_w;
                                }
                            }
                        }
                    }
                }
                NodeTag::Input => {
                    for x in 0..content_w {
                        buffer.set(content_x + x, content_y, Cell {
                            ch: '_',
                            fg: resolved_fg,
                            bg: resolved_bg,
                            ..Default::default()
                        });
                    }
                }
                _ => {}
            }

            for &child_id in &node.children {
                Self::render_recursive(
                    doc,
                    layout,
                    buffer,
                    child_id,
                    parent_x,
                    parent_y,
                    resolved_fg,
                    resolved_bg,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::engine::LayoutEngine;
    use oxiterm_proto::dom::{Node, NodeTag};
    use oxiterm_proto::style::{AnsiColor, BorderStyle, BorderChars};

    #[test]
    fn test_border_and_transparency_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut parent = Node::new(NodeTag::Box);
        parent.style.width = Some(5);
        parent.style.height = Some(3);
        parent.style.bg = AnsiColor::TrueColor(10, 20, 30);
        parent.style.border = Some(BorderStyle {
            fg: AnsiColor::TrueColor(100, 100, 100),
            chars: BorderChars::default(),
        });
        
        let mut child = Node::new(NodeTag::Text);
        child.text = Some("A".to_string());
        child.style.fg = AnsiColor::Reset;
        child.style.width = Some(1);
        child.style.height = Some(1);
        
        let parent_id = doc.arena.alloc(parent);
        let child_id = doc.arena.alloc(child);
        doc.append_child(parent_id, child_id).unwrap();
        doc.append_child(doc.root, parent_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 3).unwrap();

        let mut buffer = CellBuffer::new(5, 3);
        Renderer::render_node(&doc, &layout, &mut buffer);

        // Check top-left corner border character '┌'
        let tl_idx = buffer.flat_idx(0, 0).unwrap();
        let tl_cell = &buffer.cells[tl_idx];
        assert_eq!(tl_cell.ch, '┌');
        assert_eq!(tl_cell.fg, AnsiColor::TrueColor(100, 100, 100));
        assert_eq!(tl_cell.bg, AnsiColor::TrueColor(10, 20, 30));

        // Check offset child text character 'A' at (1, 1) inside border box
        let content_idx = buffer.flat_idx(1, 1).unwrap();
        let content_cell = &buffer.cells[content_idx];
        assert_eq!(content_cell.ch, 'A');
        assert_eq!(content_cell.bg, AnsiColor::TrueColor(10, 20, 30)); // Inherited bg
    }

    #[test]
    fn test_wide_character_rendering() {
        let mut doc = THTMLDocument::new();
        
        let mut text_node = Node::new(NodeTag::Text);
        text_node.text = Some("🚀A".to_string()); // Rocket (width 2) + A (width 1)
        text_node.style.width = Some(5);
        text_node.style.height = Some(1);
        text_node.style.bg = AnsiColor::TrueColor(5, 5, 5);
        
        let node_id = doc.arena.alloc(text_node);
        doc.append_child(doc.root, node_id).unwrap();

        let mut engine = LayoutEngine::new();
        let layout = engine.compute(&mut doc, 5, 1).unwrap();

        let mut buffer = CellBuffer::new(5, 1);
        Renderer::render_node(&doc, &layout, &mut buffer);

        // Check index 0: contains '🚀'
        assert_eq!(buffer.cells[0].ch, '🚀');
        assert_eq!(buffer.cells[0].bg, AnsiColor::TrueColor(5, 5, 5));

        // Check index 1: contains ' ' (continuation cell filled with styled space)
        assert_eq!(buffer.cells[1].ch, ' ');
        assert_eq!(buffer.cells[1].bg, AnsiColor::TrueColor(5, 5, 5));

        // Check index 2: contains 'A' (advanced correctly by 2 cells)
        assert_eq!(buffer.cells[2].ch, 'A');
        assert_eq!(buffer.cells[2].bg, AnsiColor::TrueColor(5, 5, 5));
    }
}
