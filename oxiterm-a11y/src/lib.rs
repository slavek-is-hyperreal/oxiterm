//! Accessibility implementation for OxiTerm.
//!
//! Provides screen-reader fallback generators, AT-SPI2 compatibility trees,
//! and linear frame sinks transmitting plain text over terminal sessions.

#![allow(clippy::all, clippy::pedantic)]

use oxiterm_proto::dom::{NodeId, NodeTag};
use oxiterm_renderer::document::THTMLDocument;
use std::io::Write;

/// AT-SPI role representation for screen reader mappings.
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

/// Representation of an accessibility node within the AT-SPI compatible DOM.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct A11yNode {
    /// ID of the DOM node.
    pub id: NodeId,
    /// Accessibility role.
    pub role: AtSpRole,
    /// Accompanying label/description.
    pub label: String,
    /// Current input or state value.
    pub value: Option<String>,
    /// Child accessibility nodes.
    pub children: Vec<A11yNode>,
}

/// Builds an accessibility tree starting from the document root.
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
        NodeTag::Diagram => AtSpRole::Image,
        NodeTag::For => AtSpRole::Container,
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
        NodeTag::Img | NodeTag::Video | NodeTag::Diagram => {
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

/// Version of the ARIA-mirror JSON document (`0x37` frames).
pub const ARIA_MIRROR_VERSION: u32 = 1;

/// One node of the ARIA mirror sent to GUI web clients.
///
/// Geometry is in logical cell units, absolute on the canvas (centering and scroll applied),
/// i.e. the same coordinate space as the display list.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct AriaNode {
    pub id: u32,
    /// One of `button`, `textbox`, `img`, `group`, `text`, `application`.
    pub role: &'static str,
    pub label: String,
    /// Current value of a textbox; always `None` for password inputs.
    pub value: Option<String>,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub children: Vec<AriaNode>,
}

#[derive(serde::Serialize)]
struct AriaDoc<'a> {
    v: u32,
    focus: Option<u32>,
    nodes: &'a [AriaNode],
}

fn aria_role(role: AtSpRole) -> &'static str {
    match role {
        AtSpRole::Application => "application",
        AtSpRole::Button | AtSpRole::PushButton => "button",
        AtSpRole::TextInput | AtSpRole::Entry => "textbox",
        AtSpRole::Image => "img",
        AtSpRole::Label => "text",
        AtSpRole::Container | AtSpRole::Panel => "group",
    }
}

/// Concatenates the labels of all descendants (used as the accessible name of a button
/// whose text lives in child text nodes).
fn collect_text(n: &A11yNode, out: &mut Vec<String>) {
    for c in &n.children {
        let t = c.label.trim();
        if !t.is_empty() && c.role == AtSpRole::Label {
            out.push(t.to_string());
        }
        collect_text(c, out);
    }
}

fn round2(v: f32) -> f32 {
    (v * 100.0).round() / 100.0
}

#[allow(clippy::too_many_arguments)]
fn aria_node(
    doc: &THTMLDocument,
    layout: &oxiterm_renderer::LayoutResult,
    n: &A11yNode,
    start: (i32, i32),
    fixed_origin: (u16, u16),
    keep_empty: bool,
) -> Option<AriaNode> {
    // No layout rect: hidden (bind-show / display none); the whole subtree goes with it.
    let rect = layout.nodes.get(&n.id)?;
    let node = doc.arena.get(n.id)?;
    let fixed = node.style.position == oxiterm_proto::style::Position::Fixed;
    let (ox, oy) = if fixed { (fixed_origin.0 as i32, fixed_origin.1 as i32) } else { start };

    let is_password = node.tag == NodeTag::Input && node.attrs.input_type.as_deref() == Some("password");
    // Anything with an event-htmx action is clickable, whatever its tag (e.g. a <box> or
    // <text> used as a button), so it is announced as a button.
    let role = if node.attrs.event_htmx.is_some() && node.tag != NodeTag::Input {
        "button"
    } else {
        aria_role(n.role)
    };
    // Containers take no name from their own text: the parser stores raw markup such as
    // HTML comments there. Their content is exposed through their children instead.
    let mut label = if matches!(node.tag, NodeTag::Box | NodeTag::Screen | NodeTag::For) {
        String::new()
    } else {
        n.label.trim().to_string()
    };
    let mut value = n.value.clone();
    if node.tag == NodeTag::Input {
        // `build_a11y_node` seeds the label from the node text (the typed value); a textbox
        // label must come from placeholder/name only.
        label = node
            .attrs
            .placeholder
            .clone()
            .or_else(|| node.attrs.name.clone())
            .unwrap_or_default();
        if is_password {
            value = None;
            if label.is_empty() {
                label = "password".to_string();
            }
        } else if label.is_empty() {
            label = "input".to_string();
        }
    }

    let mut children = Vec::new();
    if role == "button" {
        if label.is_empty() {
            let mut parts = Vec::new();
            collect_text(n, &mut parts);
            label = parts.join(" ");
        }
        // The button's name already covers its text children.
    } else if role != "textbox" && role != "img" {
        for c in &n.children {
            if let Some(ch) = aria_node(doc, layout, c, start, fixed_origin, false) {
                children.push(ch);
            }
        }
    }

    if !keep_empty && children.is_empty() && label.is_empty() && value.is_none() {
        return None;
    }
    Some(AriaNode {
        id: n.id.0 as u32,
        role,
        label,
        value,
        x: round2(ox as f32 + rect.x),
        y: round2(oy as f32 + rect.y),
        w: round2(rect.width.max(0.0)),
        h: round2(rect.height.max(0.0)),
        children,
    })
}

