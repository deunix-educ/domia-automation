// SPDX-License-Identifier: AGPL-3.0-or-later
//! Authorisation of camera viewing (ADR 0064): MediaMTX (`authMethod: http`) asks this route before every read.
//!
//! The route is internal. nginx does not proxy it, and the handler also refuses a caller that is not on a private network or
//! that went through a proxy (a forwarding header is present), so it is never reachable from outside the Docker network.
//! MediaMTX only reads the status: `200` allows, anything else denies; a `401` to empty credentials is what makes an RTSP client
//! send its credentials. The body of a decision never says why it was refused.
//!
//! Verified on MediaMTX 1.21.3: a bare Bearer token arrives in `token` with no `user`; a Basic or RTSP credential arrives in
//! `user` and `password` (the password is copied into `token` too); `authInternalUsers` is ignored; `publish` is excluded by
//! the configuration (`authHTTPExclude`) but is still decided here if it arrives.

use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::Router;
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use chrono::{DateTime, SecondsFormat, Utc};
use dominet_domain::repo::StreamAccessRepo;
use dominet_domain::stream_access::StreamAccessKind;
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::auth::service::AuthError;

/// Longest user name written to the attempts log (a forged one must not fill it).
const MAX_LOGGED_USER: usize = 150;

/// What MediaMTX sends. Every field may be absent or empty.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct AuthRequest {
    pub user: String,
    pub password: String,
    pub token: String,
    pub ip: String,
    pub action: String,
    pub path: String,
    pub protocol: String,
    pub id: String,
    pub query: String,
}

/// The outcome of a decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Allowed; a credential carries its end date, which closes the session at that time.
    Allow { expire_at: Option<DateTime<Utc>> },
    /// Refused. For empty credentials this is the RTSP challenge.
    Deny,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/internal/mediamtx/auth", post(auth))
}

/// An address that can only belong to the machine or to a private network (loopback, RFC 1918, unique local, link-local).
#[must_use]
pub fn is_internal(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_internal(IpAddr::V4(v4)))
        }
    }
}

/// Whether this HTTP call may be a MediaMTX one: from a private address, and not relayed by a proxy.
fn caller_allowed(peer: Option<SocketAddr>, headers: &HeaderMap) -> bool {
    let relayed = ["x-forwarded-for", "x-real-ip", "forwarded"]
        .iter()
        .any(|h| headers.contains_key(*h));
    !relayed && peer.is_some_and(|p| is_internal(p.ip()))
}

