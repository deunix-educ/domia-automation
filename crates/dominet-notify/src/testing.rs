// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fake SMTP and Free Mobile servers on the loopback interface (tests, demonstration): they record what they receive and nothing
//! leaves the machine. The fake SMTP does not offer `STARTTLS`: a connection that requires it must therefore fail without sending anything.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// A message received by the fake SMTP.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReceivedMail {
    pub from: String,
    pub to: Vec<String>,
    /// Credentials presented by `AUTH`, if there were any.
    pub auth: Option<(String, String)>,
    /// Headers and body as received (without the final dot).
    pub data: String,
}

impl ReceivedMail {
    /// Values of the `name` header (case-insensitive), one per header line.
    #[must_use]
    pub fn header(&self, name: &str) -> Vec<String> {
        let head = self.data.split("\r\n\r\n").next().unwrap_or("");
        head.lines()
            .filter_map(|l| {
                let (k, v) = l.split_once(':')?;
                k.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
            })
            .collect()
    }

    /// Body (after the empty line).
    #[must_use]
    pub fn body(&self) -> &str {
        self.data.split_once("\r\n\r\n").map_or("", |(_, b)| b)
    }
}

/// Behaviour of the fake SMTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmtpBehavior {
    Accept,
    /// Refuses every recipient (550).
    RejectRecipients,
    /// Refuses authentication (535).
    RejectAuth,
}

#[derive(Debug)]
struct SmtpState {
    mails: Mutex<Vec<ReceivedMail>>,
    behavior: Mutex<SmtpBehavior>,
}

#[derive(Debug, Clone)]
pub struct FakeSmtp {
    addr: SocketAddr,
    state: Arc<SmtpState>,
}

fn b64_decode(s: &str) -> Vec<u8> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let (mut out, mut acc, mut bits) = (vec![], 0u32, 0u32);
    for c in s.bytes().filter(|c| *c != b'=') {
        let Some(v) = T.iter().position(|t| *t == c) else {
            continue;
        };
        acc = (acc << 6) | u32::try_from(v).unwrap_or(0);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).unwrap_or(0));
        }
    }
    out
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

impl FakeSmtp {
    /// Starts on a free port of `127.0.0.1`.
    ///
    /// # Panics
    /// If the port cannot be opened.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let state = Arc::new(SmtpState {
            mails: Mutex::new(vec![]),
            behavior: Mutex::new(SmtpBehavior::Accept),
        });
        let st = state.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let st = st.clone();
                tokio::spawn(async move {
                    let _ = serve_smtp(stream, st).await;
                });
            }
        });
        Self { addr, state }
    }

    #[must_use]
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    pub fn set_behavior(&self, b: SmtpBehavior) {
        *self.state.behavior.lock().expect("lock") = b;
    }

    #[must_use]
    pub fn mails(&self) -> Vec<ReceivedMail> {
        self.state.mails.lock().expect("lock").clone()
    }
}