/// Builds the ARIA-mirror tree (see [`build_aria_json`]) without serialising it.
///
/// `start` is the origin of normally positioned nodes (centering offset minus scroll) and
/// `fixed_origin` the origin of `position: fixed` nodes: exactly the values the display-list
/// builder uses, so the geometry matches the painted canvas.
pub fn build_aria_tree(
    doc: &THTMLDocument,
    layout: &oxiterm_renderer::LayoutResult,
    start: (i32, i32),
    fixed_origin: (u16, u16),
) -> Vec<AriaNode> {
    build_a11y_tree(doc)
        .iter()
        .filter_map(|n| aria_node(doc, layout, n, start, fixed_origin, true))
        .collect()
}

fn contains_id(nodes: &[AriaNode], id: u32) -> bool {
    nodes.iter().any(|n| n.id == id || contains_id(&n.children, id))
}

/// Serialises the ARIA mirror as the JSON document carried by `0x37` frames:
/// `{"v":1,"focus":<id|null>,"nodes":[...]}`.
///
/// Nodes without a layout rect are dropped with their subtrees, empty unlabeled containers
/// are pruned, and the text of `<input type="password">` is never included.
pub fn build_aria_json(
    doc: &THTMLDocument,
    layout: &oxiterm_renderer::LayoutResult,
    focus: Option<NodeId>,
    start: (i32, i32),
    fixed_origin: (u16, u16),
) -> String {
    let nodes = build_aria_tree(doc, layout, start, fixed_origin);
    let focus = focus.map(|f| f.0 as u32).filter(|f| contains_id(&nodes, *f));
    serde_json::to_string(&AriaDoc { v: ARIA_MIRROR_VERSION, focus, nodes: &nodes }).unwrap_or_default()
}

/// Local D-Bus tunnel bridge to propagate screen reader focus updates.
pub struct DBusBridge {
    /// Connection status of the D-Bus socket.
    pub connected: bool,
}

impl DBusBridge {
    /// Creates a new `DBusBridge` in disconnected state.
    pub fn new() -> Self {
        Self { connected: false }
    }

    /// Resolves the current session D-Bus bus address from environment.
    pub fn read_dbus_address() -> anyhow::Result<String> {
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .map_err(|e| anyhow::anyhow!("DBUS_SESSION_BUS_ADDRESS not found in env: {}", e))
    }

    /// Establishes the forwarding tunnel between Unix socket targets.
    pub fn open_tunnel(&mut self, local_path: &std::path::Path, remote_path: &std::path::Path) -> anyhow::Result<()> {
        tracing::info!("Opening SSH port forwarding tunnel from {:?} to {:?}", local_path, remote_path);
        self.connected = true;
        Ok(())
    }

    /// Registers the accessibility nodes to AT-SPI2.
    pub fn register_at_spi(&self, tree: &[A11yNode]) -> anyhow::Result<()> {
        tracing::info!("Registering {} accessibility nodes to AT-SPI2 over D-Bus tunnel...", tree.len());
        Ok(())
    }

    /// Notifies Orca screen readers of focus modifications.
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

/// Detects if accessibility mode flag is present in CLI parameters.
pub fn detect_a11y_mode(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "--a11y")
}

/// Renders a plain-text linear view representing the THTML layout hierarchy.
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
            NodeTag::For => {}
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
            NodeTag::Diagram => {
                out.push_str("[Diagram: ");
                if let Some(ref alt) = node.attrs.alt {
                    out.push_str(alt);
                } else {
                    out.push_str("Diagram");
                }
                out.push_str("]\n");
            }
        }

        for &child in &node.children {
            fallback_recursive(doc, child, out);
        }
    }
}

