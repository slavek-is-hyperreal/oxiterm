//! BAD ACTOR 7.6 against the web transport's media path (`WsFrameSink::sync_preamble`): what bytes
//! (0x20) and positions (0x21) a browser gets. `#[ignore]`d tests assert the desired behaviour and
//! fail today (`--ignored`); the others pin what held.
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use oxiterm_renderer::{CellBuffer, FrameSink};
use oxiterm_server::session::{MediaRenderInfo, SessionRegistry};
use oxiterm_server::web::web_impl::{hash_path, WsFrameSink};

struct Fx {
    root: PathBuf,
}
impl Fx {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ba76_srv_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        for d in ["app/a", "app/b", "app/assets", "outside"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        Self { root }
    }
}
impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

type Rx = tokio::sync::mpsc::Receiver<Vec<u8>>;

fn sink(fx: &Fx, page: &str, media: &[&str]) -> (WsFrameSink, Arc<oxiterm_server::session::ClientSession>, Rx) {
    let reg = SessionRegistry::new(Arc::new(prometheus::Registry::new()), 20);
    let session = reg.create_session().unwrap();
    set_media(&session, media);
    *session.app_base_dir.write() = Some(fx.root.join("app"));
    *session.current_page.write() = Some(page.to_string());
    let (tx, rx) = tokio::sync::mpsc::channel(64);
    *session.web_frame_tx.write() = Some((1, tx));
    session.connection_epoch.store(1, std::sync::atomic::Ordering::SeqCst);
    std::mem::forget(reg);
    (WsFrameSink::new(session.clone(), Some(fx.root.join("app").join(page))), session, rx)
}

fn set_media(session: &oxiterm_server::session::ClientSession, media: &[&str]) {
    *session.active_media.write() =
        media.iter().map(|p| MediaRenderInfo { path: (*p).to_string(), x: 1, y: 1, width: 10, height: 5 }).collect();
}

fn frame(s: &mut WsFrameSink) {
    let _ = s.send_frame(&CellBuffer::new(80, 24), &CellBuffer::new(80, 24));
}

/// (hash, bytes) of every 0x20 and the hashes of every 0x21 with a body.
fn drain(rx: &mut Rx) -> (Vec<(u64, Vec<u8>)>, Vec<u64>) {
    let (mut blobs, mut pos) = (Vec::new(), Vec::new());
    while let Ok(m) = rx.try_recv() {
        match m[0] {
            0x20 => blobs.push((u64::from_le_bytes(m[1..9].try_into().unwrap()), m[9..].to_vec())),
            0x21 if m.len() > 1 => pos.push(u64::from_le_bytes(m[1..9].try_into().unwrap())),
            _ => {}
        }
    }
    (blobs, pos)
}

