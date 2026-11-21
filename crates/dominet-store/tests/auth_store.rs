// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests du store de sessions et du journal des tentatives de connexion (base isolee par test).

use chrono::{Duration, Utc};
use dominet_domain::repo::{LoginAttemptRepo, UserRepo};
use dominet_domain::user::{NewUser, Role, UserId, Username};
use dominet_store::{PgStore, USER_ID_KEY};
use sqlx::PgPool;
use time::OffsetDateTime;
use tower_sessions_core::session::{Id, Record};
use tower_sessions_core::session_store::{ExpiredDeletion, SessionStore};

async fn add_user(store: &PgStore, name: &str) -> UserId {
    let new = NewUser {
        username: Username::parse(name).unwrap(),
        email: String::new(),
        first_name: String::new(),
        last_name: String::new(),
        mobile: String::new(),
        role: Role::User,
        is_active: true,
        password_hash: "h".into(),
    };
    store.users().create(&new).await.unwrap().id
}

fn record(user: Option<UserId>, valid_for: time::Duration) -> Record {
    let mut data = std::collections::HashMap::new();
    data.insert("theme".to_string(), serde_json::json!("dark"));
    if let Some(u) = user {
        data.insert(USER_ID_KEY.to_string(), serde_json::json!(u.0));
    }
    Record {
        id: Id::default(),
        data,
        expiry_date: OffsetDateTime::now_utc() + valid_for,
    }
}

