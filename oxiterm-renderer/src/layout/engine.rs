use taffy::TaffyTree;
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
}

impl Default for LayoutEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutEngine {

    pub fn compute(&mut self, _doc: &THTMLDocument) -> anyhow::Result<()> {
        // Build Taffy tree and compute layout
        Ok(())
    }
}