#[tokio::test]
async fn traversal_forms_send_nothing() {
    let fx = Fx::new("trav");
    std::fs::write(fx.root.join("outside/s.png"), b"SECRET").unwrap();
    let forms = [
        "../../outside/s.png", "..%2f..%2foutside/s.png", "%2e%2e/%2e%2e/outside/s.png", "..\\..\\outside\\s.png", "/etc/passwd",
        "../../outside/s.png?x", "a/../../../outside/s.png",
    ];
    let (mut s, _sess, mut rx) = sink(&fx, "a/p.thtml", &forms);
    frame(&mut s);
    let (blobs, pos) = drain(&mut rx);
    assert!(blobs.is_empty() && pos.is_empty(), "{blobs:?} {pos:?}");
    // Literal look-alikes (fullwidth dots, double-encoded) are plain missing file names inside the base.
    let (mut s, sess, mut rx) = sink(&fx, "a/p.thtml", &[]);
    set_media(&sess, &["\u{ff0e}\u{ff0e}/\u{ff0e}\u{ff0e}/outside/s.png", "%252e%252e/%252e%252e/outside/s.png"]);
    frame(&mut s);
    let (blobs, _) = drain(&mut rx);
    assert!(blobs.is_empty(), "{blobs:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_out_of_the_base_sends_nothing() {
    let fx = Fx::new("sym");
    std::fs::write(fx.root.join("outside/s.png"), b"SECRET").unwrap();
    std::os::unix::fs::symlink(fx.root.join("outside/s.png"), fx.root.join("app/assets/l.png")).unwrap();
    std::os::unix::fs::symlink(fx.root.join("outside"), fx.root.join("app/assets/d")).unwrap();
    let (mut s, _sess, mut rx) = sink(&fx, "a/p.thtml", &["../assets/l.png", "../assets/d/s.png"]);
    frame(&mut s);
    let (blobs, pos) = drain(&mut rx);
    assert!(blobs.is_empty() && pos.is_empty());
}

/// F-02: the asset route / page index / `FsPageSource` refuse multiply linked files (7.1); the
/// `0x20` path reads whatever `resolve_media` allows.
#[cfg(unix)]
#[tokio::test]
async fn hard_linked_file_is_not_sent() {
    let fx = Fx::new("hard");
    std::fs::write(fx.root.join("outside/s.png"), b"SECRET").unwrap();
    std::fs::hard_link(fx.root.join("outside/s.png"), fx.root.join("app/assets/h.png")).unwrap();
    let (mut s, _sess, mut rx) = sink(&fx, "a/p.thtml", &["../assets/h.png"]);
    frame(&mut s);
    let (blobs, _) = drain(&mut rx);
    assert!(blobs.iter().all(|(_, b)| b != b"SECRET"), "leaked through a hard link");
}

/// F-07: no size cap: a file of any size is read whole into memory (per session) and queued.
#[tokio::test]
async fn oversize_media_is_not_read_whole() {
    let fx = Fx::new("big");
    let f = std::fs::File::create(fx.root.join("app/assets/big.png")).unwrap();
    f.set_len(300 * 1024 * 1024).unwrap(); // sparse 300 MiB
    let (mut s, _sess, mut rx) = sink(&fx, "a/p.thtml", &["../assets/big.png"]);
    let t = std::time::Instant::now();
    frame(&mut s);
    let (blobs, _) = drain(&mut rx);
    eprintln!("300 MiB file: sent {} blob(s), {:?}", blobs.len(), t.elapsed());
    assert!(blobs.iter().all(|(_, b)| b.len() < 32 * 1024 * 1024), "sent {} bytes", blobs.iter().map(|b| b.1.len()).max().unwrap_or(0));
}

/// F-08: a FIFO named like an image blocks the sink (blocking `std::fs::read` on the render path).
#[cfg(unix)]
#[test]
fn fifo_does_not_block_the_sink() {
    let fx = Fx::new("fifo");
    let fifo = fx.root.join("app/assets/f.png");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    let (tx, rx) = std::sync::mpsc::channel();
    let (s_path, root) = (fifo.clone(), fx.root.clone());
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        rt.block_on(async {
            let fx2 = Fx { root: root.clone() };
            let (mut s, _sess, _rx) = sink(&fx2, "a/p.thtml", &["../assets/f.png"]);
            std::mem::forget(fx2);
            frame(&mut s);
        });
        let _ = tx.send(());
    });
    let done = rx.recv_timeout(Duration::from_secs(3)).is_ok();
    // Unblock a stuck reader so its thread can end. Test correction (fixes.md F-08): only when the
    // sink did block; a blocking write-open of a FIFO nobody reads would itself hang forever.
    if !done {
        let _ = std::fs::OpenOptions::new().write(true).open(&s_path);
    }
    assert!(done, "sink blocked on a FIFO for >3 s");
}

