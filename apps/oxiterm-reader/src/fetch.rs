//! The safe fetcher: the only code in the reader that touches the network.
//!
//! Defence in depth, in the order a request meets it:
//!
//! 1. **Static URL vetting** (`netguard::validate_url`): http/https only, no userinfo,
//!    ports 80/443, no private IP literals (any spelling: `url` normalises decimal/hex),
//!    no `localhost`-style names, optional host allow-list. Re-run on *every* redirect hop.
//! 2. **Connection-time DNS vetting** (`VettedResolver`): ureq asks our resolver for the
//!    addresses of the host and connects to exactly what we return, so the address that was
//!    vetted is the address that is connected to. There is no second lookup between check
//!    and connect (no DNS-rebinding window). Non-public addresses are filtered out; if none
//!    remain the connection fails.
//! 3. **Manual redirects** (`redirects(0)` in the agent): at most N hops, each re-validated.
//! 4. **Limits**: connect + total timeouts, a hard body cap enforced on the *decoded*
//!    stream (gzip bombs) plus a Content-Length pre-check, a per-action fetch budget, a
//!    global concurrency cap and a per-session rate limiter.
//! 5. **No ambient authority**: no cookie jar, no credentials, no proxies from the
//!    environment, fixed honest User-Agent, `Accept` set by us.

use crate::config::Config;
use crate::netguard::{self, UrlRefusal};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use url::Url;

/// Honest, fixed User-Agent (Wikimedia requires a contact hint).
pub const USER_AGENT: &str = "oxiterm-reader/0.1 (+https://github.com/slavekm/oxiterm; reader mode demo)";

/// What the caller expects, used to pick the body cap and `Accept` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// HTML / JSON / XML document.
    Document,
    /// Raster image.
    Image,
}

/// Fetch failures. `Display` is short and safe to show to the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// URL refused before any traffic.
    Refused(UrlRefusal),
    /// DNS resolved only to non-public addresses (or not at all).
    Unreachable(String),
    /// More redirect hops than allowed.
    TooManyRedirects,
    /// A redirect without a usable Location.
    BadRedirect,
    /// Body larger than the cap.
    TooLarge,
    /// Non-2xx final status.
    Status(u16),
    /// Transport failure / timeout.
    Network(String),
    /// Per-action fetch budget used up.
    BudgetExceeded,
    /// Global concurrency cap reached.
    Busy,
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FetchError::Refused(r) => write!(f, "address refused ({r:?})"),
            FetchError::Unreachable(m) => write!(f, "host unreachable: {m}"),
            FetchError::TooManyRedirects => write!(f, "too many redirects"),
            FetchError::BadRedirect => write!(f, "bad redirect"),
            FetchError::TooLarge => write!(f, "response too large"),
            FetchError::Status(c) => write!(f, "server answered HTTP {c}"),
            FetchError::Network(m) => write!(f, "network error: {m}"),
            FetchError::BudgetExceeded => write!(f, "too many requests for one action"),
            FetchError::Busy => write!(f, "reader is busy, try again"),
        }
    }
}

impl std::error::Error for FetchError {}

/// A raw single-hop response from a [`Backend`].
#[derive(Debug, Clone)]
pub struct RawResponse {
    /// HTTP status.
    pub status: u16,
    /// `Location` header, if any.
    pub location: Option<String>,
    /// `Content-Type` header, if any.
    pub content_type: Option<String>,
    /// Body, at most the cap handed to the backend.
    pub body: Vec<u8>,
}

/// One HTTP GET without redirect following. Mockable in tests.
pub trait Backend: Send + Sync {
    /// Performs the GET. Must not follow redirects and must fail with
    /// [`FetchError::TooLarge`] when the (decoded) body exceeds `max_bytes`.
    fn get(&self, url: &Url, accept: &str, max_bytes: usize) -> Result<RawResponse, FetchError>;
}

/// Final result of a (possibly redirected) fetch.
#[derive(Debug, Clone)]
pub struct Fetched {
    /// URL after redirects (validated).
    pub final_url: Url,
    /// Content-Type header.
    pub content_type: Option<String>,
    /// Body bytes.
    pub body: Vec<u8>,
}

/// Resolver handed to ureq: returns only public addresses.
pub struct VettedResolver;

