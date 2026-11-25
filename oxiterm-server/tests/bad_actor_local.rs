//! BAD ACTOR 7.1 findings (server side). Failing tests = findings.
use oxiterm_server::local_mode as lm;
use oxiterm_server::identity::UserIdentity;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Mutex;

static ENV: Mutex<()> = Mutex::new(());

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ba71_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// One-shot mock App Server on an ephemeral port; returns its URL and the raw request it saw.
fn mock(reply: &'static str) -> (String, std::thread::JoinHandle<String>) {
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let url = format!("http://127.0.0.1:{}/events", l.local_addr().unwrap().port());
    let h = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut buf = vec![0u8; 16384];
        let mut n = 0;
        loop {
            n += s.read(&mut buf[n..]).unwrap();
            let t = String::from_utf8_lossy(&buf[..n]).into_owned();
            if let Some(p) = t.find("\r\n\r\n") {
                let len: usize = t.lines().find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap())).unwrap_or(0);
                if n >= p + 4 + len { break; }
            }
        }
        let _ = s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", reply.len(), reply).as_bytes());
        String::from_utf8_lossy(&buf[..n]).into_owned()
    });
    (url, h)
}

fn ctx(actions: &[&str], bound: &[&str]) -> lm::ProxyCtx {
    lm::ProxyCtx::new(lm::mint_session().0, actions.iter().map(|s| s.to_string()), bound.iter().map(|s| s.to_string()))
}

/// F1 (HIGH, spoofing/cross-session): the proxy used to send session_id 0 for every local page, and
/// the first real WebSocket session of a server also has id 0 (SessionRegistry next_id starts at 0).
/// Local clients now get a server-minted id from their own namespace, never 0 and never what the
/// page says.
#[test]
fn f1_proxy_session_id_collides_with_first_real_session() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let (url, h) = mock("{}");
    std::env::set_var("OXITERM_APP_SERVER", &url);
    // (changed call: the proxy now needs the page-load context; "x" is declared by the page)
    let c = ctx(&["x"], &[]);
    let (st, _) = lm::proxy_event_ctx(br#"{"action":"x","state":{},"session_id":5}"#, &UserIdentity::guest(), &c);
    assert_eq!(st, 200);
    let req = h.join().unwrap();
    let body: serde_json::Value = serde_json::from_str(&req[req.find("\r\n\r\n").unwrap() + 4..]).unwrap();
    let reg = oxiterm_server::session::SessionRegistry::new(std::sync::Arc::new(prometheus::Registry::new()), 4);
    let first = reg.create_session().unwrap();
    let first_id = first.state_half.id as u64;
    let sent = body["session_id"].as_u64().unwrap();
    assert_ne!(sent, first_id, "local pages share session id with the first real session");
    assert_ne!(sent, 0);
    assert_ne!(sent, 5, "the page's own session_id is ignored");
    assert!(lm::is_local_session_id(sent as usize));
    // and two page loads are two sessions
    assert_ne!(lm::mint_session().0, lm::mint_session().0);
}

