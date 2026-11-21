// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests d'integration de l'authentification : routeur complet, vraie base (isolee par test).

mod common;

use std::net::SocketAddr;

use axum::http::{Method, StatusCode, header};
use common::{Client, PASSWORD, add_user, code, state};
use dominet_domain::repo::UserRepo;
use dominet_domain::user::{Role, UserProfile};
use dominet_store::USER_ID_KEY;
use serde_json::json;
use sqlx::PgPool;
use tower_sessions::session::Id;
use tower_sessions::session_store::SessionStore;

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn login_returns_the_user_and_a_hardened_cookie(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    let r = c.login("bob", PASSWORD).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(
        (r.body["username"].as_str(), r.body["role"].as_str()),
        (Some("bob"), Some("user"))
    );
    assert!(!r.body.to_string().contains("password"), "{}", r.body);

    let cookie = r.headers[header::SET_COOKIE].to_str().unwrap();
    assert!(cookie.starts_with("dominet_session="), "{cookie}");
    for attr in ["HttpOnly", "SameSite=Lax", "Path=/", "Max-Age=28800"] {
        assert!(cookie.contains(attr), "{attr} manquant dans {cookie}");
    }
    assert!(!cookie.contains("Secure"), "COOKIE_SECURE=false");
    assert!(
        r.headers.get(header::STRICT_TRANSPORT_SECURITY).is_none(),
        "pas de HSTS sans TLS"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn behind_tls_the_cookie_is_secure_and_hsts_is_sent(pool: PgPool) {
    let st = state(pool, &[("COOKIE_SECURE", "true")]);
    add_user(&st, "bob", Role::User).await;
    let r = Client::new(&st).login("bob", PASSWORD).await;
    assert!(
        r.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Secure")
    );
    assert!(
        r.headers[header::STRICT_TRANSPORT_SECURITY]
            .to_str()
            .unwrap()
            .contains("max-age=31536000")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn session_lifecycle_login_me_logout(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::UNAUTHORIZED);
    c.login("bob", PASSWORD).await;
    let me = c.get("/api/auth/me").await;
    assert_eq!(
        (me.status, me.body["username"].as_str()),
        (StatusCode::OK, Some("bob"))
    );
    assert_eq!(me.headers[header::CACHE_CONTROL], "no-store");

    let stale_cookie = c.cookie.clone().unwrap();
    assert_eq!(
        c.send(Method::POST, "/api/auth/logout", None, &[])
            .await
            .status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::UNAUTHORIZED);
    // L'ancien cookie ne reouvre rien : la session est supprimee cote serveur.
    c.cookie = Some(stale_cookie);
    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(st.store.sessions().count().await.unwrap(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn unknown_user_and_wrong_password_are_indistinguishable(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    let wrong = c.login("bob", "pas le bon mot de passe").await;
    let unknown = c.login("personne", "pas le bon mot de passe").await;
    assert_eq!(
        (wrong.status, unknown.status),
        (StatusCode::UNAUTHORIZED, StatusCode::UNAUTHORIZED)
    );
    assert_eq!(code(&wrong), "invalid_credentials");
    assert_eq!(
        wrong.body, unknown.body,
        "memes reponses : aucune enumeration de comptes"
    );
    assert!(wrong.headers.get(header::SET_COOKIE).is_none());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn usernames_are_case_insensitive_at_login(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "Bob", Role::User).await;
    assert_eq!(
        Client::new(&st).login("BOB", PASSWORD).await.status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn five_failures_lock_the_pair_even_for_the_right_password(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    add_user(&st, "carol", Role::User).await;
    let mut c = Client::new(&st);
    for _ in 0..5 {
        assert_eq!(
            c.login("bob", "mauvais mot de passe").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let locked = c.login("bob", PASSWORD).await;
    assert_eq!(
        locked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "meme avec le bon mot de passe"
    );
    assert_eq!(code(&locked), "too_many_requests");
    let retry: u64 = locked.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=900).contains(&retry), "{retry}");
    assert!(locked.headers.get(header::SET_COOKIE).is_none());

    // Un autre compte depuis la meme IP n'est pas touche (seuil par couple et par IP bien plus haut).
    assert_eq!(c.login("carol", PASSWORD).await.status, StatusCode::OK);
    // Le meme compte depuis une autre IP non plus (le seuil par compte est de 20).
    assert_eq!(
        c.from([192, 168, 1, 77])
            .login("bob", PASSWORD)
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_success_resets_the_failure_counter(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    for _ in 0..4 {
        c.login("bob", "mauvais mot de passe").await;
    }
    assert_eq!(c.login("bob", PASSWORD).await.status, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    for _ in 0..4 {
        assert_eq!(
            c.login("bob", "mauvais mot de passe").await.status,
            StatusCode::UNAUTHORIZED,
            "pas encore bloque"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_distributed_attack_on_one_account_is_stopped_at_twenty_failures(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let base = Client::new(&st);
    for i in 0..20u8 {
        let mut c = base.from([10, 0, 0, i]); // 20 IP differentes, 1 echec chacune
        assert_eq!(
            c.login("bob", "mauvais mot de passe").await.status,
            StatusCode::UNAUTHORIZED
        );
    }
    let r = base.from([10, 9, 9, 9]).login("bob", PASSWORD).await;
    assert_eq!(
        r.status,
        StatusCode::TOO_MANY_REQUESTS,
        "le compte est verrouille quelle que soit l'IP"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn login_rotates_the_session_id_against_fixation(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    // Session pre-existante (celle qu'un attaquant aurait imposee a la victime).
    let mut planted = tower_sessions::session::Record {
        id: Id::default(),
        data: std::collections::HashMap::from([("fixed".to_string(), json!(true))]),
        expiry_date: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
    };
    st.store.sessions().create(&mut planted).await.unwrap();

    let mut c = Client::new(&st);
    c.cookie = Some(format!("dominet_session={}", planted.id));
    let r = c.login("bob", PASSWORD).await;
    assert_eq!(r.status, StatusCode::OK);
    let new_cookie = c.cookie.clone().unwrap();
    assert_ne!(
        new_cookie,
        format!("dominet_session={}", planted.id),
        "nouvel identifiant apres connexion"
    );
    assert!(
        st.store
            .sessions()
            .load(&planted.id)
            .await
            .unwrap()
            .is_none(),
        "l'ancienne session est detruite"
    );
    // Et l'identifiant impose ne donne aucun acces.
    c.cookie = Some(format!("dominet_session={}", planted.id));
    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_disabled_account_cannot_log_in_and_loses_its_open_session(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    c.login("bob", PASSWORD).await;
    let bob = st
        .store
        .users()
        .find_by_username("bob")
        .await
        .unwrap()
        .unwrap();
    let profile = UserProfile {
        email: String::new(),
        first_name: String::new(),
        last_name: String::new(),
        mobile: String::new(),
        role: Role::User,
        is_active: false,
    };
    st.store
        .users()
        .update_profile(bob.id, &profile)
        .await
        .unwrap();

    assert_eq!(
        c.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED,
        "session ouverte avant la desactivation"
    );
    let r = Client::new(&st).login("bob", PASSWORD).await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::UNAUTHORIZED, "invalid_credentials"),
        "meme reponse qu'un mauvais mot de passe"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn changing_the_password_revokes_every_session(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let (mut laptop, mut phone) = (Client::new(&st), Client::new(&st));
    laptop.login("bob", PASSWORD).await;
    phone.login("bob", PASSWORD).await;

    let change = |cur: &str, new: &str| Some(json!({"current_password": cur, "new_password": new}));
    let bad_current = laptop
        .send(
            Method::POST,
            "/api/auth/password",
            change("faux mot de passe", "un nouveau mot de passe"),
            &[],
        )
        .await;
    assert_eq!(bad_current.status, StatusCode::UNAUTHORIZED);
    let weak = laptop
        .send(
            Method::POST,
            "/api/auth/password",
            change(PASSWORD, "court"),
            &[],
        )
        .await;
    assert_eq!(
        (weak.status, code(&weak)),
        (StatusCode::BAD_REQUEST, "bad_request")
    );
    assert_eq!(
        laptop.get("/api/auth/me").await.status,
        StatusCode::OK,
        "un echec ne ferme rien"
    );

    let ok = laptop
        .send(
            Method::POST,
            "/api/auth/password",
            change(PASSWORD, "un nouveau mot de passe"),
            &[],
        )
        .await;
    assert_eq!(ok.status, StatusCode::NO_CONTENT);
    assert_eq!(
        laptop.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        phone.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED,
        "l'autre appareil aussi"
    );
    assert_eq!(
        Client::new(&st).login("bob", PASSWORD).await.status,
        StatusCode::UNAUTHORIZED,
        "ancien mot de passe"
    );
    assert_eq!(
        Client::new(&st)
            .login("bob", "un nouveau mot de passe")
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn admin_routes_distinguish_anonymous_user_and_admin(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "root", Role::Admin).await;
    add_user(&st, "bob", Role::User).await;
    let mut anon = Client::new(&st);
    assert_eq!(
        anon.get("/api/users").await.status,
        StatusCode::UNAUTHORIZED
    );

    let mut user = Client::new(&st);
    user.login("bob", PASSWORD).await;
    let denied = user.get("/api/users").await;
    assert_eq!(
        (denied.status, code(&denied)),
        (StatusCode::FORBIDDEN, "forbidden")
    );

    let mut admin = Client::new(&st);
    admin.login("root", PASSWORD).await;
    let list = admin.get("/api/users").await;
    assert_eq!(list.status, StatusCode::OK);
    let names: Vec<&str> = list
        .body
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|u| u["username"].as_str())
        .collect();
    assert_eq!(names, ["bob", "root"]);
    let dump = list.body.to_string();
    assert!(
        !dump.contains("password") && !dump.contains("argon2"),
        "aucun condense dans la reponse : {dump}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn cross_site_posts_are_refused_before_anything_happens(pool: PgPool) {
    let st = state(
        pool.clone(),
        &[("CSRF_TRUSTED_ORIGINS", "https://dominet.example.org")],
    );
    add_user(&st, "bob", Role::User).await;
    let body = || Some(json!({"username": "bob", "password": PASSWORD}));
    let mut c = Client::new(&st);

    let evil = c
        .send(
            Method::POST,
            "/api/auth/login",
            body(),
            &[("origin", "https://evil.example")],
        )
        .await;
    assert_eq!(
        (evil.status, code(&evil)),
        (StatusCode::FORBIDDEN, "forbidden")
    );
    assert!(c.cookie.is_none());
    let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM login_attempt")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        attempts, 0,
        "la requete n'a meme pas atteint la verification"
    );

    assert_eq!(
        c.send(
            Method::POST,
            "/api/auth/login",
            body(),
            &[("origin", "http://dominet.test")]
        )
        .await
        .status,
        StatusCode::OK
    );
    c.cookie = None;
    assert_eq!(
        c.send(
            Method::POST,
            "/api/auth/login",
            body(),
            &[("origin", "https://dominet.example.org")]
        )
        .await
        .status,
        StatusCode::OK
    );
    // Lecture : jamais bloquee par le controle d'origine.
    assert_eq!(
        c.send(
            Method::GET,
            "/api/auth/me",
            None,
            &[("origin", "https://evil.example")]
        )
        .await
        .status,
        StatusCode::OK
    );
    // Un navigateur annonce le contexte cross-site meme sans Origin.
    let sec = c
        .send(
            Method::POST,
            "/api/auth/logout",
            None,
            &[("sec-fetch-site", "cross-site")],
        )
        .await;
    assert_eq!(sec.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn security_headers_are_on_every_response(pool: PgPool) {
    let st = state(pool, &[]);
    let r = Client::new(&st).get("/healthz").await;
    assert_eq!(r.headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(r.headers[header::X_FRAME_OPTIONS], "DENY");
    assert_eq!(r.headers[header::REFERRER_POLICY], "same-origin");
    assert!(
        r.headers["permissions-policy"]
            .to_str()
            .unwrap()
            .contains("camera=()")
    );
    let denied = Client::new(&st).get("/api/users").await;
    assert_eq!(
        denied.headers[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff",
        "aussi sur les erreurs"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_session_older_than_the_maximum_age_is_refused_and_deleted(pool: PgPool) {
    let st = state(pool, &[("SESSION_MAX_DAYS", "1")]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    c.login("bob", PASSWORD).await;
    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::OK);

    // On vieillit artificiellement la connexion de 2 jours directement dans le store.
    let id: Id = c
        .cookie
        .as_ref()
        .unwrap()
        .trim_start_matches("dominet_session=")
        .parse()
        .unwrap();
    let mut rec = st.store.sessions().load(&id).await.unwrap().unwrap();
    rec.data.insert(
        "auth_at".into(),
        json!(chrono::Utc::now().timestamp() - 2 * 86_400),
    );
    st.store.sessions().save(&rec).await.unwrap();

    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::UNAUTHORIZED);
    assert!(
        st.store.sessions().load(&id).await.unwrap().is_none(),
        "session supprimee"
    );
    assert!(rec.data[USER_ID_KEY].as_i64().is_some());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_idle_timeout_is_configurable(pool: PgPool) {
    let st = state(pool, &[("SESSION_IDLE_MINUTES", "30")]);
    add_user(&st, "bob", Role::User).await;
    let r = Client::new(&st).login("bob", PASSWORD).await;
    assert!(
        r.headers[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=1800")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn behind_a_trusted_proxy_attempts_are_recorded_against_the_real_client(pool: PgPool) {
    let st = state(pool.clone(), &[("TRUSTED_PROXIES", "172.18.0.1")]);
    add_user(&st, "bob", Role::User).await;
    let mut proxy = Client::new(&st);
    proxy.peer = SocketAddr::from(([172, 18, 0, 1], 5000));
    let r = proxy
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "bob", "password": "faux faux faux"})),
            &[("x-forwarded-for", "192.168.1.99")],
        )
        .await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    // Un client direct qui forge l'en-tete n'est pas cru.
    let mut direct = Client::new(&st);
    direct
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "bob", "password": "faux faux faux"})),
            &[("x-forwarded-for", "6.6.6.6")],
        )
        .await;

    let ips: Vec<String> = sqlx::query_scalar("SELECT ip FROM login_attempt ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(ips, ["192.168.1.99", "192.168.1.50"]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn malformed_or_oversized_logins_are_client_errors_not_crashes(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    let huge = "x".repeat(10_000);
    let r = c.login("bob", &huge).await;
    assert_eq!(
        (r.status, code(&r)),
        (StatusCode::UNAUTHORIZED, "invalid_credentials"),
        "pas de hachage d'un mot de passe geant"
    );
    let r = c
        .send(
            Method::POST,
            "/api/auth/login",
            Some(json!({"username": "bob"})),
            &[],
        )
        .await;
    assert!(r.status.is_client_error(), "{}", r.status);
    let r = c.login("", "").await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn password_change_has_the_same_lockout_as_login(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    assert_eq!(c.login("bob", PASSWORD).await.status, StatusCode::OK);
    let change = |cur: &str| {
        Some(json!({"current_password": cur, "new_password": "un nouveau mot de passe"}))
    };
    for _ in 0..4 {
        let r = c
            .send(
                Method::POST,
                "/api/auth/password",
                change("faux mot de passe"),
                &[],
            )
            .await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    }
    // Le cinquieme echec atteint le seuil : le sixieme essai est bloque.
    let fifth = c
        .send(
            Method::POST,
            "/api/auth/password",
            change("faux mot de passe"),
            &[],
        )
        .await;
    assert_eq!(fifth.status, StatusCode::UNAUTHORIZED);
    let locked = c
        .send(Method::POST, "/api/auth/password", change(PASSWORD), &[])
        .await;
    assert_eq!(
        locked.status,
        StatusCode::TOO_MANY_REQUESTS,
        "meme avec le bon mot de passe"
    );
    assert_eq!(code(&locked), "too_many_requests");
    let retry: u64 = locked.headers[header::RETRY_AFTER]
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert!((1..=900).contains(&retry), "{retry}");
    // Rien n'a change : la session est ouverte et l'ancien mot de passe vaut toujours (le blocage touche aussi la
    // connexion de ce couple compte/adresse, ce qui est voulu).
    assert_eq!(c.get("/api/auth/me").await.status, StatusCode::OK);
    assert_eq!(
        c.login("bob", PASSWORD).await.status,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_correct_current_password_resets_the_failure_count(pool: PgPool) {
    let st = state(pool, &[]);
    add_user(&st, "bob", Role::User).await;
    let mut c = Client::new(&st);
    c.login("bob", PASSWORD).await;
    let change = |cur: &str, new: &str| Some(json!({"current_password": cur, "new_password": new}));
    for _ in 0..4 {
        c.send(
            Method::POST,
            "/api/auth/password",
            change("faux mot de passe", "x"),
            &[],
        )
        .await;
    }
    // Bon mot de passe actuel, nouveau trop court : refuse pour la politique, mais l'ancien est prouve.
    let weak = c
        .send(
            Method::POST,
            "/api/auth/password",
            change(PASSWORD, "court"),
            &[],
        )
        .await;
    assert_eq!(weak.status, StatusCode::BAD_REQUEST);
    for _ in 0..4 {
        let r = c
            .send(
                Method::POST,
                "/api/auth/password",
                change("faux mot de passe", "x"),
                &[],
            )
            .await;
        assert_eq!(
            r.status,
            StatusCode::UNAUTHORIZED,
            "le compteur est reparti de zero"
        );
    }
}
