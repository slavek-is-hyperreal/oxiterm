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
**Status: In Progress**

### Accomplishments:
- **Node Arena**: Implemented ID-based arena with compaction (defragmentation) support.
- **DOM Model**: Defined `Node`, `NodeTag`, and `NodeAttributes` for OxiTerm.
- **THTML Parser**: Basic `nom`-based parser structure with defensive design.
- **Sanitization**: Robust `style_raw` sanitization using regex to block ANSI injection attacks.

## Sprint 3: TCSS & Layout Engine
**Status: In Progress**

### Accomplishments:
- **Double Buffering**: Implemented `CellBuffer` and `DoubleBuffer` for flicker-free rendering.
- **Diff Engine**: Minimal ANSI generation with Synchronized Updates (BSU/ESU) support.
- **Layout Foundation**: Integrated Taffy (0.6) for character-grid based Flexbox layout.

## Next Steps & Roadmap

### Upcoming Milestones:
- **Sprint 4: Interactivity & HTMX Events**
    - Implement Resilient Reactor Thread (RRT) for input handling.
    - Support for Kitty Keyboard Protocol and SGR Mouse.
    - Hit-testing and HTMX event dispatching logic.
- **Sprint 5: Optimization & Backpressure**
    - Capability negotiation (DA1/DA2) with terminal emulators.
    - Latency mitigation and Predictive Echo (Mosh-style).
    - Flow control (XON/XOFF) and bounded frame channels.

### Current Priorities:
- Complete recursive THTML tag parsing in Sprint 2.
- Implement TCSS-to-Taffy property mapping and bypass diff optimizations in Sprint 3.
