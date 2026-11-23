// SPDX-License-Identifier: AGPL-3.0-or-later
//! Public URLs (WebRTC, R6) adapted to the host the browser arrived through.
//!
//! The settings (`MEDIAMTX_WEBRTC_DOMAIN`) contain the local network name (`dominet.local`). A browser that reaches DomiNet through
//! another address (local network IP, second VPN...) may not be able to resolve that name: the configured host is then replaced
//! by the request's, **if** the configured host is one of the instance's own hosts (`ALLOWED_HOSTS`). MediaMTX and DomiNet are
//! served by the same nginx, so on the same host.

/// Hosts of the instance itself: `ALLOWED_HOSTS` without the purely local addresses or the wildcards.
#[must_use]
pub fn own_hosts(allowed: &[String]) -> Vec<String> {
    allowed
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| {
            !h.is_empty()
                && !matches!(h.as_str(), "localhost" | "127.0.0.1" | "web")
                && !h.starts_with(['*', '.'])
        })
        .collect()
}

/// Host (without port) of a text `[scheme://]host[:port][/path]`; the brackets of an IPv6 address are removed.
fn host_of(authority_and_path: &str) -> String {
    let authority = authority_and_path
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split(']').next().unwrap_or("").to_ascii_lowercase();
    }
    authority
        .split(':')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// `configured` with the request's host, if the configured host is one of ours; otherwise `configured` as is.
///
/// The configured scheme, port and path are kept. A value without a scheme (`dominet.local:8889`) stays without a scheme.
/// `request_host` is the `Host` header (with its possible port).
#[must_use]
pub fn public_url(configured: &str, request_host: Option<&str>, own_hosts: &[String]) -> String {
    let Some(request_host) = request_host else {
        return configured.to_string();
    };
    let (scheme, rest) = match configured.split_once("://") {
        Some((s, r)) => (Some(s), r),
        None => (None, configured),
    };
    let configured_host = host_of(rest);
    let wanted = host_of(request_host);
    if configured_host.is_empty()
        || wanted.is_empty()
        || wanted == configured_host
        || !own_hosts.contains(&configured_host)
    {
        return configured.to_string();
    }
    // What remains after the host: port and path.
    let after_host = if rest.starts_with('[') {
        rest.split_once(']').map_or("", |(_, r)| r)
    } else {
        rest.strip_prefix(configured_host.as_str())
            .or_else(|| {
                // Different case in the configuration: we remove as many characters as the host.
                rest.get(configured_host.len()..)
            })
            .unwrap_or("")
    };
    let host = if wanted.contains(':') {
        format!("[{wanted}]")
    } else {
        wanted
    };
    match scheme {
        Some(s) => format!("{s}://{host}{after_host}"),
        None => format!("{host}{after_host}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn own() -> Vec<String> {
        own_hosts(&[
            "dominet.local".into(),
            "192.168.1.2".into(),
            "localhost".into(),
            "*.exemple.org".into(),
            ".exemple.net".into(),
            " ".into(),
        ])
    }

    #[test]
    fn own_hosts_drop_local_names_and_wildcards() {
        assert_eq!(own(), ["dominet.local", "192.168.1.2"]);
    }

    #[test]
    fn the_request_host_replaces_a_configured_own_host_and_keeps_scheme_port_and_path() {
        let own = own();
        assert_eq!(
            public_url("http://dominet.local:8889", Some("192.168.1.2"), &own),
            "http://192.168.1.2:8889"
        );
        assert_eq!(
            public_url(
                "https://dominet.local/webrtc",
                Some("192.168.1.2:8443"),
                &own
            ),
            "https://192.168.1.2/webrtc"
        );
        // Without a scheme, without a scheme.
        assert_eq!(
            public_url("dominet.local:8889", Some("192.168.1.2"), &own),
            "192.168.1.2:8889"
        );
        // IPv6 request host.
        assert_eq!(
            public_url("http://dominet.local:8889", Some("[fd00::1]:3000"), &own),
            "http://[fd00::1]:8889"
        );
    }

    #[test]
    fn the_configured_value_is_kept_in_every_other_case() {
        let own = own();
        // No request, same host, configured host that is not ours (external, localhost), empty value.
        assert_eq!(
            public_url("http://dominet.local:8889", None, &own),
            "http://dominet.local:8889"
        );
        assert_eq!(
            public_url(
                "http://dominet.local:8889",
                Some("dominet.local:3000"),
                &own
            ),
            "http://dominet.local:8889"
        );
        assert_eq!(
            public_url("http://localhost:8889", Some("192.168.1.2"), &own),
            "http://localhost:8889"
        );
        assert_eq!(
            public_url("https://cdn.exemple.com", Some("192.168.1.2"), &own),
            "https://cdn.exemple.com"
        );
        assert_eq!(public_url("", Some("192.168.1.2"), &own), "");
        // A host name is not compared case-sensitively.
        assert_eq!(
            public_url("http://DOMINET.local:8889", Some("192.168.1.2"), &own),
            "http://192.168.1.2:8889"
        );
    }
}
