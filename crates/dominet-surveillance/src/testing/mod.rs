// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fake MediaMTX for the tests and the demonstration: the paths configuration API, in memory. No stream, no network contact
//! beyond `127.0.0.1`.

pub mod discovery;
pub mod night;
pub mod onvif;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use serde_json::{Value, json};

#[derive(Default)]
struct Inner {
    paths: BTreeMap<String, Value>,
    /// If present, every request receives this status (simulated outage).
    fail_with: Option<u16>,
    requests: Vec<String>,
    /// Open sessions by API family (`rtspsessions`, `webrtcsessions`): identifier and user name.
    sessions: BTreeMap<String, Vec<(String, String)>>,
    kicked: Vec<String>,
}

#[derive(Clone)]
pub struct FakeMediaMtx {
    addr: SocketAddr,
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for FakeMediaMtx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FakeMediaMtx({})", self.addr)
    }
}

type Shared = Arc<Mutex<Inner>>;
type Reply = (StatusCode, Json<Value>);

fn ok() -> Reply {
    (StatusCode::OK, Json(json!({"status": "ok"})))
}

fn err(status: u16, msg: &str) -> Reply {
    (
        StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
        Json(json!({"status": "error", "error": msg})),
    )
}

/// Records the request; returns the simulated outage, if there is one.
fn enter(s: &Shared, line: String) -> Option<Reply> {
    let mut g = s.lock().unwrap();
    g.requests.push(line);
    g.fail_with.map(|c| err(c, "simulated failure"))
}

impl FakeMediaMtx {
    pub async fn start() -> Self {
        let inner: Shared = Shared::default();
        // Like the dev configuration: a wildcard path already there, which is not a camera.
        inner
            .lock()
            .unwrap()
            .paths
            .insert("~.*".into(), json!({"name": "~.*", "source": "publisher"}));
        let app = Router::new()
            .route("/v3/config/paths/list", get(list))
            .route("/v3/config/paths/get/{name}", get(get_path))
            .route("/v3/config/paths/add/{name}", post(add))
            .route("/v3/config/paths/replace/{name}", post(replace))
            .route("/v3/config/paths/patch/{name}", patch(patch_path))
            .route("/v3/config/paths/delete/{name}", delete(remove))
            .route("/v3/{kind}/list", get(list_sessions))
            .route("/v3/{kind}/kick/{id}", post(kick_session))
            .with_state(inner.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { addr, inner }
    }

    #[must_use]
    pub fn url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Current configuration of a path.
    #[must_use]
    pub fn path(&self, name: &str) -> Option<Value> {
        self.inner.lock().unwrap().paths.get(name).cloned()
    }

    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.inner.lock().unwrap().paths.keys().cloned().collect()
    }

    /// Adds a path as if another tool had created it.
    pub fn insert(&self, name: &str, conf: Value) {
        self.inner
            .lock()
            .unwrap()
            .paths
            .insert(name.into(), with_name(name, conf));
    }

    /// An open session (`kind`: `rtspsessions` or `webrtcsessions`) that authenticated as `user` (empty: no user name).
    pub fn add_session(&self, kind: &str, id: &str, user: &str) {
        self.inner
            .lock()
            .unwrap()
            .sessions
            .entry(kind.into())
            .or_default()
            .push((id.into(), user.into()));
    }

    /// Identifiers of the sessions that were kicked, in order.
    #[must_use]
    pub fn kicked(&self) -> Vec<String> {
        self.inner.lock().unwrap().kicked.clone()
    }

    /// `Some(code)`: every request receives this code; `None`: back to normal.
    pub fn fail_with(&self, code: Option<u16>) {
        self.inner.lock().unwrap().fail_with = code;
    }

    /// Requests received (`METHOD path`), in order.
    #[must_use]
    pub fn requests(&self) -> Vec<String> {
        self.inner.lock().unwrap().requests.clone()
    }
}

