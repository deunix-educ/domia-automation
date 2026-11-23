// SPDX-License-Identifier: AGPL-3.0-or-later
//! Day / night toggle, IR-cut filter and light (R6 lot 5, ADR 0047): four methods behind the same trait, like
//! the reference.
//!
//! | `night_mode_api` | Call |
//! |---|---|
//! | `openipc` | `GET http://<ip>/night/<toggle\|light>` (basic authentication if a web identifier exists) |
//! | `thingino` | `POST http://<ip>:<web port>/x/json-imp.cgi` in JSON, `X-API-Key`; the toggle remembers the state (24 h) |
//! | `onvif` | imaging service: `IrCutFilter` (`DAY` / `NIGHT` / `AUTO`) or brightness |
//! | `goform` | `POST /goform/frm*`, JSON `{Type: 1, Data: ...}`, `X-Digest` authentication (Longse / OEM) |
//!
//! No secret in the errors or in `Debug`.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use md5::{Digest, Md5};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, WWW_AUTHENTICATE};
use serde_json::{Value, json};

use crate::onvif::{Credentials, OnvifClient, OnvifError};

const TIMEOUT: Duration = Duration::from_secs(5);
/// How long a Thingino toggle remembers its last state (like the reference's cache).
const STATE_TTL: Duration = Duration::from_hours(24);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NightAction {
    DayNight,
    IrCut,
    Light,
}

impl NightAction {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DayNight => "daynight",
            Self::IrCut => "ircut",
            Self::Light => "light",
        }
    }
}

impl FromStr for NightAction {
    type Err = NightError;

    fn from_str(s: &str) -> Result<Self, NightError> {
        match s {
            "daynight" => Ok(Self::DayNight),
            "ircut" => Ok(Self::IrCut),
            "light" => Ok(Self::Light),
            other => Err(NightError::UnknownAction(other.chars().take(30).collect())),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NightError {
    #[error("action inconnue : {0}")]
    UnknownAction(String),
    #[error("aucune methode jour/nuit n'est configuree")]
    NotConfigured,
    #[error("la camera est injoignable ou a refuse : {0}")]
    Http(String),
    #[error("identifiants refuses par la camera")]
    Denied,
    #[error("reponse de la camera illisible : {0}")]
    BadResponse(&'static str),
    #[error(transparent)]
    Onvif(#[from] OnvifError),
}

/// What the camera did: the action and, like the reference, the useful values (`mode`, `was`, `value`, `control`, `brightness`, `body`).
#[derive(Debug, Clone, PartialEq)]
pub struct NightOutcome {
    pub action: NightAction,
    pub details: serde_json::Map<String, Value>,
}

impl NightOutcome {
    fn new(action: NightAction, details: &[(&str, Value)]) -> Self {
        Self {
            action,
            details: details
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
        }
    }

    /// The reference's shape: `{"ok": true, "action": ..., ...}`.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("ok".into(), json!(true));
        m.insert("action".into(), json!(self.action.as_str()));
        m.extend(self.details.clone());
        Value::Object(m)
    }
}

pub trait NightControl {
    /// Executes the action (a toggle) on the camera.
    fn run(
        &self,
        action: NightAction,
    ) -> impl Future<Output = Result<NightOutcome, NightError>> + Send;
}

/// What must be known about a camera to command it. The secrets are **in clear** (they have just been decrypted): `Debug` hides them.
#[derive(Clone)]
pub struct NightTarget {
    /// Public identifier of the camera (key of the Thingino toggle's remembered state).
    pub id: String,
    pub host: String,
    pub web_port: u16,
    pub web_user: String,
    pub web_pass: String,
    pub web_api_key: String,
    pub onvif_port: u16,
    pub onvif_user: String,
    pub onvif_pass: String,
}

impl fmt::Debug for NightTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NightTarget")
            .field("id", &self.id)
            .field("host", &self.host)
            .field("secrets", &"***")
            .finish_non_exhaustive()
    }
}

/// Last state sent to each Thingino toggle (the camera does not read it back): shared memory, 24 h.
#[derive(Debug, Clone, Default)]
pub struct NightState(Arc<Mutex<StateMap>>);

/// Last value sent and its time, per (camera, action).
type StateMap = HashMap<(String, NightAction), (Value, Instant)>;

impl NightState {
    fn get(&self, id: &str, action: NightAction) -> Option<Value> {
        let g = self.0.lock().unwrap();
        g.get(&(id.to_string(), action))
            .filter(|(_, at)| at.elapsed() < STATE_TTL)
            .map(|(v, _)| v.clone())
    }

    fn set(&self, id: &str, action: NightAction, value: Value) {
        self.0
            .lock()
            .unwrap()
            .insert((id.to_string(), action), (value, Instant::now()));
    }
}

#[derive(Debug, Clone)]
pub enum NightBackend {
    OpenIpc(NightTarget),
    Thingino(NightTarget, NightState),
    Onvif(NightTarget),
    Goform(NightTarget),
}

impl NightControl for NightBackend {
    async fn run(&self, action: NightAction) -> Result<NightOutcome, NightError> {
        match self {
            Self::OpenIpc(t) => openipc(t, action).await,
            Self::Thingino(t, state) => thingino(t, state, action).await,
            Self::Onvif(t) => onvif(t, action).await,
            Self::Goform(t) => goform(t, action).await,
        }
    }
}

fn http() -> Result<reqwest::Client, NightError> {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| NightError::Http(e.without_url().to_string()))
}

fn http_error(e: reqwest::Error) -> NightError {
    NightError::Http(e.without_url().to_string())
}

fn host_port(host: &str, port: u16) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    }
}

