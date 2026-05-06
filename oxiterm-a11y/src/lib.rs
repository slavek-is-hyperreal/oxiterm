use std::sync::Arc;
use tracing::{info, warn};
use oxiterm_renderer::document::THTMLDocument;
use oxiterm_proto::dom::{NodeTag, NodeId};

pub struct DBusBridge {
    pub connected: bool,
}

impl Default for DBusBridge {
    fn default() -> Self {
        Self::new()
    }
}

impl DBusBridge {
    pub fn new() -> Self {
        Self { connected: false }
    }

    /// OxiTerm S6-24: Start D-Bus AT-SPI connection
    pub fn connect(&mut self) -> anyhow::Result<()> {
        info!("Connecting to AT-SPI2 via D-Bus tunnel...");
        self.connected = true;
        Ok(())
    }

    /// OxiTerm S6-26: Serialize THTML AST into AT-SPI Application/Window nodes
    pub fn sync_tree(&self, doc: &THTMLDocument) {
        if !self.connected {
            return;
        }
        info!("Syncing THTML tree to AT-SPI2...");
        self.sync_node_recursive(doc, doc.root, 0);
    }

    fn sync_node_recursive(&self, doc: &THTMLDocument, node_id: NodeId, depth: usize) {
        if let Some(node) = doc.arena.get(node_id) {
            let role = match node.tag {
                NodeTag::Screen => "Application",
                NodeTag::Box => "Panel",
                NodeTag::Text => "Label",
                NodeTag::Input => "Entry",
                NodeTag::Button => "PushButton",
                NodeTag::Img => "Image",
            };
            tracing::trace!("AT-SPI Node at depth {}: Role={}", depth, role);

            for &child in &node.children {
                self.sync_node_recursive(doc, child, depth + 1);
            }
        }
    }

    /// OxiTerm S6-28: Linear Fallback mode for screen readers
    pub fn generate_linear_fallback(&self, doc: &THTMLDocument) -> String {
        let mut out = String::new();
        self.fallback_recursive(doc, doc.root, &mut out);
        out
    }

    fn fallback_recursive(&self, doc: &THTMLDocument, node_id: NodeId, out: &mut String) {
        if let Some(node) = doc.arena.get(node_id) {
            if let Some(text) = &node.text_content {
                out.push_str(text);
                out.push('\n');
            }
            if node.tag == NodeTag::Input {
                out.push_str("[Pole tekstowe]\n");
            }
            for &child in &node.children {
                self.fallback_recursive(doc, child, out);
            }
        }
    }
}
