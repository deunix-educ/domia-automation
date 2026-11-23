// SPDX-License-Identifier: AGPL-3.0-or-later
//! Client IP address, including behind a trusted proxy (nginx).

use std::net::{IpAddr, SocketAddr};

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use ipnet::IpNet;

use crate::AppState;

fn trusted(ip: IpAddr, proxies: &[IpNet]) -> bool {
    proxies.iter().any(|n| n.contains(&ip))
}

/// Determines the client's IP: the peer's address, unless that peer is a trusted proxy, in which case we
/// go up `X-Forwarded-For` from right to left to the first address that is not a trusted proxy.
/// A header forged by a direct client is therefore ignored.
#[must_use]
pub fn resolve(peer: Option<IpAddr>, forwarded_for: Option<&str>, proxies: &[IpNet]) -> String {
    let Some(peer) = peer else {
        return "unknown".into();
    };
    if !trusted(peer, proxies) {
        return peer.to_string();
    }
    let hops = forwarded_for
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty());
    for hop in hops.rev() {
        match hop.parse::<IpAddr>() {
            Ok(ip) if trusted(ip, proxies) => {}
            Ok(ip) => return ip.to_string(),
            Err(_) => break, // unreadable value: we stop at the peer
        }
    }
    peer.to_string()
}

/// Extractor: client IP (`"unknown"` if the peer's address is unknown, for example in tests).
#[derive(Debug, Clone)]
pub struct ClientIp(pub String);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = std::convert::Infallible;

    #[allow(clippy::unused_async_trait_impl)] // the trait requires `async`
    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        let xff = parts
            .headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok());
        Ok(Self(resolve(peer, xff, &state.config.trusted_proxies)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nets(v: &[&str]) -> Vec<IpNet> {
        v.iter().map(|s| s.parse().unwrap()).collect()
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    #[test]
    fn a_direct_client_is_identified_by_its_address_and_cannot_forge_the_header() {
        assert_eq!(
            resolve(Some(ip("192.168.1.50")), Some("6.6.6.6"), &[]),
            "192.168.1.50"
        );
        assert_eq!(
            resolve(
                Some(ip("192.168.1.50")),
                Some("6.6.6.6"),
                &nets(&["172.18.0.1/32"])
            ),
            "192.168.1.50"
        );
    }

    #[test]
    fn behind_a_trusted_proxy_the_forwarded_address_is_used() {
        let p = nets(&["172.18.0.1/32"]);
        assert_eq!(
            resolve(Some(ip("172.18.0.1")), Some("192.168.1.50"), &p),
            "192.168.1.50"
        );
    }

    #[test]
    fn the_rightmost_untrusted_hop_wins_so_a_spoofed_prefix_is_ignored() {
        let p = nets(&["172.18.0.0/16", "10.0.0.1/32"]);
        // The client sent "1.1.1.1"; nginx added its view of the client (192.168.1.50) then an internal proxy.
        assert_eq!(
            resolve(
                Some(ip("172.18.0.1")),
                Some("1.1.1.1, 192.168.1.50, 10.0.0.1"),
                &p
            ),
            "192.168.1.50"
        );
    }

    #[test]
    fn missing_or_garbled_headers_fall_back_to_the_peer() {
        let p = nets(&["172.18.0.1/32"]);
        assert_eq!(resolve(Some(ip("172.18.0.1")), None, &p), "172.18.0.1");
        assert_eq!(
            resolve(Some(ip("172.18.0.1")), Some("pas-une-ip"), &p),
            "172.18.0.1"
        );
        assert_eq!(
            resolve(
                Some(ip("172.18.0.1")),
                Some("192.168.1.50, n'importe quoi"),
                &p
            ),
            "172.18.0.1"
        );
    }

    #[test]
    fn unknown_peer_is_reported_as_unknown() {
        assert_eq!(resolve(None, Some("1.2.3.4"), &[]), "unknown");
    }

    #[test]
    fn ipv6_is_supported() {
        let p = nets(&["::1/128"]);
        assert_eq!(
            resolve(Some(ip("::1")), Some("2001:db8::7"), &p),
            "2001:db8::7"
        );
    }
}
