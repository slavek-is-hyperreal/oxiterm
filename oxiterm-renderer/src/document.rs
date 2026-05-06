use crate::arena::NodeArena;
use oxiterm_proto::dom::{Node, NodeId, NodeTag};
use anyhow::{Result, anyhow};

pub struct THTMLDocument {
    pub arena: NodeArena,
    pub root: NodeId,
    pub dirty_nodes: Vec<NodeId>,
}

impl THTMLDocument {
    pub fn new() -> Self {
        let mut arena = NodeArena::new();
        let root = arena.alloc(Node::new(NodeTag::Screen));
        Self {
            arena,
            root,
            dirty_nodes: Vec::new(),
        }
    }
}

impl Default for THTMLDocument {
    fn default() -> Self {
        Self::new()
    }
}

impl THTMLDocument {

    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        let parent_node = self.arena.get_mut(parent)
            .ok_or_else(|| anyhow!("Parent node {parent:?} not found"))?;
        parent_node.children.push(child);
        self.mark_dirty(parent);
        Ok(())
    }

    pub fn detach_child(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        let parent_node = self.arena.get_mut(parent)
            .ok_or_else(|| anyhow!("Parent node {parent:?} not found"))?;
        
        if let Some(pos) = parent_node.children.iter().position(|&id| id == child) {
            parent_node.children.remove(pos);
            self.mark_dirty(parent);
            Ok(())
        } else {
            Err(anyhow!("Child node {child:?} not found in parent {parent:?}"))
        }
    }

    pub fn mark_dirty(&mut self, id: NodeId) {
        if !self.dirty_nodes.contains(&id) {
            self.dirty_nodes.push(id);
        }
    }

    pub fn clear_dirty(&mut self) {
        self.dirty_nodes.clear();
    }

    /// BUG-M02: Compact the arena and update root/dirty nodes
    pub fn compact(&mut self) {
        let remap = self.arena.compact();
        if let Some(&new_root) = remap.get(&self.root) {
            self.root = new_root;
        }
        
        // Also update dirty_nodes
        let mut new_dirty = Vec::new();
        for id in &self.dirty_nodes {
            if let Some(&new_id) = remap.get(id) {
                new_dirty.push(new_id);
            }
        }
        self.dirty_nodes = new_dirty;
    }

    pub fn clone_subtree(&self, root_id: NodeId) -> Result<THTMLDocument> {
        let mut new_doc = THTMLDocument {
            arena: NodeArena::new(),
            root: NodeId(0), // Placeholder
            dirty_nodes: Vec::new(),
        };
        
        let new_root = self.copy_node_recursive(&mut new_doc, root_id)?;
        new_doc.root = new_root;
        Ok(new_doc)
    }

    fn copy_node_recursive(&self, target_doc: &mut THTMLDocument, source_id: NodeId) -> Result<NodeId> {
        let source_node = self.arena.get(source_id)
            .ok_or_else(|| anyhow!("Source node {source_id:?} not found"))?;
        
        let mut new_node = source_node.clone();
        new_node.children.clear();
        
        let new_id = target_doc.arena.alloc(new_node);
        
        for &child_id in &source_node.children {
            let new_child_id = self.copy_node_recursive(target_doc, child_id)?;
            target_doc.append_child(new_id, new_child_id)?;
        }
        
        Ok(new_id)
    }
}
