// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tools shared by the integration tests: test state, minimal "browser" with a cookie, accounts.
#![allow(dead_code)] // each test file only uses part of these tools

pub mod broker;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use dominet_domain::user::Role;
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

pub const PASSWORD: &str = "correct horse battery";

pub fn config(extra: &[(&str, &str)]) -> Config {
    let mut vars = vec![
        ("DATABASE_URL", "postgres://u:p@127.0.0.1:1/none"),
        ("COOKIE_SECURE", "false"),
    ];
    vars.extend_from_slice(extra);
    Config::from_lookup(|k| {
        vars.iter()
            .rev()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| (*v).to_string())
    })
    .unwrap()
}

pub fn state(pool: PgPool, extra: &[(&str, &str)]) -> AppState {
    AppState::with_hasher(
        pool,
        Arc::new(config(extra)),
        Hasher::with_params(1024, 1, 1).unwrap(),
    )
}

pub struct Resp {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Value,
}

/// Minimal browser: keeps the session cookie and always sends the same peer IP.
pub struct Client {
    pub app: Router,
    pub cookie: Option<String>,
    pub peer: SocketAddr,
}

impl Client {
    pub fn new(state: &AppState) -> Self {
        Self {
            app: web::router(state.clone()),
            cookie: None,
            peer: SocketAddr::from(([192, 168, 1, 50], 40_000)),
        }
    }

    pub fn from(&self, ip: [u8; 4]) -> Self {
        Self {
            app: self.app.clone(),
            cookie: None,
            peer: SocketAddr::from((ip, 40_000)),
        }
    }

    pub async fn send(
        &mut self,
        method: Method,
        path: &str,
        body: Option<Value>,
        extra: &[(&str, &str)],
    ) -> Resp {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "dominet.test");
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        }
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let mut req = req
            .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
            .unwrap();
        req.extensions_mut().insert(ConnectInfo(self.peer));
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        for sc in headers.get_all(header::SET_COOKIE) {
            let sc = sc.to_str().unwrap();
            let pair = sc.split(';').next().unwrap();
            let expired = sc.contains("Max-Age=0") || pair.ends_with('=');
            self.cookie = if expired {
                None
            } else {
                Some(pair.to_string())
            };
        }
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        Resp {
            status,
            headers,
            body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        }
    }

    /// Request with a binary body (upload); returns status, headers and raw body.
    pub async fn send_bytes(
        &mut self,
        method: Method,
        path: &str,
        content_type: &str,
        body: Vec<u8>,
        extra: &[(&str, &str)],
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, "dominet.test")
            .header(header::CONTENT_TYPE, content_type);
        if let Some(c) = &self.cookie {
            req = req.header(header::COOKIE, c);
        }
        for (k, v) in extra {
            req = req.header(*k, *v);
        }
        let mut req = req.body(Body::from(body)).unwrap();
        req.extensions_mut().insert(ConnectInfo(self.peer));
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let (status, headers) = (resp.status(), resp.headers().clone());
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, headers, bytes.to_vec())
    }

    pub async fn login(&mut self, user: &str, password: &str) -> Resp {
        self.send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": user, "password": password})),
            &[],
        )
        .await
    }

    pub async fn get(&mut self, path: &str) -> Resp {
        self.send(Method::GET, path, None, &[]).await
    }
}

pub async fn add_user(state: &AppState, name: &str, role: Role) {
    state
        .auth
        .create_user(name, &format!("{name}@example.org"), "", role, PASSWORD)
        .await
        .unwrap();
}

pub fn code(r: &Resp) -> &str {
    r.body["error"]["code"].as_str().unwrap_or("")
}
