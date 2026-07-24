# OxiTerm Project Site & Interactive Showcase Demo

This document describes the interactive showcase application (`examples/demos/showcase.thtml`) and project site structure built as a comprehensive feature demonstration for the OxiTerm engine.

---

## 🌟 Overview

The interactive showcase demonstrates how to build rich, reactive terminal user interfaces using server-side rendered THTML/TCSS layouts. It features:
- **Progressive Site Structure**: Level 0 (Landing) → Level 1 (Features & Quickstart) → Level 2 (Individual Demos) → Level 3 (Specifications) → Level 4 (Spotify OAuth App).
- **Responsive Flexbox Grid**: Node layout calculated via the Taffy engine.
- **Rich Vector Media**: Real-time rasterized SVG vector graphics (`mascot.svg`), Lottie animations (`bell.json`), and interactive Rive procedural widgets (`toggle.riv`).
- **HTMX-style Event Actions**: Reactive buttons, input fields, tab switching, and state lists.

---

## 🛠 Features and Demos

### 1. Tabbed Navigation Using `bind-show`
The showcase features interactive tabs (e.g. **Home**, **Media Showcase**, **Interactive Controls**).
- Clicking or focusing and pressing `Enter` on tab headers executes a `set:tab=value` action.
- Content boxes use `bind-show="tab=value"` to conditionally mount layout blocks.

### 2. SVG Vector Mascot (`assets/mascot.svg`)
- Displays OxiTerm's vector mascot on the terminal grid.
- Rasterized on the server using `resvg` and `tiny-skia` library pipelines.
- Transmitted in high-fidelity to compatible terminals using Kitty Graphics Protocol or Sixel.

### 3. Active Lottie Vector Ticker (`assets/bell.json`)
- Runs a vector bell loop animation.
- When active, OxiTerm automatically dynamically scales the session's event loop tick rate to 15 FPS (`66ms` tick) for smooth playback.
- When inactive, the session drops back to an idle sleep state (`5ms` poll) to conserve server CPU cycles.

### 4. Interactive Rive Toggle (`assets/toggle.riv`)
- Renders an interactive slider toggle component in `examples/demos/media.thtml`.
- Implemented as a **procedural widget** — OxiTerm's renderer reads the `.riv` file format for geometry, then draws and animates it internally. **No Rive runtime is involved**; the file is not executed.
- Hover and click coordinates drive the toggle state directly via OxiTerm's hit-test system.

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

## 🚀 Running the Project Site & Showcase

### 1. Launch with Startup Script
Run the project site and Spotify control server using the root startup script:
```bash
./run_site.sh
```

Or manually start the engine:
```bash
cargo run --release -p oxiterm-cli -- serve examples/index.thtml --port 2222 --web-port 8087 --no-auth
```

### 2. Connect via SSH
Connect from any standard terminal emulator:
```bash
ssh localhost -p 2222
```

### 3. Connect via Web Browser
Open your browser and navigate to:
```
http://localhost:8087/
```
The browser view uses OxiTerm's own Rust→WASM `WebTerminal` client to paint the cell grid onto an HTML `<canvas>` (no xterm.js involved).
