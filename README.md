# 🚀 OxiTerm — Build TUI like a Website

OxiTerm is a high-performance TUI (Terminal User Interface) platform that lets you host interactive terminal applications over SSH and Web (Canvas/WASM). No client installation required — connect via `ssh yourdomain.com` or open a web browser.

### 🌟 "TUI as a Website"
Why build complex terminal apps with low-level libraries when you can use **THTML**?
- **HTML-like Syntax**: Define layouts, colors, and buttons in THTML files.
- **HTMX-style Interactivity**: Simple `event-htmx` and `bind-state` attributes for real-time interaction.
- **Hot Reload**: Change a `.thtml` file and see the terminal update instantly.

### 🛠 Developer Workflow
```bash
# 1. Clone the repository and build from source
git clone https://github.com/slavek-is-hyperreal/oxiterm.git
cd oxiterm
cargo build --release -p oxiterm-cli

# 2. Start a local dev server with Hot Reload
./target/release/oxiterm-cli serve ./examples/hello.thtml --port 2222

# 3. Connect from any terminal or web browser
ssh localhost -p 2222
# or navigate to http://localhost:8080
```
*(Publishing `oxiterm-cli` to crates.io is planned for a future release).*

### 💎 Key Features
- **App Server Integration**: Connect external backends (Python, Node.js, Rust) via `event-htmx` HTTP requests (`POST /events`) and receive real-time state patches or push updates (`POST /sessions/{id}/patch`). See [app-server-guide.md](docs/app-server-guide.md).
- **Vector Graphics & Animations**: Native support for SVG, Lottie (.json) frame-ticking loops, and a built-in procedural toggle widget for `.riv` sources (no Rive runtime involved).
- **Auto-Negotiated Rendering**: Dynamic detection of terminal capabilities (Kitty Graphics Protocol, Sixel, Unicode half-blocks) with automatic fallbacks.
- **Interactive Mouse Mapping**: Direct translation of cell grid hover/click events to relative coordinates inside canvas nodes.
- **Mobile-Responsive Layouts**: Viewport-aware routing and server-side device detection with dynamic `_mobile.thtml` template resolution.
- **Bounded Backpressure**: Secure `BoundedFrameChannel` architecture prevents memory exhaustion.
- **PUA-B Unicode Stabilization**: Pixel-perfect layouts across different terminal emulators.
- **Predictive Echo**: Zero-latency feedback for keyboard input.

### 📁 Repository Layout
- `examples/` — Isolated single-feature demonstrations of the OxiTerm engine (THTML tags, SVG, Lottie, input fields).
- `spotify-app-server/` — Complete, end-to-end multi-user Spotify Control Center application showing full App Server integration (FastAPI, OAuth 2.0, background push patches, web/SSH/mobile UI). See [spotify-demo.md](docs/spotify-demo.md).

### 🚀 Quick Start (THTML Example)
```html
<box style="bg: #1e293b; padding: 2; flex-direction: column;">
  <box style="flex-direction: row;">
    <text style="fg: #38bdf8; height: 1;">Counter: </text>
    <text bind-state="count" style="fg: #38bdf8; height: 1;">0</text>
  </box>
  <button event-htmx="inc:count" style="fg: #4ade80; height: 1;">[ + ] Increment</button>
  <img src="mascot.svg" style="width: 20; height: 10;" />
</box>
```