#[allow(clippy::too_many_lines)]
async fn serve_smtp(stream: TcpStream, st: Arc<SmtpState>) -> std::io::Result<()> {
    let (r, mut w) = stream.into_split();
    let mut r = BufReader::new(r);
    w.write_all(b"220 fake.local ESMTP\r\n").await?;
    let mut mail = ReceivedMail::default();
    let mut line = String::new();
    loop {
        line.clear();
        if r.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let cmd = line.trim_end().to_string();
        let upper = cmd.to_ascii_uppercase();
        let behavior = *st.behavior.lock().expect("lock");
        if upper.starts_with("EHLO") || upper.starts_with("HELO") {
            w.write_all(b"250-fake.local\r\n250-AUTH PLAIN LOGIN\r\n250 8BITMIME\r\n")
                .await?;
        } else if upper.starts_with("AUTH PLAIN") {
            let blob = text(&b64_decode(cmd[10..].trim()));
            let mut parts = blob.split('\0').skip(1);
            let (user, pass) = (
                parts.next().unwrap_or("").to_string(),
                parts.next().unwrap_or("").to_string(),
            );
            if behavior == SmtpBehavior::RejectAuth {
                w.write_all(b"535 5.7.8 refused\r\n").await?;
            } else {
                mail.auth = Some((user, pass));
                w.write_all(b"235 2.7.0 ok\r\n").await?;
            }
        } else if upper.starts_with("AUTH LOGIN") {
            w.write_all(b"334 VXNlcm5hbWU6\r\n").await?;
            line.clear();
            r.read_line(&mut line).await?;
            let user = text(&b64_decode(line.trim()));
            w.write_all(b"334 UGFzc3dvcmQ6\r\n").await?;
            line.clear();
            r.read_line(&mut line).await?;
            let pass = text(&b64_decode(line.trim()));
            if behavior == SmtpBehavior::RejectAuth {
                w.write_all(b"535 5.7.8 refused\r\n").await?;
            } else {
                mail.auth = Some((user, pass));
                w.write_all(b"235 2.7.0 ok\r\n").await?;
            }
        } else if upper.starts_with("MAIL FROM:") {
            mail.from = cmd[10..]
                .trim()
                .trim_start_matches('<')
                .split('>')
                .next()
                .unwrap_or("")
                .to_string();
            w.write_all(b"250 ok\r\n").await?;
        } else if upper.starts_with("RCPT TO:") {
            if behavior == SmtpBehavior::RejectRecipients {
                w.write_all(b"550 5.1.1 no such user\r\n").await?;
            } else {
                mail.to.push(
                    cmd[8..]
                        .trim()
                        .trim_start_matches('<')
                        .split('>')
                        .next()
                        .unwrap_or("")
                        .to_string(),
                );
                w.write_all(b"250 ok\r\n").await?;
            }
        } else if upper == "DATA" {
            w.write_all(b"354 go ahead\r\n").await?;
            let mut data = String::new();
            loop {
                line.clear();
                if r.read_line(&mut line).await? == 0 {
                    return Ok(());
                }
                if line == ".\r\n" {
                    break;
                }
                data.push_str(
                    line.strip_prefix('.')
                        .filter(|_| line.starts_with(".."))
                        .unwrap_or(&line),
                );
            }
            mail.data = data.trim_end_matches("\r\n").to_string();
            st.mails.lock().expect("lock").push(mail.clone());
            mail = ReceivedMail {
                auth: mail.auth,
                ..ReceivedMail::default()
            };
            w.write_all(b"250 queued\r\n").await?;
        } else if upper == "QUIT" {
            w.write_all(b"221 bye\r\n").await?;
            return Ok(());
        } else if upper == "RSET" || upper == "NOOP" {
            w.write_all(b"250 ok\r\n").await?;
        } else {
            w.write_all(b"502 not implemented\r\n").await?;
        }
    }
}

// ------------------------------------------------------------------ Free Mobile

/// A request received by the fake Free Mobile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FreeRequest {
    pub user: String,
    pub pass: String,
    pub msg: String,
}

#[derive(Debug)]
struct FreeState {
    requests: Mutex<Vec<FreeRequest>>,
    status: Mutex<u16>,
}

#[derive(Debug, Clone)]
pub struct FakeFreeMobile {
    addr: SocketAddr,
    state: Arc<FreeState>,
}

impl FakeFreeMobile {
    /// # Panics
    /// If the port cannot be opened.
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let state = Arc::new(FreeState {
            requests: Mutex::new(vec![]),
            status: Mutex::new(200),
        });
        let st = state.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let st = st.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 8192];
                    let mut n = 0;
                    while n < buf.len() {
                        let Ok(k) = stream.read(&mut buf[n..]).await else {
                            return;
                        };
                        if k == 0 {
                            break;
                        }
                        n += k;
                        if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let head = text(&buf[..n]);
                    let target = head.split_whitespace().nth(1).unwrap_or("/");
                    if let Ok(url) = reqwest::Url::parse(&format!("http://fake{target}")) {
                        let get = |k: &str| {
                            url.query_pairs()
                                .find(|(n, _)| n == k)
                                .map(|(_, v)| v.into_owned())
                                .unwrap_or_default()
                        };
                        st.requests.lock().expect("lock").push(FreeRequest {
                            user: get("user"),
                            pass: get("pass"),
                            msg: get("msg"),
                        });
                    }
                    let status = *st.status.lock().expect("lock");
                    let reply = format!(
                        "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                    );
                    let _ = stream.write_all(reply.as_bytes()).await;
                });
            }
        });
        Self { addr, state }
    }

    /// Base address to give to `FreeMobile::new` (`SMS_FREE_URL`).
    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}/sendmsg", self.addr)
    }

    /// HTTP status returned to the following requests (200 by default).
    pub fn set_status(&self, status: u16) {
        *self.state.status.lock().expect("lock") = status;
    }

    #[must_use]
    pub fn requests(&self) -> Vec<FreeRequest> {
        self.state.requests.lock().expect("lock").clone()
    }
}
