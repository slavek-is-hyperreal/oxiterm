//! DOM document representation for THTML templates.
//!
//! Exposes an API for structural changes (appending/detaching children),
//! sub-tree cloning, and tracking modified nodes (dirty nodes) that need
//! layout recomputation.

use crate::arena::NodeArena;
use oxiterm_proto::dom::{Node, NodeId, NodeTag};
use anyhow::{Result, anyhow};

/// Represents a THTML DOM document wrapping a node arena and dirty node registry.
#[derive(Clone)]
pub struct THTMLDocument {
    /// The flat arena storing all document nodes.
    pub arena: NodeArena,
    /// The unique identifier of the root screen node.
    pub root: NodeId,
    /// Identifiers of nodes modified since the last layout computation.
    pub dirty_nodes: Vec<NodeId>,
}

impl THTMLDocument {
    /// Creates a new document with an empty node arena and a root [`NodeTag::Screen`] node.
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
    /// Appends a child node to a parent node.
    ///
    /// Marks the parent node as dirty.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent node does not exist in the document arena.
    pub fn append_child(&mut self, parent: NodeId, child: NodeId) -> Result<()> {
        let parent_node = self.arena.get_mut(parent)
            .ok_or_else(|| anyhow!("Parent node {parent:?} not found"))?;
        parent_node.children.push(child);
        self.mark_dirty(parent);
        Ok(())
    }

    /// Detaches a child node from its parent.
    ///
    /// Marks the parent node as dirty.
    ///
    /// # Errors
    ///
    /// Returns an error if the parent node does not exist or does not contain the specified child.
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

    /// Marks the specified node as modified (dirty), queueing it for style/layout recomputation.
    pub fn mark_dirty(&mut self, id: NodeId) {
        if !self.dirty_nodes.contains(&id) {
            self.dirty_nodes.push(id);
        }
    }

    /// Clears the list of dirty nodes.
    pub fn clear_dirty(&mut self) {
        self.dirty_nodes.clear();
    }

    /// Defragments the node arena and updates all active node mappings.
    ///
    /// Updates the root reference ID and the list of registered dirty node IDs.
    pub fn compact(&mut self) {
        let remap = self.arena.compact();
        if let Some(&new_root) = remap.get(&self.root) {
            self.root = new_root;
        }
        
        let mut new_dirty = Vec::new();
        for id in &self.dirty_nodes {
            if let Some(&new_id) = remap.get(id) {
                new_dirty.push(new_id);
            }
        }
        self.dirty_nodes = new_dirty;
    }

    /// Clones a sub-tree of this document starting at `root_id` into a separate document.
    ///
    /// # Errors
    ///
    /// Returns an error if the target root node does not exist in the source document.
    pub fn clone_subtree(&self, root_id: NodeId) -> Result<THTMLDocument> {
        let mut new_doc = THTMLDocument {
            arena: NodeArena::new(),
            root: NodeId(0),
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

    /// Retrieves a reference to the node at the specified ID, if it exists.
    pub fn get_node(&self, id: NodeId) -> Option<&Node> {
        self.arena.get(id)
    }

    /// Retrieves a reference to the root node.
    ///
    /// # Panics
    ///
    /// Panics if the root node does not exist in the arena.
    pub fn get_root(&self) -> &Node {
        self.arena.get(self.root).expect("Root node must exist")
    }
}
