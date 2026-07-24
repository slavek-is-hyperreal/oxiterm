# Changelog

## [0.4.0] — 2026-07-19

### 🖥 Web Client & Responsive Design
- **Custom WASM Canvas Renderer**: The browser client is a purpose-built `WebTerminal` (Rust → WASM, `oxiterm-web`) that paints the cell grid onto an HTML `<canvas>` — it does **not** use xterm.js. A single responsive `index.html` is served at `/` for all devices.
- **Mobile-Responsive Layouts**: Client-side viewport classification (`< 800px` = mobile) with a `0x11` viewport-sync protocol, server-side hot swap (`SwitchViewport`), and automatic `_mobile.thtml` template resolution (mobile grid `48x30`).
- **DPR-Aware Rendering**: Device-pixel-perfect canvas sizing; pixel↔cell hit-mapping (clicks, wheel, media overlays) uses the true rendered cell size so hit targets stay aligned on any zoom/DPI.
- **Mouse-Wheel Scrolling (web)**: The wheel scrolls a few lines per notch via the existing wheel-mouse path, throttled to avoid flooding.

### 🔌 Sessions, Identity & App Server (Plan 2 / 2.1 / 2.2)
- **Session Reattachment & Takeover**: Browser sessions persist server-side and can be reattached via a `?session=<token>` token; opening a new tab takes over the session (old connection notified via `0xFF`). Session tokens are moved to `sessionStorage` and stripped from the URL.
- **Auto-Reconnection**: Exponential backoff capped at 8s; reattaching with a different `?page=` navigates the session.
- **Identity in App-Server Payloads**: `event-htmx` dispatch payloads now also carry optional `username` and `auth_method` fields alongside `action` / `state` / `session_id`.
- **Activation Integrity**: The input throttle no longer swallows mouse/keyboard activation (Plan 2.2) — a Press always runs hit-test → activation.

### 🧱 Layout, Rendering & Input Fixes
- **Word Wrapping**: `<text>` now supports `wrap: word`, resolved through Taffy measure functions so height follows the final wrapped width (previously auto-width text never wrapped).
- **`<for>` Loop Templates**: Added the `<for each="listKey">` element — its child template is cloned once per item of a `List` state value, with `{item}` substituted in text.
- **Clickable Hit Boxes**: A clickable `<text>` node's layout width now equals its rendered glyph span (min-width floor), so every visible column activates it.
- **Scrolling**: Vertical centering tracks the true content extent (bottom of the document is now reachable); a consistent position-model status indicator (`line X/Y` + bar + %); auto-scroll-to-focused fires only when focus changes, so it no longer fights manual scrolling.
- **Text Input**: Fixed a regression where character input was gated behind the Enter key — typing now updates the focused input's `bind-value` state on every keystroke again.
- **Ambiguous-Width Glyph Handling**: Hardened the whole "clickable label drifts off its hit box" class across all render paths — unambiguous focus-ring/label glyphs, an SSH-emitter cursor re-anchor after ambiguous glyphs, and the web cell-size fix above. A load-time warning and a test guard flag ambiguous-width glyphs used inside clickable labels.

### 🧩 Engine, Media & Accessibility (mid-cycle, since 0.3.0)
- **Input Primitives**: `<input>` with `bind-value`, real-time `TextInput`, and the `AppDispatcher` that POSTs `event-htmx` actions + state to an external App Server and applies returned JSON state patches.
- **Reactive StateManager Growth**: Dynamic tab highlighting and multi-action `event-htmx` chains (`a;b` / `a,b`); state patching applied back into the live session.
- **Scroll & WASM Pipeline**: Server-side scroll rendering and the WASM web rendering path (the "v0.4 pipeline"), including a 6-byte binary cell-diff protocol shared with the web client.
- **Accessibility Layer**: `LinearFrameSink` (`--a11y`) renders the document as a linear text tree for screen readers. The AT-SPI/D-Bus API surface (`DBusBridge`, `register_at_spi`, `update_focus`) is a **skeleton without transport implementation** — no D-Bus connections are established at runtime.
- **Media Buffering**: Frame buffering for video/animation media alongside the SVG/asset caches.
- **Parser & Server Robustness**: CSS-comment stripping, HTML-entity decoding, SSH hot-reload, resilient file watching, and a fix for a navigation focus leak on hidden (`bind-show`) nodes.
- **Web Fixes**: Resolved coordinate translation / connection-init hang and a blank-screen bug (canvas clear-dimension init + handshake packet ordering).
- **Plan 2 App Primitives**: Audit repairs, identity plumbing, and app-server primitives (see identity fields above); expanded contract/unit test coverage.

## [0.3.0] — 2026-05-22

### 🎨 Vector Graphics & Interactive Animations
- **High-Fidelity SVG Rendering**: Integrated `resvg` and `tiny-skia` for rendering SVG vectors.
- **Lottie & Rive Procedural Rendering**:
  - Implemented procedural vector loading spinner animation for `.json` Lottie loops.
  - Implemented procedural Rive toggle switch reacting dynamically to hover and click coordinates.
