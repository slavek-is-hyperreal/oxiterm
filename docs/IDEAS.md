# 💡 OxiTerm Ideas & Strategic Roadmaps

Status column: the stage ids refer to the roadmaps in [roadmap/](roadmap/README.md), which are the source of truth. This document serves as the master catalog and index of architectural ideas, concept designs, and future evolution roadmaps for the OxiTerm ecosystem. Each idea is summarized below with direct links to its dedicated, in-depth architectural specification.

---

## 🗺️ Master Index of Ideas

| Idea | Scope / Target | Status | Dedicated Specification |
|---|---|---|---|
| **1. OxiDESK / OxiDE** | Retro Linux Desktop Environment (Pentium II/III/4, 64–512 MB RAM) | Window features and software sink merged; runner and shell: [7.2b](roadmap/oxiui.md#s-7-2b), [U.OxiDE](roadmap/oxiui.md#s-u-oxide) | [docs/retro-desktop-environment.md](retro-desktop-environment.md) |
| **2. OxiTerm Embedded** | Embedded Linux (SBCs), Industrial HMI, IoT Gateways & OpenWrt | Blueprint; stages [U.IPC](roadmap/oxiui.md#s-u-ipc), [U.Screens](roadmap/oxiui.md#s-u-screens), [U.Bench](roadmap/oxiui.md#s-u-bench) | [docs/embedded-iot-platform.md](embedded-iot-platform.md) |
| **3. Framebuffer Engine (`oxiterm-fb`)** | Direct hardware `/dev/fb0` & DRM/KMS blitter (Zero X11 / Zero Wayland) | `oxiterm-fb` software sink merged (7.2); console runner [7.2b](roadmap/oxiui.md#s-7-2b) | [docs/embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers](embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers) |
| **4. Smooth Physics & Transitions** | Non-linear easing (Cubic Bézier & Damped Spring Oscillator) at 60 FPS | Implemented (0.6.0) | [docs/tcss-reference.md#10-css-transitions--timing-curves](tcss-reference.md#10-css-transitions--timing-curves) |
| **5. Reactive Hover States (`:hover`)** | Any-event cursor tracking (1003h), `:hover` pseudoclass & animated color/glow transitions | Not implemented: [U.Hover](roadmap/oxiui.md#s-u-hover) | [#5-reactive-hover-states--pseudoclasses-hover-focus](#5-reactive-hover-states--pseudoclasses-hover-focus) |
| **6. OxiUI** | One THTML/TCSS source → TUI, web canvas, native GPU window, mobile; server-driven and local modes | Phases 0–6, 7.1, 7.2, 7.6, 5.2 done; rest: [roadmap/oxiui.md](roadmap/oxiui.md) | [docs/roadmap/oxiui.md](roadmap/oxiui.md) |
| **7. OxiScreens** | Many dumb networked screens (tablets, ESP32 LCD, e-ink) driven by one home server; Home Assistant bridge; server-side 3D | Blueprint; stage [U.Screens](roadmap/oxiui.md#s-u-screens) (waits for hardware) | [docs/iot-screens.md](iot-screens.md) |
| **8. Server-rendered high-risk surfaces** | Login, PIN and payment screens with no DOM and no secrets in the browser | Idea: [U.Secure](roadmap/oxiui.md#s-u-secure) | [#8](#8-server-rendered-high-risk-surfaces-zero-dom-zero-secrets-in-the-browser) |
| **9. OxiScript** | General-purpose typed language compiled to WebAssembly, OxiUI as first host | 0.0.1 released; stages in [roadmap/oxiscript.md](roadmap/oxiscript.md) | [docs/oxiscript/](oxiscript/grammar.md) |
| **10. 3D in OxiUI** | Three tiers of server-driven 3D, GPU path in local mode | Draft, not scheduled: [9.1-9.4](roadmap/oxiui.md#s-9-1) | [roadmap/oxiui.md](roadmap/oxiui.md#s-9-1) |
| **11. Terminal+ concept documents** | The original Terminal+ specification and amendment | Historical, superseded by the roadmaps | [docs/history/](history/) |

---

## 📌 Idea Summaries

### 1. [OxiDESK / OxiDE: Retro Linux Desktop Environment](retro-desktop-environment.md)
* **The Vision:** Transform OxiTerm's high-speed rendering engine into a full-fledged, responsive graphical desktop environment (comparable in user experience to Windows 95/98, Cinnamon, or XFCE) tailored specifically for vintage computers (Pentium II/III/4, Core 2 Duo, 64 MB – 512 MB RAM).
* **Key Innovations:**
  - Running directly on Linux Virtual Consoles (`/dev/tty1`), DRM dumb buffers, or minimal X11/Wayland kiosks.
  - Multi-window manager with z-index stacking contexts, window dragging, collapsing (window shading), and taskbar integration.
  - Sub-50 MB total system RAM consumption and <0.1% CPU idle overhead.
  - Planned to sit on top of [OxiUI](roadmap/oxiui.md) (idea #6) as a DisplayList consumer instead of a separate cell-grid fork.
* **Read the full blueprint:** 📄 [docs/retro-desktop-environment.md](retro-desktop-environment.md)

---

### 2. [OxiTerm Embedded: IoT, SBCs & Industrial HMI Platform](embedded-iot-platform.md)
* **The Vision:** Eliminate the bloated Chromium/Electron/Qt stacks on Single-Board Computers (Raspberry Pi Zero, Orange Pi, Allwinner, BeagleBone) and replace procedural C programming (LVGL) with modern declarative THTML/TCSS layouts for machine panels and IoT hubs.
* **Key Innovations:**
  - Direct output to physical displays via Linux Framebuffer (`/dev/fb0`) or SPI/I2C screens using embedded bitmap fonts.
  - Diagnostic serial console output over UART / RS-485 (`/dev/ttyS0`) for headless industrial machinery.
  - Remote zero-install management over SSH and Web (WebSocket Canvas).
  - Native 0.0% idle CPU power efficiency for battery-powered devices.
  - Real-time sensor state patches over Unix Domain Sockets.
* **Read the full blueprint:** 📄 [docs/embedded-iot-platform.md](embedded-iot-platform.md)

---

### 3. Direct Hardware Framebuffer Engine (`oxiterm-fb`)
* **The Vision:** A lightweight, standalone Rust driver crate that takes OxiTerm's double-buffered cell grid and renders it directly to raw display memory (`/dev/fb0` or DRM/KMS dumb buffers) without requiring X11, Wayland, Mesa, or GPU 3D acceleration.
* **Key Innovations:**
  - Embedded crisp monospace bitmap font glyphs (Terminus, Unifont).
  - Dirty-region damage rect blitting to maximize refresh rate on low-bandwidth SPI/parallel LCD buses.
  - Direct `/dev/input/event*` (evdev) touchscreen and mouse event handling.
* **Read the full blueprint:** 📄 [docs/embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers](embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers)

---

### 4. Physics-Based Transitions & Animations
* **The Vision:** Fluid non-linear interface motion in the terminal without CPU overhead.
* **Key Innovations:**
  - Newton-Raphson numerical cubic Bézier solver (`ease`, `ease-in`, `ease-out`, `ease-in-out`).
  - Analytic second-order differential equation solver for physical damped springs (`spring`, `spring(stiffness, damping, mass)`).
  - Adaptive event loop: active animations tick at 60 FPS (~16ms), while idle states drop immediately to zero CPU usage with a 1000ms idle timeout.
  - Declarative style bindings (`bind-width`, `bind-height`, `bind-top`, `bind-left`, `bind-opacity`, `bind-z-index`) connecting state to motion.
* **Read the full documentation:** 📄 [docs/tcss-reference.md#10-css-transitions--timing-curves](tcss-reference.md#10-css-transitions--timing-curves)

---

### 5. Reactive Hover States & Pseudoclasses (`:hover`, `:focus`)
* **The Vision:** Bring web-grade cursor interactivity and smooth hover transitions to the text terminal, allowing elements to organically highlight, glow, or expand as the mouse moves across the screen.
* **Key Innovations:**
  - **Any-Event Terminal Tracking (`X1003h`):** Upgrade SSH mouse negotiator to stream raw cursor motion without requiring buttons to be pressed.
  - **Dynamic Hit-Testing on Move:** Real-time evaluation of `hovered_node` in the layout engine protected by `InputThrottle` token-bucket rate limiting.
  - **TCSS Pseudoclass Cascade:** Support for `:hover`, `:active`, and `:focus` selectors that seamlessly override base styles in `resolve_style`.
  - **Seamless Hover Transitions:** Coupling with `AnimationController` for smooth RGB color fades (`transition: bg 200ms ease-out, fg 200ms ease-out`) and opacity changes on mouse enter/leave without visual jarring.


---

### 6. [OxiUI: One Source → Terminal, Web, Desktop & Mobile](roadmap/oxiui.md)
* **The Vision:** Turn the terminal renderer into one backend among several. The same THTML/TCSS documents, sessions and App Server protocol drive a TUI, a web canvas, a native `winit` + `wgpu` window, and later mobile and framebuffer targets.
* **Key Innovations:**
  - **DisplayList** of `f32` primitives (rect, border, text run, clip, image) between layout and output; `CellBuffer` becomes just the TUI sink.
  - **One language:** `@media (render: tui|gui)` plus silent-ignore of GUI-only properties instead of a separate "UIhtml/UIcss".
  - **Golden cell snapshots** gate every phase so existing TUI apps never regress.
  - **Two execution modes from the same app source:** *server-driven* (thin client paints a DisplayList received over WS, LiveView-style) and *local core* (`oxiterm-core` compiled into a desktop/Android/WASM app, offline, Flutter-style).
  - **No font drift:** a shared font bundle in both modes, and positioned glyph runs in the DisplayList so thin clients never re-measure text.
* **Read the roadmap:** 📄 [docs/roadmap/oxiui.md](roadmap/oxiui.md)

---

### 7. [OxiScreens: Many Dumb Screens, One Home Server](iot-screens.md)
* **The Vision:** Cheap displays around the home (old tablets, ESP32 boards with SPI LCDs, e-ink panels) hold no layout and no logic. One central server renders every screen, keeps shared state, and pushes updates; screens only paint and send touch input back.
* **Key Innovations:**
  - **Zero layout on the device:** change a page once on the server and every screen updates, with no reflashing.
  - **Home Assistant bridge** as an ordinary App Server: entity states become state patches, screen actions become service calls.
  - **ESP32 thin client** that decodes the existing binary cell-diff stream with a bitmap font.
  - **Server-side 3D in three tiers:** pixels (JPEG/H.264), projected 2D vector polygons rendered as DisplayList items, or a cached mesh plus camera updates.
* **Read the full blueprint:** 📄 [docs/iot-screens.md](iot-screens.md)

### 8. Server-rendered high-risk surfaces ("zero DOM, zero secrets in the browser")
*Idea from the owner, 2026-11-25. Not scheduled; does not change the phase order.*
In server-driven GUI mode the browser never receives THTML/TCSS or a DOM form, only draw commands (`0x36`), and sends back pointer/key events. That makes OxiUI a natural fit for login, PIN-entry, payment-approval and admin screens:
- **No DOM to steal from:** no `<input type="password">` for extensions, injected third-party scripts or XSS to read; the ARIA mirror already never sends password values.
- **Logic and validation only on the server:** nothing on the client to tamper with or skip.
- **Shuffled on-screen keypad (bank style):** the client only sends coordinates on a server-drawn image whose digit layout the server reshuffles every time — event loggers see meaningless points.
- **Harder for credential-stuffing bots** (no form selectors) — an obstacle, not a security guarantee (headless browser + OCR).
What it does **not** do, stated honestly: it does not stop phishing (a fake site can stream too — passkeys/WebAuthn, being origin-bound, do); keystrokes typed on a real keyboard are still visible to a keyboard-event logger; password managers and autofill stop working.
**Shape worth building:** a server-rendered login surface plus a narrow thin-client bridge to `navigator.credentials` (WebAuthn/passkeys) for phishing resistance, and an optional shuffled keypad for PINs on untrusted/shared devices, kiosks and machine HMIs (IEC 62443 fit).
**Market evidence:** Remote Browser Isolation does exactly this for untrusted sites — e.g. Cloudflare Browser Isolation's "Network Vector Rendering" streams draw commands instead of the DOM for security reasons. OxiUI is the same architecture for one's own apps.
**Positioning:** "server-rendered high-risk surfaces: zero DOM, zero secrets in the browser" — not "secure login".
