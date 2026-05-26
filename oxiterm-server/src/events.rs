//! Event definitions and dispatch bus.
//!
//! Exposes HTMX event types, dynamic event handlers, and dispatch mechanisms
//! to route user interactions (clicks, inputs, mouse coordinates) to their DOM node hooks.

use std::collections::HashMap;
use oxiterm_proto::dom::NodeId;
use oxiterm_renderer::document::THTMLDocument;
use anyhow::Result;
use tracing::info;

/// User interface event triggers parsed from terminal input streams.
#[derive(Debug, Clone)]
pub enum HtmxEvent {
    /// Element click or press event.
    Click(NodeId),
    /// Input value updated event.
    Input(NodeId, String),
    /// Element focused event.
    Focus(NodeId),
    /// Element blurred/unfocused event.
    Blur(NodeId),
    /// Mouse tracking motion or click event.
    Mouse(oxiterm_proto::input::MouseInput),
}

/// Interface for handling interactive events on specific DOM nodes.
pub trait EventHandler: Send + Sync {
    /// Executes reaction logic when an event triggers on the associated node.
    fn handle(&self, event: &HtmxEvent, doc: &mut THTMLDocument) -> Result<()>;
}

/// Central routing registry map linking node identifiers to event handlers.
pub struct EventBus {
    handlers: HashMap<NodeId, Box<dyn EventHandler>>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    /// Creates an empty event bus.
    pub fn new() -> Self {
        Self { handlers: HashMap::new() }
    }

    /// Registers a node event handler.
    pub fn register(&mut self, node_id: NodeId, handler: Box<dyn EventHandler>) {
        info!("Registering handler for node: {:?}", node_id);
        self.handlers.insert(node_id, handler);
    }

    /// Dispatches an event to the registered handler for the targeted node.
    ///
    /// Performs hit-testing if the event is a mouse motion coordinate.
    pub fn dispatch(&self, event: &HtmxEvent, doc: &mut THTMLDocument, layout: Option<&oxiterm_renderer::layout::types::LayoutResult>) -> Result<()> {
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

    /// Special mouse event dispatch path.
    ///
    /// Anchored by spec [QUAL-01].
    pub fn dispatch_mouse(&self, mouse: oxiterm_proto::input::MouseInput, doc: &mut THTMLDocument, layout: &oxiterm_renderer::layout::types::LayoutResult) -> Result<()> {
        self.dispatch(&HtmxEvent::Mouse(mouse), doc, Some(layout))
    }
}

/// Marks a sub-tree of the document dirty to trigger layouts and partial frames.
pub fn partial_update(doc: &mut THTMLDocument, node_id: NodeId) -> Result<()> {
    info!("Performing partial update for node: {:?}", node_id);
    doc.mark_dirty(node_id);
    Ok(())
}