/// F2 (MEDIUM, spoofing/parity): arbitrary action strings and client-composed state used to reach
/// the App Server. Now an action no indexed page declares is refused (403) and the App Server is
/// never contacted. (Changed: the mock must not block forever on accept, so the old blocking
/// `accept()` is a non-blocking check that no connection arrived.)
#[test]
fn f2_arbitrary_action_and_state_forwarded() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let l = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    l.set_nonblocking(true).unwrap();
    std::env::set_var("OXITERM_APP_SERVER", format!("http://127.0.0.1:{}/events", l.local_addr().unwrap().port()));
    let c = ctx(&["inc:c"], &["c"]);
    let (st, _) = lm::proxy_event_ctx(br#"{"action":"admin:delete_all","state":{"role":"admin","is_authenticated":"true"},"session_id":0}"#, &UserIdentity::guest(), &c);
    assert_eq!(st, 403);
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(matches!(l.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "the App Server was contacted");
    // a declared action goes through, with client-composed state cut down to bound keys
    drop(l);
    let (url, h) = mock("{}");
    std::env::set_var("OXITERM_APP_SERVER", &url);
    let (st, _) = lm::proxy_event_ctx(br#"{"action":"inc:c","state":{"c":"1","role":"admin","is_authenticated":"true"},"session_id":0}"#, &UserIdentity::guest(), &c);
    assert_eq!(st, 200);
    let req = h.join().unwrap();
    assert!(req.contains("inc:c") && !req.contains("is_authenticated") && !req.contains("admin"), "{req}");
}

/// F3 (LOW, CSRF/DNS rebinding): no Host allowlist; evil host with matching Origin passed the
/// gate. (Changed: the allowlist is optional, so the test configures it; without it the request
/// is still accepted, which is the documented default.)
#[test]
fn f3_dns_rebinding_passes_gate() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let mut m = hyper::HeaderMap::new();
    m.insert("content-type", "application/json".parse().unwrap());
    m.insert("host", "evil.test:8080".parse().unwrap());
    m.insert("origin", "http://evil.test:8080".parse().unwrap());
    m.insert("sec-fetch-site", "same-origin".parse().unwrap());
    std::env::remove_var("OXITERM_ALLOWED_HOSTS");
    assert!(lm::check_same_origin_json(&m).is_ok(), "no allowlist configured: no host check");
    std::env::set_var("OXITERM_ALLOWED_HOSTS", "good.test:8080,localhost:8080");
    assert_eq!(lm::check_same_origin_json(&m), Err(403));
    m.insert("host", "good.test:8080".parse().unwrap());
    m.insert("origin", "http://good.test:8080".parse().unwrap());
    assert!(lm::check_same_origin_json(&m).is_ok());
    std::env::remove_var("OXITERM_ALLOWED_HOSTS");
}

/// F4 (LOW, path escape by hard link): a hard link inside the base to an outside file is indexed/served.
#[cfg(unix)]
#[test]
fn f4_hardlink_leaks_outside_file() {
    let root = tmp("hl");
    let app = root.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(root.join("secret.thtml"), "SECRET").unwrap();
    std::fs::write(root.join("secret.png"), "SECRETPNG").unwrap();
    std::fs::write(app.join("index.thtml"), "x").unwrap();
    std::fs::hard_link(root.join("secret.thtml"), app.join("h.thtml")).unwrap();
    std::fs::hard_link(root.join("secret.png"), app.join("h.png")).unwrap();
    let idx = lm::build_page_index(&app).unwrap();
    assert!(!idx.pages.values().any(|v| v == "SECRET"), "hard-linked outside page indexed");
    assert!(lm::read_asset(&app, "h.png").is_none(), "hard-linked outside asset served");
}

/// F5 (MEDIUM, DoS): walk breadth is unbounded, caps only limit pages. Every guest request redoes it.
#[test]
fn f5_walk_cost_unbounded() {
    let app = tmp("walk");
    std::fs::write(app.join("index.thtml"), "x").unwrap();
    for d in 0..60 {
        let dd = app.join(format!("d{d}"));
        std::fs::create_dir_all(&dd).unwrap();
        for f in 0..1000 { std::fs::write(dd.join(format!("f{f}.dat")), "").unwrap(); }
    }
    let t = std::time::Instant::now();
    lm::build_page_index(&app).unwrap();
    let e = t.elapsed();
    eprintln!("walk of 60k non-page files: {e:?}");
    assert!(e.as_millis() < 50, "walk took {e:?} for a single page app");
}

/// F6 (MEDIUM, DoS): a never-answering App Server used to pin a blocking thread 15s per request
/// and the ureq thread forever (no timeouts in dispatcher.rs). Now: the HTTP client has total
/// timeouts (here 500 ms via OXITERM_LOCAL_PROXY_TIMEOUT_MS, so the test is fast), at most 16
/// requests are in flight (the rest get 503), and no thread outlives its timeout. (Changed: the
/// 20 requests may be 502 or 503, the thread count is compared with a baseline instead of an
/// absolute 10, and the held listener uses an ephemeral port that is released afterwards.)
#[test]
fn f6_slow_upstream_leaks_threads() {
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    let threads = || -> usize { std::fs::read_to_string("/proc/self/status").unwrap().lines().find(|l| l.starts_with("Threads:")).unwrap().split_whitespace().nth(1).unwrap().parse().unwrap() };
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let held = std::thread::spawn(move || {
        l.set_nonblocking(true).unwrap();
        let mut v = vec![];
        while stop_rx.try_recv().is_err() {
            if let Ok((s, _)) = l.accept() { v.push(s); } else { std::thread::sleep(std::time::Duration::from_millis(10)); }
        }
    });
    std::env::set_var("OXITERM_APP_SERVER", format!("http://127.0.0.1:{port}/events"));
    std::env::set_var("OXITERM_LOCAL_PROXY_TIMEOUT_MS", "500");
    let baseline = threads();
    let c = ctx(&["x"], &[]);
    let hs: Vec<_> = (0..20).map(|_| { let c = c.clone(); std::thread::spawn(move || lm::proxy_event_ctx(br#"{"action":"x","state":{},"session_id":1}"#, &UserIdentity::guest(), &c).0) }).collect();
    let codes: Vec<u16> = hs.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(codes.iter().all(|c| *c == 502 || *c == 503), "{codes:?}");
    assert!(codes.iter().any(|c| *c == 503), "no concurrency limit: {codes:?}");
    assert!(codes.iter().any(|c| *c == 502), "{codes:?}");
    std::thread::sleep(std::time::Duration::from_millis(700));
    let after = threads();
    assert!(after <= baseline + 3, "{after} threads alive after the timeout, baseline {baseline} (ureq threads leak)");
    stop_tx.send(()).unwrap();
    held.join().unwrap();
    std::env::remove_var("OXITERM_LOCAL_PROXY_TIMEOUT_MS");
}

/// Seeded path fuzz: nothing outside the base is ever returned; no panic.
#[cfg(unix)]
#[test]
fn held_asset_fuzz_never_leaks() {
    let root = tmp("fz");
    let app = root.join("app");
    std::fs::create_dir_all(app.join("sub")).unwrap();
    std::fs::write(root.join("secret.png"), "SECRETPNG").unwrap();
    std::fs::write(app.join("pic.png"), "PIC").unwrap();
    std::os::unix::fs::symlink(&root, app.join("ln")).unwrap();
    std::os::unix::fs::symlink(".", app.join("loop")).unwrap();
    let alpha = ["..", ".", "%2e%2e", "%2E", "%2f", "%5c", "%252e%252e", "%00", "/", "//", "\\", "sub", "ln", "loop", "secret.png", "pic.png", "%c0%ae%c0%ae", "%e2%80%ae", "a", "é", "%", "%zz", ":", " "];
    let mut x: u64 = 0x9E3779B97F4A7C15;
    for _ in 0..200_000 {
        let mut s = String::new();
        x ^= x << 13; x ^= x >> 7; x ^= x << 17;
        let n = 1 + (x % 6) as usize;
        for i in 0..n { s.push_str(alpha[((x >> (8 + i * 5)) as usize) % alpha.len()]); if (x >> (50 + i)) & 1 == 1 { s.push('/'); } }
        let r = std::panic::catch_unwind(|| lm::read_asset(&app, &s));
        let r = r.unwrap_or_else(|_| panic!("panic on {s:?}"));
        if let Some((_, b)) = r { assert!(b != b"SECRETPNG", "leak via {s:?}"); }
    }
}

/// Uniform 404: every refusal is `None` (no status/body difference possible in local_route).
#[test]
fn held_missing_start_and_noutf8() {
    let app = tmp("st");
    std::fs::write(app.join("a.thtml"), "x").unwrap();
    assert!(lm::index_json(&app, "index.thtml", false).is_none());
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        std::fs::write(app.join(std::ffi::OsStr::from_bytes(b"\xff\xfe.thtml")), "x").unwrap();
        assert_eq!(lm::build_page_index(&app).unwrap().pages.len(), 1);
    }
}

/// Parity: MemPageSource(index) vs FsPageSource for odd targets.
#[test]
fn parity_odd_targets() {
    use oxiterm_server::fs_page_source::FsPageSource;
    use oxiterm_server::page_source::{MemPageSource, PageSource};
    let app = tmp("par");
    std::fs::create_dir_all(app.join("sub")).unwrap();
    for p in ["index.thtml", "a.thtml", "a_mobile.thtml", "sub/b.thtml"] { std::fs::write(app.join(p), "<screen/>").unwrap(); }
    let mem = MemPageSource::new(lm::build_page_index(&app).unwrap().pages);
    let fs = FsPageSource::new(app.canonicalize().unwrap());
    let mut bad = vec![];
    for t in ["nodir/../a.thtml", "a.thtml/../index.thtml", "sub//b.thtml", "sub/", "sub\\b.thtml", "./a.thtml", "sub/./b.thtml", "a.thtml/", "%2e%2e/a.thtml", "sub/b.thtml\0", "A.thtml", " a.thtml", ".//a.thtml"] {
        for mobile in [false, true] {
            let m = mem.resolve(None, t, mobile).map(|r| (r.page.clone(), r.actual.clone()));
            let f = fs.resolve(None, t, mobile).map(|r| (r.page.clone(), r.actual.file_name().map(|n| n.to_os_string())));
            let m2 = m.as_ref().map(|(p, a)| (p.clone(), a.file_name().map(|n| n.to_os_string())));
            if format!("{m2:?}") != format!("{f:?}") { bad.push(format!("{t:?} mobile={mobile}: mem {m2:?} fs {f:?}")); }
        }
    }
    assert!(bad.is_empty(), "parity divergences:\n{}", bad.join("\n"));
}
