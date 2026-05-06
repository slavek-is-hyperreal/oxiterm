# OxiTerm — SSR Terminal Thin Client

OxiTerm is a high-performance, server-side rendered (SSR) terminal-based application framework. It allows developers to build rich, interactive terminal user interfaces (TUI) using a declarative HTML-like syntax (THTML) and CSS-like styling (TCSS), all rendered on the server and delivered via SSH.

## 🌤 Weather Dashboard Showcase (New!)

We have successfully implemented a full-scale Weather Dashboard as a demonstration of OxiTerm's capabilities.
- **Real-time Data**: Integrated with Open-Meteo API.
- **Responsive Layout**: Adapts to terminal window resizing.
- **Multi-view UI**: Current weather, 7-day forecast, and detailed metrics.
- **Low Latency**: Optimized via Predictive Local Echo and Diff-based rendering.

## Key Features

- **Zero-Client Logic**: Only a terminal emulator and SSH client are required.
- **THTML & TCSS**: Familiar declarative structure for terminal layouts.
- **Flexbox Layout**: Powered by Taffy for high-performance terminal designs.
- **Resilient Reactor Thread (RRT)**: Dedicated thread for I/O to prevent blocking the event loop.
- **Predictive Local Echo**: Mitigation for high-latency connections.
- **Synchronized Updates**: Prevents screen tearing via BSU/ESU (Synchronized Updates protocol).
- **Secure Auth**: Configurable password-based SSH authentication.
- **Deep Screen Clearing**: Prevents scrollback artifacts using `\x1b[3J`.

## Project Status

- **Sprint 1**: ✅ SSH Transport Layer & Security (Completed)
- **Sprint 2 & 3**: ✅ AST Arena, THTML Parser, Layout Engine & TCSS (Completed)
- **Sprint 4 & 5**: ✅ Interactivity, Weather Demo & Production Polish (Completed)

## Architecture

OxiTerm follows a "Thin Client" architecture:
1. **Server**: Manages application state, parses THTML/TCSS, calculates layouts, and generates ANSI escape sequences.
2. **Transport**: SSH tunnel delivers optimized diffs to the client.
3. **Client**: Any modern terminal emulator acting as a passive renderer.

## Getting Started

### Prerequisites
- Rust 1.75+
- OpenSSH client

### Running the Weather Demo
1. Set the password for the session:
   ```bash
   export OXITERM_PASSWORD=krakow
   ```
2. Start the server in release mode:
   ```bash
   cargo run --release -p oxiterm-server
   ```
3. Connect from another terminal:
   ```bash
   ssh -p 2222 localhost
   ```

## Project Structure

- `oxiterm-server`: The main SSH daemon and SSR engine.
- `oxiterm-proto`: Shared types and protocol definitions.
- `oxiterm-renderer`: Layout calculation and ANSI generation.
- `oxiterm-a11y`: Accessibility and screen reader integration.

## License
MIT
