// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests of the application settings: service (types, encrypted secrets, cache, seed) and `/api/settings` routes.

mod common;

use axum::http::{Method, StatusCode};
use common::{Client, PASSWORD, add_user, code, state};
use dominet_domain::registry::SETTINGS;
use dominet_domain::repo::{NavbarRepo, SettingRepo};
use dominet_domain::setting::SettingValue;
use dominet_domain::user::Role;
use dominet_server::commands::seed_data;
use dominet_server::secrets::MasterKey;
use dominet_server::settings::SettingsError;
use serde_json::json;
use sqlx::PgPool;

fn with_key() -> (String, [(&'static str, String); 1]) {
    let k = MasterKey::generate();
    (k.clone(), [("DOMINET_MASTER_KEY", k)])
}

/// Test state with (or without) a master key.
fn state_with(pool: PgPool, key: Option<&str>) -> dominet_server::AppState {
    match key {
        Some(k) => state(pool, &[("DOMINET_MASTER_KEY", k)]),
        None => state(pool, &[]),
    }
}

async fn raw_value(pool: &PgPool, key: &str) -> Option<String> {
    sqlx::query_scalar("SELECT value FROM app_setting WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
        .unwrap()
}

// ------------------------------------------------------------------ reading

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn unknown_settings_use_their_defaults_before_any_seed(pool: PgPool) {
    let st = state_with(pool, None);
    assert_eq!(
        st.settings.get("MOTION_THRESHOLD").await.unwrap(),
        SettingValue::Float(0.01)
    );
    assert_eq!(
        st.settings.get("MOTION_ALERT_COOLDOWN").await.unwrap(),
        SettingValue::Int(600)
    );
    assert_eq!(
        st.settings.get_string("Z2M_BASE_TOPIC").await.unwrap(),
        "zigbee2mqtt"
    );
    assert_eq!(
        st.settings.get_string("SMS_FREE_KEY").await.unwrap(),
        "",
        "secret non renseigne"
    );
    assert!(
        matches!(st.settings.get("INCONNU").await, Err(SettingsError::UnknownKey(k)) if k == "INCONNU")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn list_returns_the_thirteen_settings_sorted_with_their_origin(pool: PgPool) {
    let st = state_with(pool, None);
    let before = st.settings.list().await.unwrap();
    assert_eq!(before.len(), 17);
    assert!(
        before
            .iter()
            .all(|v| v.from_default && v.updated_at.is_none() && v.readable)
    );
    let order: Vec<_> = before.iter().map(|v| (v.category, v.key)).collect();
    let mut sorted = order.clone();
    sorted.sort_unstable();
    assert_eq!(order, sorted);

    st.settings.set("MOTION_THRESHOLD", "0.2").await.unwrap();
    let after = st.settings.list().await.unwrap();
    let v = after.iter().find(|v| v.key == "MOTION_THRESHOLD").unwrap();
    assert_eq!(
        (v.value.as_str(), v.from_default, v.updated_at.is_some()),
        ("0.2", false, true)
    );
}

// ------------------------------------------------------------------ writing

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn values_are_validated_against_their_type_and_normalized(pool: PgPool) {
    let st = state_with(pool.clone(), None);
    st.settings
        .set("MOTION_ALERT_COOLDOWN", " 300 ")
        .await
        .unwrap();
    assert_eq!(
        st.settings.get("MOTION_ALERT_COOLDOWN").await.unwrap(),
        SettingValue::Int(300)
    );
    assert_eq!(
        raw_value(&pool, "MOTION_ALERT_COOLDOWN").await.as_deref(),
        Some("300"),
        "normalise"
    );

    for (key, bad) in [
        ("MOTION_ALERT_COOLDOWN", "abc"),
        ("MOTION_ALERT_COOLDOWN", "1.5"),
        ("MOTION_ALERT_COOLDOWN", ""),
        ("MOTION_THRESHOLD", "0,05"),
        ("MOTION_THRESHOLD", "NaN"),
    ] {
        let err = st.settings.set(key, bad).await.unwrap_err();
        assert!(
            matches!(err, SettingsError::Invalid(_)),
            "{key}={bad:?} -> {err:?}"
        );
    }
    assert_eq!(
        raw_value(&pool, "MOTION_ALERT_COOLDOWN").await.as_deref(),
        Some("300"),
        "un refus ne modifie rien"
    );
    assert!(matches!(
        st.settings.set("INCONNU", "1").await,
        Err(SettingsError::UnknownKey(_))
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn secrets_are_encrypted_at_rest_and_masked_for_display(pool: PgPool) {
    let (key, _) = with_key();
    let st = state_with(pool.clone(), Some(&key));
    let view = st
        .settings
        .set("SMS_FREE_KEY", "cle-en-clair-12345")
        .await
        .unwrap();
    assert_eq!(
        (view.value.as_str(), view.is_secret, view.is_set),
        ("********", true, true)
    );
    assert!(
        !format!("{view:?}").contains("cle-en-clair"),
        "pas de clair dans le Debug"
    );

    let stored = raw_value(&pool, "SMS_FREE_KEY").await.unwrap();
    assert!(
        stored.starts_with("v1:") && !stored.contains("cle-en-clair"),
        "{stored}"
    );
    assert_eq!(
        st.settings.get_string("SMS_FREE_KEY").await.unwrap(),
        "cle-en-clair-12345",
        "dechiffre pour l'usage interne"
    );
    let listed = st.settings.list().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .find(|v| v.key == "SMS_FREE_KEY")
            .unwrap()
            .value,
        "********"
    );
    assert!(!format!("{listed:?}").contains("cle-en-clair"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_secret_cannot_be_written_without_the_master_key(pool: PgPool) {
    let st = state_with(pool.clone(), None);
    let err = st.settings.set("SMS_FREE_KEY", "valeur").await.unwrap_err();
    assert!(matches!(err, SettingsError::MasterKeyMissing), "{err:?}");
    assert_eq!(
        raw_value(&pool, "SMS_FREE_KEY").await,
        None,
        "rien n'est ecrit en clair"
    );
    // Clearing a secret has nothing to protect: allowed, and stored as is (no ciphertext of an empty string).
    let cleared = st.settings.set("SMS_FREE_KEY", "").await.unwrap();
    assert_eq!((cleared.value.as_str(), cleared.is_set), ("", false));
    assert_eq!(raw_value(&pool, "SMS_FREE_KEY").await.as_deref(), Some(""));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_changed_master_key_makes_only_the_secrets_unreadable(pool: PgPool) {
    let (key_a, _) = with_key();
    state_with(pool.clone(), Some(&key_a))
        .settings
        .set("SMS_FREE_KEY", "secret")
        .await
        .unwrap();
    state_with(pool.clone(), Some(&key_a))
        .settings
        .set("EMAIL_RECIPIENT", "moi@example.org")
        .await
        .unwrap();

    let other = state_with(pool, Some(&MasterKey::generate()));
    let sms = other
        .settings
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|v| v.key == "SMS_FREE_KEY")
        .unwrap();
    assert!(!sms.readable && sms.is_set);
    assert!(matches!(
        other.settings.get("SMS_FREE_KEY").await,
        Err(SettingsError::Unreadable(_))
    ));
    assert_eq!(
        other.settings.get_string("EMAIL_RECIPIENT").await.unwrap(),
        "moi@example.org",
        "les autres reglages vivent"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_ciphertext_copied_to_another_setting_is_refused(pool: PgPool) {
    let (key, _) = with_key();
    let st = state_with(pool.clone(), Some(&key));
    st.settings.set("SMS_FREE_KEY", "secret").await.unwrap();
    // An attacker (or an error) copies the ciphertext into another simulated secret setting.
    let stolen = raw_value(&pool, "SMS_FREE_KEY").await.unwrap();
    sqlx::query("UPDATE app_setting SET is_secret = TRUE, value = $1 WHERE key = 'SMS_FREE_KEY'")
        .bind(&stolen)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO app_setting (key, value, is_secret) VALUES ('SMS_FREE_USER', $1, TRUE)",
    )
    .bind(&stolen)
    .execute(&pool)
    .await
    .unwrap();
    st.settings.invalidate().await;
    assert!(st.settings.get_string("SMS_FREE_KEY").await.is_ok());
    assert!(
        matches!(
            st.settings.get("SMS_FREE_USER").await,
            Err(SettingsError::Unreadable(_))
        ),
        "donnee associee differente"
    );
}

// ------------------------------------------------------------------ cache

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn writes_update_the_cache_and_notify_subscribers(pool: PgPool) {
    let st = state_with(pool.clone(), None);
    let mut events = st.settings.subscribe();
    assert_eq!(
        st.settings.get_string("Z2M_BASE_TOPIC").await.unwrap(),
        "zigbee2mqtt"
    );

    st.settings.set("Z2M_BASE_TOPIC", "z2m").await.unwrap();
    assert_eq!(
        st.settings.get_string("Z2M_BASE_TOPIC").await.unwrap(),
        "z2m",
        "pas de lecture obsolete apres ecriture"
    );
    assert_eq!(events.try_recv().unwrap(), "Z2M_BASE_TOPIC");

    // A change made outside the service is only seen after invalidation.
    sqlx::query("UPDATE app_setting SET value = 'autre' WHERE key = 'Z2M_BASE_TOPIC'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        st.settings.get_string("Z2M_BASE_TOPIC").await.unwrap(),
        "z2m"
    );
    st.settings.invalidate().await;
    assert_eq!(
        st.settings.get_string("Z2M_BASE_TOPIC").await.unwrap(),
        "autre"
    );
}

// ------------------------------------------------------------------ seed

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn seed_creates_everything_once_and_never_overwrites(pool: PgPool) {
    let st = state_with(pool.clone(), None);
    let first = seed_data(&st.settings, &st.store, |_| None, false)
        .await
        .unwrap();
    assert_eq!(
        (
            first.settings_created,
            first.settings_existing,
            first.navbar_created,
            first.navbar_existing
        ),
        (17, 0, 7, 0)
    );
    assert_eq!(st.store.settings().list().await.unwrap().len(), 17);
    assert_eq!(st.store.navbar().list().await.unwrap().len(), 7);

    st.settings.set("MOTION_THRESHOLD", "0.5").await.unwrap();
    let second = seed_data(&st.settings, &st.store, |_| Some("ecrase".into()), false)
        .await
        .unwrap();
    assert_eq!(
        (
            second.settings_created,
            second.settings_existing,
            second.navbar_created,
            second.navbar_existing
        ),
        (0, 17, 0, 7)
    );
    assert_eq!(
        st.settings.get("MOTION_THRESHOLD").await.unwrap(),
        SettingValue::Float(0.5),
        "valeur modifiee conservee"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn seed_takes_initial_values_from_the_environment_and_encrypts_secrets(pool: PgPool) {
    let (key, _) = with_key();
    let st = state_with(pool.clone(), Some(&key));
    let env = |k: &str| match k {
        "EMAIL_RECIPIENT" => Some("moi@example.org".to_string()),
        "SMS_FREE_KEY" => Some("cle-initiale".to_string()),
        "MOTION_ALERT_COOLDOWN" => Some("120".to_string()),
        _ => None,
    };
    seed_data(&st.settings, &st.store, env, false)
        .await
        .unwrap();
    assert_eq!(
        st.settings.get_string("EMAIL_RECIPIENT").await.unwrap(),
        "moi@example.org"
    );
    assert_eq!(
        st.settings.get("MOTION_ALERT_COOLDOWN").await.unwrap(),
        SettingValue::Int(120)
    );
    assert_eq!(
        st.settings.get_string("SMS_FREE_KEY").await.unwrap(),
        "cle-initiale"
    );
    let stored = raw_value(&pool, "SMS_FREE_KEY").await.unwrap();
    assert!(stored.starts_with("v1:") && !stored.contains("cle-initiale"));
    assert_eq!(
        raw_value(&pool, "DEFAULT_FROM_EMAIL").await.as_deref(),
        Some("dominet@localhost"),
        "defaut pour le reste"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_dry_run_reports_but_writes_nothing(pool: PgPool) {
    let st = state_with(pool.clone(), None);
    let r = seed_data(&st.settings, &st.store, |_| None, true)
        .await
        .unwrap();
    assert_eq!((r.settings_created, r.navbar_created), (17, 7));
    assert_eq!(st.store.settings().list().await.unwrap().len(), 0);
    assert_eq!(st.store.navbar().list().await.unwrap().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_failing_seed_writes_nothing_at_all(pool: PgPool) {
    // Secret supplied without a master key: refused, and none of the other settings is written either.
    let st = state_with(pool.clone(), None);
    let err = seed_data(
        &st.settings,
        &st.store,
        |k| (k == "SMS_FREE_KEY").then(|| "secret".to_string()),
        false,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("DOMINET_MASTER_KEY"), "{err}");
    assert_eq!(st.store.settings().list().await.unwrap().len(), 0);
    assert_eq!(st.store.navbar().list().await.unwrap().len(), 0);

    // Invalid-type value in the environment: same behaviour.
    let err = seed_data(
        &st.settings,
        &st.store,
        |k| (k == "MOTION_THRESHOLD").then(|| "beaucoup".to_string()),
        false,
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("decimal"), "{err}");
    assert_eq!(st.store.settings().list().await.unwrap().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn seed_does_not_resurrect_navigation_the_admin_removed(pool: PgPool) {
    let st = state_with(pool, None);
    seed_data(&st.settings, &st.store, |_| None, false)
        .await
        .unwrap();
    let items = st.store.navbar().list().await.unwrap();
    let android = items.iter().find(|i| i.label == "Android").unwrap();
    st.store.navbar().delete(android.id).await.unwrap();
    let again = seed_data(&st.settings, &st.store, |_| None, false)
        .await
        .unwrap();
    assert_eq!((again.navbar_created, again.navbar_existing), (0, 6));
    assert!(
        st.store
            .navbar()
            .list()
            .await
            .unwrap()
            .iter()
            .all(|i| i.label != "Android")
    );
}

// ------------------------------------------------------------------ routes

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn settings_routes_are_reserved_to_administrators(pool: PgPool) {
    let st = state_with(pool, None);
    add_user(&st, "root", Role::Admin).await;
    add_user(&st, "bob", Role::User).await;
    assert_eq!(
        Client::new(&st).get("/api/settings").await.status,
        StatusCode::UNAUTHORIZED
    );

    let mut user = Client::new(&st);
    user.login("bob", PASSWORD).await;
    assert_eq!(
        user.get("/api/settings").await.status,
        StatusCode::FORBIDDEN
    );
    let put = user
        .send(
            Method::PUT,
            "/api/settings/MOTION_THRESHOLD",
            Some(json!({"value": "0.3"})),
            &[],
        )
        .await;
    assert_eq!(put.status, StatusCode::FORBIDDEN);

    let mut admin = Client::new(&st);
    admin.login("root", PASSWORD).await;
    let list = admin.get("/api/settings").await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body.as_array().unwrap().len(), SETTINGS.len());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_admin_can_update_a_setting_and_gets_clear_errors(pool: PgPool) {
    let st = state_with(pool, None);
    add_user(&st, "root", Role::Admin).await;
    let mut admin = Client::new(&st);
    admin.login("root", PASSWORD).await;

    let ok = admin
        .send(
            Method::PUT,
            "/api/settings/MOTION_THRESHOLD",
            Some(json!({"value": "0.2"})),
            &[],
        )
        .await;
    assert_eq!(
        (
            ok.status,
            ok.body["value"].as_str(),
            ok.body["from_default"].as_bool()
        ),
        (StatusCode::OK, Some("0.2"), Some(false))
    );
    assert_eq!(
        st.settings.get("MOTION_THRESHOLD").await.unwrap(),
        SettingValue::Float(0.2)
    );

    let bad = admin
        .send(
            Method::PUT,
            "/api/settings/MOTION_THRESHOLD",
            Some(json!({"value": "beaucoup"})),
            &[],
        )
        .await;
    assert_eq!(
        (bad.status, code(&bad)),
        (StatusCode::BAD_REQUEST, "bad_request")
    );
    let unknown = admin
        .send(
            Method::PUT,
            "/api/settings/INCONNU",
            Some(json!({"value": "1"})),
            &[],
        )
        .await;
    assert_eq!(
        (unknown.status, code(&unknown)),
        (StatusCode::NOT_FOUND, "not_found")
    );
    // Foreign origin: refused before any write.
    let csrf = admin
        .send(
            Method::PUT,
            "/api/settings/MOTION_THRESHOLD",
            Some(json!({"value": "0.9"})),
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(csrf.status, StatusCode::FORBIDDEN);
    assert_eq!(
        st.settings.get("MOTION_THRESHOLD").await.unwrap(),
        SettingValue::Float(0.2)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn secrets_never_travel_back_to_the_client(pool: PgPool) {
    let (key, _) = with_key();
    let st = state_with(pool, Some(&key));
    add_user(&st, "root", Role::Admin).await;
    let mut admin = Client::new(&st);
    admin.login("root", PASSWORD).await;

    let put = admin
        .send(
            Method::PUT,
            "/api/settings/SMS_FREE_KEY",
            Some(json!({"value": "valeur-ultra-secrete"})),
            &[],
        )
        .await;
    assert_eq!(put.status, StatusCode::OK);
    assert_eq!(
        (
            put.body["value"].as_str(),
            put.body["is_secret"].as_bool(),
            put.body["is_set"].as_bool()
        ),
        (Some("********"), Some(true), Some(true))
    );
    let list = admin.get("/api/settings").await;
    for body in [put.body.to_string(), list.body.to_string()] {
        assert!(
            !body.contains("valeur-ultra-secrete") && !body.contains("v1:"),
            "{body}"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn writing_a_secret_without_a_master_key_is_a_service_error_not_a_crash(pool: PgPool) {
    let st = state_with(pool, None);
    add_user(&st, "root", Role::Admin).await;
    let mut admin = Client::new(&st);
    admin.login("root", PASSWORD).await;
    let r = admin
        .send(
            Method::PUT,
            "/api/settings/SMS_FREE_KEY",
            Some(json!({"value": "x"})),
            &[],
        )
        .await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::SERVICE_UNAVAILABLE, "unavailable")
    );
    assert!(
        r.body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("DOMINET_MASTER_KEY")
    );
}
