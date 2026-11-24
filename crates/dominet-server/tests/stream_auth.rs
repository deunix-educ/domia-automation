// SPDX-License-Identifier: AGPL-3.0-or-later
//! The route MediaMTX asks before every camera read (ADR 0064): who may call it, and the decisions (full router, isolated database).

mod common;

use axum::http::{Method, StatusCode};
use chrono::{Duration, Utc};
use common::{Client, Resp, add_user, state};
use dominet_domain::repo::{StreamAccessRepo, UserRepo};
use dominet_domain::stream_access::NewStreamCredential;
use dominet_domain::user::{Role, UserId};
use dominet_server::AppState;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::PgPool;

const PATH: &str = "/internal/mediamtx/auth";

fn digest(secret: &str) -> Vec<u8> {
    Sha256::digest(secret.as_bytes()).to_vec()
}

async fn account(state: &AppState, name: &str) -> UserId {
    add_user(state, name, Role::User).await;
    state
        .store
        .users()
        .find_credentials(name)
        .await
        .unwrap()
        .unwrap()
        .user
        .id
}

async fn credential(state: &AppState, user: UserId, secret: &str, days: u32) {
    let new = NewStreamCredential::new("VLC", days).unwrap();
    state
        .store
        .stream_access()
        .create_credential(user, &new, &digest(secret), "dns_test", Utc::now())
        .await
        .unwrap();
}

async fn ask(client: &mut Client, body: Value) -> Resp {
    client.send(Method::POST, PATH, Some(body), &[]).await
}

