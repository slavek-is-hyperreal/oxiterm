//! Exercises the real ureq backend against a throw-away loopback server (no internet).
//! The production resolver refuses loopback, so these tests inject a permissive resolver;
//! a separate test proves the production backend cannot reach the same server.

use oxiterm_reader::config::Config;
use oxiterm_reader::fetch::{Backend, FetchError, UreqBackend};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, ToSocketAddrs};
use url::Url;

struct Loopback;
impl ureq::Resolver for Loopback {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        Ok(netloc.to_socket_addrs()?.collect())
    }
}

/// Serves each connection with `responder(request_head) -> raw bytes`, then closes.
fn serve(responder: impl Fn(&str) -> Vec<u8> + Send + 'static) -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in l.incoming() {
            let Ok(mut s) = s else { break };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let head = String::from_utf8_lossy(&buf[..n]).to_string();
            let _ = s.write_all(&responder(&head));
        }
    });
    port
}

fn url(port: u16, path: &str) -> Url {
    // Port is not 80/443, which is fine here: the backend itself does no URL policy.
    Url::parse(&format!("http://127.0.0.1:{port}{path}")).unwrap()
}

#[test]
fn redirect_and_error_statuses_arrive_without_following() {
    let port = serve(|head| {
        if head.starts_with("GET /r ") {
            b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
        } else if head.starts_with("GET /missing ") {
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno".to_vec()
        } else {
            let ua_ok = head.contains("oxiterm-reader/0.1");
            let no_cookie = !head.to_ascii_lowercase().contains("cookie:") && !head.to_ascii_lowercase().contains("authorization:");
            let body = format!("ua={ua_ok} clean={no_cookie}");
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).into_bytes()
        }
    });
    let b = UreqBackend::with_resolver(&Config::default(), Loopback);
    let r = b.get(&url(port, "/r"), "*/*", 1000).unwrap();
    assert_eq!((r.status, r.location.as_deref()), (302, Some("/elsewhere")));
    let r = b.get(&url(port, "/missing"), "*/*", 1000).unwrap();
    assert_eq!(r.status, 404);
    let r = b.get(&url(port, "/ok"), "*/*", 1000).unwrap();
    assert_eq!(String::from_utf8(r.body).unwrap(), "ua=true clean=true");
}

#[test]
fn size_cap_via_content_length_and_via_stream() {
    let port = serve(|head| {
        if head.starts_with("GET /cl ") {
            let body = vec![b'x'; 5000];
            let mut v = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
            v.extend(body);
            v
        } else {
            // No Content-Length: close-delimited body.
            let mut v = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
            v.extend(vec![b'y'; 5000]);
            v
        }
    });
    let b = UreqBackend::with_resolver(&Config::default(), Loopback);
    assert_eq!(b.get(&url(port, "/cl"), "*/*", 1000).unwrap_err(), FetchError::TooLarge);
    assert_eq!(b.get(&url(port, "/stream"), "*/*", 1000).unwrap_err(), FetchError::TooLarge);
    assert_eq!(b.get(&url(port, "/stream"), "*/*", 5000).unwrap().body.len(), 5000);
}

#[test]
fn production_backend_cannot_reach_loopback() {
    let port = serve(|_| b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nhi".to_vec());
    let b = UreqBackend::new(&Config::default());
    let r = b.get(&url(port, "/"), "*/*", 1000);
    assert!(matches!(r, Err(FetchError::Network(_))), "{r:?}");
}
