// SPDX-License-Identifier: AGPL-3.0-or-later
//! Bearer authentication of `/mcp` (R11 M1, ADR 0059): access tokens stored hashed, with scopes, an end date and immediate
//! revocation, plus a per-token rate limit. The middleware puts the resulting [`Actor`] in the request extensions; the tools read
//! it from there (see `tools.rs`).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{Request, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use dominet_domain::mcp::{Actor, McpTokenId};
use dominet_server::{AppState, mcp_tokens};

/// Sliding window of one minute.
const WINDOW: Duration = Duration::from_secs(60);

/// Calls per minute and per token. In memory: a restart forgets the counts, which only ever favours the caller by a minute.
#[derive(Debug)]
pub struct RateLimiter {
    per_minute: usize,
    calls: Mutex<HashMap<McpTokenId, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> Self {
        Self {
            per_minute: usize::try_from(per_minute).unwrap_or(usize::MAX),
            calls: Mutex::default(),
        }
    }

    /// Counts a call at `now`; `Err(wait)` when the token has used up its minute.
    pub fn check(&self, id: McpTokenId, now: Instant) -> Result<(), Duration> {
        let mut calls = self
            .calls
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let window = calls.entry(id).or_default();
        while window
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW)
        {
            window.pop_front();
        }
        if window.len() >= self.per_minute {
            let wait = window.front().map_or(WINDOW, |oldest| {
                WINDOW.saturating_sub(now.saturating_duration_since(*oldest))
            });
            return Err(wait.max(Duration::from_secs(1)));
        }
        window.push_back(now);
        Ok(())
    }
}

/// State of the middleware.
#[derive(Clone)]
pub struct Guard {
    state: AppState,
    limiter: Arc<RateLimiter>,
}

impl Guard {
    pub fn new(state: AppState) -> Self {
        let limiter = Arc::new(RateLimiter::new(state.config.mcp_rate_limit_per_min));
        Self { state, limiter }
    }
}

/// The token of an `Authorization: Bearer <token>` header.
fn bearer(value: &HeaderValue) -> Option<&str> {
    let text = value.to_str().ok()?;
    let (scheme, token) = text.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then_some(token.trim())
        .filter(|t| !t.is_empty())
}

fn refused() -> Response {
    // Neither the header nor the path is logged: only the fact.
    tracing::warn!("MCP access refused: token missing, unknown, expired or revoked");
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, "Bearer")],
    )
        .into_response()
}

pub async fn require_bearer(State(guard): State<Guard>, mut req: Request, next: Next) -> Response {
    let Some(presented) = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(bearer)
        .map(str::to_owned)
    else {
        return refused();
    };
    let token = match mcp_tokens::authenticate(&guard.state, &presented).await {
        Ok(Some(token)) => token,
        Ok(None) => return refused(),
        Err(e) => {
            tracing::error!(error = %e, "MCP authentication impossible: storage unavailable");
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        }
    };
    if let Err(wait) = guard.limiter.check(token.id, Instant::now()) {
        tracing::warn!(token = %token.name, "MCP rate limit reached");
        return (
            StatusCode::TOO_MANY_REQUESTS,
            [(header::RETRY_AFTER, wait.as_secs().max(1).to_string())],
        )
            .into_response();
    }
    req.extensions_mut().insert(Actor::Token {
        id: token.id,
        name: token.name,
        scopes: token.scopes,
        owner: token.created_by,
    });
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bearer_scheme_is_case_insensitive_and_the_token_required() {
        let h = |s: &str| HeaderValue::from_str(s).unwrap();
        assert_eq!(bearer(&h("Bearer abc")), Some("abc"));
        assert_eq!(bearer(&h("bearer abc")), Some("abc"));
        assert_eq!(bearer(&h("Basic abc")), None);
        assert_eq!(bearer(&h("Bearer ")), None);
        assert_eq!(bearer(&h("abc")), None);
    }

    #[test]
    fn the_limit_is_per_token_and_the_window_slides() {
        let limiter = RateLimiter::new(3);
        let t0 = Instant::now();
        let (a, b) = (McpTokenId(1), McpTokenId(2));
        for i in 0..3 {
            assert!(limiter.check(a, t0 + Duration::from_secs(i)).is_ok());
        }
        // The fourth call within the minute is refused, with the time to wait for the oldest call to age out.
        let wait = limiter.check(a, t0 + Duration::from_secs(10)).unwrap_err();
        assert_eq!(wait, Duration::from_secs(50));
        // Another token is not affected.
        assert!(limiter.check(b, t0 + Duration::from_secs(10)).is_ok());
        // The window slides: the calls made at 0, 1 and 2 s leave it at 60, 61 and 62 s, one slot each time.
        for s in [60, 61, 62] {
            assert!(limiter.check(a, t0 + Duration::from_secs(s)).is_ok(), "{s}");
        }
        // Three calls are now inside the last minute (60, 61, 62 s): the fourth waits.
        assert!(limiter.check(a, t0 + Duration::from_secs(63)).is_err());
    }

    #[test]
    fn a_refused_call_is_not_counted() {
        let limiter = RateLimiter::new(1);
        let t0 = Instant::now();
        let id = McpTokenId(7);
        assert!(limiter.check(id, t0).is_ok());
        for s in 1..5 {
            assert!(limiter.check(id, t0 + Duration::from_secs(s)).is_err());
        }
        // Had the refused calls been counted, the window would still be full a minute after the first accepted call.
        assert!(limiter.check(id, t0 + Duration::from_secs(60)).is_ok());
    }
}
