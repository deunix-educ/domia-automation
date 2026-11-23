// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fake ONVIF server: replays the responses of `fixtures/onvif/<family>/` and **checks the WS-Security authentication** like a
//! real camera (digest, user, `Created` close to the camera's clock). No network contact beyond `127.0.0.1`.

use std::fmt::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use chrono::{DateTime, Datelike, Timelike, Utc};
use sha1::{Digest, Sha1};

/// Camera family whose responses are replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Jooan Q3R / A2R: Device, Media, PTZ, Imaging.
    Thingino,
    /// Ascam / Vatilon: `/onvif/Media` paths.
    Vatilon,
    /// Xiaomi under Thingino: Device and Media only.
    Xiaomi,
    /// "NVT IPC": stream URL with the password in clear.
    Nvt,
}

macro_rules! fixtures {
    ($($dir:literal => $family:ident),+) => {
        fn fixture(family: Family, op: &str) -> Option<&'static str> {
            match (family, op) {
                $(
                    (Family::$family, "GetSystemDateAndTime") => Some(include_str!(concat!("../../fixtures/onvif/", $dir, "/GetSystemDateAndTime.xml"))),
                    (Family::$family, "GetDeviceInformation") => Some(include_str!(concat!("../../fixtures/onvif/", $dir, "/GetDeviceInformation.xml"))),
                    (Family::$family, "GetCapabilities") => Some(include_str!(concat!("../../fixtures/onvif/", $dir, "/GetCapabilities.xml"))),
                    (Family::$family, "GetProfiles") => Some(include_str!(concat!("../../fixtures/onvif/", $dir, "/GetProfiles.xml"))),
                    (Family::$family, "GetStreamUri") => Some(include_str!(concat!("../../fixtures/onvif/", $dir, "/GetStreamUri.xml"))),
                )+
                _ => None,
            }
        }
    };
}
fixtures!("thingino" => Thingino, "vatilon" => Vatilon, "xiaomi" => Xiaomi, "nvt" => Nvt);

struct Inner {
    family: Family,
    user: String,
    password: String,
    /// Advance (or delay) of the camera's clock, in seconds.
    clock_offset_s: i64,
    /// Host announced in the service addresses (camera behind an address translation).
    announce_host: Option<String>,
    /// Response to a refused authentication: `true` = HTTP 401, `false` = SOAP `Fault` (HTTP 400, like Thingino).
    http_401: bool,
    calls: Vec<String>,
    port: u16,
    ptz: PtzState,
    imaging: Vec<(String, String)>,
}

/// PTZ state of the simulated camera.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PtzState {
    pub position: (f64, f64, f64),
    /// Last speed ordered (`ContinuousMove`): `None` after a `Stop` or at rest.
    pub velocity: Option<(f64, f64, f64)>,
    /// `(token, name, position)`.
    pub presets: Vec<(String, String, (f64, f64, f64))>,
    /// Number of `GetStatus` that will still answer `MOVING` (after a `GotoPreset` or an `AbsoluteMove`).
    pub moving_polls: u32,
    /// Readable log of the orders received (`ContinuousMove 0.5 0 0`, `Stop`, `GotoPreset preset_1`...).
    pub log: Vec<String>,
    next_preset: u32,
}

#[derive(Clone)]
pub struct FakeOnvif {
    addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for FakeOnvif {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeOnvif({})", self.addr)
    }
}

impl FakeOnvif {
    pub async fn start(family: Family, user: &str, password: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let inner = Arc::new(Mutex::new(Inner {
            family,
            user: user.into(),
            password: password.into(),
            clock_offset_s: 0,
            announce_host: None,
            http_401: false,
            calls: vec![],
            port: addr.port(),
            ptz: PtzState::default(),
            imaging: vec![
                ("BacklightCompensation/Mode".into(), "OFF".into()),
                ("Brightness".into(), "35".into()),
                ("IrCutFilter".into(), "AUTO".into()),
                ("Sharpness".into(), "50".into()),
            ],
        }));
        let app = Router::new().fallback(handle).with_state(inner.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, inner }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// The camera's clock advances by `seconds` (negative: delay).
    pub fn set_clock_offset(&self, seconds: i64) {
        self.inner.lock().unwrap().clock_offset_s = seconds;
    }

    pub fn announce_host(&self, host: &str) {
        self.inner.lock().unwrap().announce_host = Some(host.into());
    }

    pub fn refuse_with_http_401(&self) {
        self.inner.lock().unwrap().http_401 = true;
    }

    /// Current PTZ state of the simulated camera.
    #[must_use]
    pub fn ptz_state(&self) -> PtzState {
        self.inner.lock().unwrap().ptz.clone()
    }

    /// Current image settings (`path`, `value`), in order.
    #[must_use]
    pub fn imaging(&self) -> Vec<(String, String)> {
        self.inner.lock().unwrap().imaging.clone()
    }

    /// Value of an image setting (`IrCutFilter`, `Brightness`...).
    #[must_use]
    pub fn imaging_value(&self, name: &str) -> Option<String> {
        self.imaging()
            .into_iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v)
    }

