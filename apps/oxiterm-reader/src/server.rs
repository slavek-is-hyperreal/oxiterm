//! `/events` endpoint (hyper 1): Bearer auth in constant time, fail closed, size-capped.

use crate::app::{App, Event};
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::{Method, Request, Response, StatusCode};
use std::sync::Arc;
use subtle::ConstantTimeEq;

/// Request body cap (OxiTerm payloads are a few hundred bytes).
pub const MAX_BODY: usize = 64 * 1024;

/// Constant-time Bearer check. An empty/absent expected token never matches (fail closed).
pub fn authorized(expected: &str, header: Option<&str>) -> bool {
    if expected.is_empty() {
        return false;
    }
    let Some(h) = header else { return false };
    let Some(given) = h.strip_prefix("Bearer ") else { return false };
    given.as_bytes().ct_eq(expected.as_bytes()).into()
}

/// Transport-independent request routing: returns (status, JSON body).
pub async fn route(app: Arc<App>, token: &str, method: &Method, path: &str, auth: Option<&str>, body: &[u8]) -> (StatusCode, Option<String>) {
    if !authorized(token, auth) {
        return (StatusCode::UNAUTHORIZED, None);
    }
    if method != Method::POST || path != "/events" {
        return (StatusCode::NOT_FOUND, None);
    }
    let Ok(ev) = serde_json::from_slice::<Event>(body) else {
        return (StatusCode::BAD_REQUEST, None);
    };
    // Fetching, parsing and image decoding are blocking and CPU-heavy.
    let patch = tokio::task::spawn_blocking(move || app.handle(&ev)).await.ok().flatten();
    match patch {
        Some(v) => (StatusCode::OK, Some(v.to_string())),
        None => (StatusCode::NO_CONTENT, None),
    }
}

async fn serve_one(app: Arc<App>, token: Arc<String>, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, std::convert::Infallible> {
    let (parts, body) = req.into_parts();
    let auth = parts.headers.get(hyper::header::AUTHORIZATION).and_then(|v| v.to_str().ok()).map(str::to_string);
    // Authenticate before reading the body.
    if !authorized(&token, auth.as_deref()) {
        return Ok(resp(StatusCode::UNAUTHORIZED, None));
    }
    let bytes = match Limited::new(body, MAX_BODY).collect().await {
        Ok(c) => c.to_bytes(),
        Err(_) => return Ok(resp(StatusCode::PAYLOAD_TOO_LARGE, None)),
    };
    let (code, json) = route(app, &token, &parts.method, parts.uri.path(), auth.as_deref(), &bytes).await;
    Ok(resp(code, json))
}

fn resp(code: StatusCode, body: Option<String>) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from(body.clone().unwrap_or_default())));
    *r.status_mut() = code;
    if body.is_some() {
        r.headers_mut().insert(hyper::header::CONTENT_TYPE, "application/json".parse().unwrap());
    }
    r
}

/// Runs the server until the process ends.
pub async fn run(app: Arc<App>, token: String, listen: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!token.is_empty(), "OXITERM_APP_TOKEN must be set (the reader fails closed without it)");
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!("oxiterm-reader listening on http://{listen}/events");
    let token = Arc::new(token);
    loop {
        let (stream, _) = listener.accept().await?;
        let (app, token) = (app.clone(), token.clone());
        tokio::spawn(async move {
            let io = hyper_util::rt::TokioIo::new(stream);
            let svc = hyper::service::service_fn(move |req| serve_one(app.clone(), token.clone(), req));
            let conn = hyper::server::conn::http1::Builder::new().timer(hyper_util::rt::TokioTimer::new()).header_read_timeout(std::time::Duration::from_secs(10)).serve_connection(io, svc);
            let _ = conn.await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_check_fails_closed() {
        assert!(authorized("s3cret", Some("Bearer s3cret")));
        assert!(!authorized("s3cret", Some("Bearer wrong")));
        assert!(!authorized("s3cret", Some("s3cret")));
        assert!(!authorized("s3cret", Some("bearer s3cret")));
        assert!(!authorized("s3cret", None));
        assert!(!authorized("", None), "empty token must never authorise");
        assert!(!authorized("", Some("Bearer ")), "empty token vs empty bearer must not match");
        assert!(!authorized("", Some("Bearer anything")));
    }
}
