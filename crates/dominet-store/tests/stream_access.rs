// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera viewing secrets (ADR 0064): one isolated database per test (`#[sqlx::test]`).

use chrono::{DateTime, Duration, Utc};
use dominet_domain::repo::StreamAccessRepo;
use dominet_domain::stream_access::{
    AccessState, NewStreamCredential,
    StreamAccessKind::{Credential, Viewing},
};
use dominet_domain::user::UserId;
use dominet_store::PgStore;
use sqlx::PgPool;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-24T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn hash(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

fn cred(name: &str, days: u32) -> NewStreamCredential {
    NewStreamCredential::new(name, days).unwrap()
}

async fn user(pool: &PgPool, name: &str, active: bool) -> UserId {
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, is_active) VALUES ($1, 'x', $2) RETURNING id",
    )
    .bind(name)
    .bind(active)
    .fetch_one(pool)
    .await
    .unwrap();
    UserId(id)
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_credential_is_found_by_its_hash_with_its_account(pool: PgPool) {
    let alice = user(&pool, "Alice", true).await;
    let repo = PgStore::new(pool).stream_access();
    let c = repo
        .create_credential(
            alice,
            &cred("Phone of Anna", 90),
            &hash(1),
            "dns_ab12cd34",
            now(),
        )
        .await
        .unwrap();
    assert_eq!(
        (c.kind, c.name.as_str(), c.user),
        (Credential, "Phone of Anna", alice)
    );
    assert_eq!(c.expires_at, now() + Duration::days(90));
    assert_eq!(c.state(now()), AccessState::Active);

    let found = repo
        .find_active(Credential, &hash(1), now())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.access.id, c.id);
    assert!(found.is_user("alice"));
    assert!(
        repo.find_active(Credential, &hash(2), now())
            .await
            .unwrap()
            .is_none()
    );
    // A credential is not a viewing token: the same value does not open the other door.
    assert!(
        repo.find_active(Viewing, &hash(1), now())
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_viewing_token_expires_after_a_few_minutes(pool: PgPool) {
    let alice = user(&pool, "alice", true).await;
    let repo = PgStore::new(pool).stream_access();
    let t = repo
        .create_viewing(alice, &hash(1), "dnv_ab12cd34", now())
        .await
        .unwrap();
    assert_eq!((t.kind, t.name.as_str()), (Viewing, ""));
    assert!(
        repo.find_active(Viewing, &hash(1), now() + Duration::minutes(4))
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        repo.find_active(Viewing, &hash(1), now() + Duration::minutes(5))
            .await
            .unwrap()
            .is_none()
    );
    // Viewing tokens are not listed with the credentials.
    assert_eq!(repo.list_credentials(alice).await.unwrap().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_revoked_credential_is_refused_and_only_its_owner_can_revoke_it(pool: PgPool) {
    let alice = user(&pool, "alice", true).await;
    let bob = user(&pool, "bob", true).await;
    let repo = PgStore::new(pool).stream_access();
    let c = repo
        .create_credential(alice, &cred("VLC", 30), &hash(1), "dns_x", now())
        .await
        .unwrap();
    assert!(!repo.revoke(c.id, bob, now()).await.unwrap());
    assert!(
        repo.find_active(Credential, &hash(1), now())
            .await
            .unwrap()
            .is_some()
    );
    assert!(repo.revoke(c.id, alice, now()).await.unwrap());
    assert!(
        repo.find_active(Credential, &hash(1), now())
            .await
            .unwrap()
            .is_none()
    );
    // A second revocation does not change anything.
    assert!(
        !repo
            .revoke(c.id, alice, now() + Duration::hours(1))
            .await
            .unwrap()
    );
    let listed = repo.list_credentials(alice).await.unwrap();
    assert_eq!(listed[0].revoked_at, Some(now()));
    assert_eq!(listed[0].state(now()), AccessState::Revoked);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_disabled_account_has_no_access_and_a_deleted_one_loses_its_secrets(pool: PgPool) {
    let alice = user(&pool, "alice", true).await;
    let repo = PgStore::new(pool.clone()).stream_access();
    repo.create_credential(alice, &cred("VLC", 30), &hash(1), "dns_x", now())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET is_active = FALSE WHERE id = $1")
        .bind(alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert!(
        repo.find_active(Credential, &hash(1), now())
            .await
            .unwrap()
            .is_none()
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(alice.0)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(repo.list_credentials(alice).await.unwrap().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn expired_viewing_tokens_are_purged_and_credentials_are_kept(pool: PgPool) {
    let alice = user(&pool, "alice", true).await;
    let repo = PgStore::new(pool).stream_access();
    repo.create_viewing(alice, &hash(1), "dnv_a", now())
        .await
        .unwrap();
    repo.create_viewing(alice, &hash(2), "dnv_b", now() + Duration::hours(1))
        .await
        .unwrap();
    repo.create_credential(alice, &cred("VLC", 1), &hash(3), "dns_c", now())
        .await
        .unwrap();
    let later = now() + Duration::days(2);
    assert_eq!(repo.purge_expired_viewing(later).await.unwrap(), 2);
    assert_eq!(repo.list_credentials(alice).await.unwrap().len(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_last_use_is_written_at_most_once_a_minute(pool: PgPool) {
    let alice = user(&pool, "alice", true).await;
    let repo = PgStore::new(pool).stream_access();
    let c = repo
        .create_credential(alice, &cred("VLC", 30), &hash(1), "dns_x", now())
        .await
        .unwrap();
    repo.touch(c.id, now()).await.unwrap();
    repo.touch(c.id, now() + Duration::seconds(30))
        .await
        .unwrap();
    assert_eq!(
        repo.list_credentials(alice).await.unwrap()[0].last_used_at,
        Some(now())
    );
    repo.touch(c.id, now() + Duration::seconds(90))
        .await
        .unwrap();
    assert_eq!(
        repo.list_credentials(alice).await.unwrap()[0].last_used_at,
        Some(now() + Duration::seconds(90))
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_database_refuses_a_nameless_credential_and_a_short_hash(pool: PgPool) {
    let alice = user(&pool, "alice", true).await;
    for (kind, name, hash_len) in [("credential", "", 32), ("viewing", "", 31)] {
        let r = sqlx::query(
            "INSERT INTO stream_access (user_id, kind, name, secret_hash, prefix, expires_at)
             VALUES ($1, $2, $3, $4, 'p', now() + interval '1 day')",
        )
        .bind(alice.0)
        .bind(kind)
        .bind(name)
        .bind(vec![1u8; hash_len])
        .execute(&pool)
        .await;
        assert!(r.is_err(), "{kind} {hash_len}");
    }
}