/// F-01 (functional): bookkeeping keyed by the raw `src`, not by the resolved file. After
/// navigating to a page in another directory whose `src` string is the same, the new file is
/// never sent, the browser keeps showing the old one.
#[tokio::test]
async fn same_src_on_another_page_gets_its_own_bytes() {
    let fx = Fx::new("stale");
    std::fs::write(fx.root.join("app/a/logo.png"), b"AAA").unwrap();
    std::fs::write(fx.root.join("app/b/logo.png"), b"BBB").unwrap();
    let (mut s, sess, mut rx) = sink(&fx, "a/p.thtml", &["logo.png"]);
    frame(&mut s);
    let (b1, _) = drain(&mut rx);
    assert_eq!(b1.iter().map(|b| b.1.clone()).collect::<Vec<_>>(), vec![b"AAA".to_vec()]);
    // navigation (no new WebSocket, so no epoch bump)
    *sess.current_page.write() = Some("b/q.thtml".to_string());
    set_media(&sess, &["logo.png"]);
    frame(&mut s);
    let (b2, _) = drain(&mut rx);
    assert_eq!(b2.iter().map(|b| b.1.clone()).collect::<Vec<_>>(), vec![b"BBB".to_vec()], "b/logo.png never arrived");
}

/// F-09: the client key is a 32-bit djb2 of the `src`; "Ez" and "FY" collide (so do countless
/// others), the second blob overwrites the first in the browser's `mediaCache`.
#[tokio::test]
async fn distinct_srcs_have_distinct_keys() {
    assert_ne!(hash_path("Ez.png"), hash_path("FY.png"));
}

/// F-04: any file type, hidden files included, is shipped when a page names it (the local asset
/// route refuses dotfiles and non-media types; `src` is author-controlled, so author-level only).
#[tokio::test]
async fn non_media_files_are_not_sent() {
    let fx = Fx::new("any");
    std::fs::write(fx.root.join("app/.env"), b"KEY=V").unwrap();
    let (mut s, _sess, mut rx) = sink(&fx, "a/p.thtml", &["../.env"]);
    frame(&mut s);
    let (blobs, _) = drain(&mut rx);
    assert!(blobs.is_empty(), "sent {blobs:?}");
}

// ---- local mode: `.json` / `.mmd` served only when an indexed page names that exact file ------

use oxiterm_server::local_mode as lm;