// ------------------------------------------------------------------ OpenIPC

async fn openipc(t: &NightTarget, action: NightAction) -> Result<NightOutcome, NightError> {
    // `/night/ircut` does not activate the motor pin on GK7205V200 boards: the reference sends it to `toggle`, which does the right sequence.
    let path = match action {
        NightAction::DayNight | NightAction::IrCut => "toggle",
        NightAction::Light => "light",
    };
    let url = format!("http://{}/night/{path}", host_port(&t.host, t.web_port));
    let mut req = http()?.get(url);
    if !t.web_user.is_empty() {
        req = req.basic_auth(&t.web_user, Some(&t.web_pass));
    }
    let resp = req.send().await.map_err(http_error)?;
    if resp.status().as_u16() == 401 {
        return Err(NightError::Denied);
    }
    if !resp.status().is_success() {
        return Err(NightError::Http(format!("HTTP {}", resp.status().as_u16())));
    }
    let body: String = resp
        .text()
        .await
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect();
    Ok(NightOutcome::new(action, &[("body", json!(body))]))
}

// ------------------------------------------------------------------ Thingino

async fn thingino(
    t: &NightTarget,
    state: &NightState,
    action: NightAction,
) -> Result<NightOutcome, NightError> {
    // (command, toggle, default value): like the reference's `_THINGINO_CMD`.
    let (cmd, current, next) = match action {
        NightAction::DayNight => {
            let now = state
                .get(&t.id, action)
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "day".into());
            (
                "daynight",
                json!(now),
                json!(if now == "day" { "night" } else { "day" }),
            )
        }
        NightAction::IrCut | NightAction::Light => {
            let default = i64::from(action == NightAction::IrCut);
            let now = state
                .get(&t.id, action)
                .and_then(|v| v.as_i64())
                .unwrap_or(default);
            let cmd = if action == NightAction::IrCut {
                "ircut"
            } else {
                "white"
            };
            (cmd, json!(now), json!(1 - now))
        }
    };
    let url = format!("http://{}/x/json-imp.cgi", host_port(&t.host, t.web_port));
    let mut req = http()?
        .post(url)
        .header(CONTENT_TYPE, "application/json")
        .json(&json!({"cmd": cmd, "val": next}));
    if !t.web_api_key.is_empty() {
        req = req.header("X-API-Key", &t.web_api_key);
    }
    if !t.web_user.is_empty() {
        req = req.basic_auth(&t.web_user, Some(&t.web_pass));
    }
    let resp = req.send().await.map_err(http_error)?;
    if matches!(resp.status().as_u16(), 401 | 403) {
        return Err(NightError::Denied);
    }
    if !resp.status().is_success() {
        return Err(NightError::Http(format!("HTTP {}", resp.status().as_u16())));
    }
    state.set(&t.id, action, next.clone());
    let _ = current;
    Ok(NightOutcome::new(action, &[("value", next)]))
}

// ------------------------------------------------------------------ ONVIF