fn read(user: &str, password: &str, token: &str) -> Value {
    json!({"action": "read", "user": user, "password": password, "token": token, "ip": "192.168.1.50",
           "path": "cam1", "protocol": "rtsp", "id": "x", "query": ""})
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_a_direct_private_caller_may_ask(pool: PgPool) {
    let state = state(pool, &[]);
    let body = read("", "", "");
    let mut inside = Client::new(&state);
    assert_eq!(
        ask(&mut inside, body.clone()).await.status,
        StatusCode::UNAUTHORIZED
    );
    let mut outside = inside.from([8, 8, 8, 8]);
    assert_eq!(
        ask(&mut outside, body.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    // Through nginx the peer is private too: the forwarding header is refused.
    let r = inside
        .send(
            Method::POST,
            PATH,
            Some(body),
            &[("x-forwarded-for", "203.0.113.9")],
        )
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn empty_credentials_get_the_rtsp_challenge(pool: PgPool) {
    let state = state(pool, &[]);
    let mut c = Client::new(&state);
    assert_eq!(
        ask(&mut c, read("", "", "")).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ask(&mut c, json!({"action": "read"})).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_viewing_token_opens_a_read_until_it_expires(pool: PgPool) {
    let state = state(pool, &[]);
    let alice = account(&state, "alice").await;
    let repo = state.store.stream_access();
    repo.create_viewing(alice, &digest("good"), "dnv_good", Utc::now())
        .await
        .unwrap();
    repo.create_viewing(
        alice,
        &digest("old"),
        "dnv_old",
        Utc::now() - Duration::minutes(10),
    )
    .await
    .unwrap();
    let mut c = Client::new(&state);
    let ok = ask(&mut c, read("", "", "good")).await;
    // No end date: the stream a token opens is not cut when the token expires.
    assert_eq!((ok.status, ok.body), (StatusCode::OK, Value::Null));
    assert_eq!(
        ask(&mut c, read("", "", "old")).await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        ask(&mut c, read("", "", "nope")).await.status,
        StatusCode::UNAUTHORIZED
    );
    // A credential value is not a viewing token.
    credential(&state, alice, "pw", 30).await;
    assert_eq!(
        ask(&mut c, read("", "", "pw")).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_personal_credential_needs_its_own_account_and_ends_with_its_date(pool: PgPool) {
    let state = state(pool, &[]);
    let alice = account(&state, "alice").await;
    account(&state, "bob").await;
    credential(&state, alice, "secret-1", 30).await;
    let mut c = Client::new(&state);
    // MediaMTX copies the password into `token` too.
    let ok = ask(&mut c, read("Alice", "secret-1", "secret-1")).await;
    assert_eq!(ok.status, StatusCode::OK);
    let end = ok.body["expireAt"].as_str().unwrap();
    assert!(
        end.ends_with('Z') && end > Utc::now().to_rfc3339().as_str(),
        "{end}"
    );
    for (user, pw) in [("bob", "secret-1"), ("alice", "wrong"), ("alice", "")] {
        let r = ask(&mut c, read(user, pw, pw)).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{user}/{pw}");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_revoked_credential_or_a_disabled_account_is_refused(pool: PgPool) {
    let state = state(pool.clone(), &[]);
    let alice = account(&state, "alice").await;
    credential(&state, alice, "secret-1", 30).await;
    let repo = state.store.stream_access();
    let id = repo.list_credentials(alice).await.unwrap()[0].id;
    let mut c = Client::new(&state);
    assert_eq!(
        ask(&mut c, read("alice", "secret-1", "")).await.status,
        StatusCode::OK
    );
    sqlx::query("UPDATE users SET is_active = FALSE WHERE id = $1")
        .bind(alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        ask(&mut c, read("alice", "secret-1", "")).await.status,
        StatusCode::UNAUTHORIZED
    );
    sqlx::query("UPDATE users SET is_active = TRUE WHERE id = $1")
        .bind(alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.revoke(id, alice, Utc::now()).await.unwrap());
    assert_eq!(
        ask(&mut c, read("alice", "secret-1", "")).await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn failed_attempts_lock_the_pair_out_like_the_login(pool: PgPool) {
    let state = state(pool, &[]);
    let alice = account(&state, "alice").await;
    credential(&state, alice, "secret-1", 30).await;
    let mut c = Client::new(&state);
    for _ in 0..5 {
        assert_eq!(
            ask(&mut c, read("alice", "guess", "")).await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    // The right credential is refused while the pair is locked...
    assert_eq!(
        ask(&mut c, read("alice", "secret-1", "")).await.status,
        StatusCode::UNAUTHORIZED
    );
    // ...but another address is not concerned.
    let mut other = c.from([192, 168, 1, 77]);
    let mut body = read("alice", "secret-1", "");
    body["ip"] = json!("192.168.1.77");
    assert_eq!(ask(&mut other, body).await.status, StatusCode::OK);
    // Empty credentials (the RTSP challenge) are never counted as failures.
    let mut fresh = Client::new(&state).from([192, 168, 1, 90]);
    for _ in 0..10 {
        let mut b = read("", "", "");
        b["ip"] = json!("192.168.1.90");
        ask(&mut fresh, b).await;
    }
    let mut b = read("alice", "secret-1", "");
    b["ip"] = json!("192.168.1.90");
    assert_eq!(ask(&mut fresh, b).await.status, StatusCode::OK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_service_credential_lets_the_video_role_read(pool: PgPool) {
    let state = state(
        pool,
        &[
            ("MEDIAMTX_READ_USER", "dominet-video"),
            ("MEDIAMTX_READ_PASSWORD", "service-pw"),
        ],
    );
    let mut c = Client::new(&state);
    assert_eq!(
        ask(&mut c, read("dominet-video", "service-pw", "service-pw"))
            .await
            .status,
        StatusCode::OK
    );
    for (u, p) in [("dominet-video", "other"), ("someone", "service-pw")] {
        assert_eq!(
            ask(&mut c, read(u, p, p)).await.status,
            StatusCode::UNAUTHORIZED,
            "{u}"
        );
    }
    // Without the setting, no such account exists.
    assert!(common::config(&[]).mediamtx_read.is_none());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_control_api_and_publishing_are_decided_by_origin_not_by_credentials(pool: PgPool) {
    let state = state(pool, &[]);
    let mut c = Client::new(&state);
    for action in ["api", "metrics", "pprof", "publish"] {
        let inside = json!({"action": action, "ip": "172.18.0.7"});
        assert_eq!(ask(&mut c, inside).await.status, StatusCode::OK, "{action}");
        let outside =
            json!({"action": action, "ip": "203.0.113.9", "user": "alice", "password": "x"});
        assert_eq!(
            ask(&mut c, outside).await.status,
            StatusCode::UNAUTHORIZED,
            "{action}"
        );
    }
    assert_eq!(
        ask(&mut c, json!({"action": "unknown", "ip": "127.0.0.1"}))
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    // A body that is not JSON is a bad request, not a decision.
    let (status, _, _) = c
        .send_bytes(
            Method::POST,
            PATH,
            "application/json",
            b"not json".to_vec(),
            &[],
        )
        .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_token_issued_for_the_browser_is_accepted_and_kept_hashed(pool: PgPool) {
    let state = state(pool.clone(), &[]);
    let alice = account(&state, "alice").await;
    let issued = dominet_server::stream_access::issue_viewing(&state, alice)
        .await
        .unwrap();
    assert!(issued.value.starts_with("dnv_"));
    assert!(
        issued.expires_at > Utc::now() && issued.expires_at < Utc::now() + Duration::minutes(6)
    );
    // Only the fingerprint is in the database.
    let stored: Vec<u8> =
        sqlx::query_scalar("SELECT secret_hash FROM stream_access WHERE user_id = $1")
            .bind(alice.0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, digest(&issued.value));
    let mut c = Client::new(&state);
    // MediaMTX puts a Bearer token in `token` and nothing in `user`.
    assert_eq!(
        ask(&mut c, read("", "", &issued.value)).await.status,
        StatusCode::OK
    );
    // Two tokens for the same account are independent.
    let second = dominet_server::stream_access::issue_viewing(&state, alice)
        .await
        .unwrap();
    assert_ne!(issued.value, second.value);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn issuing_a_token_tidies_up_the_expired_ones(pool: PgPool) {
    let state = state(pool.clone(), &[]);
    let alice = account(&state, "alice").await;
    let repo = state.store.stream_access();
    repo.create_viewing(
        alice,
        &digest("ancient"),
        "dnv_a",
        Utc::now() - Duration::hours(3),
    )
    .await
    .unwrap();
    repo.create_viewing(
        alice,
        &digest("recent"),
        "dnv_b",
        Utc::now() - Duration::minutes(10),
    )
    .await
    .unwrap();
    dominet_server::stream_access::issue_viewing(&state, alice)
        .await
        .unwrap();
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM stream_access")
        .fetch_one(&pool)
        .await
        .unwrap();
    // The three-hour-old one is gone; the ten-minute-old one (expired, but recent) and the new one remain.
    assert_eq!(left, 2);
}