async fn list(State(s): State<Shared>, Query(q): Query<BTreeMap<String, String>>) -> Reply {
    if let Some(r) = enter(&s, "GET list".into()) {
        return r;
    }
    let g = s.lock().unwrap();
    let per: usize = q
        .get("itemsPerPage")
        .and_then(|v| v.parse().ok())
        .unwrap_or(100)
        .max(1);
    let page: usize = q.get("page").and_then(|v| v.parse().ok()).unwrap_or(0);
    let all: Vec<&Value> = g.paths.values().collect();
    let items: Vec<&Value> = all.iter().skip(page * per).take(per).copied().collect();
    (
        StatusCode::OK,
        Json(
            json!({"itemCount": all.len(), "pageCount": all.len().div_ceil(per).max(1), "items": items}),
        ),
    )
}

async fn get_path(State(s): State<Shared>, Path(name): Path<String>) -> Reply {
    if let Some(r) = enter(&s, format!("GET get/{name}")) {
        return r;
    }
    match s.lock().unwrap().paths.get(&name) {
        Some(v) => (StatusCode::OK, Json(v.clone())),
        None => err(404, "path configuration not found"),
    }
}

fn with_name(name: &str, mut conf: Value) -> Value {
    conf["name"] = json!(name);
    conf
}

async fn add(State(s): State<Shared>, Path(name): Path<String>, Json(conf): Json<Value>) -> Reply {
    if let Some(r) = enter(&s, format!("POST add/{name}")) {
        return r;
    }
    let mut g = s.lock().unwrap();
    if g.paths.contains_key(&name) {
        return err(400, "path already exists");
    }
    g.paths.insert(name.clone(), with_name(&name, conf));
    ok()
}

async fn replace(
    State(s): State<Shared>,
    Path(name): Path<String>,
    Json(conf): Json<Value>,
) -> Reply {
    if let Some(r) = enter(&s, format!("POST replace/{name}")) {
        return r;
    }
    let mut g = s.lock().unwrap();
    // Like the real MediaMTX: `replace` creates the path if it is missing.
    g.paths.insert(name.clone(), with_name(&name, conf));
    ok()
}

async fn patch_path(
    State(s): State<Shared>,
    Path(name): Path<String>,
    Json(change): Json<Value>,
) -> Reply {
    if let Some(r) = enter(&s, format!("PATCH patch/{name}")) {
        return r;
    }
    let mut g = s.lock().unwrap();
    let Some(conf) = g.paths.get_mut(&name) else {
        return err(404, "path not found");
    };
    if let (Some(conf), Some(change)) = (conf.as_object_mut(), change.as_object()) {
        for (k, v) in change {
            conf.insert(k.clone(), v.clone());
        }
    }
    ok()
}

async fn remove(State(s): State<Shared>, Path(name): Path<String>) -> Reply {
    if let Some(r) = enter(&s, format!("DELETE delete/{name}")) {
        return r;
    }
    if s.lock().unwrap().paths.remove(&name).is_some() {
        ok()
    } else {
        err(404, "path not found")
    }
}

async fn list_sessions(State(s): State<Shared>, Path(kind): Path<String>) -> Reply {
    if let Some(r) = enter(&s, format!("GET {kind}/list")) {
        return r;
    }
    let g = s.lock().unwrap();
    let items: Vec<Value> = g
        .sessions
        .get(&kind)
        .map(|v| {
            v.iter()
                .map(|(id, user)| json!({"id": id, "user": user}))
                .collect()
        })
        .unwrap_or_default();
    (
        StatusCode::OK,
        Json(json!({"itemCount": items.len(), "pageCount": 1, "items": items})),
    )
}

async fn kick_session(State(s): State<Shared>, Path((kind, id)): Path<(String, String)>) -> Reply {
    if let Some(r) = enter(&s, format!("POST {kind}/kick/{id}")) {
        return r;
    }
    let mut g = s.lock().unwrap();
    let Some(list) = g.sessions.get_mut(&kind) else {
        return err(404, "session not found");
    };
    let before = list.len();
    list.retain(|(sid, _)| *sid != id);
    if list.len() == before {
        return err(404, "session not found");
    }
    g.kicked.push(id);
    ok()
}
