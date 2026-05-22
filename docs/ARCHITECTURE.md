# OxiTerm Architecture

## 1. THTML (Terminal HTML)
THTML is a subset of HTML optimized for the character grid.
- `<screen>`: The root container.
- `<box>`: Layout container (equivalent to `div`).
- `<text>`: Text content with wrapping support.
- `<input>`: Text input field.
- `<button>`: Interactive button.
- `<img>`: Graphic element. Supports SVG (`.svg`), Lottie (`.json`), and interactive Rive (`.riv`) formats. Rasterized natively using `resvg` and `tiny-skia` before protocol transmission.

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
- **Vector Rendering Engine**: Multi-tier pipeline using `resvg` for static SVGs, and procedural generators for Lottie loading spinners (.json) and interactive Rive toggles (.riv).
- **Asset Caching System**: Dual cache structures: `SvgCache` storing once-parsed `usvg::Tree` structures, and `AssetCache` storing resolution-keyed Sixel/Kitty graphic byte streams to bypass redundant rendering cycles.
- **Ticking Animation Loop**: Dynamically adjusts event loop timeout to `66ms` (15 FPS) when Lottie or Rive sources are present in the DOM, returning to a low-overhead `5ms` sleep state when idle.
- **Client-Relative Mouse Mapping**: Translates terminal cell grid coordinates to relative canvas offsets inside Rive interactives, driving hover and click toggle animations.
- **Fast Sixel Encoder**: Uses a pre-allocated static 256-color palette to quantize raw RGBA pixel data rapidly, avoiding CPU-heavy palette searches.
- **Predictive Local Echo**: Mitigation for high-latency connections.
- **Synchronized Updates**: Prevents screen tearing via BSU/ESU (Synchronized Updates protocol).
- **Backpressure**: Frame dropping to prevent memory exhaustion during slow rendering.
- **Deep Screen Clearing**: Uses `\x1b[3J` to clear the terminal's scrollback buffer, ensuring a clean slate on startup and resize.

## 6. Diagnostics & Debugging
- **Log Isolation**: All server-side logs are redirected to `/tmp/oxiterm.log` to prevent interference with the SSH TUI data stream.
- **Event Tracing**: Detailed tracing of keyboard and resize events for easier debugging of client-server interactions.
