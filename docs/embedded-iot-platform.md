# ⚡ OxiTerm Embedded: Ultra-Lightweight GUI & HMI Platform for IoT, SBCs, and Industrial Systems

This document explores the architectural blueprint, hardware feasibility, display output pipelines, and implementation roadmap for utilizing **OxiTerm** as an embedded UI framework, Human-Machine Interface (HMI), and headless device management dashboard.

---

## 1. Executive Summary & The Embedded UI Dilemma

Developing interactive user interfaces for embedded hardware has historically forced engineers into an uncompromising dilemma:

1. **Heavyweight Web / Modern GUI Stacks (Electron, Chromium Embedded, Qt6, GTK4):**
   - **Cost:** Requires 150 MB to 1.5 GB of RAM, GPU hardware acceleration (OpenGL/Vulkan), complex system shared libraries, and high CPU consumption.
   - **Result:** Unusable on low-cost micro-Linux System-on-Chips (SoCs), battery-operated devices, or industrial hardware without active cooling.
2. **Low-Level Microcontroller GUI Libraries (LVGL, TouchGFX, raw C/C++ framebuffer APIs):**
   - **Cost:** Procedural C code, manual pixel and coordinate calculations, static recompilation of firmware for every layout adjustment, and high developer friction.
   - **Result:** Very fast, but building modern, responsive, or remote-accessible interfaces takes weeks of tedious low-level plumbing.
3. **Traditional Terminal UIs (ncurses, Ratatui):**
   - **Cost:** Imperative logic, terminal-bound, lacks declarative markup, no built-in multi-transport (SSH + Web + Framebuffer), no declarative Flexbox styling.

### The OxiTerm Opportunity
OxiTerm bridges this gap by combining **declarative, web-like authoring (THTML/TCSS)** with **bare-metal system efficiency (Rust, Arena DOM, 0% idle CPU)**:

| Metric / Requirement | Chromium / WebKit Embedded | Qt Embedded / GTK4 | LVGL (C library) | OxiTerm Embedded |
|---|---|---|---|---|
| **RAM Footprint** | 250 MB – 1.2 GB | 60 MB – 250 MB | < 1 MB | **2 MB – 15 MB** |
| **Idle CPU Overhead** | 2% – 8% (JS engine, GC) | 1% – 3% | 0% | **0.0% (event-driven sleep)** |
| **Display Dependencies** | Wayland / X11 / Mesa | X11 / Wayland / LinuxFB | Direct Framebuffer / SPI | **Framebuffer (/dev/fb0), TTY, or Serial** |
| **Remote Access Built-in**| Requires separate web server | VNC / RDP plugin | None | **Built-in SSH & WebSocket Canvas** |
| **Layout Paradigm** | HTML / CSS (Flexbox) | QML / Widgets | Procedural C structs | **THTML / TCSS (Flexbox + Transitions)** |
| **Hot Reload** | Yes (in dev server) | Partial (QML) | No (recompile firmware) | **Instant (.thtml live reload on disk)** |

---

## 2. Hardware Tiers & Target Platforms

```
┌───────────────────────────────────────────────────────────────────────────┐
│ Tier 1: Embedded Linux Single-Board Computers (SBCs)                      │
│ - Raspberry Pi Zero 2 W / Pi 3 / Pi 4 / Compute Modules                   │
│ - Allwinner H2+ / H3 / V3s (Orange Pi Zero, Lichee Pi)                    │
│ - NXP i.MX6 / i.MX8, TI Sitara AM335x (BeagleBone Black)                  │
│ - OpenWrt / MIPS / ARM Routers & Network Gateways                         │
│ RAM: 64 MB – 512 MB | OS: Minimal Buildroot, Yocto, Alpine, Debian Lite    │
└───────────────────────────────────────────────────────────────────────────┘
                                      │
┌─────────────────────────────────────▼─────────────────────────────────────┐
│ Tier 2: Microcontroller & RTOS Systems (Future / Core Subset)             │
│ - Espressif ESP32-S3 (with 8MB PSRAM, FreeRTOS / Rust esp-idf)            │
│ - Raspberry Pi RP2350 / STM32H7 (with external RAM)                       │
│ RAM: 2 MB – 16 MB | OS: FreeRTOS / NuttX / Bare-metal with alloc          │
└───────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Physical Display & I/O Pipeline Architecture

OxiTerm Embedded eliminates the need for heavyweight window managers or X11/Wayland display servers. It can output directly to physical hardware through three primary pipelines:

```
                                 ┌───────────────────────────┐
                                 │   Sensor & Hardware APIs  │
                                 │ (I2C, SPI, GPIO, Modbus)  │
                                 └─────────────┬─────────────┘
                                               │ JSON State Patches
                                               ▼
┌────────────────────────────────────────────────────────────────────────────┐
│                       OxiTerm Embedded Engine Runtime                      │
│                                                                            │
│  ┌────────────────────────┐  ┌───────────────────────┐  ┌───────────────┐  │
│  │ StateManager (Sensors) │  │ THTML DOM Arena       │  │ Taffy Flexbox │  │
│  └────────────────────────┘  └───────────────────────┘  └───────────────┘  │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │ Double-Buffer Cell Grid (80x24 / 128x64) + Cell Diff Engine          │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└───────┬───────────────────────────────┬────────────────────────────┬───────┘
        │                               │                            │
        ▼                               ▼                            ▼