    /// Calls received, in order: `Operation` (or `Operation!` if the authentication was refused).
    #[must_use]
    pub fn calls(&self) -> Vec<String> {
        self.inner.lock().unwrap().calls.clone()
    }
}

fn soap(status: StatusCode, body: String) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/soap+xml; charset=utf-8")],
        body,
    )
        .into_response()
}

fn fault(status: StatusCode, reason: &str) -> Response {
    soap(
        status,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><SOAP-ENV:Envelope xmlns:SOAP-ENV="http://www.w3.org/2003/05/soap-envelope"><SOAP-ENV:Body><SOAP-ENV:Fault><SOAP-ENV:Code><SOAP-ENV:Value>SOAP-ENV:Sender</SOAP-ENV:Value></SOAP-ENV:Code><SOAP-ENV:Reason><SOAP-ENV:Text xml:lang="en">{reason}</SOAP-ENV:Text></SOAP-ENV:Reason></SOAP-ENV:Fault></SOAP-ENV:Body></SOAP-ENV:Envelope>"#
        ),
    )
}

fn element_text(doc: &roxmltree::Document, local: &str) -> Option<String> {
    doc.descendants()
        .find(|d| d.is_element() && d.tag_name().name() == local)
        .and_then(|e| e.text())
        .map(|t| t.trim().to_string())
}

/// Is the WS-Security digest valid for this camera?
fn authenticated(doc: &roxmltree::Document, inner: &Inner, now: DateTime<Utc>) -> bool {
    let (Some(user), Some(digest), Some(nonce), Some(created)) = (
        element_text(doc, "Username"),
        element_text(doc, "Password"),
        element_text(doc, "Nonce"),
        element_text(doc, "Created"),
    ) else {
        return false;
    };
    let Ok(nonce) = B64.decode(nonce) else {
        return false;
    };
    let Ok(created_at) = DateTime::parse_from_rfc3339(&created) else {
        return false;
    };
    let mut h = Sha1::new();
    h.update(&nonce);
    h.update(created.as_bytes());
    h.update(inner.password.as_bytes());
    user == inner.user
        && B64.encode(h.finalize()) == digest
        && (created_at.with_timezone(&Utc) - now).num_seconds().abs() <= 30
}

async fn handle(
    State(state): State<Arc<Mutex<Inner>>>,
    _headers: HeaderMap,
    body: Bytes,
) -> Response {
    let text = String::from_utf8_lossy(&body).into_owned();
    let Ok(doc) = roxmltree::Document::parse(&text) else {
        return fault(StatusCode::BAD_REQUEST, "malformed request");
    };
    let Some(op) = doc
        .descendants()
        .find(|d| d.is_element() && d.tag_name().name() == "Body")
        .and_then(|b| b.children().find(roxmltree::Node::is_element))
        .map(|e| e.tag_name().name().to_string())
    else {
        return fault(StatusCode::BAD_REQUEST, "empty body");
    };
    let mut g = state.lock().unwrap();
    let now = Utc::now() + chrono::Duration::seconds(g.clock_offset_s);
    if op != "GetSystemDateAndTime" && !authenticated(&doc, &g, now) {
        g.calls.push(format!("{op}!"));
        return if g.http_401 {
            (StatusCode::UNAUTHORIZED, "").into_response()
        } else {
            fault(StatusCode::BAD_REQUEST, "Sender not Authorized")
        };
    }
    g.calls.push(op.clone());
    if let Some(body) = dynamic(&mut g, &op, &doc) {
        return soap(StatusCode::OK, body);
    }
    let Some(template) = fixture(g.family, &op) else {
        return fault(StatusCode::BAD_REQUEST, "Action not supported");
    };
    let host_only = "127.0.0.1".to_string();
    let announced = g.announce_host.clone().unwrap_or_else(|| host_only.clone());
    let out = template
        .replace("{{HOST_ONLY}}", &host_only)
        .replace("{{HOST}}", &format!("{announced}:{}", g.port))
        .replace("{{PASSWORD}}", &g.password)
        .replace("{{YEAR}}", &now.year().to_string())
        .replace("{{MONTH}}", &now.month().to_string())
        .replace("{{DAY}}", &now.day().to_string())
        .replace("{{HOUR}}", &now.hour().to_string())
        .replace("{{MINUTE}}", &now.minute().to_string())
        .replace("{{SECOND}}", &now.second().to_string());
    soap(StatusCode::OK, out)
}

