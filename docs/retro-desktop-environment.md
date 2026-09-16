# 🐧 OxiDE: OxiTerm as a Lightweight Desktop Environment for Retro Linux

This document outlines the architectural blueprint, hardware feasibility, gap analysis, and roadmap for transforming **OxiTerm** into **OxiDE** (OxiTerm Desktop Environment) — an ultra-lightweight, responsive graphical desktop environment (comparable in UX to Cinnamon, XFCE, or IceWM) specifically optimized for vintage and resource-constrained Linux hardware.

---

## 1. Executive Summary & Retro Hardware Context

### 1.1 The Problem with Modern Linux Desktops
Modern Linux desktop environments (GNOME 40+, Cinnamon, KDE Plasma) and their standard application stacks are unusable on legacy or embedded x86/ARM hardware (e.g., Intel Pentium II/III/4, Pentium M, early Intel Atom, Core 2 Duo, or Geode chips with 64 MB to 512 MB of RAM):
- **Window Managers & Compositors:** Require OpenGL 2.1+ or Vulkan, hardware 3D acceleration, and modern DRM/KMS drivers.
- **Toolkit Bloat:** Modern GTK4 and Qt6 allocate hundreds of megabytes of resident memory just to render basic application frames.
- **Web Runtime Dominance:** Everyday utilities built on Electron, CEF, or WebKit consume 500 MB to 1.5 GB RAM each.

### 1.2 The OxiTerm Advantage
OxiTerm's architecture provides an ideal foundation for a retro desktop environment:
- **Featherweight Footprint:** The entire Rust runtime, DOM arena, and double-buffer diff engine run within **15–25 MB of RAM**.
- **Declarative Web Paradigms without a Browser:** THTML and TCSS provide modern declarative layouts, Flexbox, reactive state bindings, vector rendering, and event dispatching without running WebKit or V8.
- **Direct Terminal/Framebuffer Affordance:** Can render directly to Linux Virtual Consoles (`/dev/tty1`), Linux Framebuffer (`/dev/fb0`), DRM/KMS dumb buffers, or bare X11/Wayland kiosks, bypassing heavy compositors.

| Characteristic | Modern DE (Cinnamon / GNOME) | Lightweight DE (XFCE / LXQt) | OxiDE (OxiTerm Desktop Engine) |
|---|---|---|---|
| **Base RAM Usage** | 800 MB – 1.5 GB | 250 MB – 450 MB | **18 MB – 35 MB** |
| **Graphics API** | OpenGL 3.3+ / Vulkan | X11 XRender / OpenGL 2.0 | **ANSI Diff on TTY / Framebuffer blit** |
| **Rendering Pipeline** | 3D Scene Graph + Shaders | X11 Drawing primitives | **Double-Buffer Cell Grid + Delta Diff** |
| **CPU Overhead (Idle)** | 2% – 8% (modern multi-core) | 1% – 3% (modern multi-core) | **< 0.1% (single-core Pentium III)** |
| **Cold Boot Time** | 20 – 45 seconds | 8 – 15 seconds | **< 0.5 seconds** |

---

## 2. System Architecture

