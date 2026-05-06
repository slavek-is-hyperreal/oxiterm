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

## Next Steps
- **Sprint 2**: Implementing the THTML Parser and AST Arena.
- **Sprint 3**: Implementing TCSS and the Layout Engine.
