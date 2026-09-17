# 💡 OxiTerm Ideas & Strategic Roadmaps

This document serves as the master catalog and index of architectural ideas, concept designs, and future evolution roadmaps for the OxiTerm ecosystem. Each idea is summarized below with direct links to its dedicated, in-depth architectural specification.

---

## 🗺️ Master Index of Ideas

| Idea | Scope / Target | Status | Dedicated Specification |
|---|---|---|---|
| **1. OxiDESK / OxiDE** | Retro Linux Desktop Environment (Pentium II/III/4, 64–512 MB RAM) | Core foundations merged; DE fork planned | [docs/retro-desktop-environment.md](retro-desktop-environment.md) |
| **2. OxiTerm Embedded** | Embedded Linux (SBCs), Industrial HMI, IoT Gateways & OpenWrt | Architecture blueprint defined | [docs/embedded-iot-platform.md](embedded-iot-platform.md) |
| **3. Framebuffer Engine (`oxiterm-fb`)** | Direct hardware `/dev/fb0` & DRM/KMS blitter (Zero X11 / Zero Wayland) | Conceptual design complete | [docs/embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers](embedded-iot-platform.md#31-direct-linux-framebuffer-devfb0-or-drmkms-dumb-buffers) |
| **4. Smooth Physics & Transitions** | Non-linear easing (Cubic Bézier & Damped Spring Oscillator) at 60 FPS | Implemented in core | [docs/tcss-reference.md#10-css-transitions--timing-curves](tcss-reference.md#10-css-transitions--timing-curves) |

---

## 📌 Idea Summaries

### 1. [OxiDESK / OxiDE: Retro Linux Desktop Environment](retro-desktop-environment.md)
* **The Vision:** Transform OxiTerm's high-speed rendering engine into a full-fledged, responsive graphical desktop environment (comparable in user experience to Windows 95/98, Cinnamon, or XFCE) tailored specifically for vintage computers (Pentium II/III/4, Core 2 Duo, 64 MB – 512 MB RAM).
* **Key Innovations:**
  - Running directly on Linux Virtual Consoles (`/dev/tty1`), DRM dumb buffers, or minimal X11/Wayland kiosks.
  - Multi-window manager with z-index stacking contexts, window dragging, collapsing (window shading), and taskbar integration.
  - Sub-50 MB total system RAM consumption and <0.1% CPU idle overhead.
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
  - Analytic second-order differential equation solver for physical damped springs (`spring`, `spring(mass, stiffness, damping)`).
  - Adaptive event loop: active animations tick at 60 FPS (~16ms), while idle states drop immediately to zero CPU usage with a 1000ms idle timeout.
* **Read the full documentation:** 📄 [docs/tcss-reference.md#10-css-transitions--timing-curves](tcss-reference.md#10-css-transitions--timing-curves)