impl VettedResolver {
    /// Filters `addrs` to the public ones.
    pub fn vet(addrs: Vec<SocketAddr>) -> Vec<SocketAddr> {
        addrs.into_iter().filter(|a| netguard::is_public_ip(a.ip())).collect()
    }
}

impl ureq::Resolver for VettedResolver {
    fn resolve(&self, netloc: &str) -> std::io::Result<Vec<SocketAddr>> {
        let all: Vec<SocketAddr> = netloc.to_socket_addrs()?.collect();
        let ok = Self::vet(all);
        if ok.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "host resolves to no public address"));
        }
        Ok(ok)
    }
}

/// The real backend: ureq 2 with the vetted resolver.
pub struct UreqBackend {
    agent: ureq::Agent,
}

impl UreqBackend {
    /// Builds the production backend (vetted resolver, no redirects, no cookies, no pooling).
    pub fn new(cfg: &Config) -> Self {
        Self::with_resolver(cfg, VettedResolver)
    }

    /// Builds a backend with a custom resolver (tests use this to reach a loopback server).
    pub fn with_resolver(cfg: &Config, resolver: impl ureq::Resolver + 'static) -> Self {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .user_agent(USER_AGENT)
            .timeout_connect(Duration::from_secs(cfg.connect_timeout_secs))
            .timeout(Duration::from_secs(cfg.total_timeout_secs))
            .max_idle_connections(0)
            .try_proxy_from_env(false)
            .resolver(resolver)
            .build();
        Self { agent }
    }
}

impl Backend for UreqBackend {
    fn get(&self, url: &Url, accept: &str, max_bytes: usize) -> Result<RawResponse, FetchError> {
        let resp = match self.agent.get(url.as_str()).set("Accept", accept).call() {
            Ok(r) => r,
            // With redirects(0) 3xx arrive as Ok; 4xx/5xx arrive as Error::Status.
            Err(ureq::Error::Status(_, r)) => r,
            Err(ureq::Error::Transport(t)) => return Err(FetchError::Network(short(&t.to_string()))),
        };
        let status = resp.status();
        let location = resp.header("location").map(str::to_string);
        let content_type = resp.header("content-type").map(str::to_string);
        if let Some(len) = resp.header("content-length").and_then(|v| v.parse::<usize>().ok()) {
            if len > max_bytes {
                return Err(FetchError::TooLarge);
            }
        }
        let body = read_capped(resp.into_reader(), max_bytes)?;
        Ok(RawResponse { status, location, content_type, body })
    }
}

/// Reads at most `max` bytes; one extra byte proves the cap was exceeded.
pub fn read_capped(reader: impl Read, max: usize) -> Result<Vec<u8>, FetchError> {
    let mut buf = Vec::new();
    reader
        .take(max as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| FetchError::Network(short(&e.to_string())))?;
    if buf.len() > max {
        return Err(FetchError::TooLarge);
    }
    Ok(buf)
}

fn short(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(120).collect()
}

/// Per-action fetch budget.
pub struct Budget(AtomicUsize);

impl Budget {
    /// New budget of `n` fetches.
    pub fn new(n: usize) -> Self {
        Budget(AtomicUsize::new(n))
    }
    fn take(&self) -> bool {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| v.checked_sub(1))
            .is_ok()
    }
}

/// Global cap on simultaneous outbound requests.
pub struct Gate {
    max: usize,
    cur: AtomicUsize,
}

/// RAII slot in the [`Gate`].
pub struct Permit<'a>(&'a Gate);

