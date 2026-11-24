// SPDX-License-Identifier: AGPL-3.0-or-later
//! Personal stream credentials (ADR 0064 lot 4): creation, use at the authorisation route, revocation and the closing of open sessions
//! (fake MediaMTX, isolated database).

mod common;

use axum::http::{Method, StatusCode};
use common::{Client, add_user, state};
use dominet_domain::repo::{StreamAccessRepo, UserRepo};
use dominet_domain::stream_access::{AccessState, NewStreamCredential};
use dominet_domain::user::{Role, UserId};
use dominet_server::AppState;
use dominet_server::stream_access::{issue_credential, revoke_credential};
use dominet_surveillance::testing::FakeMediaMtx;
use serde_json::{Value, json};
use sqlx::PgPool;

async fn rig(pool: PgPool) -> (AppState, FakeMediaMtx, UserId) {
    let fake = FakeMediaMtx::start().await;
    let state = state(pool, &[("MEDIAMTX_API_URL", &fake.url())]);
    add_user(&state, "alice", Role::User).await;
    add_user(&state, "bob", Role::User).await;
    let alice = state
        .store
        .users()
        .find_credentials("alice")
        .await
        .unwrap()
        .unwrap()
        .user
        .id;
    (state, fake, alice)
}

async fn status_of(client: &mut Client, user: &str, password: &str) -> StatusCode {
    let body: Value = json!({"action": "read", "user": user, "password": password, "token": password, "ip": "192.168.1.50"});
    client
        .send(Method::POST, "/internal/mediamtx/auth", Some(body), &[])
        .await
        .status
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_new_credential_works_at_the_route_with_the_account_name(pool: PgPool) {
    let (state, _fake, alice) = rig(pool).await;
    let issued = issue_credential(&state, alice, &NewStreamCredential::new("VLC", 30).unwrap())
        .await
        .unwrap();
    assert!(issued.password.starts_with("dns_"));
    assert_eq!(issued.access.name, "VLC");
    assert!(!format!("{issued:?}").contains(&issued.password));
    let mut c = Client::new(&state);
    assert_eq!(
        status_of(&mut c, "alice", &issued.password).await,
        StatusCode::OK
    );
    // Another account's name does not go with it.
    assert_eq!(
        status_of(&mut c, "bob", &issued.password).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn revoking_refuses_the_credential_and_closes_that_accounts_sessions(pool: PgPool) {
    let (state, fake, alice) = rig(pool).await;
    let issued = issue_credential(&state, alice, &NewStreamCredential::new("VLC", 30).unwrap())
        .await
        .unwrap();
    fake.add_session("rtspsessions", "r-alice", "alice");
    fake.add_session("rtspsessions", "r-bob", "bob");
    fake.add_session("webrtcsessions", "w-browser", "");
    let outcome = revoke_credential(&state, alice, "alice", issued.access.id)
        .await
        .unwrap();
    assert!(outcome.revoked);
    assert_eq!(outcome.closed, Some(1));
    assert_eq!(fake.kicked(), ["r-alice"]);
    let mut c = Client::new(&state);
    assert_eq!(
        status_of(&mut c, "alice", &issued.password).await,
        StatusCode::UNAUTHORIZED
    );
    let listed = state
        .store
        .stream_access()
        .list_credentials(alice)
        .await
        .unwrap();
    assert_eq!(listed[0].state(chrono::Utc::now()), AccessState::Revoked);
    // A second revocation changes nothing and touches no session.
    fake.add_session("rtspsessions", "r-again", "alice");
    let again = revoke_credential(&state, alice, "alice", issued.access.id)
        .await
        .unwrap();
    assert!(!again.revoked);
    assert_eq!(fake.kicked(), ["r-alice"]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn one_cannot_revoke_someone_elses_credential(pool: PgPool) {
    let (state, fake, alice) = rig(pool).await;
    let bob = state
        .store
        .users()
        .find_credentials("bob")
        .await
        .unwrap()
        .unwrap()
        .user
        .id;
    let issued = issue_credential(&state, alice, &NewStreamCredential::new("VLC", 30).unwrap())
        .await
        .unwrap();
    fake.add_session("rtspsessions", "r-bob", "bob");
    let outcome = revoke_credential(&state, bob, "bob", issued.access.id)
        .await
        .unwrap();
    assert!(!outcome.revoked);
    assert_eq!(fake.kicked().len(), 0);
    let mut c = Client::new(&state);
    assert_eq!(
        status_of(&mut c, "alice", &issued.password).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_revocation_holds_even_when_mediamtx_cannot_be_reached(pool: PgPool) {
    let (state, fake, alice) = rig(pool).await;
    let issued = issue_credential(&state, alice, &NewStreamCredential::new("VLC", 30).unwrap())
        .await
        .unwrap();
    fake.fail_with(Some(500));
    let outcome = revoke_credential(&state, alice, "alice", issued.access.id)
        .await
        .unwrap();
    assert!(outcome.revoked);
    assert_eq!(outcome.closed, None);
    let mut c = Client::new(&state);
    assert_eq!(
        status_of(&mut c, "alice", &issued.password).await,
        StatusCode::UNAUTHORIZED
    );
}
