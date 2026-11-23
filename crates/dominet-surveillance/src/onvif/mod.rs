// SPDX-License-Identifier: AGPL-3.0-or-later
//! Minimal ONVIF client (ADR 0008 and 0045): hand-written SOAP, responses read with `roxmltree` (ignoring namespace prefixes),
//! WS-Security `PasswordDigest` authentication with correction of the camera's clock.
//!
//! Rules: no connection reuse (Thingino's ONVIF server closes after each response); the service addresses
//! come from `GetCapabilities`, **but the host is always that of the registered camera** (a camera behind an
//! address translation sometimes announces its own, and a response must not be able to send us elsewhere); no
//! password, neither in the errors, nor in the logs, nor in `Debug`.

pub mod discovery;
pub mod imaging;
mod mask;
pub mod ptz;

use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Utc};
use roxmltree::{Document, Node};
use sha1::{Digest, Sha1};

pub use mask::mask_secrets;

const NS_DEVICE: &str = "http://www.onvif.org/ver10/device/wsdl";
const NS_MEDIA: &str = "http://www.onvif.org/ver10/media/wsdl";
const NS_PTZ: &str = "http://www.onvif.org/ver20/ptz/wsdl";
const NS_IMG: &str = "http://www.onvif.org/ver20/imaging/wsdl";
const NS_SCHEMA: &str = "http://www.onvif.org/ver10/schema";
const CALL_TIMEOUT: Duration = Duration::from_secs(6);

#[derive(Debug, thiserror::Error)]
pub enum OnvifError {
    #[error("camera injoignable : {0}")]
    Unreachable(String),
    /// Credentials refused (HTTP 401, or `NotAuthorized` in the `Fault`).
    #[error("identifiants ONVIF refuses")]
    NotAuthorized,
    #[error("la camera a repondu HTTP {0}")]
    Http(u16),
    #[error("erreur SOAP de la camera : {0}")]
    Fault(String),
    #[error("reponse ONVIF illisible : {0}")]
    BadResponse(&'static str),
    #[error("la camera n'a pas ce service ONVIF : {0}")]
    NoService(&'static str),
}

/// ONVIF identifier and password; `Debug` does not show the password.
#[derive(Clone)]
pub struct Credentials {
    pub user: String,
    pub pass: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("user", &self.user)
            .field("pass", &"***")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DeviceInfo {
    pub manufacturer: String,
    pub model: String,
    pub firmware: String,
    pub serial: String,
}

/// Service addresses (those of `GetCapabilities`, with the host of the registered camera).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Services {
    pub media: Option<String>,
    pub ptz: Option<String>,
    pub imaging: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaProfile {
    pub token: String,
    pub name: String,
    pub has_ptz: bool,
    pub encoding: String,
    pub width: u32,
    pub height: u32,
    /// Token of the video source (useful for imaging).
    pub source_token: Option<String>,
}

/// Stream URL announced by the camera. It may contain a clear-text password (NVT camera): `Debug` and [`Self::masked`] hide
/// it, [`Self::expose`] is reserved for network use.
#[derive(Clone, PartialEq, Eq)]
pub struct StreamUri {
    raw: String,
    password: String,
}

impl StreamUri {
    #[must_use]
    pub fn masked(&self) -> String {
        mask_secrets(&self.raw, &[&self.password])
    }

    /// The URL as the camera gave it. Never log or display.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.raw
    }
}

impl fmt::Debug for StreamUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StreamUri({})", self.masked())
    }
}

/// What the ONVIF synchronisation learns about a camera.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inspection {
    pub info: DeviceInfo,
    /// First profile (like the reference: `profiles[0]`).
    pub main_token: String,
    pub ptz: bool,
    pub profiles: Vec<MediaProfile>,
    /// Stream URL of the main profile, **masked**.
    pub stream_uri: String,
}

// ---------------------------------------------------------------- XML

