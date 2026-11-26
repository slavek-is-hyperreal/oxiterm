# 🖥️ OxiScreens: Many Dumb Screens, One Home Server

This document is the blueprint for using OxiTerm's **server-driven** model to run many cheap networked displays (wall panels, kitchen screens, e-ink tags, old tablets, ESP32 boards) from **one central home server**. The screens hold no layout and no logic. They paint what the server sends and send input back.

> **Status:** Blueprint. Nothing here is implemented yet beyond what OxiTerm already ships. Stage U.Screens in [roadmap/oxiui.md](roadmap/oxiui.md) schedules it (it waits for hardware). Note: the 320x240 sizing example predates the board that was bought (ESP32-S3 4.3" 800x480 touch), and the ESP32 plan predates the core step API and the display-list wire.
>
> **Relationship to other docs:** [embedded-iot-platform.md](embedded-iot-platform.md) covers the opposite model, where the UI runs *on* the device (framebuffer, UART). This document reuses the server-driven mode of [the OxiUI plan](history/oxi-ui-plan-history.md) §3.2 (see also [roadmap/oxiui.md](roadmap/oxiui.md)).

---

## 1. Why This Fits OxiTerm

| Property | What it gives a home installation |
|---|---|
| **Layout lives on the server** | Change a page once and every screen updates immediately; no reflashing |
| **Shared state** | A thermostat change on the kitchen screen appears on the living-room screen at the same moment (`StateManager` + state patches) |
| **Tiny wire format** | Cell diffs are a few KB/s; empty diffs are never sent (`FrameSink::send_frame` returns `Ok(false)`) |
| **Same page everywhere** | The wall screen, a phone browser and `ssh` from a laptop show the same app |
| **Language-agnostic logic** | The App Server protocol (`/events`, `/sessions/{id}/patch`) can be implemented by any service, e.g. a Home Assistant bridge |
| **Partial refresh friendly** | Diffs touch only changed cells, which suits e-ink partial refresh |

### Comparable solutions

| Solution | Model | Difference |
|---|---|---|
| Home Assistant dashboards on tablets | Full browser per screen | Heavy; cheap tablets struggle |
| ESPHome `display` | UI compiled into firmware | Layout changes need reflashing |
| openHASP | ESP32 + LVGL, controlled over MQTT | Closest competitor, but layout lives on the device; no web/SSH, no shared source with other screens |

OxiScreens' position: **zero layout on the device**, and the same pages also work in a browser and over SSH. The intended integration path is to become **the screen layer for Home Assistant**, not a competitor to it.

---

## 2. Client Tiers

| Client | Renders | Availability |
|---|---|---|
| Old tablet / Raspberry Pi with a browser in kiosk mode | Existing web client (`oxiterm-web`, canvas) | **Works today** |
| ESP32-S3 + SPI LCD (ST7789 / ILI9341) | Binary cell-diff stream + bitmap font | New thin client (§5) |
| E-ink panel (ESP32 or SBC) | Cell diffs with partial refresh | New thin client (§5) |
| Raspberry Pi / SBC native window | DisplayList (wire v2/v3, merged; native window is stage 5.3) | After stage 5.3 ([roadmap/oxiui.md](roadmap/oxiui.md)) |

Indicative sizing (estimates, to be measured): a 320×240 LCD with an 8×16 font is a 40×15 cell grid, enough for a room panel.

---

## 3. Server Requirements

What exists today and what is missing:

| Need | Today | Missing |
|---|---|---|
| Per-screen session | `SessionRegistry` with reconnect via token (`reattach`) | Stable **device identity**: a device token that maps to a configured page, size and group |
| Page per device | Mobile variant routing (`_mobile.thtml`, `0x11` viewport message, see ARCHITECTURE.md §7) | **Device → page routing table** (config file), e.g. `kitchen-1 → panels/kitchen.thtml` |
| Shared updates | `broadcast_input_event` (all sessions); `POST /sessions/{id}/patch` (one session) | **Group patches**: `POST /groups/{name}/patch` that fans out to every session in a group |
| Touch input | Mouse clicks hit-tested by cell | Map touch down/up/drag from thin clients to the existing mouse events |
| Offline behaviour | Reconnect with session reattach | Client keeps the last frame and shows a "disconnected" indicator; server resends a full frame on reattach |
| Security | Bearer token for the patch API (`OXITERM_APP_TOKEN`) | Per-device tokens, revocation, LAN-only default binding |

### Home Assistant bridge (App Server)

A separate service that:
1. subscribes to the Home Assistant WebSocket API (`state_changed` events) and turns entity states into OxiTerm state patches (group or per-device);
2. receives OxiTerm `/events` actions from screens (e.g. `toggle:light.kitchen`) and calls the matching Home Assistant services.

This keeps OxiTerm generic and makes the integration an ordinary App Server, as described in [app-server-guide.md](app-server-guide.md).

---

## 4. 3D on Dumb Screens

Because screens are dumb, 3D is computed on the server and the screen only displays the result. The server picks a tier per screen based on its declared capabilities, the same way `TerminalProfile` negotiates terminal features today.

| Tier | Sent over the network | Computed by | Suited for |
|---|---|---|---|
| **1. Pixels** (JPEG; H.264 for browsers) | Encoded frames for the 3D region only | Server: everything | Any scene: textures, cameras, realistic lighting |
| **2. 2D vectors** | Projected, sorted, clipped polygons with flat colours, pixel coordinates | Server projects and sorts; screen only fills | Stylized dashboards: floor plans, energy flow, schematics |
| **3. Mesh + camera** | Mesh once; then a camera matrix and colour changes per frame (~100 B) | Screen projects and sorts | Smooth rotation of small scenes with minimal traffic |

### 4.1 The `<scene>` element

A new media node, a sibling of `<video>`: the server renders it for each screen at that screen's resolution and sends output only for the node's rectangle. The rest of the UI keeps using cell diffs or the DisplayList. Touch gestures on the node come back as events that move the camera. The same pipe also carries **home camera feeds**, since they are "an image in a rectangle" too.

### 4.2 Tier 2 is just another DisplayList builder

Projected 3D becomes ordinary 2D DisplayList items (`Polygon`/`Path`, which the GUI needs anyway for rounded borders and vector icons). Consequences:
- **Crisp text in 3D:** room labels are projected as `Text` items, not blurred pixels.
- **Resolution independence:** the same stream fits 240p and 720p.
- **State changes are nearly free:** a light turning on is one polygon's colour change.

Server-side reduction for tier 2:
1. Back-face culling and back-to-front sorting (painter's algorithm, BSP splits for intersecting faces), so the screen needs no depth buffer.
2. Clipping of occluded fragments, so no pixel is painted twice.
3. Simplification after projection: merge adjacent same-colour faces, drop sub-pixel polygons, quantize coordinates to `i16` pixels (~4 B per vertex).
4. A per-screen polygon budget (LOD): a simplified model for ESP32-S3, the full one for browsers.
5. Frame deltas: with a still camera, send only changed colours and items.

Rough size (estimate): 300–500 visible polygons ≈ a few KB per frame before deflate. The gain over JPEG is modest while the camera moves and very large for a still scene where only state changes.

### 4.3 Device capability (estimates, to be measured)

| | ESP32-S3 | ESP32-P4 |
|---|---|---|
| Tier 1 | Software JPEG, ~320×240 at 10–20 fps in the 3D region | Hardware JPEG codec and 2D accelerator; 480p–720p |
| Tier 2 | Scanline polygon fill; ~1–2k flat triangles per frame at 320×240 looks plausible | Comfortable |
| Tier 3 | Vertex transform on the FPU plus face sort, for small scenes | Comfortable |
| Bottleneck | CPU decode, SPI bandwidth to the LCD | Usually Wi-Fi (separate radio chip) |

Bandwidth (estimate): a 320×240 JPEG frame is ~15–25 KB, so 15 fps ≈ 2–3 Mbit/s per screen; raw RGB565 (~150 KB per frame) is not viable over Wi-Fi. Interaction latency (network, render, encode, decode) is in the order of 60–100 ms: fine for home panels, not for action games.

### 4.4 Server load (estimates)

- Rendering a small home scene at 320×240 is trivial for any GPU; an integrated GPU can serve over a dozen concurrent views.
- JPEG encoding at 320×240 takes ~1–2 ms per frame on a CPU core, so a dozen screens at 15 fps fit one core. Browsers can use H.264 via a hardware encoder (NVENC, VA-API).
- The biggest saving: **render only on change** (camera move, sensor update). A static scene costs nothing.

Limits: no textures or realistic lighting in tiers 2–3 (flat colours, later simple gradients); dense smooth meshes inflate polygon counts (server LOD or fall back to tier 1); e-ink gets static renders only; local/offline mode on ESP32 has no 3D, since these chips have no 3D GPU.

---

## 5. ESP32 Thin Client

- Rust on `esp-hal` (no_std) or `esp-idf` (std), WebSocket client.
- Decodes the existing binary cell-diff stream (`DiffEngine::encode_binary` / `decode_binary`), which stays the default and fallback stream next to the display list (merged, OxiUI Phase 3).
- Bitmap font atlas in flash (8×16 to start; Unicode subset with a replacement glyph for anything missing, e.g. emoji).
- Touch controller (e.g. FT6236 / GT911) → cell coordinates → mouse events.
- Declares its capabilities on connect: grid size, colour depth, touch, supported 3D tiers and polygon budget, JPEG support.
- Target: the same firmware on every screen; which page it shows is decided by the server's routing table.

---

## 6. Roadmap

1. **MVP, almost no code:** 2–3 old tablets or Raspberry Pis in browser kiosk mode pointed at the home server, each opening its own page. Validate the idea on a real wall.
2. **Server:** device identity and routing table, group patches, per-device tokens, full-frame resend on reattach.
3. **Home Assistant bridge** App Server.
4. **ESP32-S3 thin client** for cell diffs with touch.
5. **`<scene>` tier 1** (server render + JPEG), including camera feeds.
6. **Tier 2 vectors** (`Path` items exist in the DisplayList since OxiUI Phase 3b; tier 2 is stage 9.1); then tier 3 for capable devices.
7. E-ink client with partial refresh.

---

## 7. Risks

| Risk | Mitigation |
|---|---|
| Wi-Fi drops make screens look broken | Keep last frame + disconnected indicator; automatic reattach with full-frame resend |
| Server becomes a single point of failure | Screens are useless without it by design; document it, and recommend running it on the always-on home server |
| ESP32 performance assumptions are wrong | Measure early (step 4) before committing to tiers 2–3 |
| Security on a home LAN | Per-device tokens, LAN-only binding by default, no device can reach another device's session |
| Competing with Home Assistant | Position as its screen layer via the bridge, not a replacement |
