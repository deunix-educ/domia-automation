// SPDX-License-Identifier: AGPL-3.0-or-later
//! The embedded documentation (ADR 0056): a folder asked without its trailing slash must be redirected inside `/docs/`.

mod common;

use axum::http::StatusCode;
use common::{Client, add_user, state};
use dominet_domain::user::Role;
use sqlx::PgPool;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_folder_without_slash_stays_under_docs(pool: PgPool) {
    let dir = std::env::temp_dir().join(format!("dominet-docs-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("fr")).unwrap();
    std::fs::write(dir.join("index.html"), "en").unwrap();
    std::fs::write(dir.join("fr/index.html"), "fr").unwrap();
    let root = dir.to_str().unwrap().to_string();
    let state = state(pool, &[("DOCS_ROOT", root.as_str())]);
    add_user(&state, "alice", Role::User).await;
    let mut browser = Client::new(&state);
    assert_eq!(
        browser.login("alice", common::PASSWORD).await.status,
        StatusCode::OK
    );

    assert_eq!(browser.get("/docs/fr/").await.status, StatusCode::OK);
    let r = browser.get("/docs/fr").await;
    let location = r
        .headers
        .get("location")
        .map(|v| v.to_str().unwrap().to_string());
    std::fs::remove_dir_all(&dir).ok();
    assert!(r.status.is_redirection(), "{}", r.status);
    let location = location.expect("a redirection has a Location");
    // `/fr/` would leave the documentation (page not found).
    assert!(location == "/docs/fr/", "redirected to {location}");
}
