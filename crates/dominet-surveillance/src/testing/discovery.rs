// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fake WS-Discovery responder: answers every received probe in unicast (CI never does multicast).

use std::net::SocketAddr;

use tokio::net::UdpSocket;

#[derive(Debug)]
pub struct FakeResponders {
    addr: SocketAddr,
}

impl FakeResponders {
    /// Listens on the loopback interface and answers each probe with each of the `matches` (`(name, hardware, xaddr)`).
    pub async fn start(matches: Vec<(String, String, String)>) -> Self {
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = socket.local_addr().unwrap();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 8192];
            while let Ok((n, from)) = socket.recv_from(&mut buf).await {
                if !String::from_utf8_lossy(&buf[..n]).contains("Probe") {
                    continue;
                }
                for (name, hw, xaddr) in &matches {
                    let reply = format!(
                        r#"<?xml version="1.0"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"><s:Body><d:ProbeMatches><d:ProbeMatch><d:Types>dn:NetworkVideoTransmitter</d:Types><d:Scopes>onvif://www.onvif.org/name/{name} onvif://www.onvif.org/hardware/{hw}</d:Scopes><d:XAddrs>{xaddr}</d:XAddrs></d:ProbeMatch></d:ProbeMatches></s:Body></s:Envelope>"#
                    );
                    let _ = socket.send_to(reply.as_bytes(), from).await;
                }
            }
        });
        Self { addr }
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}
