use oxiterm_proto::dom::{Node, NodeId};
use std::collections::HashMap;

pub struct NodeArena {
    nodes: Vec<Option<Node>>,
    free_list: Vec<u32>,
}

impl NodeArena {
    pub fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(1024),
            free_list: Vec::new(),
        }
    }
}

impl Default for NodeArena {
    fn default() -> Self {
        Self::new()
    }
}

impl NodeArena {

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

    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(usize::try_from(id.0).unwrap()).and_then(Option::as_ref)
    }

    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(usize::try_from(id.0).unwrap()).and_then(Option::as_mut)
    }

    pub fn remove(&mut self, id: NodeId) {
        if let Some(node_slot) = self.nodes.get_mut(usize::try_from(id.0).unwrap()) {
            if node_slot.is_some() {
                *node_slot = None;
                self.free_list.push(id.0);
            }
        }
    }

    /// Defragment the arena by packing active nodes.
    /// Returns a map of old IDs to new IDs.
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

        // Update children pointers in the new nodes
        for node in self.nodes.iter_mut().flatten() {
            for child_id in &mut node.children {
                if let Some(&new_id) = remap.get(child_id) {
                    *child_id = new_id;
                }
            }
        }

        remap
    }

    pub fn occupancy(&self) -> f32 {
        if self.nodes.is_empty() { return 1.0; }
        let active = self.nodes.iter().flatten().count();
        (active as f32) / (self.nodes.len() as f32)
    }
}