┌───────────────────────┐   ┌────────────────────────┐   ┌───────────────────┐
│ Framebuffer Blitter   │   │ Serial / UART Sink     │   │ Remote Server     │
│ (/dev/fb0, DRM dumb)  │   │ (/dev/ttyS0, RS485)    │   │ (SSH / WebSocket) │
│ - Embedded 8x16 font  │   │ - Pure ANSI Escapes    │   │ - Zero client     │
│ - SPI / I2C / HDMI    │   │ - Diagnostic consoles  │   │   installation    │
└───────────────────────┘   └────────────────────────┘   └───────────────────┘
```

### 3.1 Direct Linux Framebuffer (`/dev/fb0` or DRM/KMS Dumb Buffers)
For on-device screens (3.5" to 10.1" TFT/LCD panels, HDMI displays):
- **Bitmap Font Cell Rasterizer:** OxiTerm renders to a character cell grid. A small rasterizer converts each character cell to pixel blocks using an embedded, ultra-crisp monospace bitmap font (such as *Terminus*, *Unifont*, or *Spleen*).
- **Direct Memory Blit:** The rasterizer writes dirty character cells directly to `/dev/fb0` or memory-mapped DRM dumb buffers.
- **Zero GPU Requirement:** 100% software rendered; runs at 60 FPS using less than 1% of a single 400 MHz ARM core.

### 3.2 Serial Console / UART / RS-485
For industrial equipment without screens or remote maintenance ports:
- The engine emits raw ANSI delta sequences directly to a serial UART port (`/dev/ttyS0`).
- A technician plugging in a serial cable or USB-UART adapter instantly sees a rich, full-color interactive dashboard without establishing an IP network connection.

### 3.3 Headless Networked Dashboard (Embedded SSH & Web)
For network appliances, IoT gateways, and headless servers (e.g. OpenWrt routers, solar inverters, NAS boxes):
- No physical display attached.
- Exposes port 2222 (SSH) and port 8080 (Web Canvas).
- Users log in via standard terminal or web browser and get an instant, responsive management UI without running a heavy Apache/Nginx/Node.js stack.

---

## 4. Hardware I/O & Sensor Integration Flow

In industrial automation and IoT, user interfaces must continuously display live sensor readings and trigger hardware actions (relays, motors, GPIO pins).

### 4.1 Real-Time Reactive Sensor Patches
A backend daemon (written in C, Python, or Rust) monitors hardware buses (I2C, SPI, CAN-bus, Modbus) and pushes updates over a local Unix Domain Socket:

```bash
# Daemon emits JSON state patches to OxiTerm's local socket
{"temperature": "48.2°C", "pressure_bar": 4.2, "pump_active": true}
```

In THTML, the interface binds reactively without scripting:
```html
<box style="border-style: rounded; border-color: #38bdf8; padding: 1; width: 40;">
  <text style="fg: #94a3b8;">System Pressure: </text>
  <text bind-state="pressure_bar" style="fg: #4ade80;">0.0</text>
  <text style="fg: #94a3b8;"> bar</text>

  <!-- Conditional alarm banner -->
  <box bind-show="pump_active=true" style="margin-top: 1; bg: #ef4444; fg: #ffffff; padding: 1;">
    <text>⚠️ PUMP OVERHEAT WARNING</text>
  </box>
</box>
```

### 4.2 Interactive Hardware Control
Buttons on the screen or touchscreen clicks dispatch actions directly to the daemon:
```html
<button event-htmx="gpio:relay_1=toggle" style="border-style: single; border-color: #4ade80; height: 3;">
  [ Toggle Emergency Valve ]
</button>
```

---

## 5. Key Use Cases

1. **Industrial Human-Machine Interfaces (HMI):**
   - CNC machines, 3D printers (Klipper/OctoPrint alternative), pump stations, PLC diagnostic panels.
   - Resilient against crashes: static binary, no web browser memory leaks, instant recovery.
2. **Network Gateways & Edge Routers (OpenWrt / pfSense):**
   - Replaces sluggish web interfaces (LuCI) with a blazing-fast, keyboard- and mouse-friendly terminal console over SSH.
3. **Smart Home Wall Panels & Energy Monitors:**
   - Low-cost Raspberry Pi Zero connected to a 3.5" SPI LCD showing solar inverter stats, battery charge, and temperature sensors.
4. **Automotive & Marine Instrument Displays:**
   - OBD-II / NMEA 2000 telemetry dashboards running on 12V low-power SBCs.
5. **Portable Field Diagnostics & Hardware Testing:**
   - Handheld diagnostic tools with miniature monochrome or color displays.

---

## 6. Embedded Feature Roadmap

- [x] **Core Engine:** Ultra-low RAM footprint (<15 MB), 0% idle CPU sleep mode.
- [x] **Layering & Windowing:** `position: absolute`, `z-index`, draggable widgets.
- [x] **Transitions:** Smooth CSS transitions for dials, bars, and gauges.
- [ ] **`oxiterm-fb` Crate:** Standalone Linux framebuffer (`/dev/fb0`) blitter using embedded monospace bitmap fonts.
- [ ] **Linux `evdev` Touchscreen Driver:** Direct touch and mouse input translation from `/dev/input/event*` without X11.
- [ ] **Unix Domain Socket IPC:** High-speed local IPC for daemon state patches and hardware interrupts.
- [ ] **`no_std` / RTOS Port:** Evaluate minimal core allocator support for high-end microcontrollers (ESP32-S3 with PSRAM).
