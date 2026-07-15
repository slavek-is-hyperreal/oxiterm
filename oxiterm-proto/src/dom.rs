//! In-memory DOM tree representation for the THTML format.
//!
//! This module contains node tags, their structural attributes,
//! and unique node identifiers used during rendering and state updates.

use serde::{Deserialize, Serialize};
use crate::style::ComputedStyle;

/// DOM node types supported by OxiTerm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeTag {
    /// The root container representing the entire terminal screen.
    Screen,
    /// A flexible layout container (equivalent to HTML `div`).
    Box,
    /// A node containing text only.
    Text,
    /// A text input field supporting keyboard text entry.
    Input,
    /// An interactive clickable button element.
    Button,
    /// A graphical element (raster image or Lottie/Rive animation).
    Img,
    /// A video element.
    Video,
    /// A loop element.
    For,
}

/// Attributes associated with a THTML node, mapping styling,
/// identification, and state machine bindings.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct NodeAttributes {
    /// The unique identifier of the element in the DOM tree.
    pub id: Option<String>,
    /// TCSS style classes assigned to the element.
    pub class: Vec<String>,
    /// Raw inline styles defined directly in the `style` attribute.
    pub style_raw: Option<String>,
    /// The source path for media assets (e.g. images, animations).
    pub src: Option<String>,
    /// The HTMX action name triggered by an element interaction.
    pub event_htmx: Option<String>,
    /// The state key in StateManager associated with the visibility of the element.
    pub bind_state: Option<String>,
    /// Alternative text description for graphical/media elements.
    pub alt: Option<String>,
    /// Helper placeholder text displayed inside an empty input field.
    pub placeholder: Option<String>,
    /// The name attribute associated with the input control (e.g. for form submissions).
    pub name: Option<String>,
    /// A logical condition controlling the node's visibility based on the state.
    pub bind_show: Option<String>,
    /// The state key in StateManager where the current typed value of this input is stored.
    pub bind_value: Option<String>,
    /// The key representing the list of items to iterate over in a For loop.
    pub each: Option<String>,
    /// The input type (e.g. "password", "text") for masking and input behavior.
    pub input_type: Option<String>,
}

/// A unique identifier for a node inside the document's arena.
///
/// Prevents reference cycles in the DOM graph by enabling a flat arena structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

/// A single node representing an element in the THTML document tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    /// The element type (tag) determining its rendering and behavior.
    pub tag: NodeTag,
    /// The collection of XML/THTML attributes passed to the node.
    pub attrs: NodeAttributes,
    /// The list of child nodes identified by their unique `NodeId`.
    pub children: Vec<NodeId>,
    /// Optional text content stored directly inside the node.
    pub text: Option<String>,
    /// Computed styling properties after resolving the TCSS cascade.
    pub style: ComputedStyle,
}

impl Node {
    /// Creates a new node with the specified tag, default attributes, and empty style.
    pub fn new(tag: NodeTag) -> Self {
        Self {
            tag,
            attrs: NodeAttributes::default(),
            children: Vec::new(),
            text: None,
            style: ComputedStyle::default(),
        }
    }
}

/// Interface for state managers handling evaluation of conditional visibility (bind-show).
pub trait StateEvaluator {
    /// Evaluates if the given logical condition is met based on the current session state.
    fn evaluate_bind_show(&self, condition: &str) -> bool;
}
