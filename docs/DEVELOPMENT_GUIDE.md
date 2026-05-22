# OxiTerm Development Guide: THTML & TCSS

Welcome to the OxiTerm developer guide. This document explains how to build server-side rendered terminal applications using the OxiTerm engine.

## 🏗 Core Concepts

OxiTerm apps use a declarative approach similar to web development, but optimized for the character grid of a terminal.

### 1. THTML (Terminal HTML)
The UI is represented as a tree of **Nodes**. Each node has a `NodeTag` and a `Style`.

| Tag | Purpose | Equivalent |
|-----|---------|------------|
| `Box` | Container for other nodes. Supports Flexbox layout. | `<div>` |
| `Text` | Displays string content. Supports wrapping. | `<span>` / `<p>` |
| `Input` | Designated area for predictive echo and user typing. | `<input>` |
| `Image` | Displays high-fidelity SVG graphics, Lottie spinners, and Rive toggles. | `<img>` |

### 2. TCSS (Terminal CSS)
Styling is done via the `Style` struct attached to each `Node`.

#### Layout Properties
- **Width/Height**: Defined in characters (`u16`).
- **Flex Direction**: `Row` (horizontal) or `Column` (vertical).
- **Justify Content**: `Start`, `Center`, `End`, `SpaceBetween`, `SpaceAround`.
- **Align Items**: `Start`, `Center`, `End`, `Stretch`.
- **Padding**: `left`, `right`, `top`, `bottom` margins inside the node.

#### Visual Properties
- **fg**: Foreground color (Text color).
- **bg**: Background color.
- **Colors**: Use `AnsiColor::Color256(n)` for 256-color palette or `AnsiColor::TrueColor(r, g, b)` for 24-bit color.

---

## 🚀 Building an Application

### 1. Define your State
Create a struct to hold your application data.

```rust
pub struct MyApp {
    pub counter: i32,
}
```

### 2. Implement the Document Builder
This function should take terminal dimensions (`cols`, `rows`) and return a `THTMLDocument`.

```rust
pub fn build_document(&self, cols: u16, rows: u16) -> (THTMLDocument, Option<NodeId>) {
    let mut doc = THTMLDocument::new();
    
    // 1. Create Root Container
    let mut main_box = Node::new(NodeTag::Box);
    main_box.style.width = Some(cols);
    main_box.style.height = Some(rows);
    main_box.style.flex_direction = FlexDirection::Column;
    main_box.style.bg = AnsiColor::Color256(234);
    let main_id = doc.arena.alloc(main_box);
    doc.append_child(doc.root, main_id).unwrap();

    // 2. Add Content
    let mut text_node = Node::new(NodeTag::Text);
    text_node.text_content = Some(format!("Counter: {}", self.counter));
    text_node.style.fg = AnsiColor::Color256(46); // Green
    text_node.style.height = Some(1);
    let text_id = doc.arena.alloc(text_node);
    doc.append_child(main_id, text_id).unwrap();

    (doc, None)
}
```

### 3. Handle Events
Update your state based on key presses. Return `true` if the UI needs to be rebuilt.

```rust
pub fn handle_key(&mut self, ch: char) -> bool {
    match ch {
        '+' => { self.counter += 1; true }
        '-' => { self.counter -= 1; true }
        _ => false,
    }
}
```

---

## 💡 Best Practices

### Responsive Design
Always use the `cols` and `rows` passed to `build_document`. 
For fixed-size elements (like headers), use `Some(3)`. For flexible content, use `rows.saturating_sub(fixed_parts)`.

### Character Grid Constraints
- Remember that characters are roughly 2x taller than they are wide.
- Use `FlexDirection::Column` for vertical lists to ensure proper alignment.
- Always set an explicit `height: Some(1)` for `Text` nodes to avoid zero-height layout collapses.

### Predictive Echo
To provide a lag-free experience on high-latency connections:
1. Create an `Input` node in your layout.
2. Return its `NodeId` from `build_document`.
3. OxiTerm will automatically position the user's typed characters at that location.

### Vector Graphics & Animations
OxiTerm allows embedding scalable graphics directly into layout hierarchies:
1. **Static SVG**: Declare static vector files (e.g. `<img src="mascot.svg" style="width: 32; height: 16;" />`). They are automatically scaled to the tag's target cell-to-pixel bounds and rendered via `resvg`.
2. **Lottie Animations**: Use the `.json` extension (e.g. `<img src="loader.json" style="width: 12; height: 6;" />`). Active animations trigger a 15 FPS redraw frame ticking loop.
3. **Interactive Rive Widgets**: Use the `.riv` extension (e.g. `<img src="toggle.riv" style="width: 24; height: 6;" />`). They support relative sub-cell hover and click event coordinates.

---

## 🎨 Color Reference
OxiTerm supports the full 256-color ANSI palette. Common codes:
- `17-21`: Dark Blues
- `46`: Bright Green
- `226`: Bright Yellow
- `232-255`: Grayscale (Black to White)
