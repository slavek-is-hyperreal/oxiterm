use taffy::TaffyTree;
use oxiterm_proto::dom::NodeId;
use crate::document::THTMLDocument;

pub struct LayoutEngine {
    pub taffy: TaffyTree<()>,
}

impl LayoutEngine {
    pub fn new() -> Self {
        Self {
            taffy: TaffyTree::new(),
        }
    }

    pub fn compute(&mut self, _doc: &THTMLDocument) -> anyhow::Result<()> {
        // Build Taffy tree and compute layout
        Ok(())
    }
}
