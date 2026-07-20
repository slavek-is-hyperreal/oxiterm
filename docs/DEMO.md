# OxiTerm Hello & Interactive Showcase Demo

This document describes the interactive showcase application (`examples/hello.thtml`) built as a comprehensive feature demonstration for the OxiTerm engine.

---

## 🌟 Overview

The interactive showcase demonstrates how to build rich, reactive terminal user interfaces using server-side rendered THTML/TCSS layouts. It features:
- **Responsive Flexbox Grid**: Node layout calculated via the Taffy engine.
- **Rich Vector Media**: Real-time rasterized SVG vector graphics, Lottie animations, and interactive Rive widgets.
- **HTMX-style Event Actions**: Reactive buttons, input fields, and tab switching conditions.

---

## 🛠 Features and Demos

### 1. Tabbed Navigation Using `bind-show`
The showcase features interactive tabs (e.g. **Home**, **Media Showcase**, **Interactive Controls**).
- Clicking or focusing and pressing `Enter` on tab headers executes a `set:tab=value` action.
- Content boxes use `bind-show="tab=value"` to conditionally mount layout blocks.

### 2. Svg Vector Mascot (`mascot.svg`)
- Displays OxiTerm's vector mascot on the terminal grid.
- Rasterized on the server using `resvg` and `tiny-skia` library pipelines.
- Transmitted in high-fidelity to compatible terminals using Kitty Graphics Protocol or Sixel.

### 3. Active Lottie Vector Ticker (`bell.json`)
- Runs a vector bell loop animation.
- When the animation tab is active, OxiTerm automatically dynamically scales the session's event loop tick rate to 15 FPS (`66ms` tick) for smooth playback.
- When inactive, the session drops back to an idle sleep state (`5ms` poll) to conserve server CPU cycles.

### 4. Interactive Rive Slider (`toggle.riv`)
- Renders an interactive slider toggle component.
- Translates hovering and mouse clicks directly into Rive runtime inputs.
- Automatically triggers state transitions and keyframe animations inside the cell grid.

---

## ⌨️ Controls & Navigation

OxiTerm parses raw terminal input via a dedicated **Resilient Reactor Thread (RRT)**.

| Action / Key | Function |
|:---|:---|
| `Tab` \| `↓` \| `→` | Focus the next interactive element (button, input, tab header) |
| `Shift + Tab` \| `↑` \| `←` | Focus the previous interactive element |
| `Enter` | Activate the focused button/element or submit an input |
| Mouse Hover / Click | Interact with Rive components, buttons, and focused tabs |
| `PgUp` / `PgDn` | Scroll the active page view up/down |
| `Q` / `q` | Safely disconnect and close the active connection session |

---

## 🚀 Running the Showcase

### 1. Launch the Server
To run the interactive showcase on port `2222` with hot reloading active:
```bash
cargo run --bin oxiterm-cli -- serve examples/hello.thtml --port 2222 --no-auth
```

### 2. Connect via SSH
Connect from any standard terminal emulator:
```bash
ssh localhost -p 2222
```

### 3. Connect via Web Browser
OxiTerm also launches a Web/WebSocket server alongside SSH. Open your browser and navigate to:
```
http://localhost:8080/
```
The browser view uses OxiTerm's own Rust→WASM `WebTerminal` client to paint the cell grid onto an HTML `<canvas>` (no xterm.js involved).