async fn onvif(t: &NightTarget, action: NightAction) -> Result<NightOutcome, NightError> {
    let creds = (!t.onvif_user.is_empty()).then(|| Credentials {
        user: t.onvif_user.clone(),
        pass: t.onvif_pass.clone(),
    });
    let client = OnvifClient::new(&t.host, t.onvif_port, creds)?;
    match action {
        NightAction::DayNight | NightAction::IrCut => {
            let (before, after) = client.toggle_ir_cut().await?;
            Ok(NightOutcome::new(
                action,
                &[
                    ("mode", json!(after.as_str())),
                    ("was", json!(before.as_str())),
                ],
            ))
        }
        NightAction::Light => {
            let brightness = client.toggle_light_brightness().await?;
            Ok(NightOutcome::new(
                action,
                &[("brightness", json!(brightness))],
            ))
        }
    }
}

// ------------------------------------------------------------------ Goform (Longse / OEM)

fn md5_hex(s: &str) -> String {
    use std::fmt::Write;
    Md5::digest(s.as_bytes())
        .iter()
        .fold(String::new(), |mut acc, b| {
            let _ = write!(acc, "{b:02x}");
            acc
        })
}

/// Value of a `name="value"` parameter of the `WWW-Authenticate` header.
fn challenge_param(header: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let start = header.find(&key)? + key.len();
    let end = header[start..].find('"')?;
    Some(header[start..start + end].to_string())
}

/// The `Authorization: X-Digest ...` header answering a challenge (same computation as the reference: MD5, `qop=auth`, fixed `nc` and `cnonce`).
#[must_use]
pub fn x_digest(
    user: &str,
    password: &str,
    method: &str,
    path: &str,
    realm: &str,
    nonce: &str,
    opaque: &str,
) -> String {
    let (nc, cnonce) = ("00000001", "domiNet1");
    let ha1 = md5_hex(&format!("{user}:{realm}:{password}"));
    let ha2 = md5_hex(&format!("{method}:{path}"));
    let response = md5_hex(&format!("{ha1}:{nonce}:{nc}:{cnonce}:auth:{ha2}"));
    format!(
        "X-Digest username=\"{user}\", realm=\"{realm}\", nonce=\"{nonce}\", uri=\"{path}\", qop=auth, nc={nc}, cnonce=\"{cnonce}\", response=\"{response}\", opaque=\"{opaque}\""
    )
}

/// A Goform request: challenge, then response. Returns the camera's JSON (empty object if it answers nothing).
async fn goform_request(
    t: &NightTarget,
    method: &str,
    path: &str,
    body: Option<&Value>,
) -> Result<Value, NightError> {
    let (user, pass) = if t.web_user.is_empty() {
        (t.onvif_user.as_str(), t.onvif_pass.as_str())
    } else {
        (t.web_user.as_str(), t.web_pass.as_str())
    };
    let url = format!("http://{}{path}", host_port(&t.host, t.web_port));
    let client = http()?;
    let m = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|_| NightError::BadResponse("method"))?;
    // 1. Without authentication: the camera answers 401 and gives its challenge. No body here (nothing must run without credentials).
    let challenge = client
        .request(m.clone(), &url)
        .send()
        .await
        .map_err(http_error)?;
    let header = challenge
        .headers()
        .get(WWW_AUTHENTICATE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let (Some(nonce), Some(realm)) = (
        challenge_param(&header, "nonce"),
        challenge_param(&header, "realm"),
    ) else {
        return Err(NightError::BadResponse("no authentication challenge"));
    };
    let opaque = challenge_param(&header, "opaque").unwrap_or_default();
    // 2. With the answer to the challenge.
    let mut req = client.request(m, &url).header(
        AUTHORIZATION,
        x_digest(user, pass, method, path, &realm, &nonce, &opaque),
    );
    if let Some(b) = body {
        req = req.header(CONTENT_TYPE, "application/json").json(b);
    }
    let resp = req.send().await.map_err(http_error)?;
    if matches!(resp.status().as_u16(), 401 | 403) {
        return Err(NightError::Denied);
    }
    if !resp.status().is_success() {
        return Err(NightError::Http(format!("HTTP {}", resp.status().as_u16())));
    }
    let text = resp.text().await.map_err(http_error)?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|_| NightError::BadResponse("invalid JSON"))
}

