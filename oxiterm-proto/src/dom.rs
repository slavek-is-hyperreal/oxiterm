use serde::{Deserialize, Serialize};
use crate::style::ComputedStyle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeTag {
    Screen,
    Box,
    Text,
    Input,
    Button,
    Img,
    Video,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeAttributes {
    pub id: Option<String>,
    pub class: Vec<String>,
    pub style_raw: Option<String>,
    pub src: Option<String>,
    pub event_htmx: Option<String>,
    pub bind_state: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub tag: NodeTag,
    pub attrs: NodeAttributes,
    pub children: Vec<NodeId>,
    pub text: Option<String>,
    /// Computed style after applying TCSS
    pub style: ComputedStyle,
}

impl Node {
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
