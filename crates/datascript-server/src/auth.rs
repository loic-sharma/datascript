//! Authentication and origin validation middleware for `/api/*` routes.
//!
//! Every API request (HTTP and WebSocket upgrade) must present the per-launch session token and,
//! when an `Origin` header is present, come from an allowed local origin. `Host` headers, when
//! present, are restricted to loopback hostnames to guard against DNS rebinding.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::AppState;

/// Header name accepted as an alternative to `Authorization: Bearer <token>`.
pub const TOKEN_HEADER: &str = "x-datascript-token";

/// Axum middleware enforcing `Host`, `Origin`, and session-token checks on `/api/*`.
pub(crate) async fn require_auth(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let headers = request.headers();

    if let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok())
        && !is_loopback_host(host)
    {
        return api_error(
            StatusCode::FORBIDDEN,
            "forbidden_host",
            "requests must target a loopback host (127.0.0.1 or localhost)",
        );
    }

    if let Some(origin) = headers.get(header::ORIGIN) {
        let Ok(origin_str) = origin.to_str() else {
            return api_error(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                "Origin header is not valid ASCII",
            );
        };
        if !state.is_allowed_origin(origin_str) {
            return api_error(
                StatusCode::FORBIDDEN,
                "forbidden_origin",
                format!("origin `{origin_str}` is not allowed to access this server"),
            );
        }
    }

    let provided = extract_token(headers, request.uri().query());
    match provided {
        Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => {
            next.run(request).await
        }
        Some(_) => api_error(
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            "invalid session token",
        ),
        None => api_error(
            StatusCode::UNAUTHORIZED,
            "missing_token",
            "missing session token; pass `Authorization: Bearer <token>`, `X-DataScript-Token`, or `?token=<token>`",
        ),
    }
}

/// Extracts a token from `Authorization: Bearer ...`, `X-DataScript-Token`, or `?token=...`.
fn extract_token(headers: &HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(auth) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        && let Some(token) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
    {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    if let Some(token) = headers.get(TOKEN_HEADER).and_then(|v| v.to_str().ok()) {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    if let Some(query) = query {
        for pair in query.split('&') {
            if let Some(value) = pair.strip_prefix("token=") {
                let decoded = percent_decode(value);
                if !decoded.is_empty() {
                    return Some(decoded);
                }
            }
        }
    }
    None
}

/// Constant-time byte comparison to prevent token timing leaks.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Whether a `Host` header names a local loopback interface (`127.0.0.1`, `localhost`, `[::1]`).
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim();
    let hostname = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map(|(ip, _)| ip).unwrap_or("")
    } else {
        host.split_once(':').map_or(host, |(name, _)| name)
    };
    hostname.eq_ignore_ascii_case("127.0.0.1")
        || hostname.eq_ignore_ascii_case("localhost")
        || hostname == "::1"
}

/// Minimal percent-decoder for query-string values (`+` is left literal so tokens stay exact).
pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Standard JSON error envelope for non-DataScript HTTP errors (auth, routing, bad request).
pub(crate) fn api_error(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        axum::Json(json!({
            "error": {
                "code": code,
                "message": message.into(),
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_host_checks_ports_and_rejects_external() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("127.0.0.1:8080"));
        assert!(is_loopback_host("localhost:5173"));
        assert!(is_loopback_host("LOCALHOST"));
        assert!(is_loopback_host("[::1]:3000"));
        assert!(!is_loopback_host("evil.example"));
        assert!(!is_loopback_host("evil.example:8080"));
        assert!(!is_loopback_host("127.0.0.1.evil.example"));
        assert!(!is_loopback_host("localhost.evil.example"));
    }

    #[test]
    fn constant_time_comparison_works() {
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
