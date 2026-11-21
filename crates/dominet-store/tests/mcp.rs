// SPDX-License-Identifier: AGPL-3.0-or-later
//! MCP access tokens and audit trail (R11 M1, ADR 0059): one isolated database per test (`#[sqlx::test]`).

use chrono::{DateTime, Duration, Utc};
use dominet_domain::mcp::{AuditKind, NewAuditEntry, NewMcpToken, Scope};
use dominet_domain::repo::{McpAuditRepo, McpTokenRepo};
use dominet_store::PgStore;
use serde_json::json;
use sqlx::PgPool;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-16T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn new_token(name: &str, scopes: &[&str], days: u32) -> NewMcpToken {
    let scopes: Vec<String> = scopes.iter().map(ToString::to_string).collect();
    NewMcpToken::new(name, &scopes, days).unwrap()
}

fn hash(byte: u8) -> Vec<u8> {
    vec![byte; 32]
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_token_is_found_by_its_hash_with_its_scopes_and_expiry(pool: PgPool) {
    let tokens = PgStore::new(pool).mcp_tokens();
    let t = tokens
        .create(
            &new_token("Claude Code", &["lecture", "pilotage"], 90),
            &hash(1),
            "dnm_ab12cd34",
            now(),
            None,
        )
        .await
        .unwrap();
    assert_eq!(t.name, "Claude Code");
    assert_eq!(t.prefix, "dnm_ab12cd34");
    assert_eq!(
        t.scopes,
        [Scope::Read, Scope::Control].into_iter().collect()
    );
    assert_eq!(t.expires_at, now() + Duration::days(90));
    assert!(t.last_used_at.is_none() && t.revoked_at.is_none());

    let found = tokens.find_active(&hash(1), now()).await.unwrap().unwrap();
    assert_eq!(found.id, t.id);
    assert!(tokens.find_active(&hash(2), now()).await.unwrap().is_none());
    assert_eq!(tokens.list().await.unwrap().len(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_expired_token_is_not_found_even_if_never_revoked(pool: PgPool) {
    let tokens = PgStore::new(pool).mcp_tokens();
    tokens
        .create(
            &new_token("court", &["lecture"], 1),
            &hash(1),
            "dnm_x",
            now(),
            None,
        )
        .await
        .unwrap();
    assert!(
        tokens
            .find_active(&hash(1), now() + Duration::hours(23))
            .await
            .unwrap()
            .is_some()
    );
    // The end date is exclusive: at the exact instant the token is already refused.
    assert!(
        tokens
            .find_active(&hash(1), now() + Duration::days(1))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        tokens
            .find_active(&hash(1), now() + Duration::days(2))
            .await
            .unwrap()
            .is_none()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_revoked_token_is_refused_at_once_and_keeps_its_first_revocation_date(pool: PgPool) {
    let tokens = PgStore::new(pool).mcp_tokens();
    let t = tokens
        .create(
            &new_token("a", &["lecture"], 30),
            &hash(1),
            "dnm_x",
            now(),
            None,
        )
        .await
        .unwrap();
    assert!(tokens.revoke(t.id, now()).await.unwrap());
    assert!(tokens.find_active(&hash(1), now()).await.unwrap().is_none());
    // A second revocation changes nothing and says so.
    assert!(
        !tokens
            .revoke(t.id, now() + Duration::hours(1))
            .await
            .unwrap()
    );
    let listed = tokens.list().await.unwrap();
    assert_eq!(listed[0].revoked_at, Some(now()));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_last_use_is_written_at_most_once_a_minute(pool: PgPool) {
    let tokens = PgStore::new(pool).mcp_tokens();
    let t = tokens
        .create(
            &new_token("a", &["lecture"], 30),
            &hash(1),
            "dnm_x",
            now(),
            None,
        )
        .await
        .unwrap();
    tokens.touch(t.id, now()).await.unwrap();
    tokens
        .touch(t.id, now() + Duration::seconds(30))
        .await
        .unwrap();
    assert_eq!(tokens.list().await.unwrap()[0].last_used_at, Some(now()));
    tokens
        .touch(t.id, now() + Duration::seconds(90))
        .await
        .unwrap();
    assert_eq!(
        tokens.list().await.unwrap()[0].last_used_at,
        Some(now() + Duration::seconds(90))
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn two_tokens_cannot_share_a_hash_and_the_database_refuses_bad_rows(pool: PgPool) {
    let tokens = PgStore::new(pool.clone()).mcp_tokens();
    tokens
        .create(
            &new_token("a", &["lecture"], 30),
            &hash(1),
            "dnm_x",
            now(),
            None,
        )
        .await
        .unwrap();
    let dup = tokens
        .create(
            &new_token("b", &["lecture"], 30),
            &hash(1),
            "dnm_y",
            now(),
            None,
        )
        .await;
    assert!(
        matches!(dup, Err(dominet_domain::RepoError::Conflict { .. })),
        "{dup:?}"
    );
    // The table itself refuses a hash that is not 32 bytes and an empty scope list.
    let short = tokens
        .create(
            &new_token("c", &["lecture"], 30),
            &[1, 2, 3],
            "dnm_z",
            now(),
            None,
        )
        .await;
    assert!(short.is_err());
    let no_scope = sqlx::query(
        "INSERT INTO mcp_token (name, token_hash, prefix, scopes, expires_at) VALUES ('d', $1, 'p', '{}', now())",
    )
    .bind(hash(9))
    .execute(&pool)
    .await;
    assert!(no_scope.is_err());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn nothing_in_the_database_lets_one_recover_a_token_value(pool: PgPool) {
    // What is stored for a token is the hash and the visible prefix: no column can hold the value.
    let tokens = PgStore::new(pool.clone()).mcp_tokens();
    let value = "dnm_SECRETSECRETSECRETSECRETSECRETSECRETSECRE";
    tokens
        .create(
            &new_token("a", &["lecture"], 30),
            &hash(7),
            &value[..12],
            now(),
            None,
        )
        .await
        .unwrap();
    let dump: Vec<String> = sqlx::query_scalar("SELECT row_to_json(t)::text FROM mcp_token t")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        dump.iter().all(|row| !row.contains(&value[12..])),
        "{dump:?}"
    );
}

fn audit(kind: AuditKind, actor: &str) -> NewAuditEntry {
    NewAuditEntry {
        kind,
        actor: actor.into(),
        actor_kind: "web".into(),
        token_id: None,
        tool: String::new(),
        params: json!({"name": "essai"}),
        outcome: "ok".into(),
        detail: String::new(),
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_audit_trail_lists_newest_first_filters_by_kind_and_purges(pool: PgPool) {
    let audit_repo = PgStore::new(pool).mcp_audit();
    audit_repo
        .record(&audit(AuditKind::TokenCreated, "alice"), now())
        .await
        .unwrap();
    audit_repo
        .record(
            &audit(AuditKind::Denied, "Claude Code"),
            now() + Duration::hours(1),
        )
        .await
        .unwrap();
    audit_repo
        .record(
            &audit(AuditKind::TokenRevoked, "alice"),
            now() + Duration::hours(2),
        )
        .await
        .unwrap();

    // An action writes its line first (`pending`), then closes it with what happened.
    let mut pending = audit(AuditKind::ToolCall, "Claude Code");
    pending.outcome = "pending".into();
    let line = audit_repo
        .record(&pending, now() + Duration::hours(3))
        .await
        .unwrap();
    let before = audit_repo
        .recent(Some(AuditKind::ToolCall), 10)
        .await
        .unwrap();
    assert_eq!(
        (before[0].id, before[0].outcome.as_str()),
        (line, "pending")
    );
    audit_repo
        .complete(line, "ok", "commande publiee")
        .await
        .unwrap();
    let after = audit_repo
        .recent(Some(AuditKind::ToolCall), 10)
        .await
        .unwrap();
    assert_eq!(
        (after[0].outcome.as_str(), after[0].detail.as_str()),
        ("ok", "commande publiee")
    );
    assert_eq!(
        audit_repo
            .purge_before(now() + Duration::hours(4))
            .await
            .unwrap(),
        4
    );
    audit_repo
        .record(&audit(AuditKind::TokenCreated, "alice"), now())
        .await
        .unwrap();
    audit_repo
        .record(
            &audit(AuditKind::Denied, "Claude Code"),
            now() + Duration::hours(1),
        )
        .await
        .unwrap();
    audit_repo
        .record(
            &audit(AuditKind::TokenRevoked, "alice"),
            now() + Duration::hours(2),
        )
        .await
        .unwrap();

    let all = audit_repo.recent(None, 10).await.unwrap();
    let kinds: Vec<_> = all.iter().map(|e| e.kind.as_str()).collect();
    assert_eq!(kinds, ["token_revoked", "denied", "token_created"]);
    assert_eq!(all[2].params, json!({"name": "essai"}));
    assert_eq!(
        audit_repo
            .recent(Some(AuditKind::Denied), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(audit_repo.recent(None, 2).await.unwrap().len(), 2);

    // Purge: only what is older than the limit goes.
    let purged = audit_repo
        .purge_before(now() + Duration::minutes(90))
        .await
        .unwrap();
    assert_eq!(purged, 2);
    let left = audit_repo.recent(None, 10).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].kind, "token_revoked");
}
