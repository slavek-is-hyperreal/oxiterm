//! Compile-time-ish guard for the modules that are meant to move into `oxiterm-core`
//! (OxiUI Phase 4 done-criteria scanner).
//!
//! Each listed module is embedded with `include_str!`, cut at its trailing `#[cfg(test)] mod` and
//! stripped of `//` comments; it fails the test when it still names a transport, a thread, a
//! real clock or the filesystem. Modules with a server-only part are split so that the
//! core-bound part is scannable (`host_clock.rs`, `frame_limiter.rs`).

#![cfg(test)]

/// Tokens a core-bound module must not contain.
const FORBIDDEN: &[&str] = &[
    "tokio",
    "prometheus",
    "russh",
    "ureq",
    "std::fs",
    "std::thread",
    "thread::spawn",
    "recv_timeout",
    "mpsc",
    "backpressure",
    "RecvTimeoutError",
    "Instant::now",
    "std::time::Instant",
    "SystemTime",
    "ReactorMessage",
    "crate::ssh",
    "crate::session::",
    "crate::web",
    "crate::backpressure",
    "crate::metrics",
    "crate::dispatcher",
    "crate::host_clock",
    "crate::pathsafe",
    "crate::config",
];

/// Core-bound modules (production part only).
const MODULES: &[(&str, &str)] = &[
    ("clock.rs", include_str!("clock.rs")),
    ("action.rs", include_str!("action.rs")),
    ("page_source.rs", include_str!("page_source.rs")),
    ("session_state.rs", include_str!("session_state.rs")),
    ("loop_io.rs", include_str!("loop_io.rs")),
    ("animation.rs", include_str!("animation.rs")),
    ("throttle.rs", include_str!("throttle.rs")),
    ("frame_limiter.rs", include_str!("frame_limiter.rs")),
    ("state.rs", include_str!("state.rs")),
    ("expand.rs", include_str!("expand.rs")),
    ("events.rs", include_str!("events.rs")),
    ("identity.rs", include_str!("identity.rs")),
];

/// Production code of `src` without comments, as `(line_no, line)` pairs.
fn production_lines(src: &str) -> Vec<(usize, String)> {
    // Cut at the trailing test module only; a `#[cfg(test)]` on a single item stays scanned.
    let prod = match src.find("#[cfg(test)]\nmod ") {
        Some(i) => &src[..i],
        None => src,
    };
    prod.lines()
        .enumerate()
        .map(|(n, l)| {
            let code = match l.find("//") {
                Some(i) => &l[..i],
                None => l,
            };
            (n + 1, code.to_string())
        })
        .collect()
}

fn violations(name: &str, src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for (n, line) in production_lines(src) {
        for tok in FORBIDDEN {
            if line.contains(tok) {
                out.push(format!("{name}:{n}: forbidden `{tok}`: {}", line.trim()));
            }
        }
    }
    out
}

#[test]
fn core_bound_modules_have_no_transport_or_clock_dependencies() {
    let mut all = Vec::new();
    for (name, src) in MODULES {
        all.extend(violations(name, src));
    }
    assert!(all.is_empty(), "core-bound modules must stay pure:\n{}", all.join("\n"));
}

#[test]
fn scanner_detects_forbidden_tokens_and_ignores_tests_and_comments() {
    let bad = "use std::sync::mpsc::RecvTimeoutError;\nuse tokio::sync::Notify;\nfn f() { let _ = std::time::Instant::now(); }\n";
    let hits = violations("bad.rs", bad);
    assert!(hits.iter().any(|h| h.contains("`tokio`")), "{hits:?}");
    assert!(hits.iter().any(|h| h.contains("`Instant::now`")), "{hits:?}");
    assert!(hits.iter().any(|h| h.contains("`mpsc`")), "{hits:?}");

    let ok = "// uses tokio and Instant::now in a comment\nfn f() {}\n#[cfg(test)]\nmod t { use std::fs; }\n";
    assert!(violations("ok.rs", ok).is_empty());
}

#[test]
fn scanner_covers_every_listed_module_nonvacuously() {
    for (name, src) in MODULES {
        assert!(production_lines(src).len() > 10, "{name} looks empty after stripping");
    }
}
