//! Flat memory arena for DOM nodes.
//!
//! Replaces traditional reference-counted pointer graphs (e.g. `Rc<RefCell<Node>>`)
//! with a flat vector indexed by [`NodeId`]. This avoids memory leaks from reference
//! cycles and improves cache locality during DOM tree traversals.

use oxiterm_proto::dom::{Node, NodeId};
use std::collections::HashMap;

/// A memory arena managing allocation and lifecycles of DOM [`Node`] elements.
#[derive(Clone)]
pub struct NodeArena {
    nodes: Vec<Option<Node>>,
    free_list: Vec<u32>,
}

impl NodeArena {
    /// Creates a new arena with a default initial capacity.
    pub fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(1024),
            free_list: Vec::new(),
        }
    }

    /// Returns an iterator over all active nodes along with their identifiers.
    pub fn iter(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().enumerate().filter_map(|(i, n)| {
            n.as_ref().map(|node| (NodeId(i as u32), node))
        })
    }

    /// Returns a mutable iterator over all active nodes.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (NodeId, &mut Node)> {
        self.nodes.iter_mut().enumerate().filter_map(|(i, n)| {
            n.as_mut().map(|node| (NodeId(i as u32), node))
        })
    }
}

impl Default for NodeArena {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeArena {
    /// Allocates a node in the arena, reusing free slots from `free_list` if available.
    pub fn alloc(&mut self, node: Node) -> NodeId {
        if let Some(index) = self.free_list.pop() {
            self.nodes[usize::try_from(index).unwrap()] = Some(node);
            NodeId(index)
        } else {
            let index = u32::try_from(self.nodes.len()).unwrap();
            self.nodes.push(Some(node));
            NodeId(index)
        }
    }

    /// Retrieves a reference to the node at the specified ID, if active.
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(usize::try_from(id.0).unwrap()).and_then(Option::as_ref)
    }

    /// Retrieves a mutable reference to the node at the specified ID, if active.
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(usize::try_from(id.0).unwrap()).and_then(Option::as_mut)
    }

    /// Deallocates the node at the specified ID, placing its slot back onto the free list.
    pub fn remove(&mut self, id: NodeId) {
        if let Some(node_slot) = self.nodes.get_mut(usize::try_from(id.0).unwrap()) {
            if node_slot.is_some() {
                *node_slot = None;
                self.free_list.push(id.0);
            }
        }
    }

    /// Defragments the arena by packing active nodes and updating their hierarchical pointers.
    ///
    /// Relocates active nodes to the beginning of the vector, resolving slot holes.
    /// Returns a mapping showing how old IDs were translated to new packed IDs.
    pub fn compact(&mut self) -> HashMap<NodeId, NodeId> {
        let mut remap = HashMap::new();
        let mut new_nodes = Vec::with_capacity(self.nodes.len());
        
        for (old_idx, node_opt) in self.nodes.drain(..).enumerate() {
            if let Some(node) = node_opt {
                let new_idx = u32::try_from(new_nodes.len()).unwrap();
                let old_id = NodeId(u32::try_from(old_idx).unwrap());
                let new_id = NodeId(new_idx);
                remap.insert(old_id, new_id);
                new_nodes.push(Some(node));
            }
        }

        self.nodes = new_nodes;
        self.free_list.clear();

        for node in self.nodes.iter_mut().flatten() {
            for child_id in &mut node.children {
                if let Some(&new_id) = remap.get(child_id) {
                    *child_id = new_id;
                }
            }
        }

        remap
    }

    /// Calculates the occupancy ratio of the arena (active nodes / total allocated slots).
    pub fn occupancy(&self) -> f32 {
        if self.nodes.is_empty() { return 1.0; }
        let active = self.nodes.iter().flatten().count();
        (active as f32) / (self.nodes.len() as f32)
    }
}
