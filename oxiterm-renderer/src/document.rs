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
    /// Finds clickable text that contains East-Asian *Ambiguous*-width characters.
    ///
    /// A text node counts as clickable when it, or any ancestor, carries an
    /// `event-htmx` handler — i.e. clicking the glyphs triggers the handler. Ambiguous
    /// glyphs (e.g. `←`, `→`, `×`; see [`crate::render::unicode::is_ambiguous_width`])
    /// render 1 cell on some terminals and 2 on others, so the visible label drifts out
    /// from under its hit box and clicks miss. Such glyphs are safe in non-clickable text.
    ///
    /// Returns `(node, text, symbols, letters)` for each violating clickable text node.
    pub fn clickable_ambiguous_width(&self) -> Vec<(NodeId, String, Vec<char>, Vec<char>)> {
        let mut out = Vec::new();
        self.collect_clickable_ambiguous(self.root, false, &mut out);
        out
    }

    fn collect_clickable_ambiguous(
        &self,
        id: NodeId,
        clickable: bool,
        out: &mut Vec<(NodeId, String, Vec<char>, Vec<char>)>,
    ) {
        let Some(node) = self.arena.get(id) else { return };
        let clickable = clickable || node.attrs.event_htmx.is_some();
        if clickable {
            if let Some(text) = &node.text {
                let symbols: Vec<char> = text
                    .chars()
                    .filter(|&c| crate::render::unicode::is_ambiguous_width(c) && !c.is_alphabetic())
                    .collect();
                let letters: Vec<char> = text
                    .chars()
                    .filter(|&c| crate::render::unicode::is_ambiguous_width(c) && c.is_alphabetic())
                    .collect();
                if !symbols.is_empty() || !letters.is_empty() {
                    out.push((id, text.clone(), symbols, letters));
                }
            }
        }
        for &child in &node.children {
            self.collect_clickable_ambiguous(child, clickable, out);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_proto::dom::{Node, NodeTag};

    #[test]
    fn test_clickable_ambiguous_width_classification_t11_t13() {
        // T-11: < Back -> symbols empty, letters empty
        let mut doc1 = THTMLDocument::new();
        let mut btn1 = Node::new(NodeTag::Button);
        btn1.attrs.event_htmx = Some("click".to_string());
        let btn1_id = doc1.arena.alloc(btn1);
        doc1.append_child(doc1.root, btn1_id).unwrap();

        let mut txt1 = Node::new(NodeTag::Text);
        txt1.text = Some("< Back".to_string());
        let txt1_id = doc1.arena.alloc(txt1);
        doc1.append_child(btn1_id, txt1_id).unwrap();

        let res1 = doc1.clickable_ambiguous_width();
        assert!(res1.is_empty(), "T-11: < Back must yield empty results");

        // T-12: Roman Numeral 'Ⅰ' -> symbols empty, letters contains 'Ⅰ'
        let mut doc2 = THTMLDocument::new();
        let mut btn2 = Node::new(NodeTag::Button);
        btn2.attrs.event_htmx = Some("click".to_string());
        let btn2_id = doc2.arena.alloc(btn2);
        doc2.append_child(doc2.root, btn2_id).unwrap();

        let mut txt2 = Node::new(NodeTag::Text);
        txt2.text = Some("Ⅰ Back".to_string());
        let txt2_id = doc2.arena.alloc(txt2);
        doc2.append_child(btn2_id, txt2_id).unwrap();

        let res2 = doc2.clickable_ambiguous_width();
        assert_eq!(res2.len(), 1);
        let (_, _, symbols2, letters2) = &res2[0];
        assert!(symbols2.is_empty(), "T-12: Ⅰ Back symbols must be empty");
        assert_eq!(letters2, &vec!['Ⅰ'], "T-12: Ⅰ Back letters must contain 'Ⅰ'");

        // T-12b: Otwórz / Odśwież -> symbols empty, letters empty (negative control)
        let mut doc2b = THTMLDocument::new();
        let mut btn2b = Node::new(NodeTag::Button);
        btn2b.attrs.event_htmx = Some("click".to_string());
        let btn2b_id = doc2b.arena.alloc(btn2b);
        doc2b.append_child(doc2b.root, btn2b_id).unwrap();

        let mut txt2b = Node::new(NodeTag::Text);
        txt2b.text = Some("Otwórz".to_string());
        let txt2b_id = doc2b.arena.alloc(txt2b);
        doc2b.append_child(btn2b_id, txt2b_id).unwrap();

        let res2b = doc2b.clickable_ambiguous_width();
        assert!(res2b.is_empty(), "T-12b: Otwórz must yield empty results (Polish letters are Neutral)");

        // T-13: ← Back -> symbols contains '←'
        let mut doc3 = THTMLDocument::new();
        let mut btn3 = Node::new(NodeTag::Button);
        btn3.attrs.event_htmx = Some("click".to_string());
        let btn3_id = doc3.arena.alloc(btn3);
        doc3.append_child(doc3.root, btn3_id).unwrap();

        let mut txt3 = Node::new(NodeTag::Text);
        txt3.text = Some("← Back".to_string());
        let txt3_id = doc3.arena.alloc(txt3);
        doc3.append_child(btn3_id, txt3_id).unwrap();

        let res3 = doc3.clickable_ambiguous_width();
        assert_eq!(res3.len(), 1);
        let (_, _, symbols3, letters3) = &res3[0];
        assert_eq!(symbols3, &vec!['←'], "T-13: ← Back symbols must contain '←'");
        assert!(letters3.is_empty(), "T-13: ← Back letters must be empty");
    }
}