fn local_fx(name: &str) -> (Fx, PathBuf) {
    let fx = Fx::new(name);
    let app = fx.root.join("app");
    std::fs::create_dir_all(app.join("sub/deep")).unwrap();
    std::fs::create_dir_all(app.join(".hidden")).unwrap();
    std::fs::write(app.join("bell.json"), br#"{"v":"5"}"#).unwrap();
    std::fs::write(app.join("sub/flow.mmd"), b"graph TD\nA-->B\n").unwrap();
    std::fs::write(app.join("config.json"), br#"{"secret":1}"#).unwrap();
    std::fs::write(app.join("sub/other.mmd"), b"graph TD\nSECRET-->B\n").unwrap();
    std::fs::write(app.join("package.json"), br#"{"name":"x"}"#).unwrap();
    std::fs::write(app.join(".hidden.json"), b"{}").unwrap();
    std::fs::write(fx.root.join("outside/x.json"), b"{\"leak\":1}").unwrap();
    std::fs::write(fx.root.join("outside.json"), b"{\"leak\":2}").unwrap();
    std::fs::write(
        app.join("index.thtml"),
        r#"<screen><img src="bell.json" style="width:10;height:5"/><diagram src="sub/flow.mmd" alt="d" style="width:10;height:5"/>
<img src="../outside.json" style="width:10;height:5"/><img src="../../x.json" style="width:10;height:5"/><img src="%2e%2e/outside.json" style="width:10;height:5"/><img src=".hidden.json" style="width:10;height:5"/></screen>"#,
    )
    .unwrap();
    std::fs::write(app.join("sub/p.thtml"), r#"<screen><img src="../../outside/x.json" style="width:10;height:5"/><img src="../../../outside/x.json" style="width:10;height:5"/><diagram src="..\..\outside.json" alt="d" style="width:10;height:5"/></screen>"#).unwrap();
    // a page nobody can navigate to: in a hidden directory
    std::fs::write(app.join(".hidden/h.thtml"), r#"<screen><img src="../config.json" style="width:10;height:5"/></screen>"#).unwrap();
    (fx, app)
}

#[test]
fn only_exactly_named_files_are_servable_and_pages_cannot_name_outside_files() {
    let (_fx, app) = local_fx("local");
    let idx = lm::LocalIndex::build(&app).unwrap();
    let mut named: Vec<_> = idx.media_files.iter().cloned().collect();
    named.sort();
    eprintln!("media_files = {named:?}");
    // `../outside.json` etc. from index.thtml have no relative path inside the base: never indexed.
    assert!(named.iter().all(|n| !n.contains("outside") && !n.contains("..")), "{named:?}");
    let mf = &*idx.media_files;
    for ok in ["bell.json", "sub/flow.mmd"] {
        assert!(lm::resolve_asset_for(&app, ok, Some(mf)).is_some(), "{ok}");
    }
    for bad in [
        "config.json", "package.json", "sub/other.mmd", "Bell.json", "BELL.JSON", "bell.json/", "./bell.json", "%2e/bell.json", "sub/../bell.json",
        "sub%2f..%2fbell.json", "bell.json%00", "bell.json%00.png", "bell.jso%6e%00", "%252e%252e/outside.json", "..%2foutside.json", "../outside.json",
        "..%2f..%2foutside%2fx.json", "sub/deep/../../config.json", ".hidden.json", ".hidden/h.thtml", "bell.json%20", "%62ell.json%0a", "bell.json?x=1",
        "bell.json#x", "\u{ff0e}\u{ff0e}/outside.json", "%c0%ae%c0%ae/outside.json", "bell.json\\", "\\bell.json", "bell%5cjson", "//bell.json", "/bell.json",
    ] {
        match lm::resolve_asset_for(&app, bad, Some(mf)) {
            None => {}
            Some(a) => panic!("{bad:?} served {a:?}"),
        }
    }
    // Without the referenced set nothing but media types is served.
    assert!(lm::resolve_asset_for(&app, "bell.json", None).is_none());
    // A page in a hidden directory is not in the index, so its names do not count.
    assert!(!mf.contains("config.json"));
}

/// F-10: `check_opened_asset` re-checks type/size/nlink of the *opened* file but not where it is, so a
/// symlink planted between `resolve_asset_for` and `open` is followed (needs write access to the app dir).
#[cfg(unix)]
#[test]
fn asset_route_symlink_swap_between_resolve_and_open() {
    let (fx, app) = local_fx("swap");
    let idx = lm::LocalIndex::build(&app).unwrap();
    let a = lm::resolve_asset_for(&app, "bell.json", Some(&*idx.media_files)).unwrap();
    std::fs::remove_file(&a.path).unwrap();
    std::os::unix::fs::symlink(fx.root.join("outside.json"), &a.path).unwrap();
    // Test correction (fixes.md F-10): the original opened the file itself and asked the
    // Metadata-only `check_opened_asset`, which cannot know where the file is. The route now opens
    // through `open_asset` (descriptor checks incl. location below the base).
    assert!(lm::open_asset(&app, &a).is_err(), "a swapped-in symlink to an outside file must be refused");
    // A swapped *directory* is refused too.
    let (sub, moved) = (app.join("sub"), app.join("sub_moved"));
    let b = lm::resolve_asset_for(&app, "sub/flow.mmd", Some(&*idx.media_files)).unwrap();
    assert!(lm::open_asset(&app, &b).is_ok(), "the untouched file still opens");
    std::fs::rename(&sub, &moved).unwrap();
    std::os::unix::fs::symlink(fx.root.join("outside"), &sub).unwrap();
    std::fs::write(fx.root.join("outside/flow.mmd"), b"graph TD\nSECRET-->B\n").unwrap();
    assert!(lm::open_asset(&app, &b).is_err(), "a swapped directory must be refused");
}

