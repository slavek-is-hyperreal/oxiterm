//! Boundary guard for `oxiterm-core` (OxiUI Phase 4 done-criteria scanner).
//!
//! Scans every `.rs` file under `src/` (production part only: cut at the trailing
//! `#[cfg(test)]\nmod `, `//` comments stripped) and fails when it names a transport, a thread, a
//! real clock or the filesystem. A second test checks that `Cargo.toml` lists no forbidden crate.

const FORBIDDEN: &[&str] = &[
    "tokio",
    "prometheus",
    "russh",
    "ureq",
    "hyper",
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
    "oxiterm_server",
];

const FORBIDDEN_DEPS: &[&str] =
    &["tokio", "prometheus", "russh", "ureq", "hyper", "notify", "oxiterm-server"];

fn production_lines(src: &str) -> Vec<(usize, String)> {
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

fn src_files() -> Vec<(String, String)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, String)>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(&p, out);
            } else if p.extension().map_or(false, |x| x == "rs") {
                out.push((p.display().to_string(), std::fs::read_to_string(&p).unwrap()));
            }
        }
    }
    let mut out = Vec::new();
    walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut out);
    out
}

#[test]
fn core_sources_have_no_transport_or_clock_dependencies() {
    let files = src_files();
    assert!(files.len() >= 13, "scanner found only {} files", files.len());
    let mut all = Vec::new();
    for (name, src) in &files {
        assert!(production_lines(src).len() > 5, "{name} looks empty after stripping");
        all.extend(violations(name, src));
    }
    assert!(all.is_empty(), "oxiterm-core must stay pure:\n{}", all.join("\n"));
}

#[test]
fn cargo_toml_lists_no_forbidden_dependency() {
    let toml = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
    let mut bad = Vec::new();
    for line in toml.lines() {
        let l = line.trim();
        if l.starts_with('#') {
            continue;
        }
        if let Some((key, _)) = l.split_once('=') {
            let key = key.trim();
            if FORBIDDEN_DEPS.contains(&key) {
                bad.push(key.to_string());
            }
        }
    }
    assert!(bad.is_empty(), "forbidden dependencies in Cargo.toml: {bad:?}");
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

/// The renderer's `native-media` feature (rlottie C bindings, resvg, ffmpeg, threads, clock) must
/// stay off for the core, directly and through `oxiterm-a11y` (feature unification would turn it
/// on again otherwise).
#[test]
fn renderer_is_used_without_native_media() {
    for rel in ["/Cargo.toml", "/../oxiterm-a11y/Cargo.toml"] {
        let toml = std::fs::read_to_string(format!("{}{}", env!("CARGO_MANIFEST_DIR"), rel)).unwrap();
        let line = toml
            .lines()
            .find(|l| l.trim_start().starts_with("oxiterm-renderer"))
            .unwrap_or_else(|| panic!("{rel}: no oxiterm-renderer dependency"));
        assert!(
            line.contains("default-features = false") && !line.contains("native-media"),
            "{rel}: oxiterm-renderer must use default-features = false: {line}"
        );
    }
}
