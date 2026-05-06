# OxiTerm — SSR Terminal Thin Client

OxiTerm is a high-performance, server-side rendered (SSR) terminal-based application framework. It allows developers to build rich, interactive terminal user interfaces (TUI) using a declarative HTML-like syntax (THTML) and CSS-like styling (TCSS), all rendered on the server and delivered via SSH.

## Key Features

- **Zero-Client Logic**: No JavaScript or complex dependencies on the client side. Only a terminal emulator and SSH client are required.
- **THTML & TCSS**: Familiar declarative structure for terminal layouts.
- **Flexbox Layout**: Powered by Taffy/Yoga for responsive terminal designs.
- **Secure Transport**: Encrypted communication via SSH with strict public-key only authentication.
- **Tearing-Free Rendering**: Protocol-level Synchronized Updates (BSU/ESU) for atomic frame delivery.
- **High Performance**: Rust-based engine with double buffering, compacting Node Arena, and diff-based rendering.
- **Defensive Design**: Panic-free THTML parsing with ANSI-injection sanitization.
- **Accessibility**: Built-in support for screen readers via AT-SPI2 tunneling.

## Project Status

- **Sprint 0 & 1**: ✅ Infrastructure & SSH Daemon (Hardened)
- **Sprint 2 & 3**: 🚧 AST Arena, THTML Parser, Layout Engine & TCSS (In Progress)

## Architecture

OxiTerm follows a "Thin Client" architecture:
1. **Server**: Manages application state, parses THTML/TCSS, calculates layouts, and generates ANSI escape sequences.
2. **Transport**: SSH tunnel delivers optimized diffs to the client.
3. **Client**: Any modern terminal emulator (Alacritty, Ghostty, WezTerm, etc.) acting as a passive renderer.

See [Architecture Documentation](docs/ARCHITECTURE.md) for more details.

## Getting Started

### Prerequisites
- Rust 1.75+
- OpenSSH client

### Running the Server
```bash
cargo run --bin oxiterm-server
```

### Connecting
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
