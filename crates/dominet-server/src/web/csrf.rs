// SPDX-License-Identifier: AGPL-3.0-or-later
//! Protection CSRF par controle de l'origine des requetes non sures.
//!
//! En complement du cookie `SameSite=Lax` : une requete POST/PUT/PATCH/DELETE envoyee par un navigateur porte
//! `Origin` (ou a defaut `Referer`) ; elle n'est acceptee que si cette origine est celle du site (en-tete
//! `Host`) ou figure dans `CSRF_TRUSTED_ORIGINS`. Les clients hors navigateur (curl, application Android)
//! n'envoient ni `Origin` ni `Sec-Fetch-Site` : ils ne sont pas concernes par le CSRF et passent.

use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::AppState;
use crate::error::AppError;

fn is_safe(method: &Method) -> bool {
    matches!(
        *method,
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    )
}

/// Origine `schema://hote[:port]` d'une valeur `Origin` ou `Referer` (le chemin est ignore).
fn origin_of(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    if authority.is_empty() || !matches!(scheme, "http" | "https") {
        return None;
    }
    Some(format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    ))
}

/// L'origine est autorisee si son hote:port est celui de la requete ou si elle est dans la liste de confiance.
pub(crate) fn origin_allowed(origin: &str, host: Option<&str>, trusted: &[String]) -> bool {
    let Some(origin) = origin_of(origin) else {
        return false;
    };
    let authority = origin.split_once("://").map_or("", |(_, a)| a);
    if host.is_some_and(|h| h.eq_ignore_ascii_case(authority)) {
        return true;
    }
    trusted
        .iter()
        .filter_map(|t| origin_of(t))
        .any(|t| t == origin)
}

pub(crate) fn verdict(headers: &HeaderMap, trusted: &[String]) -> bool {
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok());
    let get = |name: header::HeaderName| headers.get(name).and_then(|v| v.to_str().ok());
    if let Some(origin) = get(header::ORIGIN) {
        return origin_allowed(origin, host, trusted);
    }
    if let Some(referer) = get(header::REFERER) {
        return origin_allowed(referer, host, trusted);
    }
    // Ni Origin ni Referer : un navigateur moderne annonce quand meme `Sec-Fetch-Site`.
    !matches!(
        headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()),
        Some("cross-site" | "same-site")
    )
}

pub async fn check_origin(State(state): State<AppState>, req: Request, next: Next) -> Response {
    if !is_safe(req.method()) && !verdict(req.headers(), &state.config.csrf_trusted_origins) {
        tracing::warn!(method = %req.method(), path = %super::loggable_path(req.uri().path()), "request refused: origin not allowed");
        return AppError::Forbidden.into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn same_origin_is_accepted_whatever_the_scheme_behind_the_proxy() {
        let trusted: Vec<String> = vec![];
        assert!(verdict(
            &headers(&[
                ("host", "dominet.local"),
                ("origin", "https://dominet.local")
            ]),
            &trusted
        ));
        assert!(verdict(
            &headers(&[
                ("host", "dominet.local:8000"),
                ("origin", "http://dominet.local:8000")
            ]),
            &trusted
        ));
        assert!(verdict(
            &headers(&[
                ("host", "DoMiNet.Local"),
                ("origin", "https://dominet.LOCAL")
            ]),
            &trusted
        ));
    }

    #[test]
    fn a_foreign_or_malformed_origin_is_refused() {
        let t: Vec<String> = vec![];
        for origin in [
            "https://evil.example",
            "http://dominet.local.evil.example",
            "null",
            "",
            "javascript:alert(1)",
            "https://dominet.local:9999",
        ] {
            assert!(
                !verdict(
                    &headers(&[("host", "dominet.local"), ("origin", origin)]),
                    &t
                ),
                "{origin:?}"
            );
        }
    }

    #[test]
    fn trusted_origins_are_accepted() {
        let trusted = vec!["https://dominet.example.org/".to_string()];
        assert!(verdict(
            &headers(&[
                ("host", "127.0.0.1:8000"),
                ("origin", "https://dominet.example.org")
            ]),
            &trusted
        ));
        assert!(
            !verdict(
                &headers(&[
                    ("host", "127.0.0.1:8000"),
                    ("origin", "http://dominet.example.org")
                ]),
                &trusted
            ),
            "autre schema"
        );
    }

    #[test]
    fn referer_is_used_when_origin_is_missing() {
        let t: Vec<String> = vec![];
        assert!(verdict(
            &headers(&[
                ("host", "dominet.local"),
                ("referer", "https://dominet.local/settings?x=1")
            ]),
            &t
        ));
        assert!(!verdict(
            &headers(&[
                ("host", "dominet.local"),
                ("referer", "https://evil.example/page")
            ]),
            &t
        ));
    }

    #[test]
    fn non_browser_clients_without_origin_headers_pass_but_cross_site_fetches_do_not() {
        let t: Vec<String> = vec![];
        assert!(
            verdict(&headers(&[("host", "dominet.local")]), &t),
            "curl, application Android"
        );
        assert!(verdict(
            &headers(&[("host", "dominet.local"), ("sec-fetch-site", "same-origin")]),
            &t
        ));
        assert!(!verdict(
            &headers(&[("host", "dominet.local"), ("sec-fetch-site", "cross-site")]),
            &t
        ));
        assert!(!verdict(
            &headers(&[("host", "dominet.local"), ("sec-fetch-site", "same-site")]),
            &t
        ));
    }

    #[test]
    fn safe_methods_are_never_checked() {
        for m in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(is_safe(&m));
        }
        for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(!is_safe(&m));
        }
    }
}