impl Gate {
    /// New gate allowing `max` concurrent permits.
    pub fn new(max: usize) -> Self {
        Gate { max, cur: AtomicUsize::new(0) }
    }
    /// Takes a slot or fails with [`FetchError::Busy`].
    pub fn acquire(&self) -> Result<Permit<'_>, FetchError> {
        let ok = self.cur.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |v| (v < self.max).then_some(v + 1));
        match ok {
            Ok(_) => Ok(Permit(self)),
            Err(_) => Err(FetchError::Busy),
        }
    }
    /// Slots currently in use.
    pub fn in_use(&self) -> usize {
        self.cur.load(Ordering::SeqCst)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.0.cur.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Sliding-window rate limiter keyed by session id.
pub struct RateLimiter {
    per_minute: usize,
    max_keys: usize,
    hits: Mutex<HashMap<u64, VecDeque<Instant>>>,
}

impl RateLimiter {
    /// New limiter allowing `per_minute` actions per key.
    pub fn new(per_minute: usize) -> Self {
        Self { per_minute, max_keys: 4096, hits: Mutex::new(HashMap::new()) }
    }

    /// Records an action for `key`; false when the key is over its limit.
    pub fn check(&self, key: u64) -> bool {
        self.check_at(key, Instant::now())
    }

    /// [`Self::check`] with an explicit clock (tests).
    pub fn check_at(&self, key: u64, now: Instant) -> bool {
        let window = Duration::from_secs(60);
        let mut map = self.hits.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= self.max_keys && !map.contains_key(&key) {
            // Bound memory: drop keys whose window is empty/expired.
            map.retain(|_, q| q.back().map(|t| now.duration_since(*t) < window).unwrap_or(false));
            if map.len() >= self.max_keys {
                return false;
            }
        }
        let q = map.entry(key).or_default();
        while q.front().map(|t| now.duration_since(*t) >= window).unwrap_or(false) {
            q.pop_front();
        }
        if q.len() >= self.per_minute {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// The safe fetcher.
pub struct SafeFetcher {
    backend: Box<dyn Backend>,
    max_redirects: usize,
    max_html: usize,
    max_image: usize,
    allowed_hosts: Option<Vec<String>>,
    gate: Gate,
}

impl SafeFetcher {
    /// Production fetcher.
    pub fn new(cfg: &Config) -> Self {
        Self::with_backend(cfg, Box::new(UreqBackend::new(cfg)))
    }

    /// Fetcher over any backend (tests).
    pub fn with_backend(cfg: &Config, backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            max_redirects: cfg.max_redirects,
            max_html: cfg.max_html_bytes,
            max_image: cfg.max_image_bytes,
            allowed_hosts: cfg.effective_allowed_hosts(),
            gate: Gate::new(cfg.max_concurrent_fetches),
        }
    }

    /// Parses and vets `raw` (no network).
    pub fn parse_url(&self, raw: &str) -> Result<Url, FetchError> {
        let url = Url::parse(raw.trim()).map_err(|_| FetchError::Refused(UrlRefusal::Malformed))?;
        netguard::validate_url(&url, self.allowed_hosts.as_deref()).map_err(FetchError::Refused)?;
        Ok(url)
    }

    /// Fetches `url`, following up to `max_redirects` hops, re-validating every hop.
    pub fn fetch(&self, url: &Url, kind: Kind, budget: &Budget) -> Result<Fetched, FetchError> {
        let (accept, cap) = match kind {
            Kind::Document => ("text/html,application/xhtml+xml,application/xml,application/json;q=0.9,*/*;q=0.1", self.max_html),
            Kind::Image => ("image/png,image/jpeg,image/gif,image/webp;q=0.9", self.max_image),
        };
        let mut current = url.clone();
        for hop in 0..=self.max_redirects {
            netguard::validate_url(&current, self.allowed_hosts.as_deref()).map_err(FetchError::Refused)?;
            if !budget.take() {
                return Err(FetchError::BudgetExceeded);
            }
            let resp = {
                let _permit = self.gate.acquire()?;
                self.backend.get(&current, accept, cap)?
            };
            if matches!(resp.status, 301 | 302 | 303 | 307 | 308) {
                if hop == self.max_redirects {
                    return Err(FetchError::TooManyRedirects);
                }
                let loc = resp.location.ok_or(FetchError::BadRedirect)?;
                current = current.join(loc.trim()).map_err(|_| FetchError::BadRedirect)?;
                continue;
            }
            if !(200..300).contains(&resp.status) {
                return Err(FetchError::Status(resp.status));
            }
            return Ok(Fetched { final_url: current, content_type: resp.content_type, body: resp.body });
        }
        Err(FetchError::TooManyRedirects)
    }

    /// Fetches a document and decodes it lossily as UTF-8 text.
    pub fn fetch_text(&self, url: &Url, budget: &Budget) -> Result<(Url, String), FetchError> {
        let f = self.fetch(url, Kind::Document, budget)?;
        Ok((f.final_url, String::from_utf8_lossy(&f.body).into_owned()))
    }

    /// Slots currently in use (tests/metrics).
    pub fn in_flight(&self) -> usize {
        self.gate.in_use()
    }
}

/// Shared handle type.
pub type SharedFetcher = Arc<SafeFetcher>;

#[cfg(test)]
pub(crate) mod testutil {
    use super::*;

    /// Scripted backend: answers by URL string; records every URL requested.
    pub struct Fake {
        pub routes: HashMap<String, RawResponse>,
        pub seen: Mutex<Vec<String>>,
    }

    impl Fake {
        pub fn new(routes: Vec<(&str, RawResponse)>) -> Self {
            Fake { routes: routes.into_iter().map(|(k, v)| (k.to_string(), v)).collect(), seen: Mutex::new(vec![]) }
        }
    }

    pub fn ok(body: &[u8]) -> RawResponse {
        RawResponse { status: 200, location: None, content_type: Some("text/html".into()), body: body.to_vec() }
    }

    pub fn redirect(to: &str) -> RawResponse {
        RawResponse { status: 302, location: Some(to.into()), content_type: None, body: vec![] }
    }

    impl Backend for Fake {
        fn get(&self, url: &Url, _accept: &str, max: usize) -> Result<RawResponse, FetchError> {
            self.seen.lock().unwrap().push(url.to_string());
            let r = self.routes.get(url.as_str()).cloned().ok_or(FetchError::Network("no route".into()))?;
            if r.body.len() > max {
                return Err(FetchError::TooLarge);
            }
            Ok(r)
        }
    }

    pub struct Shared(pub Arc<Fake>);
    impl Backend for Shared {
        fn get(&self, url: &Url, a: &str, m: usize) -> Result<RawResponse, FetchError> {
            self.0.get(url, a, m)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    fn cfg() -> Config {
        Config { max_redirects: 3, ..Config::default() }
    }

    fn fetcher(routes: Vec<(&str, RawResponse)>) -> (SafeFetcher, Arc<Fake>) {
        let fake = Arc::new(Fake::new(routes));
        (SafeFetcher::with_backend(&cfg(), Box::new(Shared(fake.clone()))), fake)
    }

    #[test]
    fn follows_redirects_and_resolves_relative_locations() {
        let (f, fake) = fetcher(vec![
            ("https://a.example.com/start", redirect("/mid")),
            ("https://a.example.com/mid", redirect("https://b.example.com/end")),
            ("https://b.example.com/end", ok(b"hello")),
        ]);
        let r = f.fetch(&Url::parse("https://a.example.com/start").unwrap(), Kind::Document, &Budget::new(10)).unwrap();
        assert_eq!(r.body, b"hello");
        assert_eq!(r.final_url.as_str(), "https://b.example.com/end");
        assert_eq!(fake.seen.lock().unwrap().len(), 3);
    }

    #[test]
    fn every_redirect_hop_is_revalidated() {
        for evil in [
            "http://127.0.0.1/admin",
            "http://169.254.169.254/latest/meta-data/",
            "http://[::1]/",
            "http://2130706433/",
            "file:///etc/passwd",
            "http://localhost/",
            "https://user:pw@example.com/",
            "http://example.com:22/",
        ] {
            let (f, fake) = fetcher(vec![("https://a.example.com/r", redirect(evil))]);
            let r = f.fetch(&Url::parse("https://a.example.com/r").unwrap(), Kind::Document, &Budget::new(10));
            assert!(matches!(r, Err(FetchError::Refused(_))), "{evil}: {r:?}");
            // The forbidden hop must never reach the backend.
            assert_eq!(fake.seen.lock().unwrap().len(), 1, "{evil} was requested");
        }
    }

    #[test]
    fn redirect_loop_is_capped() {
        let (f, fake) = fetcher(vec![
            ("https://a.example.com/1", redirect("/2")),
            ("https://a.example.com/2", redirect("/1")),
        ]);
        let r = f.fetch(&Url::parse("https://a.example.com/1").unwrap(), Kind::Document, &Budget::new(100));
        assert_eq!(r.unwrap_err(), FetchError::TooManyRedirects);
        assert_eq!(fake.seen.lock().unwrap().len(), 4); // 3 hops allowed + the last one
    }

    #[test]
    fn redirect_without_location_and_http_errors() {
        let (f, _) = fetcher(vec![
            ("https://a.example.com/x", RawResponse { status: 302, location: None, content_type: None, body: vec![] }),
            ("https://a.example.com/404", RawResponse { status: 404, location: None, content_type: None, body: vec![] }),
        ]);
        let b = Budget::new(10);
        assert_eq!(f.fetch(&Url::parse("https://a.example.com/x").unwrap(), Kind::Document, &b).unwrap_err(), FetchError::BadRedirect);
        assert_eq!(f.fetch(&Url::parse("https://a.example.com/404").unwrap(), Kind::Document, &b).unwrap_err(), FetchError::Status(404));
    }

    #[test]
    fn size_cap_applies_per_kind() {
        let big = vec![b'x'; 2048];
        let small_cfg = Config { max_html_bytes: 1024, max_image_bytes: 4096, ..cfg() };
        let fake = Arc::new(Fake::new(vec![("https://a.example.com/big", ok(&big))]));
        let f = SafeFetcher::with_backend(&small_cfg, Box::new(Shared(fake)));
        let u = Url::parse("https://a.example.com/big").unwrap();
        assert_eq!(f.fetch(&u, Kind::Document, &Budget::new(5)).unwrap_err(), FetchError::TooLarge);
        assert!(f.fetch(&u, Kind::Image, &Budget::new(5)).is_ok());
    }

    #[test]
    fn read_capped_detects_overflow_on_streams() {
        // Streaming path (no Content-Length): one byte over the cap is detected.
        assert_eq!(read_capped(std::io::repeat(b'a').take(10_000), 100).unwrap_err(), FetchError::TooLarge);
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), b"abc");
        assert_eq!(read_capped(&b"abcd"[..], 3).unwrap_err(), FetchError::TooLarge);
        // Never reads far beyond the cap from an endless stream.
        let mut r = std::io::repeat(0).take(u64::MAX);
        assert!(read_capped(&mut r, 1000).is_err());
    }

    #[test]
    fn budget_is_enforced_including_redirect_hops() {
        let (f, _) = fetcher(vec![
            ("https://a.example.com/1", redirect("/2")),
            ("https://a.example.com/2", ok(b"x")),
        ]);
        let b = Budget::new(1);
        assert_eq!(f.fetch(&Url::parse("https://a.example.com/1").unwrap(), Kind::Document, &b).unwrap_err(), FetchError::BudgetExceeded);
        let b = Budget::new(2);
        assert!(f.fetch(&Url::parse("https://a.example.com/1").unwrap(), Kind::Document, &b).is_ok());
    }

    #[test]
    fn gate_caps_concurrency_and_releases() {
        let g = Gate::new(2);
        let p1 = g.acquire().unwrap();
        let _p2 = g.acquire().unwrap();
        assert_eq!(g.acquire().err(), Some(FetchError::Busy));
        drop(p1);
        assert!(g.acquire().is_ok());
    }

    #[test]
    fn rate_limiter_is_per_session_and_sliding() {
        let rl = RateLimiter::new(3);
        let t0 = Instant::now();
        assert!(rl.check_at(1, t0) && rl.check_at(1, t0) && rl.check_at(1, t0));
        assert!(!rl.check_at(1, t0));
        assert!(rl.check_at(2, t0), "other sessions are unaffected");
        assert!(!rl.check_at(1, t0 + Duration::from_secs(59)));
        assert!(rl.check_at(1, t0 + Duration::from_secs(61)), "window slides");
    }

    #[test]
    fn resolver_filter_drops_private_addresses() {
        let addrs: Vec<SocketAddr> = vec!["127.0.0.1:80".parse().unwrap(), "10.0.0.1:80".parse().unwrap(), "[::1]:80".parse().unwrap(), "8.8.8.8:80".parse().unwrap()];
        let ok = VettedResolver::vet(addrs);
        assert_eq!(ok, vec!["8.8.8.8:80".parse::<SocketAddr>().unwrap()]);
        // Production resolver rejects a loopback literal outright (no DNS involved).
        assert!(ureq::Resolver::resolve(&VettedResolver, "127.0.0.1:80").is_err());
        assert!(ureq::Resolver::resolve(&VettedResolver, "[::1]:443").is_err());
    }

    #[test]
    fn parse_url_applies_allow_list() {
        let c = Config { allow_any_url: false, ..Config::default() };
        let f = SafeFetcher::with_backend(&c, Box::new(Fake::new(vec![])));
        assert!(f.parse_url("https://en.wikipedia.org/wiki/Rust").is_ok());
        assert!(matches!(f.parse_url("https://example.com/"), Err(FetchError::Refused(UrlRefusal::NotAllowed))));
        assert!(matches!(f.parse_url("not a url"), Err(FetchError::Refused(UrlRefusal::Malformed))));
    }
}
