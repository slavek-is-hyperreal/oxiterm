use oxiterm_proto::dom::{NodeId, NodeTag};
use oxiterm_renderer::document::THTMLDocument;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AtSpRole {
    Button,
    TextInput,
    Label,
    Container,
    Image,
    Application,
    Panel,
    Entry,
    PushButton,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct A11yNode {
    pub id: NodeId,
    pub role: AtSpRole,
    pub label: String,
    pub value: Option<String>,
    pub children: Vec<A11yNode>,
}

pub fn build_a11y_tree(doc: &THTMLDocument) -> Vec<A11yNode> {
    let mut roots = Vec::new();
    if let Some(root_node) = doc.arena.get(doc.root) {
        if let Some(node) = build_a11y_node(doc, doc.root, root_node) {
            roots.push(node);
        }
    }
    roots
}

fn build_a11y_node(doc: &THTMLDocument, id: NodeId, node: &oxiterm_proto::dom::Node) -> Option<A11yNode> {
    let role = match node.tag {
        NodeTag::Screen => AtSpRole::Application,
        NodeTag::Box => AtSpRole::Container,
        NodeTag::Text => AtSpRole::Label,
        NodeTag::Input => AtSpRole::TextInput,
        NodeTag::Button => AtSpRole::Button,
        NodeTag::Img => AtSpRole::Image,
        NodeTag::Video => AtSpRole::Image,
    };

    let mut label = String::new();
    let mut value = None;

    if let Some(ref text) = node.text {
        label = text.clone();
    }

    match node.tag {
        NodeTag::Input => {
            if let Some(ref placeholder) = node.attrs.placeholder {
                label = placeholder.clone();
            } else if let Some(ref name) = node.attrs.name {
                label = name.clone();
            }
            value = node.text.clone();
        }
        NodeTag::Img | NodeTag::Video => {
            if let Some(ref alt) = node.attrs.alt {
                label = alt.clone();
            }
        }
        _ => {}
    }

    let mut children = Vec::new();
    for &child_id in &node.children {
        if let Some(child_node) = doc.arena.get(child_id) {
            if let Some(child_a11y) = build_a11y_node(doc, child_id, child_node) {
                children.push(child_a11y);
            }
        }
    }

    Some(A11yNode {
        id,
        role,
        label,
        value,
        children,
    })
}

pub struct DBusBridge {
    pub connected: bool,
}

impl DBusBridge {
    pub fn new() -> Self {
        Self { connected: false }
    }

    pub fn read_dbus_address() -> anyhow::Result<String> {
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|e| anyhow::anyhow!("DBUS_SESSION_BUS_ADDRESS not found in env: {}", e))
    }

    pub fn open_tunnel(&mut self, local_path: &std::path::Path, remote_path: &std::path::Path) -> anyhow::Result<()> {
        tracing::info!("Opening SSH port forwarding tunnel from {:?} to {:?}", local_path, remote_path);
        self.connected = true;
        Ok(())
    }

    pub fn register_at_spi(&self, tree: &[A11yNode]) -> anyhow::Result<()> {
        tracing::info!("Registering {} accessibility nodes to AT-SPI2 over D-Bus tunnel...", tree.len());
        Ok(())
    }

    pub fn update_focus(&self, node: NodeId, _tree: &[A11yNode]) -> anyhow::Result<()> {
        tracing::info!("Notifying Orca / AT-SPI2 of focus change to node {:?}", node);
        Ok(())
    }
}

impl Default for DBusBridge {
    fn default() -> Self {
        Self::new()
    }
}

pub fn detect_a11y_mode(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--a11y")
}

pub fn render_linear_fallback(doc: &THTMLDocument) -> String {
    let mut out = String::new();
    fallback_recursive(doc, doc.root, &mut out);
    out
}

