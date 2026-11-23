// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fake web servers of the non-ONVIF day / night methods: `OpenIPC`, Thingino, Goform (Digest). No network contact outside the loopback.

#![allow(
    clippy::items_after_statements,
    reason = "gestionnaires locaux a chaque faux serveur"
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::{Value, json};

use crate::night::x_digest;

fn basic_ok(headers: &HeaderMap, user: &str, pass: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| B64.decode(v).ok())
        .is_some_and(|d| d == format!("{user}:{pass}").as_bytes())
}

// ------------------------------------------------------------------ OpenIPC

#[derive(Default)]
struct OpenIpcInner {
    credentials: Option<(String, String)>,
    paths: Vec<String>,
}

#[derive(Clone)]
pub struct FakeOpenIpc {
    addr: SocketAddr,
    inner: Arc<Mutex<OpenIpcInner>>,
}

impl std::fmt::Debug for FakeOpenIpc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeOpenIpc({})", self.addr)
    }
}

impl FakeOpenIpc {
    /// `credentials`: require basic authentication with this pair.
    pub async fn start(credentials: Option<(&str, &str)>) -> Self {
        let inner = Arc::new(Mutex::new(OpenIpcInner {
            credentials: credentials.map(|(u, p)| (u.into(), p.into())),
            paths: vec![],
        }));
        async fn night(
            State(s): State<Arc<Mutex<OpenIpcInner>>>,
            Path(what): Path<String>,
            headers: HeaderMap,
        ) -> Response {
            let mut g = s.lock().unwrap();
            if let Some((u, p)) = &g.credentials
                && !basic_ok(&headers, u, p)
            {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            g.paths.push(what);
            "ok".into_response()
        }
        let app = Router::new()
            .route("/night/{what}", get(night))
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, inner }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// Paths requested under `/night/` (`toggle`, `light`).
    #[must_use]
    pub fn paths(&self) -> Vec<String> {
        self.inner.lock().unwrap().paths.clone()
    }
}

// ------------------------------------------------------------------ Thingino

struct ThinginoInner {
    api_key: String,
    bodies: Vec<Value>,
}

#[derive(Clone)]
pub struct FakeThingino {
    addr: SocketAddr,
    inner: Arc<Mutex<ThinginoInner>>,
}

impl std::fmt::Debug for FakeThingino {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeThingino({})", self.addr)
    }
}

impl FakeThingino {
    pub async fn start(api_key: &str) -> Self {
        let inner = Arc::new(Mutex::new(ThinginoInner {
            api_key: api_key.into(),
            bodies: vec![],
        }));
        async fn cgi(
            State(s): State<Arc<Mutex<ThinginoInner>>>,
            headers: HeaderMap,
            body: Bytes,
        ) -> Response {
            let mut g = s.lock().unwrap();
            let key = headers
                .get("x-api-key")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if key != g.api_key {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            match serde_json::from_slice::<Value>(&body) {
                Ok(v) => {
                    g.bodies.push(v);
                    axum::Json(json!({"status": "ok"})).into_response()
                }
                Err(_) => StatusCode::BAD_REQUEST.into_response(),
            }
        }
        let app = Router::new()
            .route("/x/json-imp.cgi", post(cgi))
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, inner }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// JSON bodies received (`{"cmd": ..., "val": ...}`), in order.
    #[must_use]
    pub fn bodies(&self) -> Vec<Value> {
        self.inner.lock().unwrap().bodies.clone()
    }
}

// ------------------------------------------------------------------ Goform (Digest)

struct GoformInner {
    user: String,
    password: String,
    day_night: i64,
    light: i64,
    /// Bodies received with a valid authentication, per path.
    posts: Vec<(String, Value)>,
    /// Requests received without a valid authentication (the challenge, then the refusals).
    refused: u32,
}

#[derive(Clone)]
pub struct FakeGoform {
    addr: SocketAddr,
    inner: Arc<Mutex<GoformInner>>,
}

impl std::fmt::Debug for FakeGoform {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeGoform({})", self.addr)
    }
}

const REALM: &str = "IPCAM";
const NONCE: &str = "3a1f9c";
const OPAQUE: &str = "5ccc069c";

impl FakeGoform {
    pub async fn start(user: &str, password: &str) -> Self {
        let inner = Arc::new(Mutex::new(GoformInner {
            user: user.into(),
            password: password.into(),
            day_night: 1,
            light: 2,
            posts: vec![],
            refused: 0,
        }));
        async fn handle(
            State(s): State<Arc<Mutex<GoformInner>>>,
            uri: Uri,
            method: axum::http::Method,
            headers: HeaderMap,
            body: Bytes,
        ) -> Response {
            let mut g = s.lock().unwrap();
            let path = uri.path().to_string();
            let expected = x_digest(
                &g.user,
                &g.password,
                method.as_str(),
                &path,
                REALM,
                NONCE,
                OPAQUE,
            );
            let given = headers
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if given != expected {
                g.refused += 1;
                return (
                    StatusCode::UNAUTHORIZED,
                    [(header::WWW_AUTHENTICATE, format!(r#"X-Digest realm="{REALM}", nonce="{NONCE}", opaque="{OPAQUE}", qop="auth""#))],
                    "",
                )
                    .into_response();
            }
            match (method.as_str(), path.as_str()) {
                ("GET", "/goform/frmVideoParaEx") => axum::Json(json!({
                    "Type": 0,
                    "Data": {"DayNightMode": {"DayNightMode": g.day_night, "Sensitivity": 3}, "Mirror": 0, "Flip": 0}
                }))
                .into_response(),
                ("GET", "/goform/frmIotLightCfg") => axum::Json(json!({
                    "Type": 0,
                    "Data": {"Control": g.light, "ModeSupportList": [0, 1, 2], "Brightness": 60}
                }))
                .into_response(),
                ("POST", "/goform/frmVideoParaEx" | "/goform/frmIotLightCfg") => {
                    let Ok(v) = serde_json::from_slice::<Value>(&body) else {
                        return StatusCode::BAD_REQUEST.into_response();
                    };
                    if path.ends_with("frmVideoParaEx") {
                        g.day_night = v["Data"]["DayNightMode"]["DayNightMode"].as_i64().unwrap_or(-1);
                    } else {
                        g.light = v["Data"]["Control"].as_i64().unwrap_or(-1);
                    }
                    g.posts.push((path, v));
                    "".into_response()
                }
                _ => StatusCode::NOT_FOUND.into_response(),
            }
        }
        let app = Router::new().fallback(handle).with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, inner }
    }

    #[must_use]
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    #[must_use]
    pub fn day_night(&self) -> i64 {
        self.inner.lock().unwrap().day_night
    }

    #[must_use]
    pub fn light(&self) -> i64 {
        self.inner.lock().unwrap().light
    }

    /// Bodies received by `POST` (authenticated), with their path.
    #[must_use]
    pub fn posts(&self) -> Vec<(String, Value)> {
        self.inner.lock().unwrap().posts.clone()
    }

    /// Number of refused requests (the challenge without authentication counts).
    #[must_use]
    pub fn refused(&self) -> u32 {
        self.inner.lock().unwrap().refused
    }
}