fn find<'a, 'i>(n: Node<'a, 'i>, local: &str) -> Option<Node<'a, 'i>> {
    n.descendants()
        .find(|d| d.is_element() && d.tag_name().name() == local)
}

fn text(n: Node, local: &str) -> Option<String> {
    find(n, local)
        .and_then(|e| e.text())
        .map(|t| t.trim().to_string())
}

fn parse(xml: &str) -> Result<Document<'_>, OnvifError> {
    Document::parse(xml).map_err(|_| OnvifError::BadResponse("invalid XML"))
}

/// Host and port of the camera's address, with brackets for IPv6.
fn authority(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

/// Path and port of an announced service address; the host is replaced by the camera's.
fn rebase(xaddr: &str, host: &str, default_port: u16) -> Option<String> {
    let rest = xaddr.strip_prefix("http://")?;
    let (auth, path) = rest.split_once('/').map_or((rest, ""), |(a, p)| (a, p));
    let port = auth
        .rsplit_once(':')
        .filter(|(_, p)| p.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|(_, p)| p.parse().ok())
        .unwrap_or(default_port);
    Some(format!("http://{}/{path}", authority(host, port)))
}

// ---------------------------------------------------------------- client

#[derive(Debug)]
pub struct OnvifClient {
    http: reqwest::Client,
    host: String,
    port: u16,
    creds: Option<Credentials>,
    /// Offset (seconds) of the camera's clock relative to ours.
    offset_s: AtomicI64,
    clock_known: std::sync::atomic::AtomicBool,
}

impl OnvifClient {
    /// # Errors
    /// The HTTP client cannot be built.
    pub fn new(host: &str, port: u16, creds: Option<Credentials>) -> Result<Self, OnvifError> {
        let http = reqwest::Client::builder()
            // Thingino closes the connection after each response.
            .pool_max_idle_per_host(0)
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|e| OnvifError::Unreachable(e.to_string()))?;
        Ok(Self {
            http,
            host: host.to_string(),
            port,
            creds,
            offset_s: AtomicI64::new(0),
            clock_known: std::sync::atomic::AtomicBool::new(false),
        })
    }

    #[must_use]
    pub fn device_url(&self) -> String {
        format!(
            "http://{}/onvif/device_service",
            authority(&self.host, self.port)
        )
    }

    fn security_header(&self) -> String {
        let Some(c) = &self.creds else {
            return String::new();
        };
        let mut nonce = [0u8; 16];
        getrandom::fill(&mut nonce).expect("system random generator");
        let created = (Utc::now()
            + chrono::Duration::seconds(self.offset_s.load(Ordering::Relaxed)))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
        let mut h = Sha1::new();
        h.update(nonce);
        h.update(created.as_bytes());
        h.update(c.pass.as_bytes());
        let digest = B64.encode(h.finalize());
        format!(
            r#"<s:Header><wsse:Security s:mustUnderstand="1" xmlns:wsse="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd" xmlns:wsu="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd"><wsse:UsernameToken><wsse:Username>{}</wsse:Username><wsse:Password Type="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest">{digest}</wsse:Password><wsse:Nonce EncodingType="http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary">{}</wsse:Nonce><wsu:Created>{created}</wsu:Created></wsse:UsernameToken></wsse:Security></s:Header>"#,
            xml_escape(&c.user),
            B64.encode(nonce)
        )
    }

    /// A SOAP call; returns the response body. `authenticated`: attach the WS-Security header.
    async fn call(&self, url: &str, authenticated: bool, body: &str) -> Result<String, OnvifError> {
        let header = if authenticated {
            self.security_header()
        } else {
            String::new()
        };
        let envelope = format!(
            r#"<?xml version="1.0" encoding="utf-8"?><s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope" xmlns:tds="{NS_DEVICE}" xmlns:trt="{NS_MEDIA}" xmlns:tptz="{NS_PTZ}" xmlns:timg="{NS_IMG}" xmlns:tt="{NS_SCHEMA}">{header}<s:Body>{body}</s:Body></s:Envelope>"#
        );
        let resp = self
            .http
            .post(url)
            .header("Content-Type", "application/soap+xml; charset=utf-8")
            .body(envelope)
            .send()
            .await
            .map_err(|e| OnvifError::Unreachable(e.without_url().to_string()))?;
        let status = resp.status();
        let xml = resp
            .text()
            .await
            .map_err(|e| OnvifError::Unreachable(e.without_url().to_string()))?;
        if let Ok(doc) = Document::parse(&xml)
            && let Some(fault) = find(doc.root(), "Fault")
        {
            let reason = text(fault, "Text")
                .or_else(|| text(fault, "Value"))
                .unwrap_or_default();
            return Err(if reason.to_ascii_lowercase().contains("authorized") {
                OnvifError::NotAuthorized
            } else {
                OnvifError::Fault(mask_secrets(&reason, &[]))
            });
        }
        match status.as_u16() {
            200..=299 => Ok(xml),
            401 => Err(OnvifError::NotAuthorized),
            code => Err(OnvifError::Http(code)),
        }
    }

    /// UTC time of the camera (without authentication); also sets the offset used for `Created`.
    ///
    /// # Errors
    /// Camera unreachable or unreadable response.
    pub async fn sync_clock(&self) -> Result<DateTime<Utc>, OnvifError> {
        let xml = self
            .call(&self.device_url(), false, "<tds:GetSystemDateAndTime/>")
            .await?;
        let doc = parse(&xml)?;
        let utc =
            find(doc.root(), "UTCDateTime").ok_or(OnvifError::BadResponse("no UTCDateTime"))?;
        let n = |l: &str| text(utc, l).and_then(|v| v.parse::<u32>().ok());
        let camera_time = (|| {
            chrono::NaiveDate::from_ymd_opt(
                i32::try_from(n("Year")?).ok()?,
                n("Month")?,
                n("Day")?,
            )?
            .and_hms_opt(n("Hour")?, n("Minute")?, n("Second")?)
        })()
        .ok_or(OnvifError::BadResponse("invalid date"))?
        .and_utc();
        self.offset_s
            .store((camera_time - Utc::now()).num_seconds(), Ordering::Relaxed);
        self.clock_known.store(true, Ordering::Relaxed);
        Ok(camera_time)
    }

    async fn ensure_clock(&self) {
        if !self.clock_known.load(Ordering::Relaxed) {
            // A camera without a readable clock is tried anyway with ours.
            let _ = self.sync_clock().await;
        }
    }

    async fn authed(&self, url: &str, body: &str) -> Result<String, OnvifError> {
        self.ensure_clock().await;
        self.call(url, true, body).await
    }

    /// # Errors
    /// Credentials refused, camera unreachable or unreadable response.
    pub async fn device_information(&self) -> Result<DeviceInfo, OnvifError> {
        let xml = self
            .authed(&self.device_url(), "<tds:GetDeviceInformation/>")
            .await?;
        let doc = parse(&xml)?;
        let r = doc.root();
        Ok(DeviceInfo {
            manufacturer: text(r, "Manufacturer").unwrap_or_default(),
            model: text(r, "Model").unwrap_or_default(),
            firmware: text(r, "FirmwareVersion").unwrap_or_default(),
            serial: text(r, "SerialNumber").unwrap_or_default(),
        })
    }

    /// # Errors
    /// Credentials refused, camera unreachable or unreadable response.
    pub async fn services(&self) -> Result<Services, OnvifError> {
        let xml = self
            .authed(
                &self.device_url(),
                "<tds:GetCapabilities><tds:Category>All</tds:Category></tds:GetCapabilities>",
            )
            .await?;
        let doc = parse(&xml)?;
        let xaddr = |service: &str| {
            find(doc.root(), service)
                .and_then(|n| text(n, "XAddr"))
                .and_then(|x| rebase(&x, &self.host, self.port))
        };
        Ok(Services {
            media: xaddr("Media"),
            ptz: xaddr("PTZ"),
            imaging: xaddr("Imaging"),
        })
    }

    /// # Errors
    /// No media service, credentials refused, camera unreachable or unreadable response.
    pub async fn profiles(&self, media_url: &str) -> Result<Vec<MediaProfile>, OnvifError> {
        let xml = self.authed(media_url, "<trt:GetProfiles/>").await?;
        let doc = parse(&xml)?;
        Ok(doc
            .root()
            .descendants()
            .filter(|d| d.is_element() && d.tag_name().name() == "Profiles")
            .filter_map(|p| {
                Some(MediaProfile {
                    token: p.attribute("token")?.to_string(),
                    name: text(p, "Name").unwrap_or_default(),
                    has_ptz: find(p, "PTZConfiguration").is_some(),
                    encoding: text(p, "Encoding").unwrap_or_default(),
                    width: text(p, "Width").and_then(|v| v.parse().ok()).unwrap_or(0),
                    height: text(p, "Height").and_then(|v| v.parse().ok()).unwrap_or(0),
                    source_token: find(p, "SourceToken")
                        .and_then(|e| e.text())
                        .map(str::to_string),
                })
            })
            .collect())
    }

    /// # Errors
    /// Credentials refused, camera unreachable or unreadable response.
    pub async fn stream_uri(
        &self,
        media_url: &str,
        profile: &str,
    ) -> Result<StreamUri, OnvifError> {
        let body = format!(
            "<trt:GetStreamUri><trt:StreamSetup><tt:Stream>RTP-Unicast</tt:Stream><tt:Transport><tt:Protocol>RTSP</tt:Protocol></tt:Transport></trt:StreamSetup><trt:ProfileToken>{}</trt:ProfileToken></trt:GetStreamUri>",
            xml_escape(profile)
        );
        let xml = self.authed(media_url, &body).await?;
        let doc = parse(&xml)?;
        let raw = text(doc.root(), "Uri").ok_or(OnvifError::BadResponse("no URI"))?;
        Ok(StreamUri {
            raw,
            password: self
                .creds
                .as_ref()
                .map(|c| c.pass.clone())
                .unwrap_or_default(),
        })
    }

    /// What the ONVIF synchronisation learns: identity, first profile, PTZ, masked stream URL.
    ///
    /// # Errors
    /// Credentials refused, camera without a media service, unreachable or unreadable response.
    pub async fn inspect(&self) -> Result<Inspection, OnvifError> {
        let info = self.device_information().await?;
        let services = self.services().await?;
        let media = services
            .media
            .clone()
            .ok_or(OnvifError::NoService("Media"))?;
        let profiles = self.profiles(&media).await?;
        let main = profiles
            .first()
            .ok_or(OnvifError::BadResponse("no profile"))?;
        let stream_uri = self.stream_uri(&media, &main.token).await?.masked();
        Ok(Inspection {
            info,
            main_token: main.token.clone(),
            ptz: services.ptz.is_some(),
            profiles,
            stream_uri,
        })
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Ports on which an ONVIF service is searched (those of the reference, plus the NVT camera's 8899 and TP-Link's 2020).
pub const ONVIF_PORTS: [u16; 5] = [80, 8080, 8899, 2020, 8000];

/// Looks for a camera's ONVIF service: the first port of `ports` that answers `GetSystemDateAndTime` (without authentication).
/// Unlike the reference, a simple open port is not enough.
pub async fn probe_port(host: &str, ports: &[u16]) -> Option<u16> {
    for &port in ports {
        if let Ok(client) = OnvifClient::new(host, port, None)
            && client.sync_clock().await.is_ok()
        {
            return Some(port);
        }
    }
    None
}
