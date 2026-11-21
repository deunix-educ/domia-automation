// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests du coeur des commandes `createsuperuser` et `passwd` (base isolee par test).

use dominet_domain::RepoError;
use dominet_domain::auth::LockoutPolicy;
use dominet_domain::repo::UserRepo;
use dominet_domain::user::Role;
use dominet_server::auth::password::{Hasher, Verification};
use dominet_server::auth::service::{AuthError, AuthService};
use dominet_server::commands::{change_password, create_superuser};
use dominet_store::{PgStore, USER_ID_KEY};
use sqlx::PgPool;
use tower_sessions::session::{Id, Record};
use tower_sessions::session_store::SessionStore;

const PASSWORD: &str = "correct horse battery";

fn service(pool: PgPool) -> (AuthService, PgStore) {
    let store = PgStore::new(pool);
    (
        AuthService::new(
            store.clone(),
            Hasher::with_params(1024, 1, 1).unwrap(),
            LockoutPolicy::default(),
        ),
        store,
    )
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn createsuperuser_makes_an_active_admin_with_a_hashed_password(pool: PgPool) {
    let (auth, store) = service(pool);
    let user = create_superuser(&auth, "admin", "admin@example.org", PASSWORD)
        .await
        .unwrap();
    assert_eq!(
        (user.role, user.is_active, user.email.as_str()),
        (Role::Admin, true, "admin@example.org")
    );

    let creds = store
        .users()
        .find_credentials("admin")
        .await
        .unwrap()
        .unwrap();
    assert!(creds.password_hash.starts_with("$argon2id$"));
    assert!(!creds.password_hash.contains(PASSWORD));
    assert!(matches!(
        Hasher::with_params(1024, 1, 1)
            .unwrap()
            .verify(PASSWORD, &creds.password_hash),
        Verification::Valid { .. }
    ));
    assert_eq!(store.users().count_active_admins().await.unwrap(), 1);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn createsuperuser_enforces_the_name_and_password_rules(pool: PgPool) {
    let (auth, _) = service(pool);
    for (name, pw) in [
        ("a b", PASSWORD),
        ("", PASSWORD),
        ("admin", "court"),
        ("admin", "1234567890123"),
        ("admin", "ADMIN"),
    ] {
        let err = create_superuser(&auth, name, "", pw).await.unwrap_err();
        assert!(
            matches!(err, AuthError::Invalid(_)),
            "{name:?}/{pw:?} -> {err:?}"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn createsuperuser_refuses_an_existing_name_whatever_the_case(pool: PgPool) {
    let (auth, _) = service(pool);
    create_superuser(&auth, "Admin", "", PASSWORD)
        .await
        .unwrap();
    let err = create_superuser(&auth, "ADMIN", "", PASSWORD)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AuthError::Repo(RepoError::Conflict { .. })),
        "{err:?}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn passwd_changes_the_hash_and_revokes_the_sessions_of_that_account_only(pool: PgPool) {
    let (auth, store) = service(pool);
    let bob = auth
        .create_user("bob", "", "", Role::User, PASSWORD)
        .await
        .unwrap();
    let alice = auth
        .create_user("alice", "", "", Role::User, PASSWORD)
        .await
        .unwrap();
    let sessions = store.sessions();
    let mut ids = vec![];
    for who in [bob.id, alice.id] {
        let mut rec = Record {
            id: Id::default(),
            data: std::collections::HashMap::from([(
                USER_ID_KEY.to_string(),
                serde_json::json!(who.0),
            )]),
            expiry_date: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
        };
        sessions.create(&mut rec).await.unwrap();
        ids.push(rec.id);
    }

    let before = store
        .users()
        .find_credentials("bob")
        .await
        .unwrap()
        .unwrap();
    change_password(&auth, &store, "BOB", "un tout nouveau mot de passe")
        .await
        .unwrap();
    let after = store
        .users()
        .find_credentials("bob")
        .await
        .unwrap()
        .unwrap();
    assert_ne!(before.password_hash, after.password_hash);
    assert!(after.user.password_changed_at >= before.user.password_changed_at);

    assert!(
        sessions.load(&ids[0]).await.unwrap().is_none(),
        "sessions de bob revoquees"
    );
    assert!(
        sessions.load(&ids[1]).await.unwrap().is_some(),
        "celles d'alice intactes"
    );
    // Et le nouveau mot de passe fonctionne a la connexion.
    assert!(
        auth.authenticate("bob", "un tout nouveau mot de passe", "10.0.0.1")
            .await
            .is_ok()
    );
    assert!(matches!(
        auth.authenticate("bob", PASSWORD, "10.0.0.1").await,
        Err(AuthError::InvalidCredentials)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn passwd_rejects_unknown_accounts_and_weak_passwords(pool: PgPool) {
    let (auth, store) = service(pool);
    auth.create_user("bob", "", "", Role::User, PASSWORD)
        .await
        .unwrap();
    let err = change_password(&auth, &store, "personne", PASSWORD)
        .await
        .unwrap_err();
    assert!(
        matches!(err, AuthError::Repo(RepoError::NotFound)),
        "{err:?}"
    );
    let err = change_password(&auth, &store, "bob", "court")
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::Invalid(_)), "{err:?}");
    assert!(
        auth.authenticate("bob", PASSWORD, "10.0.0.1").await.is_ok(),
        "l'ancien mot de passe est conserve"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_outdated_hash_is_upgraded_transparently_at_login(pool: PgPool) {
    let store = PgStore::new(pool);
    let weak = Hasher::with_params(1024, 1, 1).unwrap();
    let old = AuthService::new(store.clone(), weak, LockoutPolicy::default());
    old.create_user("bob", "", "", Role::User, PASSWORD)
        .await
        .unwrap();
    let before = store
        .users()
        .find_credentials("bob")
        .await
        .unwrap()
        .unwrap()
        .password_hash;

    let current = AuthService::new(
        store.clone(),
        Hasher::with_params(2048, 2, 1).unwrap(),
        LockoutPolicy::default(),
    );
    current
        .authenticate("bob", PASSWORD, "10.0.0.1")
        .await
        .unwrap();
    let after = store
        .users()
        .find_credentials("bob")
        .await
        .unwrap()
        .unwrap()
        .password_hash;
    assert_ne!(before, after, "condensat remplace");
    assert!(after.contains("m=2048,t=2,p=1"), "{after}");
    assert!(
        current
            .authenticate("bob", PASSWORD, "10.0.0.1")
            .await
            .is_ok(),
        "le nouveau condensat fonctionne"
    );
}