/// Compares two secrets without stopping at the first different byte (both are hashed first, so the length does not leak).
fn same_secret(a: &str, b: &str) -> bool {
    let (ha, hb) = (Sha256::digest(a.as_bytes()), Sha256::digest(b.as_bytes()));
    ha.iter().zip(hb.iter()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn metric(action: &str, outcome: &str) {
    let action = match action {
        "read" | "playback" | "publish" | "api" | "metrics" | "pprof" => action,
        _ => "other",
    };
    metrics::counter!("dominet_stream_auth_total", "action" => action.to_string(), "outcome" => outcome.to_string())
        .increment(1);
}

/// Decides one request; everything it needs is in the state. A lock-out comes back as `AuthError::Locked`.
///
/// # Errors
/// Storage failure: the caller answers with an error status, so MediaMTX refuses (fail closed).
pub async fn decide(state: &AppState, req: &AuthRequest) -> Result<Verdict, AuthError> {
    let internal = req.ip.parse::<IpAddr>().is_ok_and(is_internal);
    match req.action.as_str() {
        // The control API, the metrics and the profiler: only from inside.
        "api" | "metrics" | "pprof" | "publish" => Ok(if internal {
            Verdict::Allow { expire_at: None }
        } else {
            Verdict::Deny
        }),
        "read" | "playback" => decide_read(state, req).await,
        _ => Ok(Verdict::Deny),
    }
}

async fn decide_read(state: &AppState, req: &AuthRequest) -> Result<Verdict, AuthError> {
    let now = Utc::now();
    let repo = state.store.stream_access();
    // Empty credentials: the RTSP challenge (a first request without any), not a failure.
    if req.user.is_empty() && req.password.is_empty() && req.token.is_empty() {
        return Ok(Verdict::Deny);
    }
    // DomiNet's own reads (the `video` role).
    if let Some(service) = &state.config.mediamtx_read
        && !req.user.is_empty()
        && same_secret(&req.user, &service.user)
        && same_secret(&req.password, service.password.expose())
    {
        return Ok(Verdict::Allow { expire_at: None });
    }
    // The browser: a viewing token alone. It lives a few minutes but the session it opens is not cut when it expires.
    if req.user.is_empty() {
        let found = repo
            .find_active(StreamAccessKind::Viewing, &digest(&req.token), now)
            .await?;
        return Ok(found.map_or(Verdict::Deny, |_| Verdict::Allow { expire_at: None }));
    }
    // An external player: the account's user name and a personal credential.
    let who: String = req.user.chars().take(MAX_LOGGED_USER).collect();
    state.auth.check_stream_attempt(&who, &req.ip).await?;
    let found = repo
        .find_active(StreamAccessKind::Credential, &digest(&req.password), now)
        .await?;
    match found {
        Some(f) if f.is_user(&req.user) => {
            repo.touch(f.access.id, now).await?;
            Ok(Verdict::Allow {
                expire_at: Some(f.access.expires_at),
            })
        }
        _ => {
            state.auth.record_stream_failure(&who, &req.ip).await?;
            Ok(Verdict::Deny)
        }
    }
}

/// SHA-256 of a presented secret, as stored.
fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

/// Largest body accepted: what MediaMTX sends is a few hundred bytes.
const MAX_BODY: usize = 16 * 1024;

async fn auth(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|c| c.0);
    if !caller_allowed(peer, request.headers()) {
        metric("", "refused");
        return StatusCode::FORBIDDEN.into_response();
    }
    let Ok(bytes) = axum::body::to_bytes(request.into_body(), MAX_BODY).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Ok(req) = serde_json::from_slice::<AuthRequest>(&bytes) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    match decide(&state, &req).await {
        Ok(Verdict::Allow { expire_at }) => {
            metric(&req.action, "allowed");
            match expire_at {
                Some(t) => Json(json!({"expireAt": t.to_rfc3339_opts(SecondsFormat::Secs, true)}))
                    .into_response(),
                None => StatusCode::OK.into_response(),
            }
        }
        Ok(Verdict::Deny) => {
            metric(&req.action, "denied");
            // Never the secrets: what was asked, for whom and from where.
            tracing::debug!(action = %req.action, path = %req.path, protocol = %req.protocol, ip = %req.ip, "camera access denied");
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(AuthError::Locked { .. }) => {
            metric(&req.action, "locked");
            tracing::warn!(action = %req.action, ip = %req.ip, "camera access locked out");
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(e) => {
            metric(&req.action, "error");
            tracing::error!(error = %e, "camera access decision failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn sock(ip: &str) -> SocketAddr {
        SocketAddr::new(ip.parse().unwrap(), 40_000)
    }

    #[test]
    fn private_addresses_are_internal_and_public_ones_are_not() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.18.0.5",
            "192.168.1.9",
            "::1",
            "fd00::5",
            "::ffff:172.18.0.2",
        ] {
            assert!(is_internal(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "172.32.0.1", "2001:db8::1", "::ffff:8.8.8.8"] {
            assert!(!is_internal(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn only_a_direct_call_from_a_private_address_is_a_mediamtx_call() {
        let none = HeaderMap::new();
        assert!(caller_allowed(Some(sock("172.18.0.4")), &none));
        assert!(!caller_allowed(Some(sock("8.8.8.8")), &none));
        assert!(!caller_allowed(None, &none));
        // Behind nginx the peer is private too: the forwarding header gives it away.
        for h in ["x-forwarded-for", "x-real-ip", "forwarded"] {
            let mut headers = HeaderMap::new();
            headers.insert(h, HeaderValue::from_static("203.0.113.9"));
            assert!(!caller_allowed(Some(sock("172.18.0.4")), &headers), "{h}");
        }
    }

    #[test]
    fn secrets_are_compared_whole() {
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd") && !same_secret("abc", "abcd") && !same_secret("", "a"));
    }
}
