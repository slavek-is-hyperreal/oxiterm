# OxiTerm Architecture

## 1. THTML (Terminal HTML)
THTML is a subset of HTML optimized for the character grid.
- `<screen>`: The root container.
- `<box>`: Layout container (equivalent to `div`).
- `<text>`: Text content with wrapping support.
- `<input>`: Text input field.
- `<button>`: Interactive button.
- `<img>`: Graphic element (Sixel or Kitty protocol).

## 2. TCSS (Terminal CSS)
TCSS provides styling for THTML elements.
- **Units**: `ch` (character width), `lh` (line height).
- **Layout**: Full Flexbox support.
- **Colors**: TrueColor (24-bit RGB) and 256-color fallbacks.
- **Borders**: Unicode Box Drawing characters.

## 3. SSR Engine
The Server-Side Rendering engine follows these steps:
1. **Parsing**: THTML is parsed into an Abstract Syntax Tree (AST).
2. **Styling**: TCSS rules are applied to the AST.
3. **Layout**: Taffy calculates the position and size of each element.
4. **Rendering**: The AST is drawn onto a `CellBuffer`.
5. **Diffing**: The current `CellBuffer` is compared to the previous one to generate minimal ANSI commands.

## 4. SSH Transport
Communication is handled by a custom `russh` server.
- **PTY**: Captures terminal dimensions and resize events.
- **Input**: Keyboard and mouse events are streamed back to the server.
- **Output**: ANSI diffs are streamed to the client.

## 5. Resilience & Optimization
- **Resilient Reactor Thread (RRT)**: Dedicated thread for I/O to prevent blocking the event loop. Uses `InputDecoder` with Kitty and SGR support.
- **Predictive Local Echo**: Mitigation for high-latency network connections via `PredictiveEcho` buffer.
- **Synchronized Updates**: Prevents screen tearing during high-frequency updates via BSU/ESU commands.
- **Backpressure**: `BoundedFrameChannel` drops oldest frames if the renderer is too slow to prevent memory exhaustion.
- **Hit-Testing**: `HitTester` maps screen coordinates to `NodeId` using `LayoutResult` from Taffy.