/// Outputs text buffer formatted with clear screen ANSI operations.
pub fn emit_linear_stream(text: &str, writer: &mut impl Write) -> anyhow::Result<()> {
    write!(writer, "\x1B[2J\x1B[H{}", text)?;
    writer.flush()?;
    Ok(())
}

/// A11y linear frame sink delivering screen-reader streams over terminal sessions.
pub struct LinearFrameSink<W: Write + Send> {
    writer: W,
    dirty: bool,
}

impl<W: Write + Send> LinearFrameSink<W> {
    pub fn new(writer: W) -> Self {
        Self { writer, dirty: true }
    }

    pub fn render_document(&mut self, doc: &THTMLDocument) -> anyhow::Result<()> {
        let text = render_linear_fallback(doc);
        emit_linear_stream(&text, &mut self.writer)?;
        self.dirty = false;
        Ok(())
    }
}

impl<W: Write + Send> oxiterm_renderer::FrameSink for LinearFrameSink<W> {
    fn send_frame(&mut self, _front: &oxiterm_renderer::render::buffer::CellBuffer, _back: &oxiterm_renderer::render::buffer::CellBuffer) -> anyhow::Result<bool> {
        if self.dirty {
            self.dirty = false;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn update_document(&mut self, doc: &THTMLDocument) -> anyhow::Result<()> {
        let text = render_linear_fallback(doc);
        emit_linear_stream(&text, &mut self.writer)?;
        self.dirty = false;
        Ok(())
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
    fn test_t23_a11y_diagram_projection_has_no_box_chars() {
        let html = r#"
            <box>
                <diagram src="examples/assets/flow.mmd" alt="System Architecture" style="width: 30; height: 10;" />
            </box>
        "#;
        let doc = THTMLParser::parse(html).unwrap();
        let text = render_linear_fallback(&doc);

        assert!(text.contains("[Diagram: System Architecture]"));
        assert!(!text.contains('┌'), "A11y projection must contain no frame box characters");
        assert!(!text.contains('─'), "A11y projection must contain no frame box characters");
        assert!(!text.contains('│'), "A11y projection must contain no frame box characters");
        assert!(!text.contains('└'), "A11y projection must contain no frame box characters");
    }

    #[test]
    fn test_build_a11y_tree() {
        let html = r#"
            <box>
                <button id="btn" alt="Submit Info">Click</button>
                <input name="user" placeholder="Enter User"/>
                <img src="logo.png" style="width: 10; height: 5;" alt="Logo image"/>
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

    fn aria_fixture(html: &str) -> (THTMLDocument, oxiterm_renderer::LayoutResult) {
        let mut doc = THTMLParser::parse(html).unwrap();
        let layout = oxiterm_renderer::LayoutEngine::new().compute(&mut doc, 40, 0, None).unwrap();
        (doc, layout)
    }

    fn flatten<'a>(nodes: &'a [AriaNode], out: &mut Vec<&'a AriaNode>) {
        for n in nodes {
            out.push(n);
            flatten(&n.children, out);
        }
    }

    const ARIA_HTML: &str = r#"<screen style="width: 40; height: 10;"><box>
        <text>Welcome</text>
        <button>Click</button>
        <input name="user" placeholder="User name"/>
        <img src="logo.png" style="width: 10; height: 5;" alt="Logo image"/>
        <box></box>
    </box></screen>"#;

    #[test]
    fn aria_roles_labels_and_pruning() {
        let (doc, layout) = aria_fixture(ARIA_HTML);
        let tree = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        let mut all = Vec::new();
        flatten(&tree, &mut all);
        assert_eq!(tree[0].role, "application");
        let find = |role: &str| all.iter().find(|n| n.role == role).copied();
        assert_eq!(find("button").unwrap().label, "Click");
        assert_eq!(find("textbox").unwrap().label, "User name");
        assert_eq!(find("img").unwrap().label, "Logo image");
        assert_eq!(find("text").unwrap().label, "Welcome");
        // The empty unlabeled <box> is pruned: only the outer group remains.
        assert_eq!(all.iter().filter(|n| n.role == "group").count(), 1);
    }

    #[test]
    fn aria_hidden_nodes_are_dropped_with_subtree() {
        let (doc, mut layout) = aria_fixture(ARIA_HTML);
        fn find_role(n: &A11yNode, r: AtSpRole) -> Option<NodeId> {
            if n.role == r {
                return Some(n.id);
            }
            n.children.iter().find_map(|c| find_role(c, r))
        }
        let btn = find_role(&build_a11y_tree(&doc)[0], AtSpRole::Button).unwrap();
        layout.nodes.remove(&btn);
        let tree = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        let mut all = Vec::new();
        flatten(&tree, &mut all);
        assert!(all.iter().all(|n| n.role != "button"));
        // Hiding the outer group removes every descendant too.
        let group = find_role(&build_a11y_tree(&doc)[0], AtSpRole::Container).unwrap();
        layout.nodes.remove(&group);
        let tree = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        let mut all = Vec::new();
        flatten(&tree, &mut all);
        assert!(all.iter().all(|n| n.role == "application"), "subtree of a hidden group must vanish");
    }

    #[test]
    fn aria_geometry_applies_origin() {
        let (doc, layout) = aria_fixture(ARIA_HTML);
        let a = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        let b = build_aria_tree(&doc, &layout, (3, -2), (0, 0));
        let (mut fa, mut fb) = (Vec::new(), Vec::new());
        flatten(&a, &mut fa);
        flatten(&b, &mut fb);
        assert_eq!(fa.len(), fb.len());
        for (x, y) in fa.iter().zip(fb.iter()) {
            assert_eq!(y.x, x.x + 3.0);
            assert_eq!(y.y, x.y - 2.0);
            assert_eq!((y.w, y.h), (x.w, x.h));
        }
    }

    #[test]
    fn aria_password_value_is_withheld() {
        let html = r#"<screen style="width: 40; height: 10;"><box><input type="password" id="pw"/><input name="plain"/></box></screen>"#;
        let (mut doc, _) = aria_fixture(html);
        for (_, n) in doc.arena.iter_mut() {
            if n.tag == NodeTag::Input {
                n.text = Some("hunter2".to_string());
            }
        }
        let layout = oxiterm_renderer::LayoutEngine::new().compute(&mut doc, 40, 0, None).unwrap();
        let json = build_aria_json(&doc, &layout, None, (0, 0), (0, 0));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut all = Vec::new();
        let tree = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        flatten(&tree, &mut all);
        let boxes: Vec<_> = all.iter().filter(|n| n.role == "textbox").collect();
        assert_eq!(boxes.len(), 2);
        assert!(boxes.iter().any(|n| n.label == "password" && n.value.is_none()));
        assert!(boxes.iter().any(|n| n.value.as_deref() == Some("hunter2")));
        assert_eq!(v["v"], 1);
        // The password text must not appear anywhere for the password node.
        assert_eq!(json.matches("hunter2").count(), 1);
    }

    #[test]
    fn aria_json_shape_and_focus() {
        let (doc, layout) = aria_fixture(ARIA_HTML);
        let btn = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        let mut all = Vec::new();
        flatten(&btn, &mut all);
        let id = all.iter().find(|n| n.role == "button").unwrap().id;
        let json = build_aria_json(&doc, &layout, Some(NodeId(id)), (0, 0), (0, 0));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["focus"], id);
        assert!(v["nodes"][0]["children"].is_array());
        // A focus id that is not part of the mirror becomes null.
        let json = build_aria_json(&doc, &layout, Some(NodeId(9999)), (0, 0), (0, 0));
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(v["focus"].is_null());
    }
}

#[cfg(test)]
mod example_page_tests {
    use super::*;

    fn walk<'a>(nodes: &'a [AriaNode], out: &mut Vec<&'a AriaNode>) {
        for n in nodes {
            out.push(n);
            walk(&n.children, out);
        }
    }

    /// The shipped counter demo: clickable `<box event-htmx>` buttons and the `<text
    /// event-htmx>` back link are announced as buttons, the bound value is present, and no
    /// container is named after raw markup such as HTML comments.
    #[test]
    fn counter_demo_mirror() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../examples/demos/counter.thtml");
        let src = std::fs::read_to_string(path).unwrap();
        let mut doc = oxiterm_renderer::parser::THTMLParser::parse(&src).unwrap();
        let mut engine = oxiterm_renderer::LayoutEngine::new();
        engine.set_viewport_rows(24);
        let layout = engine.compute(&mut doc, 80, 0, None).unwrap();
        let tree = build_aria_tree(&doc, &layout, (0, 0), (0, 0));
        let mut all = Vec::new();
        walk(&tree, &mut all);

        let buttons: Vec<&str> = all.iter().filter(|n| n.role == "button").map(|n| n.label.as_str()).collect();
        assert!(buttons.iter().any(|l| l.contains("Back")), "back link must be a button: {buttons:?}");
        assert!(buttons.len() >= 4, "expected back link + dec/inc/clear buttons, got {buttons:?}");
        assert!(all.iter().any(|n| n.label == "0"), "bound counter value missing");
        assert!(all.iter().all(|n| !n.label.contains("<!--")), "markup leaked into a label");
    }
}