// ---------------------------------------------------------------- sessions

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_session_round_trips(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let sessions = store.sessions();
    let mut rec = record(None, time::Duration::hours(1));
    sessions.create(&mut rec).await.unwrap();

    let loaded = sessions
        .load(&rec.id)
        .await
        .unwrap()
        .expect("session trouvee");
    assert_eq!(loaded.id, rec.id);
    assert_eq!(loaded.data["theme"], "dark");
    assert!((loaded.expiry_date - rec.expiry_date).abs() < time::Duration::milliseconds(5));

    let user_col: Option<i64> = sqlx::query_scalar("SELECT user_id FROM session")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(user_col, None, "session anonyme");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_user_id_column_follows_the_session_data(pool: PgPool) {
    let store = PgStore::new(pool.clone());
    let alice = add_user(&store, "alice").await;
    let sessions = store.sessions();
    let mut rec = record(None, time::Duration::hours(1));
    sessions.create(&mut rec).await.unwrap();

    // Connexion : la cle user_id apparait dans les donnees -> la colonne est renseignee.
    rec.data
        .insert(USER_ID_KEY.into(), serde_json::json!(alice.0));
    sessions.save(&rec).await.unwrap();
    let col: Option<i64> = sqlx::query_scalar("SELECT user_id FROM session WHERE id = $1")
        .bind(rec.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(col, Some(alice.0));

    // Deconnexion : la cle disparait -> la colonne aussi.
    rec.data.remove(USER_ID_KEY);
    sessions.save(&rec).await.unwrap();
    let col: Option<i64> = sqlx::query_scalar("SELECT user_id FROM session WHERE id = $1")
        .bind(rec.id.to_string())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(col, None);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn save_updates_data_and_expiry(pool: PgPool) {
    let sessions = PgStore::new(pool).sessions();
    let mut rec = record(None, time::Duration::minutes(1));
    sessions.create(&mut rec).await.unwrap();
    rec.data.insert("k".into(), serde_json::json!(42));
    rec.expiry_date = OffsetDateTime::now_utc() + time::Duration::days(1);
    sessions.save(&rec).await.unwrap();
    let loaded = sessions.load(&rec.id).await.unwrap().unwrap();
    assert_eq!(loaded.data["k"], 42);
    assert!(loaded.expiry_date > OffsetDateTime::now_utc() + time::Duration::hours(23));
    assert_eq!(sessions.count().await.unwrap(), 1, "pas de doublon");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn expired_sessions_are_invisible_and_purgeable(pool: PgPool) {
    let sessions = PgStore::new(pool).sessions();
    let mut live = record(None, time::Duration::hours(1));
    let mut dead = record(None, time::Duration::seconds(-5));
    sessions.create(&mut live).await.unwrap();
    sessions.create(&mut dead).await.unwrap();
    assert!(
        sessions.load(&dead.id).await.unwrap().is_none(),
        "expiree : invisible meme avant la purge"
    );
    assert!(sessions.load(&live.id).await.unwrap().is_some());
    assert_eq!(sessions.count().await.unwrap(), 2);
    sessions.delete_expired().await.unwrap();
    assert_eq!(sessions.count().await.unwrap(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn delete_removes_only_that_session(pool: PgPool) {
    let sessions = PgStore::new(pool).sessions();
    let (mut a, mut b) = (
        record(None, time::Duration::hours(1)),
        record(None, time::Duration::hours(1)),
    );
    sessions.create(&mut a).await.unwrap();
    sessions.create(&mut b).await.unwrap();
    sessions.delete(&a.id).await.unwrap();
    sessions.delete(&a.id).await.unwrap(); // idempotent
    assert!(sessions.load(&a.id).await.unwrap().is_none());
    assert!(sessions.load(&b.id).await.unwrap().is_some());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn delete_for_user_revokes_every_session_of_that_account(pool: PgPool) {
    let store = PgStore::new(pool);
    let (alice, bob) = (
        add_user(&store, "alice").await,
        add_user(&store, "bob").await,
    );
    let sessions = store.sessions();
    let mut mine = vec![];
    for who in [alice, alice, alice, bob] {
        let mut r = record(Some(who), time::Duration::hours(1));
        sessions.create(&mut r).await.unwrap();
        mine.push((who, r));
    }
    assert_eq!(sessions.delete_for_user(alice).await.unwrap(), 3);
    for (who, r) in mine {
        assert_eq!(
            sessions.load(&r.id).await.unwrap().is_some(),
            who == bob,
            "{who}"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_identifier_collision_gets_a_fresh_id(pool: PgPool) {
    let sessions = PgStore::new(pool).sessions();
    let mut first = record(None, time::Duration::hours(1));
    sessions.create(&mut first).await.unwrap();
    let taken = first.id;
    let mut second = record(None, time::Duration::hours(1));
    second.id = taken;
    sessions.create(&mut second).await.unwrap();
    assert_ne!(
        second.id, taken,
        "l'identifiant pris ne doit jamais etre reutilise"
    );
    assert_eq!(sessions.count().await.unwrap(), 2);
}

// ---------------------------------------------------------------- tentatives

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn failures_are_counted_per_pair_user_and_ip(pool: PgPool) {
    let attempts = PgStore::new(pool).attempts();
    for (user, ip) in [
        ("bob", "10.0.0.1"),
        ("bob", "10.0.0.1"),
        ("bob", "10.0.0.2"),
        ("alice", "10.0.0.1"),
    ] {
        attempts.record(user, ip, false).await.unwrap();
    }
    let s = attempts
        .stats("bob", "10.0.0.1", Utc::now() - Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!((s.failures_pair, s.failures_user, s.failures_ip), (2, 3, 3));
    assert!(s.last_failure_at.is_some());

    let other = attempts
        .stats("carol", "10.9.9.9", Utc::now() - Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!(
        (
            other.failures_pair,
            other.failures_user,
            other.failures_ip,
            other.last_failure_at
        ),
        (0, 0, 0, None)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn usernames_are_case_insensitive(pool: PgPool) {
    let attempts = PgStore::new(pool).attempts();
    attempts.record("Bob", "10.0.0.1", false).await.unwrap();
    attempts.record("BOB", "10.0.0.1", false).await.unwrap();
    let s = attempts
        .stats("bob", "10.0.0.1", Utc::now() - Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!(s.failures_user, 2);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_success_resets_account_counters_but_not_the_ip_counter(pool: PgPool) {
    let attempts = PgStore::new(pool).attempts();
    for _ in 0..3 {
        attempts.record("bob", "10.0.0.1", false).await.unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    attempts.record("bob", "10.0.0.1", true).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    attempts.record("bob", "10.0.0.1", false).await.unwrap();
    let s = attempts
        .stats("bob", "10.0.0.1", Utc::now() - Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!(
        (s.failures_pair, s.failures_user),
        (1, 1),
        "repart de zero apres la reussite"
    );
    assert_eq!(s.failures_ip, 4, "l'IP garde son historique d'echecs");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn failures_outside_the_window_are_ignored_and_can_be_purged(pool: PgPool) {
    let attempts = PgStore::new(pool.clone()).attempts();
    sqlx::query("INSERT INTO login_attempt (username, ip, succeeded, attempted_at) VALUES ('bob', '10.0.0.1', FALSE, now() - interval '2 hours')")
        .execute(&pool)
        .await
        .unwrap();
    attempts.record("bob", "10.0.0.1", false).await.unwrap();
    let s = attempts
        .stats("bob", "10.0.0.1", Utc::now() - Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!((s.failures_pair, s.failures_user, s.failures_ip), (1, 1, 1));

    let purged = attempts
        .purge_before(Utc::now() - Duration::hours(1))
        .await
        .unwrap();
    assert_eq!(purged, 1);
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM login_attempt")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(total, 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_success_never_counts_as_a_failure(pool: PgPool) {
    let attempts = PgStore::new(pool).attempts();
    for _ in 0..5 {
        attempts.record("bob", "10.0.0.1", true).await.unwrap();
    }
    let s = attempts
        .stats("bob", "10.0.0.1", Utc::now() - Duration::minutes(15))
        .await
        .unwrap();
    assert_eq!(
        (
            s.failures_pair,
            s.failures_user,
            s.failures_ip,
            s.last_failure_at
        ),
        (0, 0, 0, None)
    );
}
