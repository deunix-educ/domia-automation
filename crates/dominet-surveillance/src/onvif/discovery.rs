// SPDX-License-Identifier: AGPL-3.0-or-later
//! WS-Discovery (multicast `239.255.255.250:3702`): finds the ONVIF cameras of the local network (ADR 0008 and 0045).
//!
//! Three probes one second apart and six seconds of listening (UDP multicast gets lost from time to time on Wi-Fi). Some
//! cameras never answer the multicast (the Q3R): adding by IP address remains necessary.

use std::collections::HashSet;
use std::fmt::Write;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket as StdUdp};
use std::time::Duration;

use roxmltree::Document;
use tokio::net::UdpSocket;
use tokio::time::Instant;

pub const MULTICAST: SocketAddr =
    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(239, 255, 255, 250), 3702));
pub const LISTEN: Duration = Duration::from_secs(6);
pub const PROBES: u32 = 3;

/// A device that answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// Address the answer comes from (the reachable one).
    pub ip: IpAddr,
    /// Announced ONVIF service port.
    pub port: u16,
    pub xaddr: String,
    pub name: String,
    pub hardware: String,
}

/// The `Probe` message (type `NetworkVideoTransmitter`).
#[must_use]
pub fn probe_message() -> String {
    let mut id = [0u8; 16];
    getrandom::fill(&mut id).expect("system random generator");
    let id = id.iter().fold(String::new(), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    });
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope" xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery" xmlns:dn="http://www.onvif.org/ver10/network/wsdl"><e:Header><w:MessageID>uuid:{id}</w:MessageID><w:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To><w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action></e:Header><e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types></d:Probe></e:Body></e:Envelope>"#
    )
}

/// Reads a `ProbeMatches` response. `None` if it is not one or if it announces no usable `http://` address.
#[must_use]
pub fn parse_response(data: &str, from: IpAddr) -> Option<Discovered> {
    let doc = Document::parse(data).ok()?;
    let text = |local: &str| {
        doc.descendants()
            .find(|d| d.is_element() && d.tag_name().name() == local)
            .and_then(|e| e.text())
            .map(|t| t.trim().to_string())
    };
    let xaddrs = text("XAddrs")?;
    // The first IPv4 `http://` address; link-local IPv6 addresses are not reachable as they are.
    let xaddr = xaddrs
        .split_whitespace()
        .find(|x| x.starts_with("http://") && !x.starts_with("http://["))?;
    let authority = xaddr.strip_prefix("http://")?.split('/').next()?;
    let port = authority
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
        .unwrap_or(80);
    let scopes = text("Scopes").unwrap_or_default();
    let scope = |prefix: &str| {
        scopes
            .split_whitespace()
            .find_map(|s| s.strip_prefix(prefix))
            .map(|v| v.replace("%20", " "))
            .unwrap_or_default()
    };
    Some(Discovered {
        ip: from,
        port,
        xaddr: xaddr.to_string(),
        name: scope("onvif://www.onvif.org/name/"),
        hardware: scope("onvif://www.onvif.org/hardware/"),
    })
}

/// Outgoing interface through the default route (the one that leads to the multicast).
#[must_use]
pub fn local_ipv4() -> Ipv4Addr {
    StdUdp::bind("0.0.0.0:0")
        .and_then(|s| s.connect(MULTICAST).map(|()| s))
        .and_then(|s| s.local_addr())
        .ok()
        .and_then(|a| match a.ip() {
            IpAddr::V4(v4) if !v4.is_unspecified() => Some(v4),
            _ => None,
        })
        .unwrap_or(Ipv4Addr::UNSPECIFIED)
}

/// Probes the local network; `on_found` is called for each device, once per address.
///
/// # Errors
/// The socket cannot be opened.
pub async fn discover(on_found: impl FnMut(Discovered)) -> io::Result<()> {
    discover_at(MULTICAST, Some(local_ipv4()), PROBES, LISTEN, on_found).await
}

/// Like [`discover`], towards any target (the tests aim at a fake responder on the loopback).
///
/// # Errors
/// The socket cannot be opened.
pub async fn discover_at(
    target: SocketAddr,
    interface: Option<Ipv4Addr>,
    probes: u32,
    listen: Duration,
    mut on_found: impl FnMut(Discovered),
) -> io::Result<()> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None)?;
    socket.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0).into())?;
    socket.set_multicast_ttl_v4(2)?;
    if let Some(i) = interface {
        socket.set_multicast_if_v4(&i)?;
    }
    socket.set_nonblocking(true)?;
    let socket = UdpSocket::from_std(socket.into())?;
    let probe = probe_message();
    let start = Instant::now();
    let mut sent = 0u32;
    let mut seen = HashSet::new();
    let mut buf = vec![0u8; 16 * 1024];
    while start.elapsed() < listen {
        // One probe per second, the first right away.
        if sent < probes && start.elapsed() >= Duration::from_secs(u64::from(sent)) {
            socket.send_to(probe.as_bytes(), target).await?;
            sent += 1;
        }
        let next_probe = if sent < probes {
            start + Duration::from_secs(u64::from(sent))
        } else {
            start + listen
        };
        let until = next_probe.min(start + listen);
        if let Ok(Ok((n, from))) = tokio::time::timeout_at(until, socket.recv_from(&mut buf)).await
            && let Some(found) = parse_response(&String::from_utf8_lossy(&buf[..n]), from.ip())
            && seen.insert(found.ip)
        {
            on_found(found);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"><s:Body><d:ProbeMatches><d:ProbeMatch><d:Types>dn:NetworkVideoTransmitter</d:Types><d:Scopes>onvif://www.onvif.org/name/IPC-model onvif://www.onvif.org/hardware/NVT%20IPC onvif://www.onvif.org/Profile/Streaming</d:Scopes><d:XAddrs>http://[fe80::1]:8899/onvif/device_service http://192.168.1.29:8899/onvif/device_service</d:XAddrs></d:ProbeMatch></d:ProbeMatches></s:Body></s:Envelope>"#;

    #[test]
    fn a_probe_match_gives_the_reachable_address_and_the_labels() {
        let from: IpAddr = "192.168.1.29".parse().unwrap();
        let d = parse_response(RESPONSE, from).unwrap();
        assert_eq!((d.ip, d.port), (from, 8899));
        assert_eq!(d.xaddr, "http://192.168.1.29:8899/onvif/device_service");
        assert_eq!(
            (d.name.as_str(), d.hardware.as_str()),
            ("IPC-model", "NVT IPC")
        );
    }

    #[test]
    fn what_is_not_a_probe_match_is_ignored() {
        let from: IpAddr = "10.0.0.1".parse().unwrap();
        assert!(parse_response("pas du xml", from).is_none());
        assert!(parse_response("<a/>", from).is_none());
        let only_v6 = RESPONSE.replace("http://192.168.1.29:8899/onvif/device_service", "");
        assert!(parse_response(&only_v6, from).is_none());
    }

    #[test]
    fn the_probe_asks_for_network_video_transmitters() {
        let p = probe_message();
        assert!(p.contains("NetworkVideoTransmitter") && p.contains("discovery/Probe"));
        assert_ne!(
            p,
            probe_message(),
            "identifiant de message different a chaque sonde"
        );
    }
}
