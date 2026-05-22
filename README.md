# 🚀 OxiTerm — Build TUI like a Website

OxiTerm is a high-performance TUI (Terminal User Interface) platform that lets you host interactive terminal applications over SSH. No client installation required—just `ssh demo.oxiterm.dev`.

### 🌟 "TUI as a Website"
Why build complex terminal apps with low-level libraries when you can use **THTML**?
- **HTML-like Syntax**: Define layouts, colors, and buttons in THTML files.
- **HTMX-style Interactivity**: Simple `event-htmx` and `bind-state` attributes for real-time interaction.
- **Hot Reload**: Change a `.thtml` file and see the terminal update instantly.

### 🛠 Developer Workflow
```bash
# 1. Install the OxiTerm CLI
cargo install oxiterm-cli

# 2. Start a local dev server with Hot Reload
oxiterm serve ./myapp.thtml --port 2222

# 3. Connect from any terminal
ssh localhost -p 2222
```

### 💎 Key Features
- **Vector Graphics & Animations**: Native support for SVG, Lottie (.json) frame-ticking loops, and interactive Rive (.riv) widgets.
- **Auto-Negotiated Rendering**: Dynamic detection of terminal capabilities (Kitty Graphics Protocol, Sixel, Unicode half-blocks) with automatic fallbacks.
- **Interactive Mouse Mapping**: Direct translation of cell grid hover/click events to relative coordinates inside Rive canvas nodes.
- **Bounded Backpressure**: Secure `BoundedFrameChannel` architecture prevents memory exhaustion.
- **PUA-B Unicode Stabilization**: Pixel-perfect layouts across different terminal emulators.
- **Predictive Echo**: Zero-latency feedback for keyboard input.
- **Developer Tools**: `oxiterm check` for syntax validation and `oxiterm demo` for instant inspiration.

### 🚀 Quick Start (THTML Example)
```html
<box style="background-color: #1e293b; padding: 2;">
  <text style="color: #38bdf8; font-weight: bold;">Counter: <text bind-state="count">0</text></text>
  <button event-htmx="inc:count">[ + ] Increment</button>
  <img src="mascot.svg" style="width: 20; height: 10;" />
</box>
```

---
*Built with ❤️ in Rust for the modern terminal.*