```
┌───────────────────────────────────────────────────────────────────────────┐
│                           Display & Hardware Layer                        │
│   Linux DRM/KMS (/dev/dri/card0)  │  Linux fbdev (/dev/fb0)  │  TTY Raw   │
│   Linux evdev (/dev/input/event*) │  GPM Mouse (/dev/gpmdata)             │
└─────────────────────────────────────▲─────────────────────────────────────┘
                                      │
┌─────────────────────────────────────▼─────────────────────────────────────┐
│                    OxiDE Window Manager & Compositor                      │
│                                                                           │
│  ┌────────────────────────┐  ┌─────────────────┐  ┌────────────────────┐  │
│  │ Desktop Surface        │  │ Active Window 1 │  │ Active Window 2    │  │
│  │ (Wallpaper, Icons)     │  │ (THTML/TCSS DOM)│  │ (THTML/TCSS DOM)   │  │
│  └────────────────────────┘  └─────────────────┘  └────────────────────┘  │
│  ┌─────────────────────────────────────────────┐  ┌────────────────────┐  │
│  │ Taskbar / Panel Surface (Menu, Tray, Pager) │  │ Notification Toast │  │
│  └─────────────────────────────────────────────┘  └────────────────────┘  │
│                                                                           │
│  - Multi-Surface Z-Stack Compositor (Floating & Tiled Windows)            │
│  - Absolute & Fixed Positioning Engine (Taffy + Coordinate Transform)    │
│  - Focus Manager & Global Keybindings (Alt+Tab, Super Menu)              │
│  - Hardware CellBuffer & Dirty-Region Blitter                            │
└─────────────────────────────────────▲─────────────────────────────────────┘
                                      │ IPC via Unix Domain Socket
┌─────────────────────────────────────▼─────────────────────────────────────┐
│                       OxiDE System Daemon (Host OS)                       │
│                                                                           │
│  - Application Supervisor: Fork/exec, PTY multiplexing, XDG .desktop      │
│  - System Telemetry: Real-time CPU, RAM, Disk, Battery from /proc, /sys    │
│  - D-Bus Subsystem Bridge:                                                │
│      * org.freedesktop.Notifications (toast notifications)                │
│      * org.freedesktop.StatusNotifierWatcher (System Tray icons)          │
│      * org.freedesktop.login1 (Suspend, Reboot, Poweroff)                 │
│  - Hardware Control: ALSA (volume via amixer), NetworkManager (nmcli)     │
└───────────────────────────────────────────────────────────────────────────┘
```

---

## 3. Gap Analysis: What Must Be Added or Changed in OxiTerm

### 3.1 Styling & Layout Engine (`oxiterm-proto`, `oxiterm-renderer`)

1. **Absolute & Fixed Positioning (`position: absolute / fixed`):**
   - *Current limitation:* All containers are standard Flexbox flow. There is no `position: absolute`, `position: fixed`, or explicit `top`, `bottom`, `left`, `right`.
   - *Required change:* Extend `oxiterm_proto::style::ComputedStyle` with `position: PositionType` and offsets. Pass `Position::Absolute` to Taffy tree nodes. This allows floating windows, dropdown menus, context menus, tooltips, and taskbars to be anchored precisely.

2. **Z-Index & Stacking Context (`z-index`):**
   - *Current limitation:* Nodes are rendered in strict preorder DOM traversal. An element cannot be elevated above its siblings or ancestors without altering the tree.
   - *Required change:* Add `z_index: i32` to `ComputedStyle`. Sort layout nodes into stacking layers before rendering and hit-testing so focused windows render on top.

3. **Window Dragging & Dynamic Resizing:**
   - *Current limitation:* Mouse movement only tracks hover states; there is no dragging or resize interaction model.
   - *Required change:* Implement pointer capture and drag-state tracking. When the mouse clicks on a window titlebar or border handle, pointer delta moves mutate window `(left, top, width, height)` coordinates.

4. **Focus Management & Global Accelerators:**
   - *Current limitation:* Only `<input>` elements handle focus, and there is no Tab cycling or window switching.
   - *Required change:* Implement `FocusManager` supporting:
     - `Alt+Tab` / `Alt+Shift+Tab`: Cycle active top-level windows.
     - `Super` / `Windows Key`: Open/close application launcher menu.
     - `Tab` / `Shift+Tab`: Traverse focusable widgets within the active window.

---

### 3.2 Display Backends & Hardware I/O

Currently, OxiTerm requires either an external terminal emulator connected via SSH or a modern web browser running WASM over WebSockets. A standalone desktop environment must output directly to local hardware:

1. **Linux Virtual Console (Direct TTY / Raw Termios):**
   - Put the local console (`/dev/tty1` or running terminal) into raw mode using `termios`.
   - Enable SGR 1006 mouse tracking and Kitty Keyboard protocol directly over standard I/O.
   - Zero network or encryption overhead; instantaneous local response.

2. **Direct Linux Framebuffer (`/dev/fb0`):**
   - Memory-map (`mmap`) `/dev/fb0`.
   - Implement a built-in bitmap font blitter (e.g., 8x16 VGA console font, Terminus, or Unifont) converting `CellBuffer` cells into framebuffer pixels.
   - Allows running completely without X11 or Wayland on hardware from the 1990s and 2000s.

3. **DRM/KMS Dumb Buffers (`/dev/dri/card0`):**
   - Initialize direct kernel mode setting via `libdrm` dumb buffers.
   - Provides hardware page-flipping (`drmModePageFlip`) with zero tear without needing an X server or Wayland compositor.