// ------------------------------------------------------------------ PTZ and imaging (in-memory state)

fn envelope(family: Family, body: &str) -> String {
    let p = if family == Family::Nvt {
        "env"
    } else {
        "SOAP-ENV"
    };
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><{p}:Envelope xmlns:{p}="http://www.w3.org/2003/05/soap-envelope" xmlns:tt="http://www.onvif.org/ver10/schema" xmlns:tptz="http://www.onvif.org/ver20/ptz/wsdl" xmlns:timg="http://www.onvif.org/ver20/imaging/wsdl"><{p}:Body>{body}</{p}:Body></{p}:Envelope>"#
    )
}

fn child_text(doc: &roxmltree::Document, parent: &str, name: &str) -> Option<String> {
    let p = doc
        .descendants()
        .find(|d| d.is_element() && d.tag_name().name() == parent)?;
    p.descendants()
        .find(|d| d.is_element() && d.tag_name().name() == name)
        .and_then(|e| e.text())
        .map(|t| t.trim().to_string())
}

fn axes(doc: &roxmltree::Document, parent: &str) -> (f64, f64, f64) {
    let attr = |tag: &str, a: &str| {
        doc.descendants()
            .find(|d| d.is_element() && d.tag_name().name() == parent)
            .and_then(|p| {
                p.descendants()
                    .find(|d| d.is_element() && d.tag_name().name() == tag)
            })
            .and_then(|n| n.attribute(a))
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0)
    };
    (
        attr("PanTilt", "x"),
        attr("PanTilt", "y"),
        attr("Zoom", "x"),
    )
}

