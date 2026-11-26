//! BAD ACTOR 7.6: the media `src` resolver. Tests without `#[ignore]` pin what held; `#[ignore]`d
//! tests assert the *desired* behaviour and fail today (run with `--ignored` to reproduce).
use oxiterm_proto::pathsafe::{media_rel_path, media_src_is_plain, resolve_media};
use std::path::{Path, PathBuf};

fn fx(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ba76_proto_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    for s in ["app/demos", "app/assets", "outside"] {
        std::fs::create_dir_all(d.join(s)).unwrap();
    }
    d
}

#[test]
fn every_traversal_spelling_is_refused_or_stays_inside() {
    let d = fx("spell");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    // Refused outright.
    for bad in [
        "../../outside/x", "..//..//outside/x", "./../../outside/x", "a/../../../outside/x",
        "%2e%2e/%2e%2e/outside/x", "%2E%2E%2f%2E%2E%2foutside", "..%2foutside", "..%5coutside", "%2e%2e%5c",
        "..\\outside", "a\\b", "/etc/passwd", "//etc/passwd", "file:x", "data:image/png;base64,AAAA", "javascript:alert(1)",
        "a\0.png", "x/\0", ".", "..", "x/..", "x/.", "dir/", "../../",
    ] {
        assert!(resolve_media(Some(&page), Some(&app), bad).is_none(), "{bad:?}");
        assert!(media_rel_path("demos/p.thtml", bad).is_none() || !bad.contains(".."), "{bad:?}");
    }
    // Held: these are *literal* file names (nothing percent-decodes a src on the server), so they
    // stay inside the base: double-encoding, fullwidth dots, overlong UTF-8 as text, query/fragment.
    for lit in [
        "%252e%252e/%252e%252e/x.png", "\u{ff0e}\u{ff0e}/\u{ff0e}\u{ff0e}/x.png", "\u{2025}/x.png", "..%c0%af../x.png",
        "x.png?../../../outside/s", "x.png#../../outside", "a b.png", "..a/x.png", "...//x.png", "x.png ",
    ] {
        match resolve_media(Some(&page), Some(&app), lit) {
            None => {}
            Some(p) => assert!(p.starts_with(&app), "{lit:?} -> {p:?}"),
        }
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// F-05: `canonical_prefix` walks up one component at a time calling `canonicalize` on each
/// shorter prefix (O(n^2) syscalls) when the file does not exist. A long `src` of missing
/// directories costs seconds of CPU per resolve (and it is resolved every frame/refresh).
#[test]
#[ignore = "BAD ACTOR 7.6 F-05: a long src of missing dirs makes resolve_media quadratic (seconds)"]
fn very_long_src_is_cheap() {
    let d = fx("long");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    let long = "a/".repeat(20_000) + "x.png"; // 40 KB src
    let t = std::time::Instant::now();
    let _ = resolve_media(Some(&page), Some(&app), &long);
    let el = t.elapsed();
    eprintln!("40KB src, 20k missing dirs: {el:?}");
    assert!(el.as_millis() < 200, "resolver took {el:?}");
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn long_and_deep_up_src_are_refused_cheaply() {
    let d = fx("up");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    let t = std::time::Instant::now();
    let up = "../".repeat(200_000) + "x.png";
    assert!(resolve_media(Some(&page), Some(&app), &up).is_none());
    assert!(media_rel_path("demos/p.thtml", &up).is_none());
    let _ = media_rel_path("demos/p.thtml", &("a/".repeat(200_000) + "x.png"));
    let _ = media_src_is_plain(&"a/".repeat(200_000));
    assert!(t.elapsed().as_secs() < 2, "{:?}", t.elapsed());
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn symlink_forms_are_refused_including_chains_and_relative_links() {
    use std::os::unix::fs::symlink;
    let d = fx("sym");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    std::fs::write(d.join("outside/s.png"), "s").unwrap();
    symlink("../../outside/s.png", d.join("app/assets/rel.png")).unwrap(); // relative link
    symlink(d.join("app/assets/rel.png"), d.join("app/assets/chain.png")).unwrap(); // chain
    symlink("..", d.join("app/assets/up")).unwrap(); // link to the parent (still inside: app)
    symlink("../..", d.join("app/assets/upup")).unwrap(); // out of the base
    symlink("/etc", d.join("app/assets/etc")).unwrap();
    for s in ["../assets/rel.png", "../assets/chain.png", "../assets/upup/outside/s.png", "../assets/etc/passwd", "../assets/etc/nonexistent.png"] {
        assert!(resolve_media(Some(&page), Some(&app), s).is_none(), "{s}");
    }
    // A link that stays inside the base is allowed (held, not a regression: 7.1 allowed it too).
    assert!(resolve_media(Some(&page), Some(&app), "../assets/up/demos/x.png").is_some());
    // The base itself may be a symlink.
    symlink(&app, d.join("applink")).unwrap();
    assert!(resolve_media(Some(&d.join("applink/demos")), Some(&d.join("applink")), "../assets/a.png").is_some());
    let _ = std::fs::remove_dir_all(&d);
}

/// F-02: `resolve_media` never looks at link counts, and the web transport (`sync_preamble`) reads
/// the result with a plain `std::fs::read`. 7.1 made the asset route, the page index and
/// `FsPageSource` refuse multiply linked files (`meta_multi_linked`); the 0x20 path does not.
#[cfg(unix)]
#[test]
#[ignore = "BAD ACTOR 7.6 F-02: a hard link inside the base to a file outside it is accepted by resolve_media"]
fn hard_link_out_of_the_base_is_refused() {
    let d = fx("hard");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    std::fs::write(d.join("outside/secret.png"), "TOPSECRET").unwrap();
    std::fs::hard_link(d.join("outside/secret.png"), d.join("app/assets/h.png")).unwrap();
    assert!(resolve_media(Some(&page), Some(&app), "../assets/h.png").is_none());
    let _ = std::fs::remove_dir_all(&d);
}

/// F-03: check-then-open race. `resolve_media` returns a path; the caller opens it later and
/// follows whatever is there by then (a symlink planted in between). Deterministic version of the race.
#[cfg(unix)]
#[test]
#[ignore = "BAD ACTOR 7.6 F-03: resolve_media's verdict is not bound to the opened file (TOCTOU), the 0x20 path uses fs::read(path)"]
fn resolve_then_read_is_not_raceable() {
    let d = fx("race");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    std::fs::write(d.join("outside/secret.png"), "TOPSECRET").unwrap();
    std::fs::write(d.join("app/assets/a.png"), "fine").unwrap();
    let p = resolve_media(Some(&page), Some(&app), "../assets/a.png").unwrap();
    // attacker with write access to the app dir swaps the file between check and use
    std::fs::remove_file(&p).unwrap();
    std::os::unix::fs::symlink(d.join("outside/secret.png"), &p).unwrap();
    let got = std::fs::read(&p).unwrap(); // what sync_preamble does
    assert_ne!(got, b"TOPSECRET", "read through a swapped symlink leaked a file outside the base");
    let _ = std::fs::remove_dir_all(&d);
}

/// F-04: no file-type filter: any file inside the base (`oxi.toml`, `.env`, `users.db`) can be
/// named by `src` and is shipped to the client as a 0x20 blob. Only the page author controls `src`.
#[test]
fn non_media_and_hidden_files_are_resolvable() {
    let d = fx("any");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    std::fs::write(d.join("app/.env"), "K=V").unwrap();
    // documents the current (permissive) behaviour; the local asset route refuses dotfiles.
    assert!(resolve_media(Some(&page), Some(&app), "../.env").is_some());
    assert!(resolve_media(Some(&page), Some(&app), "../oxi.toml").is_some());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn case_tricks_do_not_matter_on_a_case_sensitive_fs() {
    let d = fx("case");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    assert!(resolve_media(Some(&page), Some(&app), "..%2F../x").is_none());
    assert!(resolve_media(Some(&page), Some(&app), "%2E%2F").is_none());
    let _ = Path::new("");
    let _ = std::fs::remove_dir_all(&d);
}