4. **Direct Linux Input Drivers (`evdev` & GPM):**
   - When running on `/dev/fb0` or DRM/KMS without an underlying terminal emulator, read keyboard and mouse events directly from `/dev/input/event*` or GPM mouse repeater (`/dev/gpmdata`).

---

### 3.3 Server & Runtime Architecture (`oxiterm-server`)

1. **Local IPC via Unix Domain Sockets:**
   - *Current limitation:* IPC relies solely on HTTP POST over TCP (`OXITERM_APP_SERVER`).
   - *Required change:* Replace TCP network sockets with a local Unix Domain Socket (e.g., `/run/user/$UID/oxide.sock`) for microsecond-latency local IPC.

2. **Event-Driven Sleep vs. Busy Polling:**
   - *Current limitation:* The session event loop sleeps for 5ms when idle, generating 200 wakeups per second. On battery-powered vintage laptops (e.g., ThinkPad T40/X60), this wastes battery and prevents deep CPU C-states.
   - *Required change:* Replace the 5ms timeout with pure event-driven blocking (`select!` on input channels and timers), waking up only when hardware input arrives or an active animation ticks.

3. **Process Supervision & Application Launcher:**
   - Add an XDG `.desktop` entry scanner to build the application menu from `/usr/share/applications/`.
   - Provide a process spawner (`fork`/`exec`) running external console programs (`htop`, `mc`, `nano`) inside embedded virtual terminal (PTY) windows.
   - Supervise child processes via `waitpid` / `SIGCHLD` and enforce resource bounds using `setrlimit` or cgroups v2.

4. **D-Bus System Integration:**
   - **Notifications:** Implement `org.freedesktop.Notifications` to render system toast notifications on the desktop.
   - **System Tray:** Implement `org.freedesktop.StatusNotifierWatcher` (SNI) to allow background applications (NetworkManager, volume control, media players) to place icons in the taskbar tray.
   - **Power Management:** Connect to `org.freedesktop.login1` for shutdown, restart, sleep, and screen lock.

---

### 3.4 Media & Graphics Optimization for Low-Power CPUs

1. **Retire Heavy Vector Pipelines on Low-End Targets:**
   - Current dependencies like `resvg`, `tiny-skia`, and `rlottie` are too demanding for 400 MHz – 1 GHz single-core CPUs with 128 MB RAM.
   - Replace or complement them with lightweight bitmap caching (BMP, XPM, PNG via `lodepng`), 8-bit palette modes, and procedural geometric icons.

2. **Dirty-Rectangle Screen Blitting:**
   - Instead of repainting the entire `CellBuffer` on every frame, propagate dirty bounding boxes so only modified window regions are re-rasterized.

---

## 4. Reference Desktop Markup (`desktop.thtml`)

Below is a working reference implementation demonstrating a Cinnamon/Windows-style desktop layout utilizing OxiTerm's THTML and TCSS:

