use std::collections::HashMap;
use oxiterm_proto::dom::NodeId;
use oxiterm_renderer::document::THTMLDocument;
use anyhow::Result;
use tracing::info;

#[derive(Debug, Clone)]
pub enum HtmxEvent {
    Click(NodeId),
    Input(NodeId, String),
    Focus(NodeId),
    Blur(NodeId),
    Mouse(oxiterm_proto::input::MouseInput),
}

pub trait EventHandler: Send + Sync {
    fn handle(&self, event: &HtmxEvent, doc: &mut THTMLDocument) -> Result<()>;
}

pub struct EventBus {
    handlers: HashMap<NodeId, Box<dyn EventHandler>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        Self { handlers: HashMap::new() }
    }

    pub fn register(&mut self, node_id: NodeId, handler: Box<dyn EventHandler>) {
        info!("Registering handler for node: {:?}", node_id);
        self.handlers.insert(node_id, handler);
    }

    pub fn dispatch(&self, event: &HtmxEvent, doc: &mut THTMLDocument, layout: Option<&oxiterm_renderer::LayoutResult>) -> Result<()> {
        let node_id = match event {
            HtmxEvent::Click(id) | HtmxEvent::Input(id, _) | HtmxEvent::Focus(id) | HtmxEvent::Blur(id) => Some(*id),
            HtmxEvent::Mouse(m) => {
                if let Some(layout) = layout {
                    let tester = oxiterm_renderer::HitTester::new(layout);
                    tester.find_node(m.col, m.row)
                } else {
                    None
                }
            }
        };

        if let Some(node_id) = node_id {
            if let Some(handler) = self.handlers.get(&node_id) {
                handler.handle(event, doc)?;
            }
        }
        
        Ok(())
    }
}

pub fn partial_update(doc: &mut THTMLDocument, node_id: NodeId) -> Result<()> {
    info!("Performing partial update for node: {:?}", node_id);
    doc.mark_dirty(node_id);
    Ok(())
}
