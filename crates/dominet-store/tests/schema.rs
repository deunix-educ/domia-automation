// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests du schema PostgreSQL : tables, contraintes, cascades, valeurs par defaut, declencheurs.
//!
//! `#[sqlx::test]` cree une base isolee par test et y applique les migrations
//! (`DATABASE_URL` = stack de dev : `make dev-up`).

use sqlx::PgPool;

const UNIQUE: &str = "23505";
const CHECK: &str = "23514";
const FOREIGN_KEY: &str = "23503";
const EXCLUSION: &str = "23P01";

/// Code SQLSTATE d'une erreur de base ("" si ce n'en est pas une).
fn code(err: &sqlx::Error) -> String {
    err.as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(|c| c.to_string())
        .unwrap_or_default()
}

async fn add_user(pool: &PgPool, username: &str) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO users (username, password_hash) VALUES ($1, 'hash') RETURNING id",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn count(pool: &PgPool, table: &str) -> i64 {
    // `table` vient de constantes du test : SQL dynamique audite.
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn all_expected_tables_exist(pool: PgPool) {
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name::text FROM information_schema.tables \
         WHERE table_schema = 'public' AND table_name <> '_sqlx_migrations' ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let expected = [
        "alert",
        "alert_rule",
        "android_device",
        "app_setting",
        "automation",
        "automation_run",
        "call_log",
        "camera",
        "dashboard",
        "detection_event",
        "detector_config",
        "device",
        "device_group",
        "device_image",
        "device_label",
        "device_state_log",
        "energy_daily_log",
        "grafana_alert_rule",
        "location",
        "login_attempt",
        "mcp_audit",
        "mcp_token",
        "message",
        "message_recipient",
        "motion_event",
        "mqtt_source",
        "navbar_item",
        "prometheus_rule",
        "ptz_preset",
        "recording",
        "scene",
        "scene_action",
        "schedule_job",
        "session",
        "sms_message",
        "sms_outbox",
        "snapshot",
        "users",
        "weather_station",
        "widget",
    ];
    assert_eq!(tables, expected);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn migrations_can_be_rerun_without_effect(pool: PgPool) {
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    dominet_store::migrate(&pool).await.unwrap();
    dominet_store::migrate(&pool).await.unwrap();
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        usize::try_from(after).unwrap(),
        dominet_store::MIGRATOR.iter().count()
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn users_defaults_and_case_insensitive_username(pool: PgPool) {
    add_user(&pool, "Bob").await;
    let (role, active, mobile): (String, bool, String) =
        sqlx::query_as("SELECT role, is_active, mobile FROM users WHERE username = 'Bob'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((role.as_str(), active, mobile.as_str()), ("user", true, ""));

    let dup = sqlx::query("INSERT INTO users (username, password_hash) VALUES ('bOB', 'x')")
        .execute(&pool)
        .await;
    assert_eq!(
        code(&dup.unwrap_err()),
        UNIQUE,
        "Bob et bOB doivent etre le meme compte"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn users_reject_unknown_role_and_empty_username(pool: PgPool) {
    let bad_role =
        sqlx::query("INSERT INTO users (username, password_hash, role) VALUES ('a', 'h', 'root')")
            .execute(&pool)
            .await;
    assert_eq!(code(&bad_role.unwrap_err()), CHECK);
    let empty = sqlx::query("INSERT INTO users (username, password_hash) VALUES ('', 'h')")
        .execute(&pool)
        .await;
    assert_eq!(code(&empty.unwrap_err()), CHECK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn updated_at_trigger_fires_on_update(pool: PgPool) {
    let id = add_user(&pool, "alice").await;
    sqlx::query("UPDATE users SET updated_at = now() - interval '1 day' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    // Le declencheur remet updated_at a now() quel que soit le contenu de la mise a jour.
    sqlx::query("UPDATE users SET mobile = '+33612345678' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    let fresh: bool = sqlx::query_scalar(
        "SELECT updated_at > now() - interval '1 minute' FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(fresh);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deleting_a_user_revokes_its_sessions_and_removes_its_data(pool: PgPool) {
    let id = add_user(&pool, "carol").await;
    sqlx::query("INSERT INTO session (id, data, expiry_date, user_id) VALUES ('s1', '{\"k\":1}', now() + interval '1 day', $1)")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO session (id, expiry_date) VALUES ('anon', now() + interval '1 day')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO android_device (owner_id, name, token_hash) VALUES ($1, 'tel', repeat('a', 64))")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "session").await,
        1,
        "seule la session anonyme survit"
    );
    assert_eq!(count(&pool, "android_device").await, 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn app_setting_key_is_unique_and_type_is_checked(pool: PgPool) {
    sqlx::query("INSERT INTO app_setting (key, value, value_type) VALUES ('MOTION_THRESHOLD', '0.01', 'float')")
        .execute(&pool)
        .await
        .unwrap();
    let dup = sqlx::query("INSERT INTO app_setting (key, value) VALUES ('MOTION_THRESHOLD', '1')")
        .execute(&pool)
        .await;
    assert_eq!(code(&dup.unwrap_err()), UNIQUE);
    let bad =
        sqlx::query("INSERT INTO app_setting (key, value, value_type) VALUES ('X', '1', 'json')")
            .execute(&pool)
            .await;
    assert_eq!(code(&bad.unwrap_err()), CHECK);
    let secret: bool =
        sqlx::query_scalar("SELECT is_secret FROM app_setting WHERE key = 'MOTION_THRESHOLD'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!secret, "non secret par defaut");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn navbar_item_defaults_and_ordering(pool: PgPool) {
    for (pos, order, label) in [
        ("bottom", 5, "Android"),
        ("top", 2, "Automation"),
        ("top", 1, "Surveillance"),
    ] {
        sqlx::query("INSERT INTO navbar_item (icon_name, label, url, position, sort_order) VALUES ('x', $1, '/', $2, $3)")
            .bind(label)
            .bind(pos)
            .bind(order)
            .execute(&pool)
            .await
            .unwrap();
    }
    let labels: Vec<String> =
        sqlx::query_scalar("SELECT label FROM navbar_item ORDER BY position, sort_order, label")
            .fetch_all(&pool)
            .await
            .unwrap();
    // 'bottom' < 'top' alphabetiquement ; l'ordre affiche est decide par l'application (haut puis bas).
    assert_eq!(labels, ["Android", "Surveillance", "Automation"]);
    let bad = sqlx::query(
        "INSERT INTO navbar_item (icon_name, label, url, position) VALUES ('x', 'y', '/', 'left')",
    )
    .execute(&pool)
    .await;
    assert_eq!(code(&bad.unwrap_err()), CHECK);
    let (target, admin): (String, bool) =
        sqlx::query_as("SELECT url_target, admin_only FROM navbar_item LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((target.as_str(), admin), ("_self", false));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn location_is_active_by_default(pool: PgPool) {
    sqlx::query("INSERT INTO location (name) VALUES ('Salon')")
        .execute(&pool)
        .await
        .unwrap();
    let active: bool = sqlx::query_scalar("SELECT active FROM location")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(active);
    let long =
        sqlx::query("INSERT INTO location (name, description) VALUES ('X', repeat('a', 129))")
            .execute(&pool)
            .await;
    assert_eq!(code(&long.unwrap_err()), CHECK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn messaging_constraints_and_cascades(pool: PgPool) {
    let (alice, bob) = (add_user(&pool, "alice").await, add_user(&pool, "bob").await);
    let msg: i64 = sqlx::query_scalar(
        "INSERT INTO message (sender_id, subject, body) VALUES ($1, 'Salut', 'corps') RETURNING id",
    )
    .bind(alice)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (channel, status): (String, String) =
        sqlx::query_as("SELECT channel, status FROM message WHERE id = $1")
            .bind(msg)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((channel.as_str(), status.as_str()), ("email", "draft"));

    sqlx::query("INSERT INTO message_recipient (message_id, recipient_id) VALUES ($1, $2)")
        .bind(msg)
        .bind(bob)
        .execute(&pool)
        .await
        .unwrap();
    let dup =
        sqlx::query("INSERT INTO message_recipient (message_id, recipient_id) VALUES ($1, $2)")
            .bind(msg)
            .bind(bob)
            .execute(&pool)
            .await;
    assert_eq!(
        code(&dup.unwrap_err()),
        UNIQUE,
        "un destinataire une seule fois par message"
    );
    let bad = sqlx::query("UPDATE message_recipient SET sms_status = 'lost'")
        .execute(&pool)
        .await;
    assert_eq!(code(&bad.unwrap_err()), CHECK);
    let bad_channel = sqlx::query("UPDATE message SET channel = 'fax'")
        .execute(&pool)
        .await;
    assert_eq!(code(&bad_channel.unwrap_err()), CHECK);
    let orphan =
        sqlx::query("INSERT INTO message_recipient (message_id, recipient_id) VALUES (999999, $1)")
            .bind(bob)
            .execute(&pool)
            .await;
    assert_eq!(code(&orphan.unwrap_err()), FOREIGN_KEY);

    sqlx::query("DELETE FROM message WHERE id = $1")
        .bind(msg)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        count(&pool, "message_recipient").await,
        0,
        "les destinataires suivent le message"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn android_token_is_hashed_unique_and_cascades(pool: PgPool) {
    let owner = add_user(&pool, "dave").await;
    let mut hashes = vec![];
    for (name, token) in [
        ("tel-1", "0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b"),
        ("tel-2", "9d1e2f3a-4b5c-4d6e-8f70-8192a3b4c5d6"),
    ] {
        let h: String = sqlx::query_scalar(
            "INSERT INTO android_device (owner_id, name, token_hash) VALUES ($1, $2, encode(sha256(convert_to($3, 'UTF8')), 'hex')) RETURNING token_hash",
        )
        .bind(owner)
        .bind(name)
        .bind(token)
        .fetch_one(&pool)
        .await
        .unwrap();
        hashes.push(h);
    }
    assert_ne!(hashes[0], hashes[1]);
    let dup =
        sqlx::query("INSERT INTO android_device (owner_id, name, token_hash) VALUES ($1, 'x', $2)")
            .bind(owner)
            .bind(&hashes[0])
            .execute(&pool)
            .await;
    assert_eq!(code(&dup.unwrap_err()), UNIQUE);
    let clear = sqlx::query("INSERT INTO android_device (owner_id, name, token_hash) VALUES ($1, 'y', '0b9a0f0e-1c5e-4d1c-8f0c-7a1f3e2d4c5b')")
        .bind(owner)
        .execute(&pool)
        .await;
    assert_eq!(
        code(&clear.unwrap_err()),
        CHECK,
        "un jeton en clair n'est pas une empreinte"
    );

    let dev: i64 = sqlx::query_scalar("SELECT id FROM android_device WHERE name = 'tel-1'")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO sms_message (device_id, direction, phone_number, body, sent_at) VALUES ($1, 'incoming', '+33600000000', 'hi', now())")
        .bind(dev)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO call_log (device_id, direction, phone_number, started_at) VALUES ($1, 'missed', '+33600000000', now())")
        .bind(dev)
        .execute(&pool)
        .await
        .unwrap();
    let bad_dir = sqlx::query("INSERT INTO sms_message (device_id, direction, phone_number, body, sent_at) VALUES ($1, 'missed', '1', 'x', now())").bind(dev).execute(&pool).await;
    assert_eq!(
        code(&bad_dir.unwrap_err()),
        CHECK,
        "'missed' n'existe que pour les appels"
    );
    let negative = sqlx::query("UPDATE call_log SET duration = -1")
        .execute(&pool)
        .await;
    assert_eq!(code(&negative.unwrap_err()), CHECK);

    sqlx::query("DELETE FROM android_device WHERE id = $1")
        .bind(dev)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(count(&pool, "sms_message").await, 0);
    assert_eq!(count(&pool, "call_log").await, 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn schedule_job_requires_the_fields_of_its_kind(pool: PgPool) {
    let ok = [
        "('every-5-min', 'check', 'interval', 5, 'minutes', NULL, NULL, NULL, NULL)",
        "('backup', 'run_backup', 'cron', NULL, NULL, '0 4 * * *', NULL, NULL, NULL)",
        "('sunset', 'scene', 'solar', NULL, NULL, NULL, 'sunset', 43.8, 2.3)",
    ];
    for row in ok {
        let sql = format!(
            "INSERT INTO schedule_job (name, task, kind, interval_every, interval_period, cron_expr, solar_event, latitude, longitude) VALUES {row}"
        );
        sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{row} : {e}"));
    }
    let bad = [
        "('a', 't', 'interval', NULL, NULL, NULL, NULL, NULL, NULL)", // intervalle sans periode
        "('b', 't', 'interval', 0, 'minutes', NULL, NULL, NULL, NULL)", // intervalle nul
        "('c', 't', 'cron', NULL, NULL, '', NULL, NULL, NULL)",       // cron vide
        "('d', 't', 'solar', NULL, NULL, NULL, 'sunset', NULL, NULL)", // solaire sans position
        "('e', 't', 'solar', NULL, NULL, NULL, 'sunset', 91, 2.3)",   // latitude hors bornes
        "('f', 't', 'weekly', NULL, NULL, NULL, NULL, NULL, NULL)",   // type inconnu
    ];
    for row in bad {
        let sql = format!(
            "INSERT INTO schedule_job (name, task, kind, interval_every, interval_period, cron_expr, solar_event, latitude, longitude) VALUES {row}"
        );
        let err = sqlx::query(sqlx::AssertSqlSafe(sql))
            .execute(&pool)
            .await
            .expect_err(row);
        assert_eq!(code(&err), CHECK, "{row}");
    }
    let dup = sqlx::query("INSERT INTO schedule_job (name, task, kind, interval_every, interval_period) VALUES ('backup', 't', 'interval', 1, 'days')")
        .execute(&pool)
        .await;
    assert_eq!(code(&dup.unwrap_err()), UNIQUE, "nom de job unique");
    assert_eq!(count(&pool, "schedule_job").await, 3);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn login_attempts_are_recorded(pool: PgPool) {
    sqlx::query(
        "INSERT INTO login_attempt (username, ip, succeeded) VALUES ('bob', '192.168.1.10', FALSE)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let failures: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM login_attempt WHERE username = 'bob' AND NOT succeeded AND attempted_at > now() - interval '15 minutes'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(failures, 1);
}

// ---------------------------------------------------------------- tableaux de bord et widgets

async fn add_dashboard(pool: &PgPool, owner: i64, name: &str, is_default: bool) -> i64 {
    sqlx::query_scalar(
        "INSERT INTO dashboard (name, owner_id, is_default) VALUES ($1, $2, $3) RETURNING id",
    )
    .bind(name)
    .bind(owner)
    .bind(is_default)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Insere un widget `text` aux coordonnees donnees ; renvoie le resultat brut pour tester les refus.
async fn put_widget(
    pool: &PgPool,
    dashboard: i64,
    col: i16,
    row: i16,
    width: i16,
    height: i16,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "INSERT INTO widget (dashboard_id, widget_type, grid_col, grid_row, width, height)
         VALUES ($1, 'text', $2, $3, $4, $5) RETURNING id",
    )
    .bind(dashboard)
    .bind(col)
    .bind(row)
    .bind(width)
    .bind(height)
    .fetch_one(pool)
    .await
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_user_has_at_most_one_default_dashboard_and_names_are_bounded(pool: PgPool) {
    let (bob, eve) = (add_user(&pool, "bob").await, add_user(&pool, "eve").await);
    add_dashboard(&pool, bob, "Salon", true).await;
    add_dashboard(&pool, bob, "Cuisine", false).await;
    add_dashboard(&pool, eve, "Salon", true).await; // un autre compte a son propre defaut
    let second_default = sqlx::query(
        "INSERT INTO dashboard (name, owner_id, is_default) VALUES ('Autre', $1, TRUE)",
    )
    .bind(bob)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(code(&second_default), UNIQUE);
    for name in ["", &"x".repeat(65)] {
        let err = sqlx::query("INSERT INTO dashboard (name, owner_id) VALUES ($1, $2)")
            .bind(name)
            .bind(bob)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(code(&err), CHECK, "{name:?}");
    }
    let orphan = sqlx::query("INSERT INTO dashboard (name, owner_id) VALUES ('X', 424242)")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&orphan), FOREIGN_KEY, "le proprietaire doit exister");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn widget_defaults_and_value_checks(pool: PgPool) {
    let owner = add_user(&pool, "bob").await;
    let dash = add_dashboard(&pool, owner, "Salon", true).await;
    let id = put_widget(&pool, dash, 0, 0, 2, 2).await.unwrap();
    let (title, pres, config, content): (String, String, serde_json::Value, String) =
        sqlx::query_as("SELECT title, presentation, config, content FROM widget WHERE id = $1")
            .bind(id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (title.as_str(), pres.as_str(), content.as_str()),
        ("", "", "")
    );
    assert_eq!(config, serde_json::json!({}));
    let defaults: (i16, i16) = sqlx::query_as("SELECT width, height FROM widget WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(defaults, (2, 2));

    let bad = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            code(
                &sqlx::query(sql)
                    .bind(dash)
                    .execute(&pool)
                    .await
                    .unwrap_err(),
            )
        }
    };
    assert_eq!(
        bad("INSERT INTO widget (dashboard_id, widget_type) VALUES ($1, 'meteo')").await,
        CHECK,
        "type inconnu"
    );
    assert_eq!(
        bad("INSERT INTO widget (dashboard_id, widget_type, grid_col) VALUES ($1, 'text', 12)")
            .await,
        CHECK
    );
    assert_eq!(
        bad("INSERT INTO widget (dashboard_id, widget_type, grid_row) VALUES ($1, 'text', 1000)")
            .await,
        CHECK
    );
    assert_eq!(
        bad("INSERT INTO widget (dashboard_id, widget_type, width) VALUES ($1, 'text', 13)").await,
        CHECK
    );
    assert_eq!(
        bad("INSERT INTO widget (dashboard_id, widget_type, height) VALUES ($1, 'text', 0)").await,
        CHECK
    );
    assert_eq!(bad("INSERT INTO widget (dashboard_id, widget_type, grid_col, width) VALUES ($1, 'text', 11, 2)").await, CHECK, "deborde de la grille");
    assert_eq!(bad("INSERT INTO widget (dashboard_id, widget_type, config) VALUES ($1, 'text', '[1]'::jsonb)").await, CHECK, "config = objet");
    assert_eq!(
        bad("INSERT INTO widget (dashboard_id, widget_type, device_id) VALUES ($1, 'device', 0)")
            .await,
        CHECK
    );
    assert_eq!(bad("INSERT INTO widget (dashboard_id, widget_type, content) VALUES ($1, 'text', repeat('x', 20001))").await, CHECK);
    assert_eq!(bad("INSERT INTO widget (dashboard_id, widget_type, title) VALUES ($1, 'text', repeat('x', 65))").await, CHECK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn widgets_of_one_dashboard_never_overlap_but_may_touch(pool: PgPool) {
    let owner = add_user(&pool, "bob").await;
    let (a, b) = (
        add_dashboard(&pool, owner, "A", true).await,
        add_dashboard(&pool, owner, "B", false).await,
    );
    put_widget(&pool, a, 0, 0, 2, 2).await.unwrap();
    put_widget(&pool, a, 2, 0, 2, 2)
        .await
        .expect("contigu a droite");
    put_widget(&pool, a, 0, 2, 2, 2)
        .await
        .expect("contigu dessous");
    for (col, row, w, h) in [(1, 1, 2, 2), (0, 0, 1, 1), (3, 1, 1, 1), (0, 0, 12, 12)] {
        let err = put_widget(&pool, a, col, row, w, h).await.unwrap_err();
        assert_eq!(code(&err), EXCLUSION, "({col},{row}) {w}x{h}");
    }
    // Un autre tableau a sa propre grille.
    put_widget(&pool, b, 0, 0, 2, 2)
        .await
        .expect("autre tableau");
    // Un deplacement sur une case occupee est refuse aussi, et la mise a jour de soi-meme ne se heurte pas a elle-meme.
    let id = put_widget(&pool, a, 6, 0, 2, 2).await.unwrap();
    let err = sqlx::query("UPDATE widget SET grid_col = 1 WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(code(&err), EXCLUSION);
    sqlx::query("UPDATE widget SET grid_col = 7, title = 'x' WHERE id = $1")
        .bind(id)
        .execute(&pool)
        .await
        .expect("deplacement libre");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_camera_reference_has_no_foreign_key_until_its_module_exists(pool: PgPool) {
    let owner = add_user(&pool, "bob").await;
    let dash = add_dashboard(&pool, owner, "A", true).await;
    // `device` (R3) et `scene` (R4) ont leur cle etrangere (voir `devices.rs` et `automation.rs`) ; `camera` (R6) pas encore.
    assert!(sqlx::query("INSERT INTO widget (dashboard_id, widget_type, device_id, grid_col) VALUES ($1, 'device', 424242, 0)")
        .bind(dash).execute(&pool).await.is_err(), "appareil inexistant");
    assert!(sqlx::query("INSERT INTO widget (dashboard_id, widget_type, scene_id, grid_col) VALUES ($1, 'scene', 424242, 4)")
        .bind(dash).execute(&pool).await.is_err(), "scene inexistante");
    sqlx::query("INSERT INTO widget (dashboard_id, widget_type, camera_uuid, grid_col) VALUES ($1, 'camera', gen_random_uuid(), 8)")
        .bind(dash).execute(&pool).await.unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deleting_a_user_or_a_dashboard_removes_its_widgets(pool: PgPool) {
    let (bob, eve) = (add_user(&pool, "bob").await, add_user(&pool, "eve").await);
    let (d1, d2) = (
        add_dashboard(&pool, bob, "A", true).await,
        add_dashboard(&pool, eve, "B", true).await,
    );
    put_widget(&pool, d1, 0, 0, 2, 2).await.unwrap();
    put_widget(&pool, d2, 0, 0, 2, 2).await.unwrap();
    sqlx::query("DELETE FROM dashboard WHERE id = $1")
        .bind(d2)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        (
            count(&pool, "dashboard").await,
            count(&pool, "widget").await
        ),
        (1, 1)
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(bob)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        (
            count(&pool, "dashboard").await,
            count(&pool, "widget").await
        ),
        (0, 0)
    );
}