- **Playback Registry & Event Loop Ticking**:
  - Designed `PlaybackRegistry` to track active animation states, timelines, and hover/click variables.
  - Updated the event loop to scale timeout from `5ms` to `66ms` (15 FPS) during active animations.
- **Interactive Mouse Mapping**: Automated cell-relative mapping for click and hover events within layout bounds.
- **Fast Sixel Encoder**: Pre-allocated 256-color palette quantization in Sixel codec to optimize rendering performance.
- **Caching Layer**: Implemented thread-safe `SvgCache` and resolution-indexed `AssetCache`.

### 🌐 Web Console & Raster Graphics
- **Web Console (Canvas)**: Added the browser console with Canvas-based rendering over WebSocket, letting apps run in a browser with no install.
- **Sixel & Kitty Graphics**: Added raster image rendering via the Kitty Graphics Protocol and a Sixel codec, with automatic terminal-capability negotiation and fallback.
- **QA Hardening**: Resolved QA Audit v3 issues across the server and parser (resource base-directory resolution, edge-case fixes).

### 🚀 Examples & Integration
- **Vector Demo Showcase**: Added `mascot.svg`, `loader.json`, and `toggle.riv` to `examples/`.
- **Linked Dashboard**: Updated default `hello.thtml` page to link to the new Vector & Animation Demo page.

## [0.2.0] — 2026-05-09

### 🚀 Major Architecture Upgrade
- **StateManager v2**: Introduced a fully reactive, subscription-based state management system. 
  - Supports typed values: `Int`, `Str`, `Bool`, and `List`.
  - Precise re-rendering: Only nodes affected by a state change are updated.
  - Native HTMX-style actions: `inc`, `dec`, `toggle`, `set`, `append`, `clear`.
- **TCSS Inline Styles**: Integrated the TCSS engine directly into THTML.
  - Use `style` attribute on any node (e.g., `<box style="fg: red; padding: 1;">`).
  - Support for named colors and TUI-friendly aliases (`fg`, `bg`).
  - Integrated into the `THTMLParser` pipeline.

### 🛡 Security & Stability
- **SSH Hardening**: Fixed a critical vulnerability where unauthorized users could log in if no password was configured.
- **HTMX Sanitization**: Added strict validation for `event-htmx` and `bind-state` to prevent path traversal and ANSI injection.
- **Memory Safety**: Resolved complex borrow checker issues in the rendering loop for better performance and reliability.
- **Improved Testing**: Added 22 comprehensive unit tests covering the state manager, parser, and renderer.

### 🛠 CLI & Developer Tools
- **Hot Reload Improvements**: More robust file watching for THTML development.
- **Optimized Release**: Binned binaries are now significantly smaller and faster.

## [0.1.0] — 2026-05-08

Initial foundation — the "TUI as a website" engine (Sprints 0-6) and first CLI release.

### 🏗 Core Engine & SSR Pipeline
- **Workspace Layout**: Split into `oxiterm-proto`, `oxiterm-renderer`, `oxiterm-server`, `oxiterm-a11y`, `oxiterm-web`, and `oxiterm-cli` crates.
- **THTML Parser**: `nom`-based parser building a DOM arena from THTML markup, with tag/attribute sanitisation.
- **TCSS Styling**: Terminal-CSS engine (colors, flexbox properties, padding/margin, borders) with a `<style>` block cascade.
- **Layout Engine**: Flexbox layout via **Taffy**, mapping the DOM onto a terminal cell grid.
- **Rendering & Diffing**: `CellBuffer` frame rendering and a `DoubleBuffer` diff engine that emits a minimal ANSI escape stream (cursor moves, colors) per frame.

### 🔌 Transport, Input & Resilience
- **SSH Server (russh)**: Asynchronous SSH daemon negotiating PTY dimensions and terminal capabilities (Kitty Graphics, SGR mouse) via DA1.
- **Input Decoding**: `InputStateMachine` parsing Kitty/SGR keyboard and mouse byte streams, driven by a non-blocking Resilient Reactor Thread (RRT).
- **Backpressure & Rate Limiting**: `BoundedFrameChannel` congestion control (safe frame dropping for slow clients) and a connection rate limiter.
- **Metrics**: Prometheus metrics endpoint.

### ♿ Accessibility & 🛠 CLI
- **Accessibility Groundwork**: AT-SPI / a11y layer foundations for screen-reader output.
- **CLI Release**: `oxiterm-cli` with `serve` (host a `.thtml` app over SSH + Web), `demo`, and `check` (template validation) commands.
- **Production Hardening**: Security-audit fixes across the transport layer and stabilised builds.

---
*This changelog was reconstructed from the full commit history; pre-0.2.0 entries summarise the foundational sprints.*