async fn goform(t: &NightTarget, action: NightAction) -> Result<NightOutcome, NightError> {
    match action {
        NightAction::DayNight | NightAction::IrCut => {
            // Read the whole state, only modify `DayNightMode`: 0 / 1 (day / auto) -> 2 (night), 2 -> 1 (auto).
            let path = "/goform/frmVideoParaEx";
            let got = goform_request(t, "GET", path, None).await?;
            let mut data = got.get("Data").cloned().unwrap_or_else(|| json!({}));
            let current = data["DayNightMode"]["DayNightMode"].as_i64().unwrap_or(1);
            let next = if current == 2 { 1 } else { 2 };
            data["DayNightMode"]["DayNightMode"] = json!(next);
            goform_request(t, "POST", path, Some(&json!({"Type": 1, "Data": data}))).await?;
            Ok(NightOutcome::new(
                action,
                &[("mode", json!(next)), ("was", json!(current))],
            ))
        }
        NightAction::Light => {
            // `Control`: 1 (on) -> 2 (auto), other -> 1 (on); `ModeSupportList` is not sent back.
            let path = "/goform/frmIotLightCfg";
            let got = goform_request(t, "GET", path, None).await?;
            let mut data = got.get("Data").cloned().unwrap_or_else(|| json!({}));
            if let Some(o) = data.as_object_mut() {
                o.remove("ModeSupportList");
            }
            let current = data["Control"].as_i64().unwrap_or(2);
            let next = if current == 1 { 2 } else { 1 };
            data["Control"] = json!(next);
            goform_request(t, "POST", path, Some(&json!({"Type": 1, "Data": data}))).await?;
            Ok(NightOutcome::new(
                action,
                &[("control", json!(next)), ("was", json!(current))],
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_parse_like_reference() {
        for (s, a) in [
            ("daynight", NightAction::DayNight),
            ("ircut", NightAction::IrCut),
            ("light", NightAction::Light),
        ] {
            assert_eq!(s.parse::<NightAction>().unwrap(), a);
            assert_eq!(a.as_str(), s);
        }
        assert!(matches!(
            "toggle".parse::<NightAction>(),
            Err(NightError::UnknownAction(_))
        ));
    }

    #[test]
    fn the_digest_follows_the_documented_calculation() {
        // md5("u:r:p") = ha1, md5("GET:/x") = ha2, response = md5(ha1:n:00000001:domiNet1:auth:ha2)
        let ha1 = md5_hex("u:r:p");
        let ha2 = md5_hex("GET:/x");
        let expected = md5_hex(&format!("{ha1}:n:00000001:domiNet1:auth:{ha2}"));
        let header = x_digest("u", "p", "GET", "/x", "r", "n", "o");
        assert!(
            header.starts_with("X-Digest username=\"u\", realm=\"r\", nonce=\"n\", uri=\"/x\"")
        );
        assert!(header.contains(&format!("response=\"{expected}\"")));
        assert!(
            !header.contains("\"p\"") && !header.contains(":p\""),
            "le mot de passe n'est pas dans l'en-tete"
        );
    }

    #[test]
    fn the_digest_matches_a_value_computed_independently_with_reference() {
        // Reference value computed separately.
        let header = x_digest(
            "admin",
            "mot-de-passe",
            "POST",
            "/goform/frmVideoParaEx",
            "IPCAM",
            "3a1f9c",
            "5ccc069c",
        );
        assert!(
            header.contains("response=\"5ac497fdaf775eb4afaac79067fccd89\""),
            "{header}"
        );
    }

    #[test]
    fn challenge_parameters_are_read_from_the_header() {
        let h = r#"X-Digest realm="cam", nonce="abc 123", opaque="zz", qop="auth""#;
        assert_eq!(challenge_param(h, "realm").as_deref(), Some("cam"));
        assert_eq!(challenge_param(h, "nonce").as_deref(), Some("abc 123"));
        assert_eq!(challenge_param(h, "missing"), None);
    }

    #[test]
    fn debug_hides_the_secrets() {
        let t = NightTarget {
            id: "x".into(),
            host: "h".into(),
            web_port: 80,
            web_user: "u".into(),
            web_pass: "motdepasse".into(),
            web_api_key: "cle-secrete".into(),
            onvif_port: 80,
            onvif_user: "u".into(),
            onvif_pass: "autre".into(),
        };
        let shown = format!("{t:?} {:?}", NightBackend::OpenIpc(t.clone()));
        for secret in ["motdepasse", "cle-secrete", "autre"] {
            assert!(!shown.contains(secret), "{shown}");
        }
    }
}
