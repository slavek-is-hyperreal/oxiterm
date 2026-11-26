//! Tests of the media `src` resolver (Stage 7.6): one rule, strict containment.
use super::*;

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("oxi_proto_media_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("app/demos")).unwrap();
    std::fs::create_dir_all(d.join("app/assets")).unwrap();
    std::fs::create_dir_all(d.join("outside")).unwrap();
    d
}

#[test]
fn page_relative_and_contained() {
    let d = tmp("ok");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    let ok = resolve_media(Some(&page), Some(&app), "../assets/a.png").unwrap();
    assert_eq!(ok, d.join("app/assets/a.png"));
    // Same directory, `./`, and a file that does not exist (placeholder, not an attack).
    assert!(resolve_media(Some(&page), Some(&app), "x.png").is_some());
    assert!(resolve_media(Some(&page), Some(&app), "./missing/dir/x.png").is_some());
    // The old root-relative spelling is simply a (missing) file below the page.
    assert_eq!(resolve_media(Some(&page), Some(&app), "examples/assets/flow.mmd"), Some(page.join("examples/assets/flow.mmd")));
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn traversal_absolute_encoded_and_symlink_are_refused() {
    let d = tmp("bad");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    std::fs::write(d.join("outside/s.txt"), "s").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(d.join("outside/s.txt"), d.join("app/assets/link.txt")).unwrap();
        std::os::unix::fs::symlink(d.join("outside"), d.join("app/assets/dir")).unwrap();
        assert!(resolve_media(Some(&page), Some(&app), "../assets/link.txt").is_none(), "symlinked file");
        assert!(resolve_media(Some(&page), Some(&app), "../assets/dir/s.txt").is_none(), "symlinked directory");
        assert!(resolve_media(Some(&page), Some(&app), "../assets/dir/new.txt").is_none(), "missing file below a symlinked directory");
    }
    for bad in [
        "../../outside/s.txt",
        "../../../etc/passwd",
        "/etc/passwd",
        "..%2f..%2foutside/s.txt",
        "%2E%2e/%2e%2E/outside/s.txt",
        "..%5coutside",
        "..\\..\\outside\\s.txt",
        "file:///etc/passwd",
        "http://x/y.png",
        "C:/x.png",
        "a\0b.png",
        "",
        ".",
        "..",
        "../",
        "dir/",
    ] {
        assert!(resolve_media(Some(&page), Some(&app), bad).is_none(), "{bad:?}");
    }
    assert!(resolve_media(Some(&page), None, "x.png").is_none(), "no app base fails closed");
    // An app base that does not exist cannot contain anything.
    assert!(resolve_media(Some(&page), Some(&d.join("nope")), "x.png").is_none());
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn virtual_root_is_lexical_only() {
    let e = Path::new("");
    assert_eq!(resolve_media(Some(Path::new("demos")), Some(e), "../assets/a.png"), Some(PathBuf::from("assets/a.png")));
    assert_eq!(resolve_media(Some(Path::new("")), Some(e), "a.png"), Some(PathBuf::from("a.png")));
    assert!(resolve_media(Some(Path::new("demos")), Some(e), "../../a.png").is_none());
    assert!(resolve_media(Some(Path::new("")), Some(e), "../a.png").is_none());
    assert!(resolve_media(Some(Path::new("/abs")), Some(e), "a.png").is_none());
}

#[test]
fn rel_path_matches_the_local_client_rule() {
    assert_eq!(media_rel_path("demos/media.thtml", "../assets/a.png").as_deref(), Some("assets/a.png"));
    assert_eq!(media_rel_path("index.thtml", "../a.png"), None);
    assert_eq!(media_rel_path("demos/x.thtml", "%2e%2e/a.png"), None);
}

#[test]
fn src_length_is_capped() {
    assert!(media_src_is_plain(&format!("{}x.png", "a/".repeat(100))));
    assert!(!media_src_is_plain(&("a".repeat(MAX_SRC_BYTES) + ".png")));
    assert!(!media_src_is_plain(&"a/".repeat(MAX_SRC_BYTES)));
}

#[test]
fn media_key_is_64_bit_and_path_based() {
    // These two collided as 32-bit djb2.
    assert_ne!(media_key("Ez.png"), media_key("FY.png"));
    assert_ne!(media_key("a/logo.png"), media_key("b/logo.png"));
    assert_eq!(media_key("a/logo.png"), media_key("a/logo.png"));
    assert!(media_key("a/logo.png") > u64::from(u32::MAX) || media_key("b/logo.png") > u64::from(u32::MAX));
}

#[test]
fn app_rel_is_below_the_base() {
    let d = tmp("rel");
    let (app, page) = (d.join("app"), d.join("app/demos"));
    let r = resolve_media(Some(&page), Some(&app), "../assets/a.png").unwrap();
    assert_eq!(app_rel(&app, &r).as_deref(), Some("assets/a.png"));
    assert_eq!(app_rel(Path::new(""), Path::new("assets/a.png")).as_deref(), Some("assets/a.png"));
    assert!(app_rel(&app, &d.join("outside/x.png")).is_none());
    let _ = std::fs::remove_dir_all(&d);
}

#[cfg(unix)]
#[test]
fn checked_open_refuses_fifo_oversize_hard_link_and_non_files() {
    let d = tmp("open");
    let app = d.join("app");
    let ok = app.join("assets/ok.png");
    std::fs::write(&ok, "12345").unwrap();
    assert_eq!(read_media_checked(&app, &ok, 16).unwrap(), b"12345");
    // too big
    assert!(read_media_checked(&app, &ok, 4).is_err());
    // FIFO: refused without blocking
    let fifo = app.join("assets/f.png");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let t = std::time::Instant::now();
    assert!(open_media_checked(&app, &fifo, 16).is_err());
    assert!(t.elapsed().as_secs() < 2);
    // directory
    assert!(open_media_checked(&app, &app.join("assets"), 16).is_err());
    // hard link
    std::fs::write(d.join("outside/s.png"), "S").unwrap();
    std::fs::hard_link(d.join("outside/s.png"), app.join("assets/h.png")).unwrap();
    assert!(open_media_checked(&app, &app.join("assets/h.png"), 16).is_err());
    // outside the base by path
    assert!(open_media_checked(&app, &d.join("outside/s.png"), 16).is_err());
    // symlink as the last component
    std::os::unix::fs::symlink(&ok, app.join("assets/l.png")).unwrap();
    assert!(open_media_checked(&app, &app.join("assets/l.png"), 16).is_err());
    let _ = std::fs::remove_dir_all(&d);
}
