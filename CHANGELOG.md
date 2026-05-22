# Changelog

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

### 🚀 Examples & Integration
- **Vector Demo Showcase**: Added `vector_demo.thtml`, `mascot.svg`, `loader.json`, and `toggle.riv` to `examples/`.
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

---
*OxiTerm v0.3.0 introduces vector capabilities and interactive animations to terminal dashboards.*

