# OxiTerm Weather Dashboard Demo

This document describes the Weather Dashboard application built as a showcase for the OxiTerm engine.

## 🌟 Overview

The Weather Dashboard demonstrates how to build a complex, multi-view TUI using server-side rendering. It fetches live data from the Open-Meteo API and renders it using OxiTerm's THTML/TCSS engine.

## 🛠 Features

### 1. Multi-View Navigation
The application features three distinct views that can be toggled using keyboard shortcuts:
- **[1] Current Weather**: Real-time temperature, wind speed, and weather condition.
- **[2] 7-Day Forecast**: Daily high/low temperatures and conditions for the upcoming week.
- **[3] Details**: Precise metrics including rain, pressure, and hourly trends.

### 2. Responsive Layout
The UI is built using a Flexbox-based layout system (Taffy). When you resize your terminal window, the application automatically:
- Re-calculates node dimensions.
- Repositions Header and Footer.
- Adjusts content padding.
- Clears the screen and scrollback to prevent artifacts.

### 3. Predictive Local Echo
To mitigate network latency, OxiTerm uses a predictive buffer. Typed characters appear instantly in the footer input area, providing immediate visual feedback even on slow SSH connections.

## ⌨️ Controls

| Key | Action |
|-----|--------|
| `1` | Switch to Current Weather view |
| `2` | Switch to Forecast view |
| `3` | Switch to Details view |
| `Tab` | Cycle through views |
| `R` | Force refresh data from API |
| `Q` | Exit application and close SSH session |

## 🏗 Implementation Details

### API Integration
Data is fetched using the `ureq` crate from `https://api.open-meteo.com/v1/forecast`. The results are cached in the `WeatherApp` struct to prevent excessive API calls.

### Rendering
The application builds a `THTMLDocument` programmatically:
- **Header**: Fixed height (3 lines), dark blue background.
- **Content**: Dynamic height, flex-column layout.
- **Footer**: Fixed height (3 lines), grey background, contains help text and the predictive echo input field.

### Performance
For the best experience, run the server with the `--release` flag. This enables Taffy's optimized layout calculations and ensures 60 FPS rendering in the terminal.

## 🔒 Security
The demo uses a configurable password for SSH access. Ensure you set the `OXITERM_PASSWORD` environment variable before starting the server.

---

## 🎨 Vector & Animation Showcase Demo

Alongside the programmatic weather app, OxiTerm provides a dedicated layout-driven vector graphics showcase demo.

### 🚀 Running the Showcase
To launch the THTML dashboard that hosts the vector showcase, run:
```bash
cargo run --bin oxiterm-cli -- serve examples/hello.thtml --port 2222
```
Then connect via:
```bash
ssh localhost -p 2222
```

### 💎 Showcase Elements
Once connected, navigate to the **🎨 Vector & Animation Demo** card. It displays:
1. **Rive Interactive Widget (`toggle.riv`)**: A custom interactive slider toggle. Hovering and clicking with mouse input dynamically triggers the slide translation animation.
2. **Lottie Vector Animation (`bell.json`)**: An active vector bell animation. When active, it automatically sets the session ticking frame cycle to 15 FPS to drive redraw frames.
3. **SVG Vector Mascot (`mascot.svg`)**: A high-fidelity rasterized SVG graphic drawing the OxiTerm rust mascot, cached and rendered via `resvg`.
