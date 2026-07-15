use std::collections::HashMap;
use oxiterm_proto::dom::{Node, NodeId, NodeTag};
use oxiterm_renderer::document::THTMLDocument;
use anyhow::{bail, Result};

pub fn expand_for_node(
    doc: &mut THTMLDocument,
    for_node_id: NodeId,
    list: &[String],
    cache: &mut HashMap<NodeId, Vec<NodeId>>,
) -> Result<()> {
    {
        let node = doc.arena.get(for_node_id)
            .ok_or_else(|| anyhow::anyhow!("For node not found"))?;
        if node.tag != NodeTag::For {
            bail!("Node is not a For node");
        }
    }

    let template_ids: Vec<NodeId> = cache
        .entry(for_node_id)
        .or_insert_with(|| {
            doc.arena.get(for_node_id)
                .map(|n| n.children.clone())
                .unwrap_or_default()
        })
        .clone();

    for &tmpl_id in &template_ids {
        validate_template_node(doc, tmpl_id)?;
    }

    {
        let node = doc.arena.get_mut(for_node_id).unwrap();
        node.children.truncate(template_ids.len());
    }

    for item in list {
        for &tmpl_id in &template_ids {
            let new_id = clone_node_replacing_item(doc, tmpl_id, item)?;
            let node = doc.arena.get_mut(for_node_id).unwrap();
            node.children.push(new_id);
        }
    }

    Ok(())
}

fn validate_template_node(doc: &THTMLDocument, node_id: NodeId) -> Result<()> {
    let node = doc.arena.get(node_id).ok_or_else(|| anyhow::anyhow!("Node not found"))?;
    if node.attrs.bind_state.is_some() || node.attrs.bind_value.is_some() || node.attrs.event_htmx.is_some() {
        bail!("Bindings in template are not supported in v1");
    }
    let children = node.children.clone();
    for cid in children {
        validate_template_node(doc, cid)?;
    }
    Ok(())
}

fn clone_node_replacing_item(
    doc: &mut THTMLDocument,
    src_id: NodeId,
    item_val: &str,
) -> Result<NodeId> {
    let (tag, attrs, text, child_ids) = {
        let src = doc.arena.get(src_id).unwrap();
        (src.tag, src.attrs.clone(), src.text.clone(), src.children.clone())
    };

    let new_text = text.map(|t| t.replace("{item}", item_val));
    let mut cloned = Node::new(tag);
    cloned.attrs = attrs;
    cloned.text = new_text;

    let new_id = doc.arena.alloc(cloned);
    let mut new_children = Vec::with_capacity(child_ids.len());
    for cid in child_ids {
        let new_child = clone_node_replacing_item(doc, cid, item_val)?;
        new_children.push(new_child);
    }
    doc.arena.get_mut(new_id).unwrap().children = new_children;
    Ok(new_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_24_expand_basic() {
        let mut doc = THTMLDocument::new();
        let mut for_node = Node::new(NodeTag::For);
        for_node.attrs.each = Some("items".to_string());
        
        let mut tmpl_node = Node::new(NodeTag::Text);
        tmpl_node.text = Some("Value: {item}".to_string());
        
        let tmpl_id = doc.arena.alloc(tmpl_node);
        for_node.children.push(tmpl_id);
        
        let for_id = doc.arena.alloc(for_node);
        doc.root = for_id;
        
        let mut cache = HashMap::new();
        let list = vec!["A".to_string(), "B".to_string()];
        
        expand_for_node(&mut doc, for_id, &list, &mut cache).unwrap();
        
        let node = doc.arena.get(for_id).unwrap();
        assert_eq!(node.children.len(), 3); 
        assert_eq!(node.children[0], tmpl_id);
        
        let val_a = doc.arena.get(node.children[1]).unwrap();
        assert_eq!(val_a.text.as_deref(), Some("Value: A"));
        
        let val_b = doc.arena.get(node.children[2]).unwrap();
        assert_eq!(val_b.text.as_deref(), Some("Value: B"));
    }

    #[test]
    fn test_25_expand_twice_shrinks() {
        let mut doc = THTMLDocument::new();
        let mut for_node = Node::new(NodeTag::For);
        for_node.attrs.each = Some("items".to_string());
        
        let mut tmpl_node = Node::new(NodeTag::Text);
        tmpl_node.text = Some("Value: {item}".to_string());
        
        let tmpl_id = doc.arena.alloc(tmpl_node);
        for_node.children.push(tmpl_id);
        
        let for_id = doc.arena.alloc(for_node);
        doc.root = for_id;
        
        let mut cache = HashMap::new();
        
        let list1 = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        expand_for_node(&mut doc, for_id, &list1, &mut cache).unwrap();
        {
            let node = doc.arena.get(for_id).unwrap();
            assert_eq!(node.children.len(), 4);
        }
        
        let list2 = vec!["D".to_string()];
        expand_for_node(&mut doc, for_id, &list2, &mut cache).unwrap();
        {
            let node = doc.arena.get(for_id).unwrap();
            assert_eq!(node.children.len(), 2);
            assert_eq!(node.children[0], tmpl_id);
            let val_d = doc.arena.get(node.children[1]).unwrap();
            assert_eq!(val_d.text.as_deref(), Some("Value: D"));
        }
    }

    #[test]
    fn test_36_restricted_template_returns_err() {
        let mut doc = THTMLDocument::new();
        let mut for_node = Node::new(NodeTag::For);
        for_node.attrs.each = Some("items".to_string());
        
        let mut tmpl_node = Node::new(NodeTag::Input);
        tmpl_node.attrs.bind_value = Some("some_key".to_string());
        
        let tmpl_id = doc.arena.alloc(tmpl_node);
        for_node.children.push(tmpl_id);
        
        let for_id = doc.arena.alloc(for_node);
        doc.root = for_id;
        
        let mut cache = HashMap::new();
        let list = vec!["A".to_string()];
        
        let res = expand_for_node(&mut doc, for_id, &list, &mut cache);
        assert!(res.is_err());
    }
}
