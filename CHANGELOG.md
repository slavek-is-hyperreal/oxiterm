# Changelog

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
*OxiTerm v0.2.0 marks the transition from a prototype to a production-ready TUI framework.*