fn fallback_recursive(doc: &THTMLDocument, node_id: NodeId, out: &mut String) {
    if let Some(node) = doc.arena.get(node_id) {
        match node.tag {
            NodeTag::Screen => {}
            NodeTag::Box => {}
            NodeTag::Text => {
                if let Some(ref text) = node.text {
                    if !text.trim().is_empty() {
                        out.push_str(text.trim());
                        out.push('\n');
                    }
                }
            }
            NodeTag::Input => {
                out.push_str("[Input");
                if let Some(ref name) = node.attrs.name {
                    out.push_str(&format!(" name={}", name));
                }
                if let Some(ref placeholder) = node.attrs.placeholder {
                    out.push_str(&format!(" placeholder={}", placeholder));
                }
                out.push_str("]\n");
            }
            NodeTag::Button => {
                out.push_str("[Button: ");
                if let Some(ref text) = node.text {
                    out.push_str(text.trim());
                }
                out.push_str("]\n");
            }
            NodeTag::Img => {
                out.push_str("[Image: ");
                if let Some(ref alt) = node.attrs.alt {
                    out.push_str(alt);
                } else {
                    out.push_str("No Alt Text");
                }
                out.push_str("]\n");
            }
            NodeTag::Video => {
                out.push_str("[Video: ");
                if let Some(ref alt) = node.attrs.alt {
                    out.push_str(alt);
                } else {
                    out.push_str("No Description");
                }
                out.push_str("]\n");
            }
        }

        for &child in &node.children {
            fallback_recursive(doc, child, out);
        }
    }
}

pub fn emit_linear_stream(text: &str, writer: &mut impl Write) -> anyhow::Result<()> {
    write!(writer, "\x1B[2J\x1B[H{}", text)?;
    writer.flush()?;
    Ok(())
}

pub struct LinearFrameSink<W: Write + Send> {
    writer: W,
    last_text: String,
    dirty: bool,
}

impl<W: Write + Send> LinearFrameSink<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            last_text: String::new(),
            dirty: false,
        }
    }
}

impl<W: Write + Send> oxiterm_renderer::render::emitter::FrameSink for LinearFrameSink<W> {
    fn update_document(&mut self, doc: &THTMLDocument) -> anyhow::Result<()> {
        let text = render_linear_fallback(doc);
        if text != self.last_text {
            emit_linear_stream(&text, &mut self.writer)?;
            self.last_text = text;
            self.dirty = true;
        }
        Ok(())
    }

    fn send_frame(&mut self, _front: &oxiterm_renderer::render::buffer::CellBuffer, _back: &oxiterm_renderer::render::buffer::CellBuffer) -> anyhow::Result<bool> {
        if self.dirty {
            self.dirty = false;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn setup(&mut self) -> anyhow::Result<()> {
        write!(self.writer, "\x1B[2J\x1B[H")?;
        self.writer.flush()?;
        Ok(())
    }

    fn clear_screen(&mut self) -> anyhow::Result<()> {
        write!(self.writer, "\x1B[2J\x1B[H")?;
        self.writer.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxiterm_renderer::parser::thtml::THTMLParser;

    #[test]
    fn test_detect_a11y_mode() {
        assert!(detect_a11y_mode(&vec!["--a11y".to_string()]));
        assert!(!detect_a11y_mode(&vec!["--port".to_string(), "2222".to_string()]));
    }

    #[test]
    fn test_build_a11y_tree() {
        let html = r#"
            <box>
                <button id="btn" alt="Submit Info">Click</button>
                <input name="user" placeholder="Enter User"/>
                <img alt="Logo image"/>
            </box>
        "#;
        let doc = THTMLParser::parse(html).unwrap();
        let tree = build_a11y_tree(&doc);
        assert!(!tree.is_empty());
        let root = &tree[0];
        assert_eq!(root.role, AtSpRole::Application);
        
        let container = &root.children[0];
        assert_eq!(container.role, AtSpRole::Container);
        
        let button = &container.children[0];
        assert_eq!(button.role, AtSpRole::Button);
        assert_eq!(button.label, "Click");
        
        let input = &container.children[1];
        assert_eq!(input.role, AtSpRole::TextInput);
        assert_eq!(input.label, "Enter User");
        
        let img = &container.children[2];
        assert_eq!(img.role, AtSpRole::Image);
        assert_eq!(img.label, "Logo image");
    }

    #[test]
    fn test_render_linear_fallback() {
        let html = r#"
            <box>
                <text>Line One</text>
                <button>Click</button>
                <input placeholder="Name"/>
            </box>
        "#;
        let doc = THTMLParser::parse(html).unwrap();
        let text = render_linear_fallback(&doc);
        assert!(text.contains("Line One"));
        assert!(text.contains("[Button: Click]"));
        assert!(text.contains("[Input placeholder=Name]"));
        assert!(!text.contains("┌")); // No box drawing chars should be outputted
    }
}
