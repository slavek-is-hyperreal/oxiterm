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
