# OxiTerm Sprint Report

## Sprint 0: Infrastructure
**Status: Completed**

### Accomplishments:
- **Workspace Setup**: Multi-crate Cargo workspace established.
- **Configuration**: Implemented robust configuration system with TOML and Environment variable support.
- **Metrics**: Integrated Prometheus metrics for session tracking (bytes, frames, drops).
- **Rate Limiting**: IP-based rate limiting to protect against connection floods.
- **CI/CD**: GitHub Actions pipeline for build, test, linting, and security audits.
- **Graceful Shutdown**: Signal handling for clean server restarts.

## Sprint 1: Transport & SSH Daemon
**Status: Completed**

### Accomplishments:
- **SSH Server**: Implemented asynchronous daemon using `russh` and `tokio`.
- **Authentication**: Strict public-key authentication enforced (passwords disabled).
- **PTY Management**: Capture and tracking of terminal dimensions and resize events.
- **Session Registry**: Thread-safe management of multiple concurrent client sessions.
- **Security**: Shell and exec requests blocked to ensure "SSR-only" environment.

## Sprint 2: AST Arena & THTML Parser
**Status: Completed**

### Accomplishments:
- **Node Arena**: Implemented ID-based arena with compaction (defragmentation) support.
- **DOM Model**: Defined `Node`, `NodeTag`, and `NodeAttributes` for OxiTerm.
- **THTML Parser**: Basic `nom`-based parser structure with defensive design.
- **Sanitization**: Robust `style_raw` sanitization using regex to block ANSI injection attacks.

## Sprint 3: TCSS & Layout Engine
**Status: Completed**

### Accomplishments:
- **Double Buffering**: Implemented `CellBuffer` and `DoubleBuffer` for flicker-free rendering.
- **Diff Engine**: Minimal ANSI generation with Synchronized Updates (BSU/ESU) support.
- **Layout Foundation**: Integrated Taffy (0.6) for character-grid based Flexbox layout.

## Sprint 4: Interactivity & HTMX Events
**Status: Completed**

### Accomplishments:
- **Resilient Reactor Thread (RRT)**: Dedicated OS thread for non-blocking input decoding.
- **Input Protocols**: Full support for Kitty Keyboard Protocol and SGR 1006 Mouse Protocol.
- **Hit-Testing**: Implemented `HitTester` for precise coordinate-to-node mapping.
- **Event Bus**: HTMX-style callback system for Click, Input, Focus, and Blur events.
- **Latency Mitigation**: `PredictiveEcho` and `ResizeDebouncer` for fluid UI interaction.

## Sprint 5: Resilience & Performance Optimization
**Status: Completed**

### Accomplishments:
- **Unicode Stabilization**: `UnicodeWidthCache` and `insert_vtm_modifier` for consistent cross-terminal layout.
- **Backpressure**: `BoundedFrameChannel` with drop-on-overflow strategy.
- **SGR Timeout**: Guard against incomplete escape sequences in `InputDecoder`.
- **CI Stabilization**: Resolved all strict clippy lints and satisfied CI requirements.

## Sprint 6: Vector Graphics & Interactive Animations
**Status: Completed**

### Accomplishments:
- **Scalable Vector Graphics (SVG)**: Integrated `resvg` and `tiny-skia` to render high-fidelity vector images onto cell grids.
- **Dynamic Lottie Frame Ticking**: Implemented procedural loaders (.json) with automatically scaled 15 FPS event loop ticking during active playback.
- **Rive State Interaction**: Integrated custom procedural slider switches (.riv) reacting to client-side hover and click coordinates.
- **Dual-Tier Asset Caches**: Built thread-safe `SvgCache` (OnceLock parsed trees) and size/frame-keyed `AssetCache` mapping to Sixel and Kitty protocols.
- **Static Quantizer Optimization**: Overhauled Sixel compression using a pre-allocated 256-color palette to minimize runtime CPU/memory overhead.
- **Interactive Showcase Integration**: Created a multi-card layout demo linking SVG, Lottie, and Rive components into the default dashboard.
