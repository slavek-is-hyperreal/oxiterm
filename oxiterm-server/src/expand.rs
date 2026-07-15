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
