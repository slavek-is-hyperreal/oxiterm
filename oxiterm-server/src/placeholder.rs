//! OxiTerm placeholder user interface.
//!
//! Generates a fallback visual document structure displaying setup and usage instructions
//! when no specific THTML template is provided.

use oxiterm_renderer::document::THTMLDocument;
use oxiterm_proto::dom::{Node, NodeTag};
use oxiterm_proto::style::{AnsiColor, FlexDirection, JustifyContent, AlignItems, BorderStyle, BorderChars};

/// Builds the default fallback template layout with setup instructions.
pub fn build_placeholder_doc(cols: u16, rows: u16) -> THTMLDocument {
    let mut doc = THTMLDocument::new();
    
    let mut main_box = Node::new(NodeTag::Box);
    main_box.style.width = Some(cols);
    main_box.style.height = Some(rows);
    main_box.style.flex_direction = FlexDirection::Column;
    main_box.style.bg = AnsiColor::Color256(234); // dark slate/grey
    main_box.style.justify_content = JustifyContent::Center;
    main_box.style.align_items = AlignItems::Center;
    let main_id = doc.arena.alloc(main_box);
    doc.append_child(doc.root, main_id).unwrap();

    let mut border_box = Node::new(NodeTag::Box);
    border_box.style.width = Some(50.min(cols.saturating_sub(4)));
    border_box.style.height = Some(12.min(rows.saturating_sub(4)));
    border_box.style.flex_direction = FlexDirection::Column;
    border_box.style.justify_content = JustifyContent::Center;
    border_box.style.align_items = AlignItems::Center;
    border_box.style.border = Some(BorderStyle {
        fg: AnsiColor::Color256(39), // blue
        chars: BorderChars::rounded(),
    });
    border_box.style.padding.left = 2;
    border_box.style.padding.right = 2;
    border_box.style.padding.top = 1;
    border_box.style.padding.bottom = 1;
    let border_id = doc.arena.alloc(border_box);
    doc.append_child(main_id, border_id).unwrap();

    let mut title = Node::new(NodeTag::Text);
    title.text = Some("🚀 OxiTerm Framework".to_string());
    title.style.fg = AnsiColor::Color256(226); // yellow
    title.style.height = Some(1);
    let title_id = doc.arena.alloc(title);
    doc.append_child(border_id, title_id).unwrap();

    let mut desc = Node::new(NodeTag::Text);
    desc.text = Some("No THTML document loaded.".to_string());
    desc.style.fg = AnsiColor::Color256(250);
    desc.style.height = Some(1);
    desc.style.margin.top = 1;
    let desc_id = doc.arena.alloc(desc);
    doc.append_child(border_id, desc_id).unwrap();

    let mut cmd1 = Node::new(NodeTag::Text);
    cmd1.text = Some("Start the server with a file:".to_string());
    cmd1.style.fg = AnsiColor::Color256(244);
    cmd1.style.height = Some(1);
    cmd1.style.margin.top = 1;
    let cmd1_id = doc.arena.alloc(cmd1);
    doc.append_child(border_id, cmd1_id).unwrap();

    let mut cmd2 = Node::new(NodeTag::Text);
    cmd2.text = Some("$ oxiterm serve myapp.thtml".to_string());
    cmd2.style.fg = AnsiColor::Color256(46); // green
    cmd2.style.height = Some(1);
    let cmd2_id = doc.arena.alloc(cmd2);
    doc.append_child(border_id, cmd2_id).unwrap();

    let mut exit_text = Node::new(NodeTag::Text);
    exit_text.text = Some("[Q] Exit".to_string());
    exit_text.style.fg = AnsiColor::Color256(240);
    exit_text.style.height = Some(1);
    exit_text.style.margin.top = 1;
    let exit_id = doc.arena.alloc(exit_text);
    doc.append_child(border_id, exit_id).unwrap();

    doc
}
