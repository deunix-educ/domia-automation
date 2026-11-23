// SPDX-License-Identifier: AGPL-3.0-or-later
//! Content security policy (CSP) of the HTML pages, with a different nonce for each response.
//!
//! Leptos generates the nonce during rendering and puts it on its inline scripts (hydration, resources). The header is
//! therefore set **during rendering**, by the same component that knows this nonce: a header set by a middleware could not
//! know it.
//!
//! Policy: nothing third-party, no inline script or style without a nonce, no `eval` (only WebAssembly compilation is
//! allowed). `style="..."` and `on...=` attributes are forbidden: the interface produces none (test in
//! `crates/dominet/tests/ui.rs`). Iframes (Grafana, R2) and media streams (R6) will add their `frame-src` /
//! `media-src` / `connect-src` directives when the time comes.

use axum::http::{HeaderValue, header};
use leptos::nonce::use_nonce;
use leptos::prelude::use_context;
use leptos_axum::ResponseOptions;

/// The policy for a given nonce.
#[must_use]
pub fn policy(nonce: &str, frame_origin: Option<&str>, webrtc_origin: Option<&str>) -> String {
    let mut directives = vec![
        "default-src 'self'".to_string(),
        format!("script-src 'self' 'nonce-{nonce}' 'wasm-unsafe-eval'"),
        "style-src 'self'".to_string(),
        "img-src 'self' data:".to_string(),
        "font-src 'self'".to_string(),
        // `'self'` also covers the WebSocket of the same origin (ws:// or wss://) in recent browsers.
        // MediaMTX's origin (WHEP signalling of the live view) is added when it is not the same.
        match webrtc_origin {
            Some(origin) => format!("connect-src 'self' {origin}"),
            None => "connect-src 'self'".to_string(),
        },
        "object-src 'none'".to_string(),
        "base-uri 'self'".to_string(),
        "form-action 'self'".to_string(),
        "frame-ancestors 'none'".to_string(),
    ];
    // Iframes are only allowed towards Grafana's origin (`GRAFANA_PUBLIC_URL` setting).
    if let Some(origin) = frame_origin {
        directives.push(format!("frame-src {origin}"));
    }
    directives.join("; ")
}

/// MediaMTX origin (scheme, host, port) for the browser, from the settings and the current request's host.
fn webrtc_origin(state: &dominet_server::AppState) -> Option<String> {
    let host = use_context::<axum::http::request::Parts>().and_then(|p| {
        p.headers
            .get(header::HOST)
            .and_then(|h| h.to_str().ok().map(str::to_string))
    });
    let url = dominet_server::cameras::whep_url_from(
        state
            .settings
            .cached_string("MEDIAMTX_WEBRTC_SCHEME")
            .as_deref(),
        state
            .settings
            .cached_string("MEDIAMTX_WEBRTC_DOMAIN")
            .as_deref(),
        host.as_deref(),
        &state.config.allowed_hosts,
        dominet_domain::camera::CameraUuid(uuid::Uuid::nil()),
    );
    dominet_domain::widget_view::http_origin(&url)
}

/// Sets the `Content-Security-Policy` header of the response being rendered.
pub fn apply() {
    let (Some(nonce), Some(response)) = (use_nonce(), use_context::<ResponseOptions>()) else {
        tracing::warn!("CSP not set: no nonce or no response context");
        return;
    };
    let frame_origin = use_context::<dominet_server::AppState>()
        .and_then(|state| state.settings.cached_string("GRAFANA_PUBLIC_URL"))
        .and_then(|url| dominet_domain::widget_view::http_origin(&url));
    let webrtc_origin =
        use_context::<dominet_server::AppState>().and_then(|state| webrtc_origin(&state));
    if let Ok(value) = HeaderValue::from_str(&policy(
        &nonce,
        frame_origin.as_deref(),
        webrtc_origin.as_deref(),
    )) {
        response.insert_header(header::CONTENT_SECURITY_POLICY, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_policy_allows_only_the_nonce_and_the_origin_for_scripts() {
        let p = policy("abc123", None, None);
        let script = p.split("; ").find(|d| d.starts_with("script-src")).unwrap();
        assert_eq!(
            script,
            "script-src 'self' 'nonce-abc123' 'wasm-unsafe-eval'"
        );
        // `'wasm-unsafe-eval'` (WebAssembly compilation) is intended; JavaScript `eval` is not.
        let without_wasm = p.replace("wasm-unsafe-eval", "");
        for forbidden in ["unsafe-inline", "unsafe-eval", "http:", "https:", "*"] {
            assert!(!without_wasm.contains(forbidden), "{forbidden} dans {p}");
        }
        for required in [
            "default-src 'self'",
            "object-src 'none'",
            "base-uri 'self'",
            "frame-ancestors 'none'",
            "form-action 'self'",
        ] {
            assert!(p.contains(required), "{required}");
        }
    }

    #[test]
    fn frames_are_allowed_only_towards_the_given_origin() {
        let p = policy("n", Some("https://grafana.local:3000"), None);
        assert!(p.contains("; frame-src https://grafana.local:3000"), "{p}");
        assert_eq!(p.matches("frame-src").count(), 1);
        assert!(!policy("n", None, None).contains("frame-src"));
    }

    #[test]
    fn the_media_server_origin_is_added_to_connect_src_only_when_given() {
        let p = policy("n", None, Some("http://127.0.0.1:18889"));
        assert!(
            p.contains("connect-src 'self' http://127.0.0.1:18889"),
            "{p}"
        );
        assert!(
            policy("n", None, None)
                .split("; ")
                .any(|d| d == "connect-src 'self'")
        );
    }
}