/// The PTZ and imaging operations, which depend on the camera's state.
#[allow(clippy::too_many_lines, reason = "une operation ONVIF par branche")]
fn dynamic(g: &mut Inner, op: &str, doc: &roxmltree::Document) -> Option<String> {
    let family = g.family;
    let body = match op {
        "ContinuousMove" => {
            let v = axes(doc, "Velocity");
            g.ptz.velocity = Some(v);
            g.ptz
                .log
                .push(format!("ContinuousMove {} {} {}", v.0, v.1, v.2));
            "<tptz:ContinuousMoveResponse/>".to_string()
        }
        "Stop" => {
            g.ptz.velocity = None;
            g.ptz.log.push("Stop".into());
            "<tptz:StopResponse/>".to_string()
        }
        "GetStatus" => {
            let moving = g.ptz.moving_polls > 0 || g.ptz.velocity.is_some();
            g.ptz.moving_polls = g.ptz.moving_polls.saturating_sub(1);
            // The NVT camera always answers 0, 0 (no readable position, ADR 0008).
            let (pan, tilt, zoom) = if family == Family::Nvt {
                (0.0, 0.0, 0.0)
            } else {
                g.ptz.position
            };
            let state = if moving { "MOVING" } else { "IDLE" };
            format!(
                r#"<tptz:GetStatusResponse><tptz:PTZStatus><tt:Position><tt:PanTilt x="{pan}" y="{tilt}" space="http://www.onvif.org/ver10/tptz/PanTiltSpaces/SphericalPositionSpace"/><tt:Zoom x="{zoom}"/></tt:Position><tt:MoveStatus><tt:PanTilt>{state}</tt:PanTilt><tt:Zoom>{state}</tt:Zoom></tt:MoveStatus></tptz:PTZStatus></tptz:GetStatusResponse>"#
            )
        }
        "GetPresets" => {
            let mut presets = String::new();
            for (t, n, _) in &g.ptz.presets {
                let n = n
                    .replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;");
                let _ = write!(
                    presets,
                    r#"<tptz:Preset token="{t}"><tt:Name>{n}</tt:Name></tptz:Preset>"#
                );
            }
            format!("<tptz:GetPresetsResponse>{presets}</tptz:GetPresetsResponse>")
        }
        "SetPreset" => {
            let name = child_text(doc, "SetPreset", "PresetName").unwrap_or_default();
            let at = g.ptz.position;
            let token = if let Some(t) = child_text(doc, "SetPreset", "PresetToken") {
                // Given slot: it is overwritten.
                g.ptz.presets.retain(|(p, _, _)| *p != t);
                t
            } else {
                g.ptz.next_preset += 1;
                format!("preset_{}", g.ptz.next_preset)
            };
            g.ptz.presets.push((token.clone(), name.clone(), at));
            g.ptz.log.push(format!("SetPreset {name}"));
            format!(
                "<tptz:SetPresetResponse><tptz:PresetToken>{token}</tptz:PresetToken></tptz:SetPresetResponse>"
            )
        }
        "RemovePreset" => {
            let token = child_text(doc, "RemovePreset", "PresetToken").unwrap_or_default();
            g.ptz.presets.retain(|(t, _, _)| *t != token);
            g.ptz.log.push(format!("RemovePreset {token}"));
            "<tptz:RemovePresetResponse/>".to_string()
        }
        "GotoPreset" => {
            let token = child_text(doc, "GotoPreset", "PresetToken").unwrap_or_default();
            g.ptz.log.push(format!("GotoPreset {token}"));
            if let Some((_, _, at)) = g.ptz.presets.iter().find(|(t, _, _)| *t == token) {
                g.ptz.position = *at;
                g.ptz.moving_polls = 2;
            }
            "<tptz:GotoPresetResponse/>".to_string()
        }
        "GotoHomePosition" => {
            g.ptz.log.push("GotoHomePosition".to_string());
            g.ptz.position = (0.0, 0.0, 0.0);
            g.ptz.moving_polls = 2;
            "<tptz:GotoHomePositionResponse/>".to_string()
        }
        "AbsoluteMove" => {
            let at = axes(doc, "Position");
            g.ptz.position = at;
            g.ptz.moving_polls = 2;
            let space = doc
                .descendants()
                .find(|d| {
                    d.is_element()
                        && d.tag_name().name() == "PanTilt"
                        && d.attribute("space").is_some()
                })
                .and_then(|n| n.attribute("space"))
                .unwrap_or("");
            g.ptz
                .log
                .push(format!("AbsoluteMove {} {} {} {space}", at.0, at.1, at.2));
            "<tptz:AbsoluteMoveResponse/>".to_string()
        }
        "GetImagingSettings" => {
            let mut inner = String::new();
            for (path, value) in &g.imaging {
                match path.split_once('/') {
                    Some((parent, leaf)) => {
                        let _ = write!(
                            inner,
                            "<tt:{parent}><tt:{leaf}>{value}</tt:{leaf}></tt:{parent}>"
                        );
                    }
                    None => {
                        let _ = write!(inner, "<tt:{path}>{value}</tt:{path}>");
                    }
                }
            }
            format!(
                "<timg:GetImagingSettingsResponse><timg:ImagingSettings>{inner}</timg:ImagingSettings></timg:GetImagingSettingsResponse>"
            )
        }
        "SetImagingSettings" => {
            // We only keep what the client sent back: a partial object would reset the other settings to zero, and the test sees it.
            let settings = doc
                .descendants()
                .find(|d| d.is_element() && d.tag_name().name() == "ImagingSettings")?;
            let mut out = vec![];
            for child in settings.children().filter(roxmltree::Node::is_element) {
                let name = child.tag_name().name().to_string();
                let leaves: Vec<_> = child
                    .children()
                    .filter(roxmltree::Node::is_element)
                    .collect();
                if leaves.is_empty() {
                    out.push((name, child.text().unwrap_or("").trim().to_string()));
                } else {
                    for leaf in leaves {
                        out.push((
                            format!("{name}/{}", leaf.tag_name().name()),
                            leaf.text().unwrap_or("").trim().to_string(),
                        ));
                    }
                }
            }
            g.imaging = out;
            "<timg:SetImagingSettingsResponse/>".to_string()
        }
        _ => return None,
    };
    Some(envelope(family, &body))
}