```html
<style>
  /* Taskbar */
  .panel       { height: 3; bg: #0f172a; border-style: single; border-color: #334155; align-items: center; padding-left: 1; padding-right: 1; }
  .btn-start   { border-style: single; border-color: #38bdf8; height: 3; padding-left: 1; padding-right: 1; align-items: center; justify-content: center; }
  .btn-window  { border-style: single; border-color: #64748b; height: 3; margin-left: 1; padding-left: 1; padding-right: 1; }
  
  /* Windows */
  .window      { border-style: rounded; border-color: #38bdf8; bg: #020617; flex-direction: column; }
  .win-header  { height: 1; bg: #1e293b; justify-content: space-between; padding-left: 1; padding-right: 1; }
  .win-title   { fg: #f8fafc; }
  .win-close   { fg: #ef4444; }
  
  /* Desktop icons */
  .icon-box    { width: 14; height: 4; border-style: single; border-color: #1e293b; align-items: center; justify-content: center; margin-bottom: 1; }
  .icon-label  { fg: #e2e8f0; height: 1; }
</style>

<!-- Root Screen: 80 columns x 25 rows (Standard VGA Console) -->
<box style="width: 80; height: 25; bg: #020617; flex-direction: column;">

  <!-- WORKSPACE / DESKTOP AREA (fills remaining height) -->
  <box style="flex: 1; flex-direction: row; padding: 1;">

    <!-- Left: Desktop Shortcut Icons -->
    <box style="width: 16; flex-direction: column;">
      <box class="icon-box" event-htmx="launch:file_manager">
        <text class="icon-label">📁 Files</text>
      </box>
      <box class="icon-box" event-htmx="launch:terminal">
        <text class="icon-label">💻 Terminal</text>
      </box>
      <box class="icon-box" event-htmx="launch:sysmon">
        <text class="icon-label">📊 Monitor</text>
      </box>
      <box class="icon-box" event-htmx="launch:settings">
        <text class="icon-label">⚙ Settings</text>
      </box>
    </box>

    <!-- Right: Active Floating/Tiled Application Window -->
    <box class="window" style="flex: 1; height: 19; margin-left: 1;">
      <!-- Titlebar -->
      <box class="win-header">
        <text class="win-title">System Monitor</text>
        <box style="flex-direction: row;">
          <text style="fg: #fbbf24; margin-right: 1;" event-htmx="win:min">_</text>
          <text class="win-close" event-htmx="win:close">✕</text>
        </box>
      </box>

      <!-- Window Content Area -->
      <box style="flex: 1; padding: 1; flex-direction: column;">
        <box style="flex-direction: row; margin-bottom: 1;">
          <text style="fg: #38bdf8; width: 12;">CPU Usage:</text>
          <text bind-state="cpu_bar" style="fg: #4ade80;">[████████░░░░░░░░] 50%</text>
        </box>
        <box style="flex-direction: row; margin-bottom: 1;">
          <text style="fg: #38bdf8; width: 12;">RAM Free:</text>
          <text bind-state="ram_bar" style="fg: #4ade80;">142 MB / 256 MB</text>
        </box>
        <box style="flex-direction: row;">
          <text style="fg: #38bdf8; width: 12;">Active Tasks:</text>
          <text bind-state="active_tasks" style="fg: #f8fafc;">34 processes</text>
        </box>
      </box>
    </box>

  </box>

  <!-- BOTTOM TASKBAR / PANEL -->
  <box class="panel" style="justify-content: space-between;">
    <box style="flex-direction: row; align-items: center;">
      <!-- Application Menu Button -->
      <box class="btn-start" event-htmx="toggle:menu">
        <text style="fg: #38bdf8; height: 1;">[ ❖ Start ]</text>
      </box>

      <!-- Taskbar Window Buttons -->
      <box class="btn-window">
        <text style="fg: #f8fafc; height: 1;">System Monitor</text>
      </box>
      <box class="btn-window">
        <text style="fg: #94a3b8; height: 1;">Terminal</text>
      </box>
    </box>

    <!-- System Tray & Clock -->
    <box style="flex-direction: row; align-items: center;">
      <text style="fg: #38bdf8; margin-right: 1;">🔊 80%</text>
      <text style="fg: #4ade80; margin-right: 2;">🖧 eth0</text>
      <text bind-state="sys_time" style="fg: #e2e8f0;">14:30</text>
    </box>
  </box>

</box>
```

---

## 5. Development Roadmap

| Phase | Target Deliverables | Expected Milestones |
|---|---|---|
| **Phase 1: Local Standalone Runner** | Add `oxiterm run <file.thtml>` to `oxiterm-cli`. | Direct TTY raw mode execution on `/dev/tty1` with zero network overhead. |
| **Phase 2: Layout & Compositing Enhancements** | Implement `position: absolute / fixed` and `z-index` in `oxiterm-renderer`. | Overlapping floating windows, modal dialogs, and draggable window titlebars. |
| **Phase 3: System Daemon & Local IPC** | Implement `oxide-daemon` with Unix Domain Sockets and `/proc` telemetry. | Live system monitoring, ALSA volume control, and network status in the taskbar. |
| **Phase 4: Hardware Display Drivers** | Implement direct `/dev/fb0` (Linux Framebuffer) and DRM/KMS drivers. | Running directly on bare metal without X11 or Wayland installed. |
| **Phase 5: Process Supervision & D-Bus** | Integrate `zbus` for Notifications and StatusNotifierItem; add PTY window runner. | Full Cinnamon-like desktop shell capable of launching terminal and GUI apps. |
