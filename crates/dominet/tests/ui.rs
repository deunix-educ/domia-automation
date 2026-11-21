// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests d'integration du routeur complet (API + interface Leptos) : rendu serveur, traductions, en-tetes de
//! securite, fonctions serveur avec session, et non-regression de l'API (base isolee par test).

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use dominet_domain::repo::UserRepo;
use dominet_domain::user::Role;
use dominet_server::auth::password::Hasher;
use dominet_server::config::Config;
use dominet_server::{AppState, web};
use http_body_util::BodyExt;
use sqlx::PgPool;
use tower::ServiceExt;

const PASSWORD: &str = "correct horse battery";

struct Resp {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
}

/// Les tests de ce fichier s'executent un par un. Chacun monte sa propre application Leptos dans son propre runtime
/// tokio (`#[sqlx::test]`), et plusieurs applications rendues en parallele dans un meme processus se perturbent
/// (rendu fige, ou contexte `AppState` introuvable dans une fonction serveur : ~15 % des executions en parallele,
/// 0 sur 25 en serie). En production il n'y a qu'une application par processus ; la concurrence de requetes sur
/// une meme application est verifiee a part sur le vrai serveur (voir `docs/interface.md`).
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn app(pool: PgPool) -> (Router, AppState, tokio::sync::MutexGuard<'static, ()>) {
    app_with_key(pool, None).await
}

/// Cle maitresse de test (32 octets nuls en base64) : permet d'enregistrer des secrets.
const TEST_MASTER_KEY: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";

async fn app_with_key(
    pool: PgPool,
    master_key: Option<&'static str>,
) -> (Router, AppState, tokio::sync::MutexGuard<'static, ()>) {
    app_with_vars(pool, master_key, &[]).await
}

/// Comme `app_with_key`, avec des variables d'environnement supplementaires.
async fn app_with_vars(
    pool: PgPool,
    master_key: Option<&'static str>,
    vars: &[(&str, &str)],
) -> (Router, AppState, tokio::sync::MutexGuard<'static, ()>) {
    let guard = ONE_AT_A_TIME.lock().await;
    let config = Config::from_lookup(|k| match k {
        "DATABASE_URL" => Some("postgres://u:p@127.0.0.1:1/none".into()),
        "COOKIE_SECURE" => Some("false".into()),
        "DOMINET_MASTER_KEY" => master_key.map(Into::into),
        other => vars
            .iter()
            .find(|(key, _)| *key == other)
            .map(|(_, v)| (*v).to_string()),
    })
    .unwrap();
    let state = AppState::with_hasher(
        pool,
        Arc::new(config),
        Hasher::with_params(1024, 1, 1).unwrap(),
    );
    let ui = dominet_web::ssr::ui_service(
        state.clone(),
        "target/site",
        SocketAddr::from(([127, 0, 0, 1], 3000)),
    );
    (web::router_with_ui(state.clone(), ui), state, guard)
}

/// Les tests ecrits en francais demandent le francais par defaut (`Accept-Language: fr`) ; l'interface, elle, est en anglais
/// sans en-tete ni cookie (voir `the_default_language_is_english_without_any_hint`).
async fn send(
    app: &Router,
    method: Method,
    path: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
) -> Resp {
    send_with(app, method, path, extra, body, true).await
}

async fn send_with(
    app: &Router,
    method: Method,
    path: &str,
    extra: &[(&str, &str)],
    body: Option<&str>,
    french_by_default: bool,
) -> Resp {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "dominet.test");
    let has_language = extra
        .iter()
        .any(|(k, _)| k.eq_ignore_ascii_case("accept-language"));
    if french_by_default && !has_language {
        req = req.header("accept-language", "fr");
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let mut req = req
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([192, 168, 1, 50], 40_000))));
    // Garde-fou : un blocage du rendu doit echouer vite et clairement, pas figer toute la suite.
    let resp = tokio::time::timeout(std::time::Duration::from_secs(20), app.clone().oneshot(req))
        .await
        .expect("la requete est restee bloquee plus de 20 s")
        .unwrap();
    let (status, headers) = (resp.status(), resp.headers().clone());
    let bytes = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        resp.into_body().collect(),
    )
    .await
    .expect("le corps de la reponse est reste bloque plus de 20 s")
    .unwrap()
    .to_bytes();
    Resp {
        status,
        headers,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

async fn get(app: &Router, path: &str, extra: &[(&str, &str)]) -> Resp {
    send(app, Method::GET, path, extra, None).await
}

async fn login_cookie(app: &Router, user: &str) -> String {
    let body = format!(r#"{{"username":"{user}","password":"{PASSWORD}"}}"#);
    let r = send(
        app,
        Method::POST,
        "/api/auth/login",
        &[("content-type", "application/json")],
        Some(&body),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    r.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_login_page_is_server_rendered_in_french_when_french_is_requested(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let r = get(&app, "/login", &[]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    assert!(
        r.text.contains(r#"<html lang="fr""#),
        "{}",
        &r.text[..r.text.len().min(300)]
    );
    assert!(r.text.contains("Connectez-vous à votre espace"));
    assert!(r.text.contains("Se connecter"));
    // Tant que l'interface n'est pas hydratee le bouton est inactif : pas de soumission HTML ordinaire.
    let button = &r.text[r.text.find(r#"id="login-submit""#).unwrap()..];
    assert!(
        button[..button.find('>').unwrap()].contains("disabled"),
        "{}",
        &button[..120]
    );
    assert!(
        r.text.contains("/pkg/dominet.js") && r.text.contains("/pkg/dominet.css"),
        "scripts d'hydratation et style"
    );
    // Le fichier WebAssembly demande doit etre celui que `cargo leptos` produit (`dominet.wasm`). Son nom depend d'une
    // variable lue a la compilation (voir `.cargo/config.toml`) : un binaire compile autrement reclame `dominet_bg.wasm`
    // (404) et l'interface ne s'hydrate jamais.
    assert!(
        r.text.contains(r#"href="/pkg/dominet.wasm""#),
        "fichier WASM demande : {}",
        &r.text[..r.text.len().min(900)]
    );
    assert!(!r.text.contains("dominet_bg.wasm"));
    // Marqueurs d'hydratation (`<!>` apres un texte reactif) : ils n'existent que si le serveur est compile comme le WASM
    // (`--cfg erase_components`, voir `.cargo/config.toml`) ; sans eux l'hydratation echoue et les boutons restent inertes.
    assert!(
        r.text.contains("espace<!>"),
        "marqueurs d'hydratation absents du HTML rendu"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_default_language_is_english_without_any_hint(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let r = send_with(&app, Method::GET, "/login", &[], None, false).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(
        r.text.contains(r#"<html lang="en""#) && r.text.contains("Sign in to your space"),
        "{}",
        &r.text[..r.text.len().min(300)]
    );
    // Un navigateur francais recoit le francais.
    let fr = send_with(
        &app,
        Method::GET,
        "/login",
        &[("accept-language", "fr-FR,fr;q=0.9")],
        None,
        false,
    )
    .await;
    assert!(fr.text.contains(r#"<html lang="fr""#));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_language_follows_accept_language_then_the_cookie(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let en = get(&app, "/login", &[("accept-language", "en-US,en;q=0.9")]).await;
    assert!(en.text.contains(r#"<html lang="en""#) && en.text.contains("Sign in to your space"));
    let cookie_wins = get(
        &app,
        "/login",
        &[("accept-language", "en"), ("cookie", "lang=fr")],
    )
    .await;
    assert!(
        cookie_wins.text.contains(r#"<html lang="fr""#),
        "le cookie prime sur l'en-tete"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn ui_pages_get_the_same_security_headers_as_the_api(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    for path in ["/", "/nope", "/healthz", "/api/auth/me"] {
        let r = get(&app, path, &[]).await;
        assert_eq!(
            r.headers[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff",
            "{path}"
        );
        assert_eq!(r.headers[header::X_FRAME_OPTIONS], "DENY", "{path}");
        assert_eq!(r.headers[header::REFERRER_POLICY], "same-origin", "{path}");
        assert!(r.headers.contains_key("x-request-id"), "{path}");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn unknown_pages_are_a_real_404_with_the_not_found_page(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let r = get(&app, "/cette/page/n/existe/pas", &[]).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.text.contains("Page introuvable"), "page 404 localisee");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_api_and_probes_are_not_shadowed_by_the_ui(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let health = get(&app, "/healthz", &[]).await;
    assert!(
        health.headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    assert!(health.text.contains(r#""status":"ok""#));
    let me = get(&app, "/api/auth/me", &[]).await;
    assert_eq!(me.status, StatusCode::UNAUTHORIZED);
    assert!(
        me.text.contains("unauthorized"),
        "reponse JSON de l'API, pas une page HTML"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_whoami_server_function_sees_the_session(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    state
        .auth
        .create_user("bob", "", "", Role::User, PASSWORD)
        .await
        .unwrap();

    let anon = send(&app, Method::POST, "/sf/whoami", &[], None).await;
    assert_eq!((anon.status, anon.text.as_str()), (StatusCode::OK, "null"));

    let cookie = login_cookie(&app, "bob").await;
    let known = send(
        &app,
        Method::POST,
        "/sf/whoami",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(known.status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&known.text).unwrap();
    assert_eq!(
        (v["username"].as_str(), v["role"].as_str()),
        (Some("bob"), Some("user"))
    );
    assert!(!known.text.contains("password"), "{}", known.text);

    // La page rendue cote serveur voit elle aussi la session.
    let page = get(&app, "/", &[("cookie", &cookie)]).await;
    assert!(
        page.text.contains(r#"id="vc-user-name""#) && page.text.contains("bob"),
        "{}",
        &page.text[..page.text.len().min(400)]
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn server_functions_are_protected_against_cross_site_posts(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let evil = send(
        &app,
        Method::POST,
        "/sf/whoami",
        &[("origin", "https://evil.example")],
        None,
    )
    .await;
    assert_eq!(evil.status, StatusCode::FORBIDDEN);
    let same = send(
        &app,
        Method::POST,
        "/sf/whoami",
        &[("origin", "http://dominet.test")],
        None,
    )
    .await;
    assert_eq!(same.status, StatusCode::OK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_closed_session_is_no_longer_seen_by_the_ui(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    state
        .auth
        .create_user("bob", "", "", Role::User, PASSWORD)
        .await
        .unwrap();
    let cookie = login_cookie(&app, "bob").await;
    let out = send(
        &app,
        Method::POST,
        "/api/auth/logout",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(out.status, StatusCode::NO_CONTENT);
    let after = send(
        &app,
        Method::POST,
        "/sf/whoami",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(after.text, "null");
}

/// Cree un compte `username` du role `role` et renvoie son cookie de session.
async fn session_for(app: &Router, state: &AppState, username: &str, role: Role) -> String {
    state
        .auth
        .create_user(username, "", "", role, PASSWORD)
        .await
        .unwrap();
    login_cookie(app, username).await
}

async fn layout(app: &Router, cookie: Option<&str>) -> serde_json::Value {
    let extra: Vec<(&str, &str)> = cookie.map(|c| ("cookie", c)).into_iter().collect();
    let r = send(app, Method::POST, "/sf/layout", &extra, None).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    serde_json::from_str(&r.text).unwrap()
}

fn labels(v: &serde_json::Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap())
        .collect()
}

async fn seed(state: &AppState) {
    dominet_server::commands::seed_data(&state.settings, &state.store, |_| None, false)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_layout_is_empty_for_anonymous_visitors(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let v = layout(&app, None).await;
    assert!(v["user"].is_null(), "{v}");
    assert!(v["nav_top"].as_array().unwrap().is_empty(), "{v}");
    assert!(v["nav_bottom"].as_array().unwrap().is_empty(), "{v}");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_navigation_is_filtered_by_role(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;

    let u = layout(&app, Some(&user)).await;
    assert_eq!(u["user"]["username"], "bob");
    assert_eq!(
        labels(&u["nav_top"]),
        [
            "Surveillance",
            "Automation",
            "Observabilite",
            "Documentation"
        ]
    );
    assert!(labels(&u["nav_bottom"]).is_empty(), "{u}");

    let a = layout(&app, Some(&admin)).await;
    assert_eq!(labels(&a["nav_top"]).len(), 4);
    assert_eq!(
        labels(&a["nav_bottom"]),
        ["Zigbee2MQTT", "Android", "Messages"]
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn navigation_icons_are_server_side_inline_svg(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let v = layout(&app, Some(&admin)).await;
    for item in v["nav_top"]
        .as_array()
        .unwrap()
        .iter()
        .chain(v["nav_bottom"].as_array().unwrap())
    {
        let svg = item["icon_svg"].as_str().unwrap();
        assert!(svg.starts_with("<svg"), "{item}");
        assert!(
            !svg.contains("<script") && !svg.contains("onload"),
            "{item}"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_unknown_icon_falls_back_to_a_generic_one(pool: PgPool) {
    use dominet_domain::repo::NavbarRepo;
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let mut draft = dominet_domain::registry::default_navbar().remove(0);
    draft.icon_name = "no-such-icon\"><script>".into();
    // Le depot ne valide pas le nom d'icone : la defense est au rendu.
    state.store.navbar().create(&draft).await.unwrap();
    let v = layout(&app, Some(&admin)).await;
    let svg = v["nav_top"][0]["icon_svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg") && !svg.contains("<script"), "{svg}");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_sidebar_has_empty_system_gauges_rendered_by_the_server(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let page = get(&app, "/", &[("cookie", &cookie)]).await;
    for id in [
        "vc-sys-stats",
        "vc-stat-cpu",
        "vc-stat-cpu-bar",
        "vc-stat-cpu-pct",
        "vc-stat-mem-bar",
        "vc-stat-mem-pct",
        "vc-stat-disk-bar",
        "vc-stat-disk-pct",
    ] {
        assert!(page.text.contains(&format!(r#"id="{id}""#)), "{id} absent");
    }
    // Sans mesure : pas de valeur inventee, et aucune largeur en ligne (la CSP interdit `style=`).
    assert!(page.text.contains("…"), "valeurs vides attendues");
    let body = &page.text[page.text.find("<body").unwrap()..];
    assert!(
        !body.contains(r#" style=""#),
        "attribut style interdit par la CSP"
    );
    let anon = get(&app, "/", &[]).await;
    assert!(
        !anon.text.contains("vc-sys-stats"),
        "pas de jauges hors session"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_page_contains_the_sidebar_for_a_signed_in_user(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;

    let anon = get(&app, "/", &[]).await;
    assert!(
        !anon.text.contains("vc-sidebar"),
        "pas de barre laterale hors session"
    );
    assert!(anon.text.contains("vc-main-wrap"));

    let page = get(&app, "/", &[("cookie", &cookie)]).await;
    assert!(
        page.text.contains("vc-sidebar"),
        "{}",
        &page.text[..400.min(page.text.len())]
    );
    for label in [
        "Tableau de bord",
        "Surveillance",
        "Déconnexion",
        "Plein écran",
    ] {
        assert!(page.text.contains(label), "libelle absent : {label}");
    }
    assert!(
        !page.text.contains("Zigbee2MQTT"),
        "entree reservee aux admins"
    );

    // Rendu complet cote serveur, dans l'ordre : pas de gabarit differe (`<template>`) que du JS replacerait, sinon le
    // contenu glisserait de la gauche a l'arrivee de la barre.
    let body = &page.text[page.text.find("<body").unwrap()..];
    let (aside, main) = (
        body.find("<aside").unwrap(),
        body.find("vc-main-wrap").unwrap(),
    );
    assert!(
        aside < main,
        "la barre laterale precede le contenu dans le HTML"
    );
    assert!(!body.contains("<template"), "aucun fragment differe");

    let en = get(&app, "/", &[("cookie", &cookie), ("accept-language", "en")]).await;
    assert!(en.text.contains("Dashboard") && en.text.contains("Sign out"));

    let missing = get(&app, "/nope", &[("cookie", &cookie)]).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(
        missing.text.contains("vc-sidebar"),
        "la 404 reste dans la mise en page"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn logging_out_through_the_server_function_closes_the_session(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    assert_eq!(layout(&app, Some(&cookie)).await["user"]["username"], "bob");
    let out = send(
        &app,
        Method::POST,
        "/sf/logout",
        &[("cookie", &cookie)],
        None,
    )
    .await;
    assert_eq!(out.status, StatusCode::OK, "{}", out.text);
    assert!(layout(&app, Some(&cookie)).await["user"].is_null());
}

// ---------------------------------------------------------------- 7c : connexion, gardes, compte

const FORM: (&str, &str) = ("content-type", "application/x-www-form-urlencoded");
/// Un navigateur annonce `text/html` ; c'est ce qui fait de la redirection une vraie reponse 302.
const HTML: (&str, &str) = ("accept", "text/html");

fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&")
}

async fn login_fn(app: &Router, user: &str, password: &str) -> Resp {
    let body = form(&[("username", user), ("password", password)]);
    send(app, Method::POST, "/sf/login", &[FORM], Some(&body)).await
}

fn session_cookie(r: &Resp) -> String {
    r.headers[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string()
}

async fn account(state: &AppState, name: &str) {
    state
        .auth
        .create_user(name, "", "", Role::User, PASSWORD)
        .await
        .unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_login_server_function_opens_a_session(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    account(&state, "bob").await;

    let ok = login_fn(&app, "bob", PASSWORD).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    let v: serde_json::Value = serde_json::from_str(&ok.text).unwrap();
    assert_eq!(v["Success"]["username"], "bob", "{v}");
    assert!(
        !ok.text.contains(PASSWORD) && !ok.text.contains("hash"),
        "{}",
        ok.text
    );
    let cookie = session_cookie(&ok);
    assert_eq!(layout(&app, Some(&cookie)).await["user"]["username"], "bob");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_failed_login_does_not_reveal_whether_the_account_exists(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    account(&state, "bob").await;
    let wrong = login_fn(&app, "bob", "mauvais mot de passe").await;
    let unknown = login_fn(&app, "personne", "mauvais mot de passe").await;
    assert_eq!(wrong.text, "\"InvalidCredentials\"");
    assert_eq!(
        unknown.text, wrong.text,
        "meme reponse, compte connu ou non"
    );
    assert!(
        !wrong.headers.contains_key(header::SET_COOKIE),
        "pas de session ouverte"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn repeated_failures_lock_the_login_even_for_the_right_password(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    account(&state, "bob").await;
    for _ in 0..5 {
        assert_eq!(
            login_fn(&app, "bob", "faux faux faux").await.text,
            "\"InvalidCredentials\""
        );
    }
    let locked = login_fn(&app, "bob", PASSWORD).await;
    let v: serde_json::Value = serde_json::from_str(&locked.text).unwrap();
    assert!(v["Locked"]["retry_after_s"].as_u64().unwrap() > 0, "{v}");
    assert!(!locked.headers.contains_key(header::SET_COOKIE));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn anonymous_visitors_are_redirected_to_the_login_page(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let home = get(&app, "/", &[HTML]).await;
    assert_eq!(home.status, StatusCode::FOUND, "{}", home.text);
    assert_eq!(home.headers[header::LOCATION], "/login");
    let acc = get(&app, "/account", &[HTML]).await;
    assert_eq!(acc.status, StatusCode::FOUND);
    assert_eq!(acc.headers[header::LOCATION], "/login?next=%2Faccount");
    assert!(
        !acc.text.contains("Changer le mot de passe"),
        "la page protegee n'est pas rendue"
    );
    // La page de connexion, elle, reste accessible.
    assert_eq!(get(&app, "/login", &[HTML]).await.status, StatusCode::OK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_login_page_sends_signed_in_users_on_their_way(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let c = ("cookie", cookie.as_str());
    let to = |r: &Resp| r.headers[header::LOCATION].to_str().unwrap().to_string();

    let plain = get(&app, "/login", &[HTML, c]).await;
    assert_eq!(
        (plain.status, to(&plain).as_str()),
        (StatusCode::FOUND, "/")
    );
    let next = get(&app, "/login?next=%2Faccount", &[HTML, c]).await;
    assert_eq!(to(&next), "/account");
    // Pas de redirection ouverte : une cible hors du site est ignoree.
    for evil in [
        "%2F%2Fevil.example",
        "https%3A%2F%2Fevil.example",
        "%2F%5Cevil.example",
    ] {
        let r = get(&app, &format!("/login?next={evil}"), &[HTML, c]).await;
        assert_eq!(to(&r), "/", "{evil}");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_login_page_shows_the_password_changed_notice(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let r = get(&app, "/login?notice=password-changed", &[]).await;
    assert!(
        r.text.contains("Mot de passe modifié"),
        "{}",
        &r.text[..300.min(r.text.len())]
    );
    assert!(
        !get(&app, "/login", &[])
            .await
            .text
            .contains("Mot de passe modifié")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_account_page_shows_identity_and_the_password_form(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let fr = get(&app, "/account", &[("cookie", &cookie)]).await;
    assert_eq!(fr.status, StatusCode::OK);
    for text in [
        "Mon compte",
        "bob",
        "Utilisateur",
        "Changer le mot de passe",
        "Mot de passe actuel",
    ] {
        assert!(fr.text.contains(text), "absent : {text}");
    }
    let en = get(
        &app,
        "/account",
        &[("cookie", &cookie), ("accept-language", "en")],
    )
    .await;
    assert!(en.text.contains("My account") && en.text.contains("Current password"));
}

async fn change_fn(app: &Router, cookie: Option<&str>, current: &str, new: &str) -> Resp {
    let body = form(&[("current_password", current), ("new_password", new)]);
    let mut extra = vec![FORM];
    if let Some(c) = cookie {
        extra.push(("cookie", c));
    }
    send(app, Method::POST, "/sf/password", &extra, Some(&body)).await
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn changing_the_password_checks_the_old_one_and_the_policy(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;

    let wrong = change_fn(
        &app,
        Some(&cookie),
        "pas le bon mot de passe",
        "un nouveau mot de passe",
    )
    .await;
    assert_eq!(wrong.text, "\"WrongCurrent\"");
    let short = change_fn(&app, Some(&cookie), PASSWORD, "court").await;
    assert_eq!(short.text, r#"{"Rejected":{"TooShort":{"min":10}}}"#);
    let digits = change_fn(&app, Some(&cookie), PASSWORD, "12345678901234").await;
    assert_eq!(digits.text, r#"{"Rejected":"AllDigits"}"#);
    let same = change_fn(&app, Some(&cookie), PASSWORD, "BOB").await;
    assert!(same.text.contains("Rejected"), "{}", same.text);
    // Aucun refus n'a ferme la session ni change le mot de passe.
    assert_eq!(layout(&app, Some(&cookie)).await["user"]["username"], "bob");
    assert_eq!(login_fn(&app, "bob", PASSWORD).await.status, StatusCode::OK);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_successful_password_change_closes_every_session(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let first = session_for(&app, &state, "bob", Role::User).await;
    let second = login_cookie(&app, "bob").await;

    let new = "un nouveau mot de passe";
    let r = change_fn(&app, Some(&first), PASSWORD, new).await;
    assert_eq!((r.status, r.text.as_str()), (StatusCode::OK, "\"Changed\""));
    assert!(!r.text.contains(new));
    for cookie in [&first, &second] {
        assert!(
            layout(&app, Some(cookie)).await["user"].is_null(),
            "session revoquee"
        );
    }
    assert!(
        login_fn(&app, "bob", PASSWORD)
            .await
            .text
            .contains("InvalidCredentials")
    );
    assert!(login_fn(&app, "bob", new).await.text.contains("Success"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_password_cannot_be_changed_without_a_session(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    account(&state, "bob").await;
    let r = change_fn(&app, None, PASSWORD, "un nouveau mot de passe").await;
    assert_ne!(r.status, StatusCode::OK, "{}", r.text);
    assert!(
        login_fn(&app, "bob", PASSWORD)
            .await
            .text
            .contains("Success")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_ui_password_change_is_locked_after_repeated_failures(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    for _ in 0..5 {
        let r = change_fn(
            &app,
            Some(&cookie),
            "faux faux faux",
            "un nouveau mot de passe",
        )
        .await;
        assert_eq!(r.text, "\"WrongCurrent\"");
    }
    let locked = change_fn(&app, Some(&cookie), PASSWORD, "un nouveau mot de passe").await;
    let v: serde_json::Value = serde_json::from_str(&locked.text).unwrap();
    assert!(v["Locked"]["retry_after_s"].as_u64().unwrap() > 0, "{v}");
    assert_eq!(
        layout(&app, Some(&cookie)).await["user"]["username"],
        "bob",
        "rien n'a ete change"
    );
}

// ---------------------------------------------------------------- 7d : administration des emplacements

const JSON: (&str, &str) = ("content-type", "application/json");

async fn sf(
    app: &Router,
    cookie: Option<&str>,
    name: &str,
    body: Option<&str>,
) -> serde_json::Value {
    let r = sf_raw(app, cookie, name, body).await;
    assert_eq!(r.status, StatusCode::OK, "{name}: {}", r.text);
    serde_json::from_str(&r.text).unwrap()
}

async fn sf_raw(app: &Router, cookie: Option<&str>, name: &str, body: Option<&str>) -> Resp {
    let mut extra = vec![if body.is_some_and(|b| b.starts_with('{')) {
        JSON
    } else {
        FORM
    }];
    if let Some(c) = cookie {
        extra.push(("cookie", c));
    }
    send(
        app,
        Method::POST,
        &format!("/sf/locations/{name}"),
        &extra,
        body,
    )
    .await
}

fn input(id: Option<i64>, name: &str, description: &str, active: bool) -> String {
    serde_json::json!({"input": {"id": id, "name": name, "description": description, "active": active}}).to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_can_use_the_location_functions(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let create = input(None, "Salon", "", true);

    for cookie in [None, Some(user.as_str())] {
        assert_eq!(
            sf_raw(&app, cookie, "list", Some("")).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            sf_raw(&app, cookie, "save", Some(&create)).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            sf_raw(&app, cookie, "delete", Some("id=1")).await.status,
            StatusCode::FORBIDDEN
        );
    }
    let listed = sf(&app, Some(&admin), "list", Some("")).await;
    assert_eq!(
        listed.as_array().unwrap().len(),
        0,
        "les refus n'ont rien cree"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_manages_locations(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let c = Some(admin.as_str());

    // Creation (le nom est rogne) puis liste triee par nom.
    let saved = sf(
        &app,
        c,
        "save",
        Some(&input(None, "  Salon  ", "Rez-de-chaussee", true)),
    )
    .await;
    assert_eq!(saved["Saved"]["name"], "Salon");
    let id = saved["Saved"]["id"].as_i64().unwrap();
    sf(&app, c, "save", Some(&input(None, "Cave", "", false))).await;
    let listed = sf(&app, c, "list", Some("")).await;
    let names: Vec<_> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Cave", "Salon"]);

    // Modification.
    let upd = sf(
        &app,
        c,
        "save",
        Some(&input(Some(id), "Salon TV", "", false)),
    )
    .await;
    assert_eq!(
        (
            upd["Saved"]["name"].as_str(),
            upd["Saved"]["active"].as_bool()
        ),
        (Some("Salon TV"), Some(false))
    );

    // Element disparu : ni creation ni erreur 500.
    let gone = sf(
        &app,
        c,
        "save",
        Some(&input(Some(id + 1000), "Fantome", "", true)),
    )
    .await;
    assert_eq!(gone, "NotFound");

    // Suppression, puis suppression d'un element deja supprime.
    assert_eq!(
        sf(&app, c, "delete", Some(&format!("id={id}"))).await,
        "Deleted"
    );
    assert_eq!(
        sf(&app, c, "delete", Some(&format!("id={id}"))).await,
        "NotFound"
    );
    assert_eq!(
        sf(&app, c, "list", Some(""))
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_locations_are_reported_field_by_field(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let c = Some(admin.as_str());

    let r = sf(
        &app,
        c,
        "save",
        Some(&input(None, "   ", &"x".repeat(129), true)),
    )
    .await;
    assert_eq!(
        r,
        serde_json::json!({"Invalid": [
            {"field": "name", "kind": "Required"},
            {"field": "description", "kind": {"TooLong": {"max": 128}}},
        ]})
    );
    let r = sf(
        &app,
        c,
        "save",
        Some(&input(None, &"n".repeat(65), "", true)),
    )
    .await;
    assert_eq!(r["Invalid"][0]["kind"]["TooLong"]["max"], 64);
    assert_eq!(
        sf(&app, c, "list", Some(""))
            .await
            .as_array()
            .unwrap()
            .len(),
        0,
        "rien n'est enregistre"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_locations_page_is_for_administrators_and_escapes_content(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    sf(
        &app,
        Some(&admin),
        "save",
        Some(&input(None, "<script>alert(1)</script>", "a & b", true)),
    )
    .await;

    let anon = get(&app, "/admin/locations", &[HTML]).await;
    assert_eq!(anon.status, StatusCode::FOUND);
    assert_eq!(
        anon.headers[header::LOCATION],
        "/login?next=%2Fadmin%2Flocations"
    );

    let denied = get(&app, "/admin/locations", &[("cookie", &user)]).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert!(denied.text.contains("Accès refusé"));
    assert!(
        !denied.text.contains("crud-table") && !denied.text.contains("&lt;script&gt;"),
        "aucune donnee"
    );

    let page = get(&app, "/admin/locations", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("Emplacements") && page.text.contains("Ajouter un emplacement"));
    assert!(page.text.contains("crud-table") && page.text.contains("a &amp; b"));
    assert!(
        page.text.contains("&lt;script&gt;alert(1)&lt;/script&gt;"),
        "le nom est echappe"
    );
    assert!(
        !page.text.contains("<script>alert(1)"),
        "aucun script injecte"
    );
    let en = get(
        &app,
        "/admin/locations",
        &[("cookie", &admin), ("accept-language", "en")],
    )
    .await;
    assert!(en.text.contains("Add a location") && en.text.contains("Actions"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_admin_menu_is_only_shown_to_administrators(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let u = get(&app, "/", &[("cookie", &user)]).await;
    assert!(!u.text.contains("vc-nav-admin") && !u.text.contains("/admin/locations"));
    let a = get(&app, "/", &[("cookie", &admin)]).await;
    assert!(a.text.contains("vc-nav-admin") && a.text.contains(r#"href="/admin/locations""#));
}

// ---------------------------------------------------------------- sources MQTT (R3 lot 4)

fn source_input(id: Option<i64>, name: &str, topic: &str, qos: u8) -> String {
    serde_json::json!({"input": {
        "id": id, "name": name, "topic_pattern": topic, "qos": qos, "active": true,
        "log_messages": false, "auto_device": true, "auto_device_type": "sensor",
        "base_topic": "solarmax/", "handler_code": "print(1)", "description": ""
    }})
    .to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_can_use_the_mqtt_source_functions(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let body = source_input(None, "Linky", "linky/#", 0);
    for cookie in [None, Some(user.as_str())] {
        for (path, payload) in [
            ("mqtt-sources/list", ""),
            ("mqtt-sources/save", body.as_str()),
            ("mqtt-sources/delete", "id=1"),
        ] {
            let r = sf_at(&app, cookie, path, payload).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_manages_mqtt_sources_and_the_listener_is_told(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let told = || async {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state.sources_changed.notified(),
        )
        .await
        .is_ok()
    };

    let saved = sf_json(
        &app,
        &admin,
        "mqtt-sources/save",
        &source_input(None, "  Linky  ", " linky/# ", 1),
    )
    .await;
    assert_eq!(saved["Saved"]["name"], "Linky");
    assert_eq!(saved["Saved"]["topic_pattern"], "linky/#");
    assert_eq!(
        saved["Saved"]["base_topic"], "solarmax",
        "le / final est retire"
    );
    assert_eq!(saved["Saved"]["handler_code"], "print(1)");
    assert!(told().await, "creation signalee a l'ecoute");
    let id = saved["Saved"]["id"].as_i64().unwrap();

    let upd = sf_json(
        &app,
        &admin,
        "mqtt-sources/save",
        &source_input(Some(id), "Linky", "linky/+/papp", 2),
    )
    .await;
    assert_eq!(upd["Saved"]["qos"], 2);
    assert!(told().await, "modification signalee");

    let listed = sf_json(&app, &admin, "mqtt-sources/list", "").await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["topic_pattern"], "linky/+/papp");

    let gone = sf_json(
        &app,
        &admin,
        "mqtt-sources/save",
        &source_input(Some(id + 1000), "X", "x/#", 0),
    )
    .await;
    assert_eq!(gone, "NotFound");

    assert_eq!(
        sf_json(&app, &admin, "mqtt-sources/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert!(told().await, "suppression signalee");
    assert_eq!(
        sf_json(&app, &admin, "mqtt-sources/delete", &format!("id={id}")).await,
        "NotFound"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_mqtt_sources_are_reported_field_by_field(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    // `#` au milieu du filtre, QoS 3, nom vide.
    let bad = sf_json(
        &app,
        &admin,
        "mqtt-sources/save",
        &source_input(None, " ", "a/#/b", 3),
    )
    .await;
    let fields: Vec<&str> = bad["Invalid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["name", "topic_pattern", "qos"]);
    let listed = sf_json(&app, &admin, "mqtt-sources/list", "").await;
    assert_eq!(listed.as_array().unwrap().len(), 0, "rien n'a ete cree");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_mqtt_sources_page_is_for_administrators_and_escapes_content(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let mut hostile = dominet_domain::device::MqttSource::draft("<script>alert(1)</script>", "x/#");
    hostile.description = "a & b".into();
    dominet_domain::repo::MqttSourceRepo::create(&state.store.mqtt_sources(), &hostile)
        .await
        .unwrap();

    let anon = get(&app, "/admin/mqtt-sources", &[HTML]).await;
    assert_eq!(anon.status, StatusCode::FOUND);
    let denied = get(&app, "/admin/mqtt-sources", &[("cookie", &user)]).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert!(!denied.text.contains("&lt;script&gt;"), "aucune donnee");

    let page = get(&app, "/admin/mqtt-sources", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("Sources MQTT") && page.text.contains("Ajouter une source"));
    assert!(
        page.text.contains(r#"href="/admin/mqtt-sources""#),
        "entree du menu"
    );
    assert!(page.text.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    assert!(!page.text.contains("<script>alert(1)"));
    let en = get(
        &app,
        "/admin/mqtt-sources",
        &[("cookie", &admin), ("accept-language", "en")],
    )
    .await;
    assert!(en.text.contains("Add a source"));
}

// ---------------------------------------------------------------- 7e : navigation et reglages

async fn sf_at(app: &Router, cookie: Option<&str>, path: &str, body: &str) -> Resp {
    let mut extra = vec![if body.starts_with('{') { JSON } else { FORM }];
    if let Some(c) = cookie {
        extra.push(("cookie", c));
    }
    send(
        app,
        Method::POST,
        &format!("/sf/{path}"),
        &extra,
        Some(body),
    )
    .await
}

async fn sf_json(app: &Router, cookie: &str, path: &str, body: &str) -> serde_json::Value {
    let r = sf_at(app, Some(cookie), path, body).await;
    assert_eq!(r.status, StatusCode::OK, "{path}: {}", r.text);
    serde_json::from_str(&r.text).unwrap()
}

#[allow(clippy::too_many_arguments)]
fn nav_input(
    id: Option<i64>,
    icon: &str,
    label: &str,
    url: &str,
    target: &str,
    admin: bool,
    position: &str,
    order: &str,
) -> String {
    serde_json::json!({"input": {
        "id": id, "icon_name": icon, "label": label, "url": url, "url_target": target,
        "admin_only": admin, "position": position, "sort_order": order
    }})
    .to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_can_use_the_navigation_and_settings_functions(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let create = nav_input(None, "video", "Pirate", "/x", "_self", false, "top", "1");
    for cookie in [None, Some(user.as_str())] {
        for (path, body) in [
            ("navbar/list", ""),
            ("navbar/save", create.as_str()),
            ("navbar/delete", "id=1"),
            ("settings/list", ""),
            (
                "settings/save",
                "key=EMAIL_RECIPIENT&value=evil%40example.org",
            ),
        ] {
            let r = sf_at(&app, cookie, path, body).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
            assert!(
                !r.text.contains("EMAIL_RECIPIENT") && !r.text.contains("Surveillance"),
                "{path}"
            );
        }
    }
    // Rien n'a ete modifie.
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let items = sf_json(&app, &admin, "navbar/list", "").await;
    assert_eq!(items.as_array().unwrap().len(), 7);
    let settings = sf_json(&app, &admin, "settings/list", "").await;
    let email = settings
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "EMAIL_RECIPIENT")
        .unwrap();
    assert_eq!(email["value"], "dominet@localhost");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_manages_the_navigation_and_the_sidebar_follows(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let user = session_for(&app, &state, "bob", Role::User).await;

    let made = sf_json(
        &app,
        &admin,
        "navbar/save",
        &nav_input(
            None,
            "house",
            "  Maison  ",
            "/maison/",
            "_self",
            false,
            "top",
            "5",
        ),
    )
    .await;
    let id = made["Saved"]["id"].as_i64().unwrap();
    assert_eq!(
        (
            made["Saved"]["label"].as_str(),
            made["Saved"]["sort_order"].as_i64()
        ),
        (Some("Maison"), Some(5))
    );
    // Visible tout de suite dans la navigation de n'importe quel compte.
    assert_eq!(
        labels(&layout(&app, Some(&user)).await["nav_top"]),
        ["Maison"]
    );

    // Reservee aux administrateurs : disparait pour un compte ordinaire.
    sf_json(
        &app,
        &admin,
        "navbar/save",
        &nav_input(
            Some(id),
            "house",
            "Maison",
            "/maison/",
            "_blank",
            true,
            "bottom",
            "",
        ),
    )
    .await;
    assert_eq!(labels(&layout(&app, Some(&user)).await["nav_top"]).len(), 0);
    let a = layout(&app, Some(&admin)).await;
    assert_eq!(labels(&a["nav_bottom"]), ["Maison"]);
    assert_eq!(a["nav_bottom"][0]["url_target"], "_blank");
    // Une entree interne en nouvel onglet est un lien HTML ordinaire qui garde sa cible.
    let page = get(&app, "/", &[("cookie", &admin)]).await;
    assert!(page.text.contains(r#"target="_blank""#) && page.text.contains("/maison/"));

    assert_eq!(
        sf_json(
            &app,
            &admin,
            "navbar/save",
            &nav_input(
                Some(id + 999),
                "house",
                "X",
                "/x",
                "_self",
                false,
                "top",
                "0"
            )
        )
        .await,
        "NotFound"
    );
    assert_eq!(
        sf_json(&app, &admin, "navbar/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "navbar/delete", &format!("id={id}")).await,
        "NotFound"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_navigation_entries_are_reported_field_by_field(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let problems = |v: &serde_json::Value| -> Vec<(String, String)> {
        v["Invalid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (p["field"].as_str().unwrap().into(), p["kind"].to_string()))
            .collect()
    };

    // Plusieurs champs a la fois ; une URL "javascript:" est refusee.
    let r = sf_json(
        &app,
        &admin,
        "navbar/save",
        &nav_input(
            None,
            "Video!",
            " ",
            "javascript:alert(1)",
            "_top",
            false,
            "milieu",
            "abc",
        ),
    )
    .await;
    let fields: Vec<_> = problems(&r).into_iter().map(|(f, _)| f).collect();
    for f in [
        "icon_name",
        "label",
        "url",
        "url_target",
        "position",
        "sort_order",
    ] {
        assert!(fields.contains(&f.to_string()), "{f} : {fields:?}");
    }
    // Icone bien formee mais inconnue de Lucide.
    let r = sf_json(
        &app,
        &admin,
        "navbar/save",
        &nav_input(None, "no-such-icon", "X", "/x", "_self", false, "top", "1"),
    )
    .await;
    assert_eq!(
        problems(&r),
        [("icon_name".to_string(), "\"Invalid\"".to_string())]
    );
    // Ordre hors bornes.
    let r = sf_json(
        &app,
        &admin,
        "navbar/save",
        &nav_input(None, "house", "X", "/x", "_self", false, "top", "40000"),
    )
    .await;
    assert_eq!(r["Invalid"][0]["kind"]["OutOfRange"]["max"], 32767);
    let r = sf_json(
        &app,
        &admin,
        "navbar/save",
        &nav_input(None, "house", "X", "/x", "_self", false, "top", "-1"),
    )
    .await;
    assert_eq!(r["Invalid"][0]["field"], "sort_order");
    assert_eq!(
        sf_json(&app, &admin, "navbar/list", "")
            .await
            .as_array()
            .unwrap()
            .len(),
        0,
        "rien n'est enregistre"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_navigation_and_settings_pages_are_for_administrators(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    for path in ["/admin/navbar", "/admin/settings"] {
        let anon = get(&app, path, &[HTML]).await;
        assert_eq!(anon.status, StatusCode::FOUND, "{path}");
        let denied = get(&app, path, &[("cookie", &user)]).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{path}");
        assert!(
            !denied.text.contains("EMAIL_RECIPIENT") && !denied.text.contains("Surveillance</td>"),
            "{path}"
        );
        assert_eq!(
            get(&app, path, &[("cookie", &admin)]).await.status,
            StatusCode::OK,
            "{path}"
        );
    }
    let nav = get(&app, "/admin/navbar", &[("cookie", &admin)]).await;
    for text in ["Surveillance", "Zigbee2MQTT", "Ajouter une entrée", "Haut"] {
        assert!(nav.text.contains(text), "absent : {text}");
    }
    let set = get(
        &app,
        "/admin/settings",
        &[("cookie", &admin), ("accept-language", "en")],
    )
    .await;
    for text in [
        "EMAIL_RECIPIENT",
        "MOTION_THRESHOLD",
        "Notifications",
        "Secret",
    ] {
        assert!(set.text.contains(text), "absent : {text}");
    }
    let menu = get(&app, "/", &[("cookie", &admin)]).await;
    assert!(
        menu.text.contains(r#"href="/admin/navbar""#)
            && menu.text.contains(r#"href="/admin/settings""#)
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn settings_are_validated_by_type_and_applied_immediately(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let save = |k: &str, v: &str| form(&[("key", k), ("value", v)]);

    let ok = sf_json(
        &app,
        &admin,
        "settings/save",
        &save("MOTION_THRESHOLD", "0.25"),
    )
    .await;
    assert_eq!(ok["Saved"]["value"], "0.25");
    assert_eq!(
        state.settings.get_string("MOTION_THRESHOLD").await.unwrap(),
        "0.25",
        "cache serveur a jour"
    );
    // La valeur est dans le HTML rendu par le serveur (attribut), pas seulement posee apres l'hydratation.
    let page = get(&app, "/admin/settings", &[("cookie", &admin)]).await;
    assert!(
        page.text.contains(r#"value="0.25""#),
        "champ prerempli cote serveur"
    );
    assert_eq!(
        sf_json(
            &app,
            &admin,
            "settings/save",
            &save("MOTION_THRESHOLD", "beaucoup")
        )
        .await,
        "Invalid"
    );
    assert_eq!(
        sf_json(
            &app,
            &admin,
            "settings/save",
            &save("PROMETHEUS_ALERT_COOLDOWN", "1.5")
        )
        .await,
        "Invalid"
    );
    assert_eq!(
        sf_json(&app, &admin, "settings/save", &save("NOPE", "x")).await,
        "UnknownKey"
    );
    // Une valeur refusee n'a rien change.
    let list = sf_json(&app, &admin, "settings/list", "").await;
    let thr = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "MOTION_THRESHOLD")
        .unwrap();
    assert_eq!(thr["value"], "0.25");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_secret_setting_never_reaches_the_browser(pool: PgPool) {
    let (app, state, _guard) = app_with_key(pool, Some(TEST_MASTER_KEY)).await;
    seed(&state).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let secret = "super-secret-cle-api-123";

    let r = sf_json(
        &app,
        &admin,
        "settings/save",
        &form(&[("key", "SMS_FREE_KEY"), ("value", secret)]),
    )
    .await;
    assert_eq!(r["Saved"]["is_set"], true, "{r}");
    assert_ne!(r["Saved"]["value"], secret);
    assert!(
        !r.to_string().contains(secret),
        "la reponse ne contient pas le secret"
    );
    let list = sf_at(&app, Some(&admin), "settings/list", "").await;
    assert!(
        !list.text.contains(secret),
        "la liste ne contient jamais le secret"
    );
    let page = get(&app, "/admin/settings", &[("cookie", &admin)]).await;
    assert!(!page.text.contains(secret), "la page non plus");
    assert!(page.text.contains("Défini"), "seul l'etat est affiche");
    // Le serveur, lui, le relit.
    assert_eq!(
        state.settings.get_string("SMS_FREE_KEY").await.unwrap(),
        secret
    );

    // Effacer : valeur vide.
    let cleared = sf_json(
        &app,
        &admin,
        "settings/save",
        &form(&[("key", "SMS_FREE_KEY"), ("value", "")]),
    )
    .await;
    assert_eq!(cleared["Saved"]["is_set"], false);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_secret_cannot_be_saved_without_the_master_key(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let r = sf_json(
        &app,
        &admin,
        "settings/save",
        &form(&[("key", "SMS_FREE_KEY"), ("value", "abc")]),
    )
    .await;
    assert_eq!(r, "MasterKeyMissing");
    let list = sf_json(&app, &admin, "settings/list", "").await;
    let key = list
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "SMS_FREE_KEY")
        .unwrap();
    assert_eq!(key["is_set"], false);
}

// ---------------------------------------------------------------- 7f : utilisateurs

#[allow(clippy::too_many_arguments)]
fn user_input(
    id: Option<i64>,
    username: &str,
    email: &str,
    role: &str,
    active: bool,
    password: &str,
) -> String {
    serde_json::json!({"input": {
        "id": id, "username": username, "first_name": "Bob", "last_name": "Martin", "email": email,
        "mobile": "+33 6 12 34 56 78", "role": role, "is_active": active, "password": password
    }})
    .to_string()
}

const NEW_PASSWORD: &str = "un mot de passe valide";

fn problem_fields(v: &serde_json::Value) -> Vec<String> {
    v["Invalid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["field"].as_str().unwrap().to_string())
        .collect()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_can_use_the_user_functions(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let create = user_input(None, "mallory", "", "admin", true, NEW_PASSWORD);
    for cookie in [None, Some(user.as_str())] {
        for (path, body) in [
            ("users/list", ""),
            ("users/save", create.as_str()),
            ("users/delete", "id=1"),
        ] {
            let r = sf_at(&app, cookie, path, body).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
            assert!(!r.text.contains("alice"), "{path}");
        }
    }
    let list = sf_json(&app, &admin, "users/list", "").await;
    assert_eq!(list.as_array().unwrap().len(), 2, "rien n'a ete cree");
    let text = list.to_string();
    assert!(
        !text.contains("argon2") && !text.contains("password"),
        "ni condensat ni mot de passe : {text}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_creates_edits_and_deletes_accounts(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;

    // Creation : l'identifiant est rogne, le mobile normalise, le compte peut se connecter.
    let made = sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(
            None,
            "carol",
            "carol@example.org",
            "user",
            true,
            NEW_PASSWORD,
        ),
    )
    .await;
    let id = made["Saved"]["id"].as_i64().unwrap();
    assert_eq!(made["Saved"]["mobile"], "+33612345678");
    assert!(!made.to_string().contains(NEW_PASSWORD));
    assert!(
        login_fn(&app, "carol", NEW_PASSWORD)
            .await
            .text
            .contains("Success")
    );

    // Identifiant deja pris, casse ignoree.
    let dup = sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(None, "CAROL", "", "user", true, NEW_PASSWORD),
    )
    .await;
    assert_eq!(
        dup,
        serde_json::json!({"Invalid": [{"field": "username", "kind": "Taken"}]})
    );

    // Modification : role, e-mail ; l'identifiant envoye est ignore (immuable) ; le mot de passe reste valable.
    let upd = sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(Some(id), "pirate", "new@example.org", "admin", true, ""),
    )
    .await;
    assert_eq!(
        (
            upd["Saved"]["username"].as_str(),
            upd["Saved"]["role"].as_str(),
            upd["Saved"]["email"].as_str()
        ),
        (Some("carol"), Some("admin"), Some("new@example.org"))
    );
    assert!(
        login_fn(&app, "carol", NEW_PASSWORD)
            .await
            .text
            .contains("Success")
    );

    assert_eq!(
        sf_json(
            &app,
            &admin,
            "users/save",
            &user_input(Some(id + 999), "x", "", "user", true, "")
        )
        .await,
        "NotFound"
    );
    assert_eq!(
        sf_json(&app, &admin, "users/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "users/delete", &format!("id={id}")).await,
        "NotFound"
    );
    assert!(
        login_fn(&app, "carol", NEW_PASSWORD)
            .await
            .text
            .contains("InvalidCredentials")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_accounts_are_reported_field_by_field(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let bad = serde_json::json!({"input": {
        "id": null, "username": "a b", "first_name": "", "last_name": "", "email": "pas-un-email",
        "mobile": "12ab", "role": "superuser", "is_active": true, "password": "court"
    }})
    .to_string();
    let fields = problem_fields(&sf_json(&app, &admin, "users/save", &bad).await);
    for f in ["username", "email", "mobile", "role", "password"] {
        assert!(fields.contains(&f.to_string()), "{f} : {fields:?}");
    }
    // Mot de passe obligatoire a la creation, motif de la politique sinon.
    let r = sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(None, "dave", "", "user", true, ""),
    )
    .await;
    assert_eq!(
        r,
        serde_json::json!({"Invalid": [{"field": "password", "kind": "Required"}]})
    );
    let r = sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(None, "dave", "", "user", true, "1234567890123"),
    )
    .await;
    assert_eq!(
        r["Invalid"][0]["kind"],
        serde_json::json!({"Password": "AllDigits"})
    );
    assert_eq!(
        sf_json(&app, &admin, "users/list", "")
            .await
            .as_array()
            .unwrap()
            .len(),
        1,
        "rien n'est cree"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn deactivating_or_resetting_a_password_cuts_the_targets_sessions(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let bob = session_for(&app, &state, "bob", Role::User).await;
    let carol = session_for(&app, &state, "carol", Role::User).await;
    let list = sf_json(&app, &admin, "users/list", "").await;
    let id_of = |name: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == name)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };

    // Desactivation : effet immediat sur la session ouverte, et plus de connexion possible.
    sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(Some(id_of("bob")), "bob", "", "user", false, ""),
    )
    .await;
    assert!(layout(&app, Some(&bob)).await["user"].is_null());
    assert!(
        login_fn(&app, "bob", PASSWORD)
            .await
            .text
            .contains("InvalidCredentials")
    );

    // Reinitialisation : toutes les sessions du compte sont fermees ; l'ancien mot de passe ne vaut plus.
    sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(
            Some(id_of("carol")),
            "carol",
            "",
            "user",
            true,
            NEW_PASSWORD,
        ),
    )
    .await;
    assert!(layout(&app, Some(&carol)).await["user"].is_null());
    assert!(
        login_fn(&app, "carol", PASSWORD)
            .await
            .text
            .contains("InvalidCredentials")
    );
    assert!(
        login_fn(&app, "carol", NEW_PASSWORD)
            .await
            .text
            .contains("Success")
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_cannot_cut_their_own_access(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let alice = session_for(&app, &state, "alice", Role::Admin).await;
    session_for(&app, &state, "bob", Role::Admin).await;
    let list = sf_json(&app, &alice, "users/list", "").await;
    let me = list
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["username"] == "alice")
        .unwrap()["id"]
        .as_i64()
        .unwrap();

    assert_eq!(
        sf_json(&app, &alice, "users/delete", &format!("id={me}")).await,
        serde_json::json!({"Blocked": "Yourself"})
    );
    for (role, active) in [("user", true), ("admin", false)] {
        let r = sf_json(
            &app,
            &alice,
            "users/save",
            &user_input(Some(me), "alice", "", role, active, ""),
        )
        .await;
        assert_eq!(
            r,
            serde_json::json!({"Blocked": "Yourself"}),
            "{role} {active}"
        );
    }
    let r = sf_json(
        &app,
        &alice,
        "users/save",
        &user_input(Some(me), "alice", "", "admin", true, NEW_PASSWORD),
    )
    .await;
    assert_eq!(r, serde_json::json!({"Blocked": "OwnPassword"}));
    // Modifier son propre profil sans toucher au role ni a l'activation reste possible.
    let ok = sf_json(
        &app,
        &alice,
        "users/save",
        &user_input(Some(me), "alice", "alice@example.org", "admin", true, ""),
    )
    .await;
    assert_eq!(ok["Saved"]["email"], "alice@example.org");
    assert_eq!(
        layout(&app, Some(&alice)).await["user"]["username"],
        "alice",
        "session intacte"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn two_administrators_demoting_each_other_never_leave_the_system_without_one(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let a = session_for(&app, &state, "alice", Role::Admin).await;
    let b = session_for(&app, &state, "bob", Role::Admin).await;
    let list = sf_json(&app, &a, "users/list", "").await;
    let id_of = |name: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|u| u["username"] == name)
            .unwrap()["id"]
            .as_i64()
            .unwrap()
    };
    let (ida, idb) = (id_of("alice"), id_of("bob"));

    // Chacun retrograde l'autre au meme instant. Au plus une requete reussit ; l'autre est refusee (403 car son
    // auteur n'est plus administrateur, ou « dernier administrateur » si elle etait deja admise).
    for round in 0..5 {
        let (body_a, body_b) = (
            user_input(Some(idb), "bob", "", "user", true, ""),
            user_input(Some(ida), "alice", "", "user", true, ""),
        );
        let (r1, r2) = tokio::join!(
            sf_at(&app, Some(&a), "users/save", &body_a),
            sf_at(&app, Some(&b), "users/save", &body_b),
        );
        let saved = [&r1, &r2]
            .iter()
            .filter(|r| r.text.contains("Saved"))
            .count();
        assert_eq!(saved, 1, "tour {round} : {} / {}", r1.text, r2.text);
        let n = state.store.users().count_active_admins().await.unwrap();
        assert_eq!(n, 1, "tour {round}");
        // Rearme pour le tour suivant.
        for id in [ida, idb] {
            state
                .store
                .users()
                .update_profile(
                    dominet_domain::user::UserId(id),
                    &dominet_domain::user::UserProfile {
                        email: String::new(),
                        first_name: String::new(),
                        last_name: String::new(),
                        mobile: String::new(),
                        role: Role::Admin,
                        is_active: true,
                    },
                )
                .await
                .unwrap();
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_users_page_is_for_administrators_and_escapes_content(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    sf_json(&app, &admin, "users/save", &serde_json::json!({"input": {
        "id": null, "username": "eve", "first_name": "<img src=x onerror=alert(1)>", "last_name": "", "email": "",
        "mobile": "", "role": "user", "is_active": true, "password": NEW_PASSWORD
    }}).to_string()).await;

    assert_eq!(
        get(&app, "/admin/users", &[HTML]).await.status,
        StatusCode::FOUND
    );
    let denied = get(&app, "/admin/users", &[("cookie", &user)]).await;
    assert_eq!(denied.status, StatusCode::FORBIDDEN);
    assert!(
        !denied.text.contains("crud-table")
            && !denied.text.contains("<td>alice</td>")
            && !denied.text.contains("<td>eve</td>"),
        "aucune donnee"
    );

    let page = get(&app, "/admin/users", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    for text in [
        "Utilisateurs",
        "Ajouter un utilisateur",
        "alice",
        "bob",
        "eve",
        "Administrateur",
        "Utilisateur",
    ] {
        assert!(page.text.contains(text), "absent : {text}");
    }
    assert!(
        page.text.contains("&lt;img src=x onerror=alert(1)&gt;"),
        "nom echappe"
    );
    assert!(!page.text.contains("<img src=x"), "aucun HTML injecte");
    assert!(!page.text.contains("argon2"), "aucun condensat");
    let menu = get(&app, "/", &[("cookie", &admin)]).await;
    assert!(menu.text.contains(r#"href="/admin/users""#));
}

// ---------------------------------------------------------------- lot 8 : CSP, cache, compression

/// Valeur du nonce annoncee par l'en-tete CSP.
fn csp_nonce(r: &Resp) -> String {
    let csp = r.headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
    let from = csp.find("'nonce-").expect("nonce dans la CSP") + "'nonce-".len();
    csp[from..from + csp[from..].find('\'').unwrap()].to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn every_html_page_carries_a_csp_whose_nonce_matches_its_scripts(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let c = ("cookie", admin.as_str());
    let mut nonces = std::collections::HashSet::new();
    for path in [
        "/login",
        "/nope",
        "/",
        "/account",
        "/admin/locations",
        "/admin/navbar",
        "/admin/settings",
        "/admin/users",
    ] {
        let r = get(&app, path, &[c]).await;
        let nonce = csp_nonce(&r);
        assert!(nonce.len() >= 16, "{path}: nonce trop court");
        nonces.insert(nonce.clone());
        // Tout script en ligne porte le nonce de CETTE reponse.
        for (i, _) in r.text.match_indices("<script") {
            let tag = &r.text[i..i + r.text[i..].find('>').unwrap()];
            assert!(
                tag.contains(&format!(r#"nonce="{nonce}""#)),
                "{path}: script sans nonce : {tag}"
            );
        }
        // Aucun attribut interdit par la politique (style en ligne, gestionnaires d'evenements).
        let body = &r.text[r.text.find("<body").unwrap()..];
        assert!(!body.contains(r#" style=""#), "{path}: attribut style");
        assert!(
            !regex_like_inline_handler(body),
            "{path}: gestionnaire en ligne"
        );
        for header_name in [
            "x-content-type-options",
            "cross-origin-opener-policy",
            "cross-origin-resource-policy",
        ] {
            assert!(r.headers.contains_key(header_name), "{path}: {header_name}");
        }
        assert_eq!(r.headers[header::CACHE_CONTROL], "no-store", "{path}");
    }
    assert_eq!(nonces.len(), 8, "un nonce different a chaque reponse");
    // La CSP n'autorise ni script en ligne sans nonce, ni eval, ni source tierce.
    let csp = get(&app, "/login", &[]).await.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_string();
    // Seule exception voulue : les iframes vers l'origine de Grafana (reglage GRAFANA_PUBLIC_URL, `http://localhost:3000` par defaut).
    assert!(csp.contains("; frame-src http://localhost:3000"), "{csp}");
    // Et l'origine de MediaMTX (signalisation WHEP du live, reglage MEDIAMTX_WEBRTC_DOMAIN, `localhost:8889` par defaut) pour `connect-src`.
    assert!(
        csp.contains("connect-src 'self' http://localhost:8889;"),
        "{csp}"
    );
    let csp_without_wasm = csp
        .replace("wasm-unsafe-eval", "")
        .replace("; frame-src http://localhost:3000", "")
        .replace(" http://localhost:8889", "");
    for forbidden in ["unsafe-inline", "unsafe-eval", "http:", "https:", " * "] {
        assert!(
            !csp_without_wasm.contains(forbidden),
            "{forbidden} dans {csp}"
        );
    }
    assert!(csp.contains("frame-ancestors 'none'") && csp.contains("object-src 'none'"));
}

/// Vrai si le HTML contient un attribut `onxxx="..."`.
fn regex_like_inline_handler(html: &str) -> bool {
    html.match_indices(" on").any(|(i, _)| {
        let rest = &html[i + 3..];
        let name_len = rest.chars().take_while(char::is_ascii_lowercase).count();
        name_len > 0 && rest[name_len..].starts_with("=\"")
    })
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn pages_are_compressed_on_request_and_never_cached(pool: PgPool) {
    let (app, _, _guard) = app(pool).await;
    let r = get(&app, "/login", &[("accept-encoding", "gzip")]).await;
    assert_eq!(r.headers[header::CONTENT_ENCODING], "gzip");
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
    let plain = get(&app, "/login", &[]).await;
    assert!(!plain.headers.contains_key(header::CONTENT_ENCODING));
    // Les fonctions serveur (reponses privees) ne sont pas non plus mises en cache.
    let sf = send(&app, Method::POST, "/sf/layout", &[], None).await;
    assert_eq!(sf.headers[header::CACHE_CONTROL], "no-store");
}

// ---------------------------------------------------------------- journaux : aucune donnee sensible

/// Collecte tout ce que `tracing` ecrit pendant le test.
#[derive(Clone, Default)]
struct LogSink(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn passwords_secrets_and_session_tokens_never_reach_the_logs(pool: PgPool) {
    let (app, state, _guard) = app_with_key(pool, Some(TEST_MASTER_KEY)).await;
    seed(&state).await;
    // Niveau maximal (TRACE) : on veut aussi ce que `sqlx` et `tower-http` ecrivent a bas niveau.
    let sink = LogSink::default();
    let writer = sink.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .finish();
    let logging = tracing::subscriber::set_default(subscriber);

    // Valeurs « appats » : si l'une d'elles apparait dans les journaux, c'est un defaut.
    let canaries = [
        "wrong-pw-LOG-CANARY-1",
        "new-pw-LOG-CANARY-2",
        "secret-LOG-CANARY-3",
        "user-pw-LOG-CANARY-4",
        "reset-pw-LOG-CANARY-5",
    ];
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let tokens: Vec<String> = [&admin, &user]
        .iter()
        .map(|c| c.split('=').nth(1).unwrap().to_string())
        .collect();

    // Connexion ratee, changement de mot de passe, secret, creation et reinitialisation de compte, diffusion.
    assert!(
        login_fn(&app, "bob", canaries[0])
            .await
            .text
            .contains("InvalidCredentials")
    );
    let r = change_fn(&app, Some(&user), PASSWORD, canaries[1]).await;
    assert_eq!(r.text, "\"Changed\"");
    let saved = sf_json(
        &app,
        &admin,
        "settings/save",
        &form(&[("key", "SMS_FREE_KEY"), ("value", canaries[2])]),
    )
    .await;
    assert_eq!(saved["Saved"]["is_set"], true);
    let made = sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(None, "carol", "", "user", true, canaries[3]),
    )
    .await;
    let id = made["Saved"]["id"].as_i64().unwrap();
    sf_json(
        &app,
        &admin,
        "users/save",
        &user_input(Some(id), "carol", "", "user", true, canaries[4]),
    )
    .await;

    drop(logging);
    let logs = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    // La capture fonctionne : on y voit bien les evenements attendus.
    for expected in [
        "login succeeded",
        "password changed",
        "setting changed",
        "account saved",
    ] {
        assert!(
            logs.contains(expected),
            "evenement absent des journaux (capture cassee ?) : {expected}\n{}",
            &logs[..logs.len().min(600)]
        );
    }
    assert!(
        logs.len() > 2_000,
        "trop peu de journaux captures ({} octets)",
        logs.len()
    );
    for canary in canaries {
        assert!(
            !logs.contains(canary),
            "valeur sensible dans les journaux : {canary}"
        );
    }
    for token in &tokens {
        assert!(
            !logs.contains(token.as_str()),
            "jeton de session dans les journaux"
        );
    }
    // Ni condensat (`$argon2id$...`) ni mot de passe. (Le NOM de la colonne `password_hash` peut figurer dans le texte SQL
    // journalise par `sqlx` a bas niveau, avec des parametres `$1` : les valeurs ne le sont pas.)
    for needle in ["$argon2", PASSWORD] {
        assert!(!logs.contains(needle), "{needle} dans les journaux");
    }
}

// ---------------------------------------------------------------- R2 lot 2 : etat d'appareil d'essai

/// Encodage pourcent minimal d'une valeur de formulaire.
fn urlenc(v: &str) -> String {
    use std::fmt::Write;
    v.bytes().fold(String::new(), |mut out, b| {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
        out
    })
}

fn inject_body(device: i64, state: &str, available: bool) -> String {
    format!(
        "device_id={device}&state={}&available={available}",
        urlenc(state)
    )
}

/// Cree un appareil et renvoie son numero (le simulateur et les widgets n'acceptent plus de numero fictif).
async fn make_device(state: &AppState, name: &str) -> i64 {
    use dominet_domain::device::DeviceDraft;
    use dominet_domain::repo::DeviceRepo;
    state
        .store
        .devices()
        .create(&DeviceDraft::new(name))
        .await
        .unwrap()
        .id
        .0
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_can_inject_a_test_device_state(pool: PgPool) {
    use dominet_proto::Topic;
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let body = inject_body(7, r#"{"state":"ON"}"#, true);
    for cookie in [None, Some(user.as_str())] {
        let r = sf_at(&app, cookie, "devices/inject", &body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
    assert!(
        state.hub.retained(&Topic::Device(7)).is_none(),
        "rien n'a ete retenu"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_injected_device_state_is_retained_for_late_subscribers_and_replaced_by_the_next(
    pool: PgPool,
) {
    use dominet_proto::{DeviceStateDto, Topic, WsServerMsg};
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let id = make_device(&state, "Sonde salon").await;
    let t = Topic::Device(id);

    // Un abonne deja present recoit l'etat en direct et le decompte le dit.
    let mut rx = state.hub.subscribe(&t);
    let first = sf_json(
        &app,
        &admin,
        "devices/inject",
        &inject_body(id, r#"{"state":"ON","temperature":21.5}"#, true),
    )
    .await;
    assert_eq!(first, serde_json::json!({"Sent": {"receivers": 1}}));
    let live = rx.recv().await.unwrap();
    let WsServerMsg::DeviceState(dto) = &*live else {
        panic!("{live:?}")
    };
    assert_eq!(
        (dto.device_id, dto.friendly_name.as_str(), dto.available),
        (id, "Sonde salon", true)
    );
    assert_eq!(dto.state["temperature"], 21.5);
    drop(rx);

    // Le dernier etat est retenu et remplace par le suivant (c'est ce que verront un nouvel abonne et le rendu serveur).
    sf_json(
        &app,
        &admin,
        "devices/inject",
        &inject_body(id, r#"{"state":"OFF"}"#, false),
    )
    .await;
    let kept = state.hub.retained(&t).unwrap();
    assert_eq!(
        *kept,
        WsServerMsg::DeviceState(DeviceStateDto {
            device_id: id,
            friendly_name: "Sonde salon".into(),
            state: serde_json::json!({"state": "OFF"}),
            available: false,
        })
    );
    assert!(
        state.hub.retained(&Topic::Device(id + 1)).is_none(),
        "les autres appareils ne sont pas touches"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_invalid_injected_state_is_refused_and_retains_nothing(pool: PgPool) {
    use dominet_proto::Topic;
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let real = make_device(&state, "Sonde cave").await;
    // Numero invalide, ou qui ne designe aucun appareil : le simulateur n'invente plus d'appareil.
    for id in [0, -1, real + 1000] {
        let r = sf_json(&app, &admin, "devices/inject", &inject_body(id, "{}", true)).await;
        assert_eq!(r, "InvalidDevice", "{id}");
        assert!(state.hub.retained(&Topic::Device(id)).is_none());
    }
    let too_big = format!(r#"{{"x":"{}"}}"#, "a".repeat(5000));
    for bad in [
        "[1,2]",
        "42",
        "\"texte\"",
        "pas du json",
        "",
        "null",
        too_big.as_str(),
    ] {
        let r = sf_json(
            &app,
            &admin,
            "devices/inject",
            &inject_body(real, bad, true),
        )
        .await;
        assert_eq!(r, "InvalidState", "{:?}", &bad[..bad.len().min(20)]);
    }
    assert!(state.hub.retained(&Topic::Device(real)).is_none());
    // La page d'administration propose l'outil.
    let page = get(&app, "/admin/settings", &[("cookie", &admin)]).await;
    assert!(page.text.contains("sim-device") && page.text.contains("Simuler un appareil"));
}

// ------------------------------------------------------------------ commandes d'appareils (R3 lot 5)

fn command_body(device: i64, command: &str) -> String {
    format!("device_id={device}&command={}", urlenc(command))
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn commands_need_a_session_a_valid_object_and_an_existing_active_device(pool: PgPool) {
    use dominet_domain::device::DeviceDraft;
    use dominet_domain::repo::DeviceRepo;
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let id = make_device(&state, "Lampe salon").await;

    // Sans session : refuse. Tout compte connecte peut commander (parite avec l'implementation de reference).
    let anon = sf_at(
        &app,
        None,
        "devices/command",
        &command_body(id, r#"{"state":"ON"}"#),
    )
    .await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);

    // Pas de client MQTT dans ce serveur de test : une commande valide est annoncee « indisponible », jamais « envoyee ».
    let ok = sf_json(
        &app,
        &user,
        "devices/command",
        &command_body(id, r#"{"state":"ON"}"#),
    )
    .await;
    assert_eq!(ok, "Unavailable");

    for bad in [
        "pas du json",
        "[1]",
        "42",
        "{}",
        r#"{"a":{"b":{"c":{"d":1}}}}"#,
    ] {
        let r = sf_json(&app, &user, "devices/command", &command_body(id, bad)).await;
        assert_eq!(r, "Invalid", "{bad}");
    }
    let gone = sf_json(
        &app,
        &user,
        "devices/command",
        &command_body(id + 1000, r#"{"state":"ON"}"#),
    )
    .await;
    assert_eq!(gone, "NotFound");

    // Un appareil desactive ne recoit plus de commande.
    let mut draft = DeviceDraft::new("Lampe salon");
    draft.active = false;
    state
        .store
        .devices()
        .update(dominet_domain::device::DeviceId(id), &draft)
        .await
        .unwrap();
    let off = sf_json(
        &app,
        &user,
        "devices/command",
        &command_body(id, r#"{"state":"ON"}"#),
    )
    .await;
    assert_eq!(off, "NotFound");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_open_the_pairing_window(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    for cookie in [None, Some(user.as_str())] {
        let r = sf_at(&app, cookie, "devices/permit-join", "enabled=true").await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
    // Sans client MQTT : indisponible, et l'etat memorise ne bouge pas.
    assert_eq!(
        sf_json(&app, &admin, "devices/permit-join", "enabled=true").await,
        "Unavailable"
    );
    assert_eq!(
        sf_json(&app, &user, "devices/permit-join/state", "").await,
        false
    );
    assert_eq!(
        sf_at(&app, None, "devices/permit-join/state", "")
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
}

// ------------------------------------------------------------------ administration des appareils (R3 lot 5)

fn device_input(id: Option<i64>, name: &str, kind: &str, group: &str, tracked: &str) -> String {
    serde_json::json!({"input": {
        "id": id, "friendly_name": name, "description": "", "device_type": kind, "group_id": group,
        "location_id": "", "base_topic": "zigbee2mqtt/", "active": true, "icon": "", "image_url": "",
        "tracked_keys": tracked, "hidden_keys": "update", "grafana_dashboard": false
    }})
    .to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_use_the_device_administration_functions(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let body = device_input(None, "Lampe", "light", "", "");
    for cookie in [None, Some(user.as_str())] {
        for (path, payload) in [
            ("devices-admin/list", ""),
            ("devices-admin/save", body.as_str()),
            ("devices-admin/delete", "id=1"),
            ("devices-admin/choices", ""),
            ("device-groups/list", ""),
            ("device-labels/list", ""),
        ] {
            let r = sf_at(&app, cookie, path, payload).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_administrator_manages_devices_groups_and_labels(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;

    // Groupe, puis appareil rattache a ce groupe.
    let g = sf_json(
        &app,
        &admin,
        "device-groups/save",
        &serde_json::json!({"input": {"id": null, "name": " Salon ", "location_id": "", "description": "", "icon": ""}}).to_string(),
    )
    .await;
    assert_eq!(g["Saved"]["name"], "Salon");
    let gid = g["Saved"]["id"].as_i64().unwrap().to_string();

    let d = sf_json(
        &app,
        &admin,
        "devices-admin/save",
        &device_input(None, " Sonde ", "sensor", &gid, "temperature, humidity,,"),
    )
    .await;
    assert_eq!(d["Saved"]["friendly_name"], "Sonde");
    assert_eq!(d["Saved"]["group_id"], gid);
    assert_eq!(
        d["Saved"]["base_topic"], "zigbee2mqtt",
        "le / final est retire"
    );
    assert_eq!(d["Saved"]["tracked_keys"], "temperature, humidity");
    let id = d["Saved"]["id"].as_i64().unwrap();

    // Nom deja pris, type inconnu, cle invalide, groupe disparu : refus champ par champ, rien n'est cree.
    let taken = sf_json(
        &app,
        &admin,
        "devices-admin/save",
        &device_input(None, "Sonde", "sensor", "", ""),
    )
    .await;
    assert_eq!(taken["Invalid"][0]["field"], "friendly_name");
    assert_eq!(taken["Invalid"][0]["kind"], "Taken");
    let bad_type = sf_json(
        &app,
        &admin,
        "devices-admin/save",
        &device_input(None, "X", "bizarre", "", ""),
    )
    .await;
    assert_eq!(bad_type["Invalid"][0]["field"], "device_type");
    let bad_key = sf_json(
        &app,
        &admin,
        "devices-admin/save",
        &device_input(None, "X", "sensor", "", "temp erature"),
    )
    .await;
    assert_eq!(bad_key["Invalid"][0]["field"], "tracked_keys");
    let gone_group = sf_json(
        &app,
        &admin,
        "devices-admin/save",
        &device_input(None, "X", "sensor", "99999", ""),
    )
    .await;
    assert_eq!(gone_group["Invalid"][0]["field"], "group_id");
    let not_a_number = sf_json(
        &app,
        &admin,
        "devices-admin/save",
        &device_input(None, "X", "sensor", "abc", ""),
    )
    .await;
    assert_eq!(not_a_number["Invalid"][0]["field"], "group_id");
    assert_eq!(
        sf_json(&app, &admin, "devices-admin/list", "")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Les listes deroulantes proposent groupes, emplacements et appareils.
    let choices = sf_json(&app, &admin, "devices-admin/choices", "").await;
    assert_eq!(choices["groups"][0][1], "Salon");
    assert_eq!(choices["devices"][0][1], "Sonde");

    // Libelle : cle unique.
    let label = |key: &str| {
        serde_json::json!({"input": {"id": null, "key": key, "name": "Temperature", "icon_name": "thermometer", "unit": "C"}}).to_string()
    };
    assert!(
        sf_json(&app, &admin, "device-labels/save", &label("temperature")).await["Saved"]
            .is_object()
    );
    let dup = sf_json(&app, &admin, "device-labels/save", &label("temperature")).await;
    assert_eq!(dup["Invalid"][0]["kind"], "Taken");

    // La page propose les groupes dans une liste deroulante et le menu d'administration.
    let page = get(&app, "/admin/devices", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.text.contains("Ajouter un appareil") && page.text.contains(r#"href="/admin/devices""#)
    );
    assert!(
        get(&app, "/admin/device-groups", &[("cookie", &admin)])
            .await
            .text
            .contains("Ajouter un groupe")
    );
    assert!(
        get(&app, "/admin/device-labels", &[("cookie", &admin)])
            .await
            .text
            .contains("Ajouter un libellé")
    );

    // Suppression d'un groupe : l'appareil reste, sans groupe.
    assert_eq!(
        sf_json(&app, &admin, "device-groups/delete", &format!("id={gid}")).await,
        "Deleted"
    );
    let list = sf_json(&app, &admin, "devices-admin/list", "").await;
    assert_eq!(list[0]["group_id"], "");
    assert_eq!(
        sf_json(&app, &admin, "devices-admin/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "devices-admin/delete", &format!("id={id}")).await,
        "NotFound"
    );
}

// ------------------------------------------------------------------ pages des appareils (R3 lot 5)

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_devices_page_lists_active_devices_with_filters_and_admin_tools(pool: PgPool) {
    use dominet_domain::device::{DeviceDraft, DeviceGroupDraft, DeviceType};
    use dominet_domain::repo::{DeviceGroupRepo, DeviceRepo};
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;

    let group = state
        .store
        .device_groups()
        .create(&DeviceGroupDraft {
            name: "Salon".into(),
            location: None,
            description: String::new(),
            icon: String::new(),
        })
        .await
        .unwrap();
    let mut lamp = DeviceDraft::new("Lampe <b>salon</b>");
    lamp.device_type = DeviceType::Light;
    lamp.group = Some(group.id);
    let mut probe = DeviceDraft::new("Sonde cave");
    probe.device_type = DeviceType::Sensor;
    let mut off = DeviceDraft::new("Ancienne");
    off.active = false;
    for d in [&lamp, &probe, &off] {
        state.store.devices().create(d).await.unwrap();
    }
    state
        .store
        .devices()
        .apply_state("Sonde cave", &serde_json::json!({"temperature": 12.5}))
        .await
        .unwrap();

    // Les deux ecritures de l'adresse (l'entree de navigation amorcee est `/automation/`) menent a la meme page.
    for p in ["/automation", "/automation/"] {
        assert_eq!(
            get(&app, p, &[("cookie", &user)]).await.status,
            StatusCode::OK,
            "{p}"
        );
    }
    let anon = get(&app, "/automation/", &[HTML]).await;
    assert_eq!(anon.status, StatusCode::FOUND);

    let page = get(&app, "/automation/", &[("cookie", &user)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("Sonde cave") && page.text.contains("12.5"));
    assert!(
        page.text.contains("Lampe &lt;b&gt;salon&lt;/b&gt;"),
        "nom echappe"
    );
    assert!(!page.text.contains("<b>salon</b>"));
    assert!(!page.text.contains("Ancienne"), "appareil inactif masque");
    assert!(
        !page.text.contains("permit-toggle"),
        "appairage reserve aux administrateurs"
    );

    // Filtres en GET : type, puis groupe.
    let by_type = get(&app, "/automation/?type=sensor", &[("cookie", &user)]).await;
    assert!(by_type.text.contains("Sonde cave") && !by_type.text.contains("Lampe"));
    let by_group = get(
        &app,
        &format!("/automation/?group={}", group.id.0),
        &[("cookie", &user)],
    )
    .await;
    assert!(by_group.text.contains("Lampe") && !by_group.text.contains("Sonde cave"));
    let nothing = get(&app, "/automation/?type=cover", &[("cookie", &user)]).await;
    assert!(nothing.text.contains("devices-empty"));

    let as_admin = get(&app, "/automation/", &[("cookie", &admin)]).await;
    assert!(as_admin.text.contains("permit-toggle") && as_admin.text.contains("Appairage fermé"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_device_page_shows_identity_topics_and_a_sparkline_of_tracked_keys(pool: PgPool) {
    use dominet_domain::device::{DeviceDraft, DeviceId, LogEntry};
    use dominet_domain::repo::{DeviceRepo, StateLogRepo};
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let mut draft = DeviceDraft::new("Sonde salon");
    draft.base_topic = "zigbee2mqtt".into();
    draft.tracked_keys = vec!["temperature".into(), "humidity".into()];
    let d = state.store.devices().create(&draft).await.unwrap();
    let now = chrono::Utc::now();
    for (i, t) in [20.0, 21.5, 19.0].into_iter().enumerate() {
        state
            .store
            .state_log()
            .record(
                d.id,
                now - chrono::Duration::minutes(10 - i64::try_from(i).unwrap()),
                &[LogEntry {
                    key: "temperature".into(),
                    value_num: Some(t),
                    value_str: String::new(),
                }],
            )
            .await
            .unwrap();
    }
    let path = format!("/automation/devices/{}", d.id.0);
    assert_eq!(get(&app, &path, &[HTML]).await.status, StatusCode::FOUND);
    let page = get(&app, &path, &[("cookie", &user)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("Sonde salon"));
    assert!(
        page.text.contains("zigbee2mqtt/Sonde salon/set"),
        "topic des commandes"
    );
    assert!(page.text.contains("vc-sparkline") && page.text.contains(r#"data-key="temperature""#));
    assert!(
        !page.text.contains(r#"data-key="humidity""#),
        "cle sans valeur : pas de courbe"
    );
    let missing = get(
        &app,
        &format!("/automation/devices/{}", DeviceId(d.id.0 + 1000).0),
        &[("cookie", &user)],
    )
    .await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert!(missing.text.contains("device-missing"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_grafana_dashboard_of_a_device_is_written_by_administrators_only(pool: PgPool) {
    use dominet_domain::device::DeviceDraft;
    use dominet_domain::repo::DeviceRepo;
    let dir = std::env::temp_dir().join(format!("dominet-ui-grafana-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let dir_text = dir.to_string_lossy().into_owned();
    let (app, state, _guard) =
        app_with_vars(pool, None, &[("GRAFANA_DASHBOARDS_DIR", &dir_text)]).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let mut with_keys = DeviceDraft::new("Sonde");
    with_keys.tracked_keys = vec!["temperature".into()];
    let with_keys = state.store.devices().create(&with_keys).await.unwrap();
    let without = state
        .store
        .devices()
        .create(&DeviceDraft::new("Lampe"))
        .await
        .unwrap();

    for cookie in [None, Some(user.as_str())] {
        let r = sf_at(
            &app,
            cookie,
            "devices/grafana",
            &format!("device_id={}", with_keys.id.0),
        )
        .await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }
    assert!(!dir.exists(), "rien d'ecrit par un refus");
    assert_eq!(
        sf_json(
            &app,
            &admin,
            "devices/grafana",
            &format!("device_id={}", without.id.0)
        )
        .await,
        "NoTrackedKeys"
    );
    assert_eq!(
        sf_json(&app, &admin, "devices/grafana", "device_id=99999").await,
        "NotFound"
    );
    assert_eq!(
        sf_json(
            &app,
            &admin,
            "devices/grafana",
            &format!("device_id={}", with_keys.id.0)
        )
        .await,
        "Written"
    );
    let written: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("device_Sonde.json")).unwrap())
            .unwrap();
    assert_eq!(written["uid"], format!("dominet-device-{}", with_keys.id.0));
    // L'appareil porte maintenant la case « tableau Grafana ».
    assert!(
        state
            .store
            .devices()
            .get(with_keys.id)
            .await
            .unwrap()
            .unwrap()
            .grafana_dashboard
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn without_a_dashboards_folder_the_generation_says_so(pool: PgPool) {
    use dominet_domain::device::DeviceDraft;
    use dominet_domain::repo::DeviceRepo;
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let mut d = DeviceDraft::new("Sonde");
    d.tracked_keys = vec!["temperature".into()];
    let d = state.store.devices().create(&d).await.unwrap();
    assert_eq!(
        sf_json(
            &app,
            &admin,
            "devices/grafana",
            &format!("device_id={}", d.id.0)
        )
        .await,
        "NotConfigured"
    );
}

// ------------------------------------------------------------------ stations meteo (R3 lot 6)

fn station_input(
    id: Option<i64>,
    name: &str,
    lat: &str,
    lon: &str,
    refresh: &str,
    suffix: &str,
) -> String {
    serde_json::json!({"input": {
        "id": id, "name": name, "latitude": lat, "longitude": lon, "altitude": "120", "timezone": "Europe/Paris",
        "mqtt_topic_suffix": suffix, "refresh_minutes": refresh, "enabled": true, "description": ""
    }})
    .to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_manage_weather_stations_and_the_publisher_is_woken(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let body = station_input(None, "Maison", "43.3", "5.4", "10", "meteo_locale");
    for cookie in [None, Some(user.as_str())] {
        for (path, payload) in [
            ("weather-stations/list", ""),
            ("weather-stations/save", body.as_str()),
            ("weather-stations/delete", "id=1"),
        ] {
            assert_eq!(
                sf_at(&app, cookie, path, payload).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }
    let told = || async {
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            state.stations_changed.notified(),
        )
        .await
        .is_ok()
    };

    // Virgule decimale acceptee (saisie francaise) ; l'altitude et l'intervalle sont lus comme des nombres.
    let saved = sf_json(
        &app,
        &admin,
        "weather-stations/save",
        &station_input(None, " Maison ", "43,3", "5.4", "10", "meteo_locale"),
    )
    .await;
    assert_eq!(saved["Saved"]["name"], "Maison");
    assert_eq!(saved["Saved"]["latitude"], "43.3");
    assert_eq!(saved["Saved"]["altitude"], "120");
    assert!(told().await, "creation signalee au publisher");
    let id = saved["Saved"]["id"].as_i64().unwrap();

    // Refus champ par champ, rien n'est cree : latitude hors plage, longitude non numerique, intervalle nul, suffixe invalide.
    let bad = sf_json(
        &app,
        &admin,
        "weather-stations/save",
        &station_input(None, "X", "91", "abc", "0", "a b"),
    )
    .await;
    let fields: Vec<&str> = bad["Invalid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["field"].as_str().unwrap())
        .collect();
    for expected in [
        "longitude",
        "latitude",
        "refresh_minutes",
        "mqtt_topic_suffix",
    ] {
        assert!(fields.contains(&expected), "{expected} dans {fields:?}");
    }
    assert_eq!(
        sf_json(&app, &admin, "weather-stations/list", "")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let upd = sf_json(
        &app,
        &admin,
        "weather-stations/save",
        &station_input(Some(id), "Maison", "43.3", "5.4", "30", "meteo_locale"),
    )
    .await;
    assert_eq!(upd["Saved"]["refresh_minutes"], "30");
    assert!(told().await, "modification signalee");
    assert_eq!(
        sf_json(&app, &admin, "weather-stations/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "weather-stations/delete", &format!("id={id}")).await,
        "NotFound"
    );

    let page = get(&app, "/admin/weather-stations", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.text.contains("Ajouter une station")
            && page.text.contains(r#"href="/admin/weather-stations""#)
    );
    assert_eq!(
        get(&app, "/admin/weather-stations", &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

// ------------------------------------------------------------------ tableau de bord (R2 lot 4)

mod dash {
    use super::*;
    use dominet_domain::dashboard::{
        DashboardDraft, DashboardId, GridRect, WidgetDraft, WidgetType,
    };
    use dominet_domain::repo::{DashboardRepo, UserRepo, WidgetRepo};

    pub async fn user_id(state: &AppState, name: &str) -> dominet_domain::user::UserId {
        state
            .store
            .users()
            .find_by_username(name)
            .await
            .unwrap()
            .unwrap()
            .id
    }

    pub async fn board(state: &AppState, owner: &str, name: &str) -> DashboardId {
        let owner = user_id(state, owner).await;
        let d = DashboardDraft {
            name: name.into(),
            is_default: false,
        };
        state.store.dashboards().create(owner, &d).await.unwrap().id
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn widget(
        state: &AppState,
        board: DashboardId,
        kind: WidgetType,
        title: &str,
        rect: (i16, i16, i16, i16),
        edit: impl FnOnce(&mut WidgetDraft),
    ) {
        let mut d = WidgetDraft::new(kind);
        d.title = title.into();
        d.rect = GridRect {
            col: rect.0,
            row: rect.1,
            width: rect.2,
            height: rect.3,
        };
        edit(&mut d);
        state.store.widgets().create(board, &d).await.unwrap();
    }

    /// Cree un appareil et renvoie son numero.
    pub async fn device(state: &AppState, name: &str) -> i64 {
        use dominet_domain::device::DeviceDraft;
        use dominet_domain::repo::DeviceRepo;
        state
            .store
            .devices()
            .create(&DeviceDraft::new(name))
            .await
            .unwrap()
            .id
            .0
    }

    pub fn config(v: serde_json::Value) -> impl FnOnce(&mut WidgetDraft) {
        move |d| d.config = v
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_new_account_sees_the_invitation_to_create_a_first_dashboard(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let page = get(&app, "/", &[("cookie", &cookie)]).await;
    assert_eq!(page.status, StatusCode::OK);
    for id in ["vc-dash-empty", "vc-dash-new-name", "vc-dash-create"] {
        assert!(page.text.contains(&format!(r#"id="{id}""#)), "{id} absent");
    }
    assert!(!page.text.contains("vc-dashboard-grid"));
    let anon = get(&app, "/", &[]).await;
    assert!(
        !anon.text.contains("vc-dash-empty"),
        "pas d'invitation hors session"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn creating_a_dashboard_checks_the_name_and_requires_a_session(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let anon = sf_at(&app, None, "dashboard/create", "name=Salon").await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let blank = sf_json(&app, &cookie, "dashboard/create", "name=%20%20").await;
    assert_eq!(blank["Invalid"][0]["field"], "name", "{blank}");
    let long = sf_json(
        &app,
        &cookie,
        "dashboard/create",
        &format!("name={}", "a".repeat(65)),
    )
    .await;
    assert_eq!(long["Invalid"][0]["field"], "name", "{long}");
    let ok = sf_json(&app, &cookie, "dashboard/create", "name=%20Salon%20").await;
    assert_eq!(ok["Saved"]["name"], "Salon");
    assert_eq!(
        ok["Saved"]["is_default"], true,
        "le premier tableau est le tableau par defaut"
    );
    let second = sf_json(&app, &cookie, "dashboard/create", "name=Cave").await;
    assert_eq!(second["Saved"]["is_default"], false);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_dashboard_page_places_widgets_with_classes_and_never_with_inline_styles(pool: PgPool) {
    use dominet_domain::dashboard::WidgetType as T;
    use dominet_proto::{DeviceStateDto, Topic, WsServerMsg};
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let b = dash::board(&state, "bob", "Salon").await;
    // Depuis R3, un widget ne peut viser qu'un appareil qui existe (cle etrangere).
    let sonde = dash::device(&state, "Sonde salon").await;
    dash::widget(&state, b, T::Text, "Note", (2, 1, 4, 2), |d| {
        d.content = r#"<p>Bonjour <b>vous</b></p><script>alert(1)</script><a href="javascript:x()">piege</a>"#.into();
    })
    .await;
    dash::widget(&state, b, T::Grille, "", (0, 0, 2, 2), |_| {}).await;
    dash::widget(&state, b, T::Device, "Capteur", (6, 0, 3, 2), |d| {
        d.device_id = Some(sonde);
    })
    .await;
    dash::widget(&state, b, T::Camera, "Porte", (0, 3, 2, 2), |d| {
        d.camera_uuid = Some(uuid::Uuid::nil());
    })
    .await;
    let scene = {
        use dominet_domain::repo::SceneRepo;
        use dominet_domain::scene::SceneDraft;
        state
            .store
            .scenes()
            .create(&SceneDraft::new("Ambiance"))
            .await
            .unwrap()
            .id
            .0
    };
    dash::widget(&state, b, T::Scene, "", (3, 3, 2, 2), |d| {
        d.scene_id = Some(scene);
    })
    .await;
    dash::widget(
        &state,
        b,
        T::Grafana,
        "Courbe",
        (6, 3, 4, 3),
        dash::config(serde_json::json!({"dashboard_uid": "abc", "panel_id": 2})),
    )
    .await;
    dash::widget(
        &state,
        b,
        T::Grafana,
        "Ailleurs",
        (0, 6, 4, 3),
        dash::config(serde_json::json!({"panel_url": "http://evil.example/d-solo/x"})),
    )
    .await;
    state.hub.publish_retained(
        &Topic::Device(sonde),
        WsServerMsg::DeviceState(DeviceStateDto {
            device_id: sonde,
            friendly_name: "Sonde salon".into(),
            state: serde_json::json!({"temperature": 21.5, "linkquality": 80}),
            available: true,
        }),
    );

    let page = get(&app, "/", &[("cookie", &cookie)]).await;
    assert_eq!(page.status, StatusCode::OK);
    let t = &page.text;
    // Position et taille par classes : colonne 3 (col 2 + 1), ligne 2 (row 1 + 1), 4 de large, 2 de haut.
    assert!(
        t.contains("vc-c-3 vc-r-2 vc-w-4 vc-h-2"),
        "classes de position absentes"
    );
    assert!(t.contains(r#"id="vc-dash-name""#) && t.contains("Salon"));
    assert!(
        !t.contains(r#" style=""#),
        "attribut style interdit par la CSP"
    );
    // Texte assaini : la mise en forme reste, script et lien javascript: disparaissent.
    assert!(t.contains("<b>vous</b>"));
    assert!(
        !t.contains("alert(1)") && !t.contains("javascript:"),
        "HTML non assaini"
    );
    // Appareil : dernier etat rendu par le serveur (valeur principale unique + unite).
    assert!(
        t.contains("21.5") && t.contains("Sonde salon"),
        "etat de l'appareil absent"
    );
    // Grafana : panneau par uid sur l'URL publique par defaut ; adresse d'une autre origine refusee.
    assert!(
        t.contains("http://localhost:3000/d/abc?viewPanel=2"),
        "iframe Grafana absente"
    );
    assert!(
        !t.contains("evil.example"),
        "adresse etrangere repercutee dans la page"
    );
    assert!(t.contains("grafana-problem-"), "message de refus absent");
    // Camera inconnue : le widget le dit (R6). Scene : nom et bouton d'activation (R4).
    assert!(t.contains("widget-camera-missing-"));
    assert!(
        t.contains("scene-body-") && t.contains("Ambiance") && t.contains("scene-activate-w"),
        "widget scene absent"
    );
    // La CSP autorise l'iframe vers cette origine seulement.
    let csp = page.headers["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("frame-src http://localhost:3000"), "{csp}");
}

// ------------------------------------------------------------------ scenes (R4 lot 2)

fn scene_input(id: Option<i64>, name: &str, actions: &[(i64, &str)]) -> String {
    serde_json::json!({"input": {
        "id": id, "name": name, "icon": "", "description": "", "location_id": "", "active": true,
        "actions": actions.iter().map(|(d, c)| (d.to_string(), (*c).to_string())).collect::<Vec<_>>()
    }})
    .to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn any_account_lists_and_activates_scenes_but_only_administrators_edit_them(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let lamp = make_device(&state, "Lampe salon").await;
    let body = scene_input(None, "Soiree", &[(lamp, r#"{"state":"ON"}"#)]);

    // Sans session : tout est refuse. Compte ordinaire : lecture et activation seulement.
    for (path, payload) in [("scenes/list", ""), ("scenes/activate", "id=1")] {
        let r = sf_at(&app, None, path, payload).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{path}");
    }
    for cookie in [None, Some(user.as_str())] {
        for (path, payload) in [
            ("scenes-admin/list", ""),
            ("scenes-admin/save", body.as_str()),
            ("scenes-admin/delete", "id=1"),
        ] {
            let r = sf_at(&app, cookie, path, payload).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
        }
    }

    let saved = sf_json(&app, &admin, "scenes-admin/save", &body).await;
    let id = saved["Saved"]["id"].as_i64().unwrap();
    assert_eq!(saved["Saved"]["actions"][0]["device"], "Lampe salon");
    assert_eq!(saved["Saved"]["actions"][0]["payload"], r#"{"state":"ON"}"#);

    // L'activation sans client MQTT est annoncee, commande par commande, comme « indisponible ».
    let done = sf_json(&app, &user, "scenes/activate", &format!("id={id}")).await;
    assert_eq!(done["Activated"]["sent"], 0, "{done}");
    assert_eq!(done["Activated"]["failed"][0]["device"], "Lampe salon");
    assert_eq!(done["Activated"]["failed"][0]["outcome"], "Unavailable");
    let gone = sf_json(&app, &user, "scenes/activate", "id=9999").await;
    assert_eq!(gone, "NotFound");

    // La page des scenes liste les scenes actives avec leurs commandes ; une scene desactivee disparait de la liste simple.
    let page = get(&app, "/automation/scenes", &[("cookie", &user)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(
        page.text.contains("Soiree") && page.text.contains("scene-activate-"),
        "{}",
        page.text
    );
    let anon = get(&app, "/automation/scenes", &[HTML]).await;
    assert_eq!(anon.status, StatusCode::FOUND);
    assert_eq!(
        anon.headers[header::LOCATION],
        "/login?next=%2Fautomation%2Fscenes"
    );
    let off = scene_input(Some(id), "Soiree", &[]);
    let off = off.replace(r#""active":true"#, r#""active":false"#);
    sf_json(&app, &admin, "scenes-admin/save", &off).await;
    let listed = sf_json(&app, &user, "scenes/list", "").await;
    assert!(listed.as_array().unwrap().is_empty(), "{listed}");
    let all = sf_json(&app, &admin, "scenes-admin/list", "").await;
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(
        sf_json(&app, &user, "scenes/activate", &format!("id={id}")).await,
        "NotFound",
        "une scene desactivee ne s'active plus"
    );

    // Suppression : le widget qui la visait perd sa scene.
    assert_eq!(
        sf_json(&app, &admin, "scenes-admin/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "scenes-admin/delete", &format!("id={id}")).await,
        "NotFound"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_scenes_are_reported_field_by_field(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let lamp = make_device(&state, "Lampe").await;
    let problems = |v: &serde_json::Value| -> Vec<(String, String)> {
        v["Invalid"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .map(|p| {
                (
                    p["field"].as_str().unwrap().to_string(),
                    serde_json::to_string(&p["kind"]).unwrap(),
                )
            })
            .collect()
    };
    let r = sf_json(
        &app,
        &admin,
        "scenes-admin/save",
        &scene_input(None, " ", &[]),
    )
    .await;
    assert_eq!(
        problems(&r),
        [("name".to_string(), r#""Required""#.to_string())]
    );
    for bad in ["pas du json", "[1]", "{}", r#"{"a":{"b":{"c":{"d":1}}}}"#] {
        let r = sf_json(
            &app,
            &admin,
            "scenes-admin/save",
            &scene_input(None, "X", &[(lamp, bad)]),
        )
        .await;
        assert_eq!(
            problems(&r),
            [("actions".to_string(), r#""Invalid""#.to_string())],
            "{bad}"
        );
    }
    // Appareil inconnu : refuse par la base, rapporte sur le champ des commandes.
    let r = sf_json(
        &app,
        &admin,
        "scenes-admin/save",
        &scene_input(None, "X", &[(lamp + 1000, r#"{"state":"ON"}"#)]),
    )
    .await;
    assert_eq!(
        problems(&r),
        [("actions".to_string(), r#""Invalid""#.to_string())]
    );
    // Nom deja pris.
    sf_json(
        &app,
        &admin,
        "scenes-admin/save",
        &scene_input(None, "Soiree", &[]),
    )
    .await;
    let r = sf_json(
        &app,
        &admin,
        "scenes-admin/save",
        &scene_input(None, "Soiree", &[]),
    )
    .await;
    assert_eq!(
        problems(&r),
        [("name".to_string(), r#""Taken""#.to_string())]
    );
    // Mise a jour d'une scene disparue.
    let r = sf_json(
        &app,
        &admin,
        "scenes-admin/save",
        &scene_input(Some(99_999), "Y", &[]),
    )
    .await;
    assert_eq!(r, "NotFound");
    // La page d'administration est reservee aux administrateurs.
    let page = get(&app, "/admin/scenes", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    let user = session_for(&app, &state, "bob", Role::User).await;
    assert_eq!(
        get(&app, "/admin/scenes", &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_dashboard_selector_only_offers_and_opens_the_accounts_own_dashboards(pool: PgPool) {
    use dominet_domain::dashboard::WidgetType as T;
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let bob = session_for(&app, &state, "bob", Role::User).await;
    let _eve = session_for(&app, &state, "eve", Role::User).await;
    let first = dash::board(&state, "bob", "Alpha").await;
    let second = dash::board(&state, "bob", "Beta").await;
    let secret = dash::board(&state, "eve", "Secret").await;
    dash::widget(&state, first, T::Text, "A", (0, 0, 2, 2), |d| {
        d.content = "<p>contenu-alpha</p>".into();
    })
    .await;
    dash::widget(&state, second, T::Text, "B", (0, 0, 2, 2), |d| {
        d.content = "<p>contenu-beta</p>".into();
    })
    .await;
    dash::widget(&state, secret, T::Text, "S", (0, 0, 2, 2), |d| {
        d.content = "<p>contenu-secret</p>".into();
    })
    .await;

    // Sans parametre : le tableau par defaut (le premier cree), selecteur visible car il y en a deux.
    let home = get(&app, "/", &[("cookie", &bob)]).await;
    assert!(home.text.contains("contenu-alpha") && !home.text.contains("contenu-beta"));
    assert!(home.text.contains(r#"id="vc-dash-picker""#));
    assert!(home.text.contains(&format!("/?d={}", second.0)));
    assert!(!home.text.contains("Secret"));
    // ?d= ouvre l'autre tableau du compte.
    let other = get(&app, &format!("/?d={}", second.0), &[("cookie", &bob)]).await;
    assert!(other.text.contains("contenu-beta") && !other.text.contains("contenu-alpha"));
    // Le tableau d'un autre compte, une valeur absurde ou un tableau inexistant : retour au tableau par defaut, sans fuite.
    for d in [
        secret.0.to_string(),
        "abc".into(),
        "999999".into(),
        "-1".into(),
    ] {
        let r = get(&app, &format!("/?d={d}"), &[("cookie", &bob)]).await;
        assert_eq!(r.status, StatusCode::OK, "d={d}");
        assert!(r.text.contains("contenu-alpha"), "d={d}");
        assert!(
            !r.text.contains("contenu-secret"),
            "d={d} : fuite du tableau d'un autre compte"
        );
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_dashboard_server_function_is_empty_for_anonymous_visitors(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let r = sf_at(&app, None, "dashboard", "").await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.text);
    let v: serde_json::Value = serde_json::from_str(&r.text).unwrap();
    assert!(
        v["current"].is_null()
            && v["dashboards"].as_array().unwrap().is_empty()
            && v["widgets"].as_array().unwrap().is_empty(),
        "{v}"
    );
}

// ------------------------------------------------------------------ edition des tableaux (R2 lot 5)

fn widget_input(board: i64, kind: &str, extra: &serde_json::Value) -> String {
    let mut input = serde_json::json!({
        "id": null, "dashboard_id": board, "widget_type": kind, "title": "", "presentation": "",
        "device_id": "", "scene_id": "", "camera_uuid": "", "config": "", "content": "",
        "col": "0", "row": "0", "width": "2", "height": "2",
    });
    for (k, v) in extra.as_object().unwrap() {
        input[k] = v.clone();
    }
    serde_json::json!({ "input": input }).to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_can_edit_dashboards_and_widgets(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let board = dash::board(&state, "bob", "Salon").await;
    let calls: [(&str, String); 8] = [
        ("widgets/page", String::new()),
        ("dashboards/list", String::new()),
        (
            "dashboards/save",
            r#"{"input":{"id":null,"name":"X","owner":"bob","is_default":false}}"#.into(),
        ),
        ("dashboards/delete", format!("id={}", board.0)),
        ("widgets/list", format!("dashboard_id={}", board.0)),
        (
            "widgets/save",
            widget_input(board.0, "grille", &serde_json::json!({})),
        ),
        ("widgets/delete", "id=1".into()),
        ("widgets/move", "id=1&col=1&row=1".into()),
    ];
    for (path, body) in &calls {
        for cookie in [None, Some(user.as_str())] {
            let r = sf_at(&app, cookie, path, body).await;
            assert_eq!(
                r.status,
                StatusCode::FORBIDDEN,
                "{path} {cookie:?}: {}",
                r.text
            );
        }
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn administrators_manage_dashboards_for_any_owner(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let _bob = session_for(&app, &state, "bob", Role::User).await;
    let save = |name: &str, owner: &str| {
        format!(r#"{{"input":{{"id":null,"name":"{name}","owner":"{owner}","is_default":false}}}}"#)
    };
    let missing = sf_json(&app, &admin, "dashboards/save", &save("Salon", "personne")).await;
    assert_eq!(missing["Invalid"][0]["field"], "owner", "{missing}");
    let empty = sf_json(&app, &admin, "dashboards/save", &save("", "bob")).await;
    assert_eq!(empty["Invalid"][0]["field"], "name", "{empty}");
    let created = sf_json(&app, &admin, "dashboards/save", &save("Salon", "BOB")).await;
    assert_eq!(created["Saved"]["owner"], "bob", "{created}");
    let id = created["Saved"]["id"].as_i64().unwrap();
    let renamed = sf_json(
        &app,
        &admin,
        "dashboards/save",
        &format!(
            r#"{{"input":{{"id":{id},"name":"Cuisine","owner":"ignore","is_default":false}}}}"#
        ),
    )
    .await;
    assert_eq!(
        (
            renamed["Saved"]["name"].as_str(),
            renamed["Saved"]["owner"].as_str()
        ),
        (Some("Cuisine"), Some("bob"))
    );
    let all = sf_json(&app, &admin, "dashboards/list", "").await;
    assert_eq!(all.as_array().unwrap().len(), 1);
    // Le tableau cree pour bob est bien celui que bob voit sur sa page d'accueil.
    let home = get(&app, "/", &[("cookie", &login_cookie(&app, "bob").await)]).await;
    assert!(home.text.contains("Cuisine"));
    let gone = sf_json(&app, &admin, "dashboards/delete", &format!("id={id}")).await;
    assert_eq!(gone, "Deleted");
    assert_eq!(
        sf_json(&app, &admin, "dashboards/delete", &format!("id={id}")).await,
        "NotFound"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn saving_a_widget_validates_each_field_and_sanitizes_text(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let board = dash::board(&state, "alice", "Salon").await.0;
    let fields = |v: &serde_json::Value| -> Vec<String> {
        v["Invalid"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["field"].as_str().unwrap().to_string())
            .collect()
    };
    let bad = sf_json(
        &app,
        &admin,
        "widgets/save",
        &widget_input(board, "device", &serde_json::json!({"col": "abc", "config": "{pas du json", "device_id": "x", "presentation": "inconnue"})),
    )
    .await;
    let mut got = fields(&bad);
    got.sort();
    assert_eq!(got, ["col", "config", "device_id", "presentation"], "{bad}");
    // Regles du domaine : un appareil est obligatoire, la largeur ne doit pas deborder de la grille.
    let missing = sf_json(
        &app,
        &admin,
        "widgets/save",
        &widget_input(board, "device", &serde_json::json!({})),
    )
    .await;
    assert_eq!(fields(&missing), ["device_id"], "{missing}");
    let wide = sf_json(
        &app,
        &admin,
        "widgets/save",
        &widget_input(
            board,
            "grille",
            &serde_json::json!({"col": "10", "width": "4"}),
        ),
    )
    .await;
    assert_eq!(fields(&wide), ["width"], "{wide}");
    let uuid = sf_json(
        &app,
        &admin,
        "widgets/save",
        &widget_input(
            board,
            "camera",
            &serde_json::json!({"camera_uuid": "pas-un-uuid"}),
        ),
    )
    .await;
    assert_eq!(fields(&uuid), ["camera_uuid"], "{uuid}");

    // Texte : assaini a l'enregistrement, pas seulement a l'affichage.
    let ok = sf_json(
        &app,
        &admin,
        "widgets/save",
        &widget_input(board, "text", &serde_json::json!({"content": "<p>Salut</p><script>alert(1)</script>", "title": " Note "})),
    )
    .await;
    assert_eq!(ok["Saved"]["content"], "<p>Salut</p>", "{ok}");
    assert_eq!(ok["Saved"]["title"], "Note");
    let listed = sf_json(
        &app,
        &admin,
        "widgets/list",
        &format!("dashboard_id={board}"),
    )
    .await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["content"], "<p>Salut</p>");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn widgets_cannot_overlap_and_moves_report_why_they_fail(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let board = dash::board(&state, "alice", "Salon").await.0;
    let mk = |col: &str, row: &str| {
        widget_input(
            board,
            "grille",
            &serde_json::json!({"col": col, "row": row}),
        )
    };
    let a = sf_json(&app, &admin, "widgets/save", &mk("0", "0")).await["Saved"]["id"]
        .as_i64()
        .unwrap();
    let b = sf_json(&app, &admin, "widgets/save", &mk("4", "0")).await["Saved"]["id"]
        .as_i64()
        .unwrap();
    // Chevauchement a la creation : signale sur le champ `col`.
    let clash = sf_json(&app, &admin, "widgets/save", &mk("1", "1")).await;
    assert_eq!(
        clash["Invalid"][0],
        serde_json::json!({"field": "col", "kind": "Overlap"}),
        "{clash}"
    );
    let mv = |id: i64, col: i16, row: i16| {
        let (app, admin) = (app.clone(), admin.clone());
        async move {
            sf_json(
                &app,
                &admin,
                "widgets/move",
                &format!("id={id}&col={col}&row={row}"),
            )
            .await
        }
    };
    assert_eq!(mv(a, 0, 3).await, "Moved");
    assert_eq!(mv(a, 4, 0).await, "Overlap", "sur le widget b");
    assert_eq!(
        mv(a, 11, 0).await,
        "Invalid",
        "2 de large : deborde de la grille"
    );
    assert_eq!(
        mv(a, 0, 100).await,
        "Invalid",
        "au-dela de la derniere ligne"
    );
    assert_eq!(mv(999_999, 0, 0).await, "NotFound");
    // Les refus n'ont rien deplace ; des cases contigues passent.
    let now = sf_json(
        &app,
        &admin,
        "widgets/list",
        &format!("dashboard_id={board}"),
    )
    .await;
    let pos = |id: i64| {
        now.as_array()
            .unwrap()
            .iter()
            .find(|w| w["id"] == id)
            .map(|w| (w["col"].clone(), w["row"].clone()))
            .unwrap()
    };
    assert_eq!(pos(a), (serde_json::json!(0), serde_json::json!(3)));
    assert_eq!(pos(b), (serde_json::json!(4), serde_json::json!(0)));
    assert_eq!(
        mv(a, 6, 0).await,
        "Moved",
        "contigu a droite de b (colonnes 4-5)"
    );
    // Modifier un widget sans le deplacer ne s'oppose pas a lui-meme.
    let same = widget_input(
        board,
        "grille",
        &serde_json::json!({"id": b, "col": "4", "row": "0", "title": "Renomme"}),
    );
    assert_eq!(
        sf_json(&app, &admin, "widgets/save", &same).await["Saved"]["title"],
        "Renomme"
    );
    assert_eq!(
        sf_json(&app, &admin, "widgets/delete", &format!("id={b}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "widgets/delete", &format!("id={b}")).await,
        "NotFound"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_widgets_page_picks_the_requested_dashboard_or_the_first_one(pool: PgPool) {
    use dominet_domain::dashboard::WidgetType as T;
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let empty = sf_json(&app, &admin, "widgets/page", "").await;
    assert!(
        empty["current"].is_null() && empty["widgets"].as_array().unwrap().is_empty(),
        "{empty}"
    );
    let first = dash::board(&state, "alice", "Premier").await;
    let second = dash::board(&state, "alice", "Second").await;
    dash::widget(
        &state,
        second,
        T::Grille,
        "Dans le second",
        (0, 0, 2, 2),
        |_| {},
    )
    .await;
    let by_default = sf_json(&app, &admin, "widgets/page", "").await;
    assert_eq!(by_default["current"], first.0, "{by_default}");
    let asked = sf_json(
        &app,
        &admin,
        "widgets/page",
        &format!("requested={}", second.0),
    )
    .await;
    assert_eq!(
        (
            asked["current"].as_i64(),
            asked["widgets"][0]["title"].as_str()
        ),
        (Some(second.0), Some("Dans le second"))
    );
    let unknown = sf_json(&app, &admin, "widgets/page", "requested=999999").await;
    assert_eq!(
        unknown["current"], first.0,
        "tableau inconnu : retour au premier"
    );
}

// ------------------------------------------------------------------ automations (R4 lot 3)

/// Formulaire d'automation par defaut : `device_state` qui active la scene, avec les remplacements donnes.
fn automation_input(id: Option<i64>, changes: &serde_json::Value) -> String {
    let mut f = serde_json::json!({
        "name": "Bouton salon", "enabled": true, "description": "", "trigger_type": "device_state", "topic": "",
        "payload_filter": "", "friendly_name": "Tele-commande", "key": "action", "value": "1_single", "cron": "",
        "event": "", "latitude": "", "longitude": "", "condition_key": "", "condition_value": "",
        "action_type": "mqtt_publish", "action_topic": "zigbee2mqtt/Volet-salon/set", "action_payload": "{\"state\":\"OPEN\"}",
        "message": "", "sms": false, "scene_id": "", "script": ""
    });
    for (k, v) in changes.as_object().unwrap() {
        f[k] = v.clone();
    }
    serde_json::json!({"input": {"id": id, "fields": f}}).to_string()
}

fn field_problems_of(v: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
    v["Invalid"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| (p["field"].as_str().unwrap().to_string(), p["kind"].clone()))
                .collect()
        })
        .unwrap_or_default()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn only_administrators_manage_automations_and_the_engine_is_woken(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let body = automation_input(None, &serde_json::json!({}));
    for cookie in [None, Some(user.as_str())] {
        for (path, payload) in [
            ("automations/list", ""),
            ("automations/save", body.as_str()),
            ("automations/delete", "id=1"),
            ("automations/runs", ""),
        ] {
            let r = sf_at(&app, cookie, path, payload).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
        }
    }

    let woken = state.automations_changed.clone();
    let wait = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), woken.notified())
            .await
            .is_ok()
    });
    let saved = sf_json(&app, &admin, "automations/save", &body).await;
    assert!(
        wait.await.unwrap(),
        "le moteur est reveille par l'enregistrement"
    );
    let id = saved["Saved"]["id"].as_i64().unwrap();
    assert_eq!(
        saved["Saved"]["trigger_summary"],
        "device_state Tele-commande : action = 1_single"
    );
    assert_eq!(
        saved["Saved"]["action_summary"],
        "mqtt_publish zigbee2mqtt/Volet-salon/set"
    );
    assert_eq!(
        saved["Saved"]["fields"]["action_payload"],
        "{\"state\":\"OPEN\"}"
    );

    // Modification : condition, puis relecture fidele.
    let upd = automation_input(
        Some(id),
        &serde_json::json!({"condition_key": "battery", "condition_value": "100", "value": "28", "description": "essai"}),
    );
    let saved = sf_json(&app, &admin, "automations/save", &upd).await;
    assert_eq!(saved["Saved"]["fields"]["condition_key"], "battery");
    assert_eq!(saved["Saved"]["fields"]["condition_value"], "100");
    let all = sf_json(&app, &admin, "automations/list", "").await;
    assert_eq!(all.as_array().unwrap().len(), 1);
    assert_eq!(all[0]["fields"]["value"], "28");
    assert_eq!(all[0]["trigger_count"], 0);

    let runs = sf_json(&app, &admin, "automations/runs", "").await;
    assert_eq!(runs.as_array().unwrap().len(), 0);

    assert_eq!(
        sf_json(&app, &admin, "automations/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "automations/delete", &format!("id={id}")).await,
        "NotFound"
    );
    let gone = sf_json(
        &app,
        &admin,
        "automations/save",
        &automation_input(Some(id), &serde_json::json!({})),
    )
    .await;
    assert_eq!(gone, "NotFound");

    // Page d'administration : admin seulement.
    let page = get(&app, "/admin/automations", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("Automations"));
    assert_eq!(
        get(&app, "/admin/automations", &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&app, "/admin/automations", &[HTML]).await.status,
        StatusCode::FOUND
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn invalid_automations_are_reported_field_by_field(pool: PgPool) {
    use serde_json::json;
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let invalid = |changes: serde_json::Value| {
        let (app, admin) = (app.clone(), admin.clone());
        async move {
            field_problems_of(
                &sf_json(
                    &app,
                    &admin,
                    "automations/save",
                    &automation_input(None, &changes),
                )
                .await,
            )
        }
    };
    let required = json!("Required");
    let bad = json!("Invalid");
    assert_eq!(
        invalid(json!({"name": " "})).await,
        [("name".to_string(), required.clone())]
    );
    assert_eq!(
        invalid(json!({"key": "", "friendly_name": ""})).await,
        [
            ("friendly_name".to_string(), required.clone()),
            ("key".to_string(), required.clone())
        ]
    );
    assert_eq!(
        invalid(json!({"action_topic": "a/#"})).await,
        [("action_topic".to_string(), bad.clone())]
    );
    assert_eq!(
        invalid(json!({"action_payload": "pas du json"})).await,
        [("action_payload".to_string(), bad.clone())]
    );
    assert_eq!(
        invalid(json!({"action_payload": "[1]"})).await,
        [("action_payload".to_string(), bad.clone())]
    );
    assert_eq!(
        invalid(json!({"condition_key": "a b", "condition_value": "1"})).await,
        [("condition_key".to_string(), bad.clone())]
    );
    assert_eq!(
        invalid(json!({"trigger_type": "mqtt_message", "topic": "a/#/b", "payload_filter": "x"}))
            .await,
        [
            ("payload_filter".to_string(), bad.clone()),
            ("topic".to_string(), bad.clone())
        ]
    );
    assert_eq!(
        invalid(json!({"trigger_type": "schedule", "cron": "61 * * * *"})).await,
        [("cron".to_string(), bad.clone())]
    );
    assert_eq!(
        invalid(json!({"trigger_type": "solar", "event": "midnight", "latitude": "abc", "longitude": ""})).await,
        [
            ("latitude".to_string(), bad.clone()),
            ("longitude".to_string(), required.clone()),
            ("event".to_string(), bad.clone())
        ]
    );
    assert_eq!(
        invalid(json!({"action_type": "scene", "scene_id": ""})).await,
        [("scene_id".to_string(), required.clone())]
    );
    assert_eq!(
        invalid(json!({"action_type": "script", "script": " "})).await,
        [("script".to_string(), required.clone())]
    );
    assert_eq!(
        invalid(json!({"action_type": "notification", "message": "x".repeat(501)})).await,
        [("message".to_string(), json!({"TooLong": {"max": 500}}))]
    );
    // Latitude avec virgule decimale : acceptee.
    let ok = sf_json(
        &app,
        &admin,
        "automations/save",
        &automation_input(
            None,
            &json!({"trigger_type": "solar", "event": "sunset", "latitude": "43,3", "longitude": "5,4", "action_type": "notification", "message": "Bonsoir"}),
        ),
    )
    .await;
    assert_eq!(ok["Saved"]["fields"]["latitude"], "43.3");
    // Une scene inexistante est refusee par la cle etrangere.
    let r = sf_json(
        &app,
        &admin,
        "automations/save",
        &automation_input(None, &json!({"action_type": "scene", "scene_id": "4242"})),
    )
    .await;
    assert_eq!(field_problems_of(&r), [("scene_id".to_string(), bad)]);
}

// ------------------------------------------------------------------ scripts (R4 lot 5)

fn dry_run_body(code: &str, topic: &str, payload: &str) -> String {
    format!(
        "code={}&topic={}&payload={}",
        urlenc(code),
        urlenc(topic),
        urlenc(payload)
    )
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_dry_run_is_for_administrators_shows_the_effects_and_contains_hostile_scripts(
    pool: PgPool,
) {
    use dominet_domain::device::DeviceDraft;
    use dominet_domain::repo::DeviceRepo;
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let mut cover = DeviceDraft::new("Volet");
    cover.device_type = dominet_domain::device::DeviceType::Cover;
    state.store.devices().create(&cover).await.unwrap();
    let body = dry_run_body("publish(\"a/b\", 1);", "t", "{}");
    for cookie in [None, Some(user.as_str())] {
        let r = sf_at(&app, cookie, "scripts/dry-run", &body).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN);
    }

    // Les effets sont decrits, rien n'est publie (ce serveur n'a d'ailleurs pas de client MQTT).
    let out = sf_json(
        &app,
        &admin,
        "scripts/dry-run",
        &dry_run_body(
            "if payload.n > 1 { publish(\"maison/\" + device.name, #{ v: payload.n }); } scene(\"Soir\"); print(\"vu\"); log_value(\"k\", 5);",
            "zigbee2mqtt/Volet",
            r#"{"n": 3}"#,
        ),
    )
    .await;
    assert_eq!(out["error"], "");
    assert_eq!(
        out["effects"],
        serde_json::json!([
            "publish on maison/Volet: {\"v\":3}",
            "activate scene Soir",
            "log k = 5"
        ])
    );
    assert_eq!(out["logs"], serde_json::json!(["vu"]));

    // Erreurs : syntaxe, execution, reference, script hostile (borne dans le temps).
    for (code, needle) in [
        ("let x = ;", "Unexpected"),
        ("def f():\n  pass", "Python"),
        ("inconnue()", "inconnue"),
        ("loop { }", ""),
    ] {
        let started = std::time::Instant::now();
        let out = sf_json(&app, &admin, "scripts/dry-run", &dry_run_body(code, "", "")).await;
        let error = out["error"].as_str().unwrap();
        assert!(
            !error.is_empty() && error.contains(needle),
            "{code} : {error}"
        );
        assert_eq!(out["effects"], serde_json::json!([]));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{code}"
        );
    }
    // Le serveur repond encore.
    assert_eq!(
        get(&app, "/admin/automations", &[("cookie", &admin)])
            .await
            .status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn scripts_and_source_handlers_must_compile_and_python_code_is_refused(pool: PgPool) {
    use serde_json::json;
    let (app, state, _guard) = app(pool).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let syntax = |v: &serde_json::Value| -> Option<(String, String)> {
        v["Invalid"].as_array().and_then(|a| a.first()).map(|p| {
            (
                p["field"].as_str().unwrap().to_string(),
                p["kind"]["Syntax"].as_str().unwrap_or_default().to_string(),
            )
        })
    };
    let script = |code: &str| {
        automation_input(
            None,
            &json!({"action_type": "script", "script": code, "name": "S"}),
        )
    };
    let bad = sf_json(&app, &admin, "automations/save", &script("let x = ;")).await;
    let (field, detail) = syntax(&bad).expect("erreur de syntaxe");
    assert_eq!(field, "script");
    assert!(detail.contains("line 1"), "{detail}");
    let python = sf_json(
        &app,
        &admin,
        "automations/save",
        &script("def f():\n  pass"),
    )
    .await;
    assert!(syntax(&python).unwrap().1.contains("Python"));
    let ok = sf_json(
        &app,
        &admin,
        "automations/save",
        &script("publish(\"a/b\", 1);"),
    )
    .await;
    assert_eq!(ok["Saved"]["fields"]["script"], "publish(\"a/b\", 1);");

    // Meme regle pour le code de traitement d'une source.
    let source = |code: &str| {
        serde_json::json!({"input": {
            "id": null, "name": "Linky", "topic_pattern": "linky/#", "qos": 0, "active": true, "log_messages": false,
            "auto_device": false, "auto_device_type": "sensor", "base_topic": "", "handler_code": code, "description": ""
        }})
        .to_string()
    };
    let bad = sf_json(
        &app,
        &admin,
        "mqtt-sources/save",
        &source("context.publish('a', 1)"),
    )
    .await;
    assert!(syntax(&bad).unwrap().1.contains("Python"));
    let ok = sf_json(&app, &admin, "mqtt-sources/save", &source("print(payload)")).await;
    assert!(ok["Saved"]["id"].as_i64().is_some());
    // Sans code : accepte.
    let mut empty = serde_json::from_str::<serde_json::Value>(&source("")).unwrap();
    empty["input"]["name"] = json!("Vide");
    let ok = sf_json(&app, &admin, "mqtt-sources/save", &empty.to_string()).await;
    assert!(ok["Saved"]["id"].as_i64().is_some());
}

// ------------------------------------------------------------------ alertes (R4 lot 6)

fn rule_input(id: Option<i64>, device: i64, changes: &serde_json::Value) -> String {
    let mut r = serde_json::json!({
        "id": id, "name": "Canicule", "device_id": device.to_string(), "key": "temperature", "operator": "gt",
        "threshold_num": "35", "threshold_str": "", "severity": "warning", "cooldown_minutes": "60",
        "notify_email": true, "notify_sms": false, "notify_android": true, "enabled": true, "description": ""
    });
    for (k, v) in changes.as_object().unwrap() {
        r[k] = v.clone();
    }
    serde_json::json!({"input": r}).to_string()
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn alert_rules_are_for_administrators_and_validated_field_by_field(pool: PgPool) {
    use serde_json::json;
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let device = make_device(&state, "meteo_locale").await;
    let body = rule_input(None, device, &json!({}));
    for cookie in [None, Some(user.as_str())] {
        for (path, payload) in [
            ("alert-rules/list", ""),
            ("alert-rules/save", body.as_str()),
            ("alert-rules/delete", "id=1"),
        ] {
            assert_eq!(
                sf_at(&app, cookie, path, payload).await.status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
    }
    let woken = state.alert_rules_changed.clone();
    let wait = tokio::spawn(async move {
        tokio::time::timeout(std::time::Duration::from_secs(5), woken.notified())
            .await
            .is_ok()
    });
    let saved = sf_json(&app, &admin, "alert-rules/save", &body).await;
    assert!(
        wait.await.unwrap(),
        "l'evaluation est reveillee par l'enregistrement"
    );
    let id = saved["Saved"]["id"].as_i64().unwrap();
    assert_eq!(saved["Saved"]["device"], "meteo_locale");
    assert_eq!(saved["Saved"]["threshold_num"], "35");
    // Virgule decimale, modification, relecture.
    let upd = rule_input(
        Some(id),
        device,
        &json!({"threshold_num": "35,5", "severity": "critical"}),
    );
    let saved = sf_json(&app, &admin, "alert-rules/save", &upd).await;
    assert_eq!(saved["Saved"]["threshold_num"], "35.5");
    assert_eq!(saved["Saved"]["severity"], "critical");

    let problems = |v: &serde_json::Value| -> Vec<String> {
        v["Invalid"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| p["field"].as_str().unwrap().to_string())
                    .collect()
            })
            .unwrap_or_default()
    };
    for (changes, field) in [
        (json!({"name": " "}), "name"),
        (json!({"key": "a b"}), "key"),
        (json!({"operator": "pareil"}), "operator"),
        (json!({"severity": "grave"}), "severity"),
        (json!({"threshold_num": "abc"}), "threshold_num"),
        (
            json!({"threshold_num": "", "threshold_str": ""}),
            "threshold_num",
        ),
        (
            json!({"operator": "contains", "threshold_str": ""}),
            "threshold_str",
        ),
        (json!({"cooldown_minutes": "2000"}), "cooldown_minutes"),
        (json!({"cooldown_minutes": "x"}), "cooldown_minutes"),
        (json!({"device_id": ""}), "device_id"),
    ] {
        let r = sf_json(
            &app,
            &admin,
            "alert-rules/save",
            &rule_input(None, device, &changes),
        )
        .await;
        assert_eq!(problems(&r), [field], "{changes}");
    }
    let r = sf_json(
        &app,
        &admin,
        "alert-rules/save",
        &rule_input(None, device + 999, &json!({})),
    )
    .await;
    assert_eq!(problems(&r), ["device_id"], "appareil inconnu");

    let listed = sf_json(&app, &admin, "alert-rules/list", "").await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["firing"], 0);
    assert_eq!(
        sf_json(&app, &admin, "alert-rules/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &admin, "alert-rules/delete", &format!("id={id}")).await,
        "NotFound"
    );
    assert_eq!(
        get(&app, "/admin/alert-rules", &[("cookie", &admin)])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, "/admin/alert-rules", &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn any_account_lists_and_acknowledges_alerts_and_the_sidebar_links_to_them(pool: PgPool) {
    use dominet_domain::alert::{AlertRuleDraft, Severity};
    use dominet_domain::device::DeviceId;
    use dominet_domain::repo::{AlertRepo, AlertRuleRepo, NewAlert};
    use dominet_proto::{Topic, WsServerMsg};
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let device = make_device(&state, "meteo_locale").await;
    let rule = state
        .store
        .alert_rules()
        .create(&AlertRuleDraft::new(
            "Canicule",
            DeviceId(device),
            "temperature",
        ))
        .await
        .unwrap();
    let mut ids = vec![];
    for (i, sev) in [Severity::Warning, Severity::Critical]
        .into_iter()
        .enumerate()
    {
        let a = state
            .store
            .alerts()
            .create(&NewAlert {
                rule: rule.id,
                device: DeviceId(device),
                severity: sev,
                value_triggered: format!("{}", 36 + i),
                fired_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        ids.push(a.id.0);
    }
    for (path, payload) in [
        ("alerts/page", "status=firing"),
        ("alerts/acknowledge", "id=1"),
        ("alerts/acknowledge-all", ""),
    ] {
        assert_eq!(
            sf_at(&app, None, path, payload).await.status,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }
    let page = sf_json(&app, &user, "alerts/page", "status=firing").await;
    assert_eq!(page["firing"], 2);
    assert_eq!(page["alerts"].as_array().unwrap().len(), 2);
    assert_eq!(page["alerts"][0]["rule"], "Canicule");
    assert_eq!(page["alerts"][0]["device"], "meteo_locale");
    // Un filtre inconnu retombe sur « en cours ».
    assert_eq!(
        sf_json(&app, &user, "alerts/page", "status=nimporte").await["status"],
        "firing"
    );

    assert_eq!(
        sf_json(&app, &user, "alerts/acknowledge", &format!("id={}", ids[0])).await,
        "Acknowledged"
    );
    assert_eq!(
        sf_json(&app, &user, "alerts/acknowledge", &format!("id={}", ids[0])).await,
        "NotFiring"
    );
    // La pastille est rediffusee (dernier etat retenu).
    let retained = state.hub.retained(&Topic::Alerts).expect("compte diffuse");
    assert!(
        matches!(&*retained, WsServerMsg::AlertCount { firing: 1 }),
        "{retained:?}"
    );
    let acked = sf_json(&app, &user, "alerts/page", "status=acknowledged").await;
    assert_eq!(acked["alerts"][0]["acknowledged_by"], "bob");
    assert_eq!(sf_json(&app, &user, "alerts/acknowledge-all", "").await, 1);
    let all = sf_json(&app, &user, "alerts/page", "status=all").await;
    assert_eq!(all["alerts"].as_array().unwrap().len(), 2);
    assert_eq!(all["firing"], 0);

    // La page et le lien de la barre laterale (avec sa pastille) sont rendus par le serveur.
    let html = get(&app, "/alerts", &[("cookie", &user)]).await;
    assert_eq!(html.status, StatusCode::OK);
    assert!(
        html.text.contains("vc-nav-alerts") && html.text.contains("vc-alert-badge"),
        "{}",
        html.text.len()
    );
    assert_eq!(
        get(&app, "/alerts", &[HTML]).await.status,
        StatusCode::FOUND
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_alerts_page_lists_platform_alerts_and_the_badge_counts_them(pool: PgPool) {
    use dominet_proto::{Topic, WsServerMsg};
    // Un faux Prometheus : une alerte en cours et une en attente (ignoree).
    let fake = axum::Router::new().route(
        "/api/v1/alerts",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({"status": "success", "data": {"alerts": [
                {"state": "firing", "activeAt": "2026-11-18T08:00:00Z",
                 "labels": {"alertname": "MqttDown", "severity": "critical"},
                 "annotations": {"summary": "MQTT hors ligne"}},
                {"state": "pending", "labels": {"alertname": "Lent"}, "annotations": {}}
            ]}}))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, fake).await.unwrap() });
    let (app, state, _guard) = app_with_vars(pool, None, &[("PROMETHEUS_URL", url.as_str())]).await;
    let user = session_for(&app, &state, "bob", Role::User).await;

    // Avant la premiere lecture, la source est dite injoignable (on ne sait rien) et rien n'est compte.
    let before = sf_json(&app, &user, "alerts/page", "status=firing").await;
    assert_eq!(before["platform"]["prometheus"], "down");
    assert_eq!(before["platform"]["grafana"], "off");

    assert!(state.external_alerts.refresh().await);
    dominet_server::alerts::publish_count(&state.hub, &state.store, &state.external_alerts).await;
    let page = sf_json(&app, &user, "alerts/page", "status=firing").await;
    assert_eq!(page["platform"]["prometheus"], "ok");
    assert_eq!(page["platform"]["alerts"].as_array().unwrap().len(), 1);
    assert_eq!(page["platform"]["alerts"][0]["name"], "MqttDown");
    assert_eq!(page["platform"]["alerts"][0]["source"], "prometheus");
    assert_eq!(page["platform"]["alerts"][0]["severity"], "critical");
    assert_eq!(
        page["firing"], 0,
        "le compte des alertes d'appareil ne change pas"
    );
    let retained = state.hub.retained(&Topic::Alerts).expect("compte diffuse");
    assert!(
        matches!(&*retained, WsServerMsg::AlertCount { firing: 1 }),
        "{retained:?}"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_unreachable_platform_source_is_said_and_counts_for_nothing(pool: PgPool) {
    let (app, state, _guard) =
        app_with_vars(pool, None, &[("PROMETHEUS_URL", "http://127.0.0.1:1")]).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    state.external_alerts.refresh().await;
    let page = sf_json(&app, &user, "alerts/page", "status=firing").await;
    assert_eq!(page["platform"]["prometheus"], "down");
    assert_eq!(page["platform"]["alerts"].as_array().unwrap().len(), 0);
    assert_eq!(state.external_alerts.firing(), 0);
}

// ------------------------------------------------------------------ Android (R5 lot 4)

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn android_devices_are_per_account_with_a_token_shown_once(pool: PgPool) {
    use dominet_domain::android::{
        AndroidDeviceDraft, CallDirection, NewCall, NewSms, SmsDirection,
    };
    use dominet_domain::repo::{AndroidDeviceRepo, CallRepo, SmsRepo};
    let (app, state, _guard) = app(pool).await;
    let bob = session_for(&app, &state, "bob", Role::User).await;
    let eve = session_for(&app, &state, "eve", Role::User).await;
    let root = session_for(&app, &state, "root", Role::Admin).await;

    for (path, payload) in [
        ("android/overview", ""),
        ("android/devices", ""),
        (
            "android/devices/add",
            r#"{"input":{"name":"x","phone_number":""}}"#,
        ),
        ("android/devices/delete", "id=1"),
        ("android/devices/token", "id=1"),
        ("android/sms", ""),
        ("android/calls", ""),
        ("android/sms/send", "device=1&to=%2B33600000000&body=x"),
    ] {
        assert_eq!(
            sf_at(&app, None, path, payload).await.status,
            StatusCode::UNAUTHORIZED,
            "{path}"
        );
    }

    // Validation champ par champ.
    let bad = sf_json(
        &app,
        &bob,
        "android/devices/add",
        r#"{"input":{"name":" ","phone_number":"abc"}}"#,
    )
    .await;
    let fields: Vec<_> = bad["Invalid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["field"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(fields, ["name", "phone_number"]);

    // Creation : le jeton est rendu une fois, jamais par les listes.
    let made = sf_json(
        &app,
        &bob,
        "android/devices/add",
        r#"{"input":{"name":"Pixel de Bob","phone_number":"+33612345678"}}"#,
    )
    .await;
    let token = made["Saved"]["token"].as_str().unwrap().to_string();
    assert_eq!(made["Saved"]["device"]["owner"], "bob");
    let id = made["Saved"]["device"]["id"].as_i64().unwrap();
    assert!(dominet_domain::android::canonical_token(&token).is_some());
    let listed = sf_json(&app, &bob, "android/devices", "").await;
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(
        !listed.to_string().contains(&token),
        "le jeton ne ressort pas des listes"
    );
    assert!(
        state
            .store
            .android_devices()
            .find_by_token(&token)
            .await
            .unwrap()
            .is_some()
    );

    // Chacun voit les siens ; l'administrateur voit tout.
    assert_eq!(
        sf_json(&app, &eve, "android/devices", "")
            .await
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        sf_json(&app, &root, "android/devices", "")
            .await
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Activite : SMS recus (non lus) et appels.
    let device = dominet_domain::android::AndroidDeviceId(id);
    let now = chrono::Utc::now();
    for n in 0..3 {
        state
            .store
            .sms()
            .insert(&NewSms {
                device,
                direction: SmsDirection::Incoming,
                phone_number: "+33699999999".into(),
                contact_name: "Alice".into(),
                body: format!("sms {n}"),
                is_read: false,
                sent_at: now + chrono::Duration::seconds(n),
            })
            .await
            .unwrap();
    }
    state
        .store
        .calls()
        .apply(&dominet_domain::android::CallPlan::Create(NewCall {
            device,
            direction: CallDirection::Missed,
            phone_number: "0612".into(),
            contact_name: String::new(),
            started_at: now,
            ended_at: Some(now),
            duration: 0,
        }))
        .await
        .unwrap();
    let overview = sf_json(&app, &bob, "android/overview", "").await;
    assert_eq!(overview["unread"], 3);
    assert_eq!(overview["recent_sms"].as_array().unwrap().len(), 3);
    assert_eq!(overview["recent_calls"][0]["direction"], "missed");
    assert_eq!(
        sf_json(&app, &eve, "android/overview", "").await["unread"],
        0
    );

    // La page des SMS marque les SMS recus comme lus (comme la reference) ; un appareil d'autrui est introuvable.
    let page = sf_json(&app, &bob, "android/sms", &format!("device={id}")).await;
    assert_eq!(page["sms"].as_array().unwrap().len(), 3);
    assert_eq!(page["sms"][0]["body"], "sms 2", "les plus recents d'abord");
    assert_eq!(
        sf_json(&app, &bob, "android/overview", "").await["unread"],
        0
    );
    assert_eq!(
        sf_json(&app, &eve, "android/sms", &format!("device={id}")).await,
        serde_json::Value::Null
    );
    assert_eq!(
        sf_json(&app, &eve, "android/calls", &format!("device={id}")).await,
        serde_json::Value::Null
    );
    // Sans parametre : le premier appareil du compte, ou aucun.
    assert_eq!(
        sf_json(&app, &bob, "android/calls", "").await["device"]["id"],
        id
    );
    assert_eq!(
        sf_json(&app, &eve, "android/calls", "").await["device"],
        serde_json::Value::Null
    );

    // Envoi : refuse hors ligne, invalide, ou sur l'appareil d'autrui ; rien n'est enregistre.
    let send = |who: String, to: &str, body: &str| {
        let form = format!("device={id}&to={to}&body={body}");
        let app = app.clone();
        async move { sf_json(&app, &who, "android/sms/send", &form).await }
    };
    assert_eq!(
        send(bob.clone(), "%2B33600000000", "salut").await,
        "Offline"
    );
    assert_eq!(
        send(eve.clone(), "%2B33600000000", "salut").await,
        "NotFound"
    );
    let invalid = send(bob.clone(), "x", "").await;
    let fields: Vec<_> = invalid["Invalid"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["field"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(fields, ["to", "body"]);
    assert_eq!(
        state
            .store
            .sms()
            .list(Some(bob_id(&state).await), Some(device), 10)
            .await
            .unwrap()
            .len(),
        3
    );

    // Jeton : regenere par le proprietaire (l'ancien cesse de marcher) ou l'administrateur, pas par un tiers.
    assert_eq!(
        sf_json(&app, &eve, "android/devices/token", &format!("id={id}")).await,
        serde_json::Value::Null
    );
    let fresh = sf_json(&app, &bob, "android/devices/token", &format!("id={id}")).await;
    let fresh = fresh.as_str().unwrap();
    assert_ne!(fresh, token);
    assert!(
        state
            .store
            .android_devices()
            .find_by_token(&token)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .store
            .android_devices()
            .find_by_token(fresh)
            .await
            .unwrap()
            .is_some()
    );

    // Suppression : refusee a un tiers, permise au proprietaire (les SMS et appels partent avec).
    assert_eq!(
        sf_json(&app, &eve, "android/devices/delete", &format!("id={id}")).await,
        "NotFound"
    );
    assert_eq!(
        sf_json(&app, &bob, "android/devices/delete", &format!("id={id}")).await,
        "Deleted"
    );
    assert_eq!(
        sf_json(&app, &bob, "android/devices/delete", &format!("id={id}")).await,
        "NotFound"
    );
    assert_eq!(
        state.store.sms().list(None, None, 10).await.unwrap().len(),
        0
    );
    let _ = AndroidDeviceDraft {
        name: String::new(),
        phone_number: String::new(),
    };
}

async fn bob_id(state: &AppState) -> dominet_domain::user::UserId {
    state
        .store
        .users()
        .find_by_username("bob")
        .await
        .unwrap()
        .unwrap()
        .id
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_android_pages_are_rendered_for_signed_in_accounts_only(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let bob = session_for(&app, &state, "bob", Role::User).await;
    for path in [
        "/android",
        "/android/devices",
        "/android/sms",
        "/android/calls",
    ] {
        assert_eq!(
            get(&app, path, &[HTML]).await.status,
            StatusCode::FOUND,
            "{path}"
        );
        let page = get(&app, path, &[("cookie", &bob)]).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
    }
    assert!(
        get(&app, "/android/devices", &[("cookie", &bob)])
            .await
            .text
            .contains("android-add-form")
    );
}

// ------------------------------------------------------------------ energie (R4 lot 7)

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_energy_api_returns_the_last_days_in_the_reference_format_and_the_page_draws_them(
    pool: PgPool,
) {
    use dominet_domain::energy::EnergyDay;
    use dominet_domain::repo::EnergyRepo;
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    // 40 jours : l'API rend les DERNIERS (reference rendait les premiers et se figeait au bout de trente jours).
    let first = chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    for i in 0..40 {
        state
            .store
            .energy()
            .upsert(&EnergyDay {
                day: first + chrono::Duration::days(i),
                linky_hp_wh: 3000 + i32::try_from(i).unwrap(),
                linky_hc_wh: 1000,
                solar_wh: 6000,
                pac_wh: 500,
            })
            .await
            .unwrap();
    }
    let anon = get(&app, "/api/energy/", &[]).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
    let api = |q: &str| {
        let (app, user) = (app.clone(), user.clone());
        let q = q.to_string();
        async move {
            let r = get(&app, &format!("/api/energy/{q}"), &[("cookie", &user)]).await;
            assert_eq!(r.status, StatusCode::OK, "{}", r.text);
            serde_json::from_str::<serde_json::Value>(&r.text).unwrap()
        }
    };
    let thirty = api("").await;
    let rows = thirty["data"].as_array().unwrap();
    assert_eq!(rows.len(), 30);
    assert_eq!(
        rows[0]["date"], "2026-01-11",
        "du plus ancien des trente derniers"
    );
    assert_eq!(rows[29]["date"], "2026-02-09");
    assert_eq!(rows[29]["linky_hp_wh"], 3039);
    assert_eq!(rows[29]["linky_total_wh"], 4039);
    assert_eq!(rows[29]["net_wh"], 6000 - 4039 - 500);
    assert_eq!(api("?days=7").await["data"].as_array().unwrap().len(), 7);
    assert_eq!(api("?days=abc").await["data"].as_array().unwrap().len(), 30);
    assert_eq!(
        api("?days=9999").await["data"].as_array().unwrap().len(),
        40
    );

    // Fonction serveur et page : graphique SVG, tableau, plage choisie.
    let page = sf_json(&app, &user, "energy/page", "days=14").await;
    assert_eq!(page["days"], 14);
    assert_eq!(page["rows"].as_array().unwrap().len(), 14);
    let svg = page["svg"].as_str().unwrap();
    assert!(
        svg.starts_with("<svg") && svg.matches("<rect").count() == 42 && !svg.contains("style="),
        "{}",
        &svg[..80]
    );
    assert_eq!(
        sf_at(&app, None, "energy/page", "days=14").await.status,
        StatusCode::UNAUTHORIZED
    );
    let html = get(&app, "/automation/energy?days=14", &[("cookie", &user)]).await;
    assert_eq!(html.status, StatusCode::OK);
    for needle in ["energy-live", "energy-chart", "energy-table", "2026-02-09"] {
        assert!(html.text.contains(needle), "{needle}");
    }
    assert_eq!(
        get(&app, "/automation/energy/", &[("cookie", &user)])
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        get(&app, "/automation/energy", &[HTML]).await.status,
        StatusCode::FOUND
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_energy_page_says_what_to_do_when_there_is_no_data(pool: PgPool) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let html = get(&app, "/automation/energy", &[("cookie", &user)]).await;
    assert!(
        html.text.contains("energy-empty") && html.text.contains("aggregate-energy"),
        "{}",
        html.text.len()
    );
    assert_eq!(
        sf_json(&app, &user, "energy/page", "days=30").await["svg"],
        ""
    );
}

// ---------------------------------------------------------------- cameras (R6 lot 2)

fn camera_input(id: Option<i64>, name: &str, pass: &str) -> String {
    serde_json::json!({"input": {
        "id": id, "name": name, "location_id": null, "active": true, "priority": 1, "notes": "",
        "proto": "rtsp", "ip": "192.168.1.29", "port": 554, "path": "stream1", "rtsp_user": "admin",
        "rtsp_pass": pass, "fps": 10, "rtsp_any_port": false, "rtsp_transport_tcp": false,
        "recording_mode": "none", "night_mode_api": "none", "web_port": 80, "web_api_key": "",
        "web_user": "", "web_pass": "", "onvif": false, "onvif_port": 80, "onvif_user": "",
        "onvif_pass": "", "ptz": false, "yolo_enabled": false, "yolo_classes": "person, car",
        "yolo_min_confidence_pct": 50,
    }})
    .to_string()
}

async fn camera_fn(app: &Router, cookie: Option<&str>, name: &str, body: &str) -> Resp {
    let mut extra = vec![JSON];
    if let Some(c) = cookie {
        extra.push(("cookie", c));
    }
    send(
        app,
        Method::POST,
        &format!("/sf/cameras/{name}"),
        &extra,
        Some(body),
    )
    .await
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn cameras_are_managed_by_administrators_and_secrets_never_come_back(pool: PgPool) {
    let fake = dominet_surveillance::testing::FakeMediaMtx::start().await;
    let (app, state, _guard) = app_with_vars(
        pool,
        Some(TEST_MASTER_KEY),
        &[("MEDIAMTX_API_URL", &fake.url())],
    )
    .await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let create = camera_input(None, "Entree", "mot-de-passe-secret");

    for cookie in [None, Some(user.as_str())] {
        for (name, body) in [
            ("list", ""),
            ("save", create.as_str()),
            ("delete", "id=1"),
            ("sync", ""),
        ] {
            assert_eq!(
                camera_fn(&app, cookie, name, body).await.status,
                StatusCode::FORBIDDEN,
                "{name}"
            );
        }
    }
    assert!(fake.names().iter().all(|n| n == "~.*"), "rien n'a ete cree");

    let saved = camera_fn(&app, Some(&admin), "save", &create).await;
    assert_eq!(saved.status, StatusCode::OK, "{}", saved.text);
    assert!(
        !saved.text.contains("mot-de-passe-secret"),
        "{}",
        saved.text
    );
    let id = serde_json::from_str::<serde_json::Value>(&saved.text).unwrap()["Saved"]["id"]
        .as_i64()
        .unwrap();
    assert_eq!(fake.names().len(), 2, "le path de la camera est cree");

    let list = camera_fn(&app, Some(&admin), "list", "").await;
    assert!(list.text.contains("Entree") && list.text.contains("\"has_rtsp_pass\":true"));
    assert!(!list.text.contains("mot-de-passe-secret") && !list.text.contains("v1:"));

    // Champs invalides renvoyes un par un ; rien n'est enregistre.
    let bad = camera_input(None, "", "").replace("\"port\":554", "\"port\":0");
    let refused = camera_fn(&app, Some(&admin), "save", &bad).await;
    assert!(
        refused.text.contains("Invalid")
            && refused.text.contains("\"name\"")
            && refused.text.contains("\"port\"")
    );

    let sync = camera_fn(&app, Some(&admin), "sync", "").await;
    assert!(sync.text.contains("\"Done\""), "{}", sync.text);
    let del = camera_fn(&app, Some(&admin), "delete", &format!("id={id}")).await;
    assert_eq!(del.status, StatusCode::OK);
    assert_eq!(fake.names().len(), 1, "le path est supprime");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_cameras_admin_page_and_the_surveillance_page_follow_the_rights(pool: PgPool) {
    let (app, state, _guard) = app_with_key(pool, Some(TEST_MASTER_KEY)).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    camera_fn(
        &app,
        Some(&admin),
        "save",
        &camera_input(None, "Entr\u{e9}e <b>", "x"),
    )
    .await;

    let anon = get(&app, "/surveillance", &[HTML]).await;
    assert_eq!(anon.status, StatusCode::FOUND);
    assert_eq!(
        anon.headers[header::LOCATION],
        "/login?next=%2Fsurveillance"
    );
    assert_eq!(
        get(&app, "/admin/cameras", &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );

    let page = get(&app, "/admin/cameras", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("Ajouter une caméra") && page.text.contains("cameras-sync"));
    assert!(
        page.text.contains("Entr\u{e9}e &lt;b&gt;"),
        "le nom est echappe"
    );

    // Tout compte connecte voit les cameras actives, sans rien de leur configuration.
    let view = get(&app, "/surveillance", &[("cookie", &user)]).await;
    assert_eq!(view.status, StatusCode::OK);
    assert!(view.text.contains("surveillance-list") && view.text.contains("Entr\u{e9}e &lt;b&gt;"));
    assert!(!view.text.contains("192.168.1.29") && !view.text.contains("stream1"));
    let en = get(
        &app,
        "/admin/cameras",
        &[("cookie", &admin), ("accept-language", "en")],
    )
    .await;
    assert!(en.text.contains("Add a camera"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn onvif_functions_are_for_administrators_and_add_by_ip_works(pool: PgPool) {
    use dominet_surveillance::testing::onvif::{FakeOnvif, Family};
    let fake = FakeOnvif::start(Family::Nvt, "admin", "secret-nvt").await;
    let (app, state, _guard) = app_with_key(pool, Some(TEST_MASTER_KEY)).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let add = serde_json::json!({"input": {
        "ip": "127.0.0.1", "onvif_port": fake.port(), "user": "admin", "pass": "secret-nvt",
    }})
    .to_string();

    for cookie in [None, Some(user.as_str())] {
        for (name, body) in [
            ("add-by-ip", add.as_str()),
            ("onvif-sync", "id=1"),
            ("discovery", ""),
        ] {
            assert_eq!(
                camera_fn(&app, cookie, name, body).await.status,
                StatusCode::FORBIDDEN,
                "{name}"
            );
        }
    }
    let added = camera_fn(&app, Some(&admin), "add-by-ip", &add).await;
    assert_eq!(added.status, StatusCode::OK, "{}", added.text);
    assert!(
        added.text.contains("Added") && added.text.contains("IPC_NT98566"),
        "{}",
        added.text
    );
    assert!(!added.text.contains("secret-nvt"));
    let id = serde_json::from_str::<serde_json::Value>(&added.text).unwrap()["Added"]["id"]
        .as_i64()
        .unwrap();

    let synced = camera_fn(&app, Some(&admin), "onvif-sync", &format!("id={id}")).await;
    assert!(
        synced.text.contains("Done") && synced.text.contains("Profile_1"),
        "{}",
        synced.text
    );
    assert!(
        synced.text.contains("password=***") && !synced.text.contains("secret-nvt"),
        "{}",
        synced.text
    );
    let missing = camera_fn(&app, Some(&admin), "onvif-sync", "id=99999").await;
    assert!(missing.text.contains("NotFound"));

    let bad =
        serde_json::json!({"input": {"ip": "cam; rm", "onvif_port": 80, "user": "", "pass": ""}})
            .to_string();
    assert!(
        camera_fn(&app, Some(&admin), "add-by-ip", &bad)
            .await
            .text
            .contains("Invalid")
    );
    // La demande de decouverte ne touche que la base (NOTIFY) : elle aboutit meme sans role `discovery`.
    assert_eq!(
        camera_fn(&app, Some(&admin), "discovery", "").await.status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_discovery_page_is_for_administrators(pool: PgPool) {
    let (app, state, _guard) = app_with_key(pool, Some(TEST_MASTER_KEY)).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    assert_eq!(
        get(&app, "/surveillance/discovery", &[HTML]).await.status,
        StatusCode::FOUND
    );
    assert_eq!(
        get(&app, "/surveillance/discovery", &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    let page = get(&app, "/surveillance/discovery", &[("cookie", &admin)]).await;
    assert_eq!(page.status, StatusCode::OK);
    for needle in [
        "discovery-start",
        "manual-ip",
        "manual-submit",
        "Lancer la découverte",
        "Ajout par adresse IP",
    ] {
        assert!(page.text.contains(needle), "{needle}");
    }
}

// ---------------------------------------------------------------- live (R6 lot 4)

async fn live_camera(state: &AppState, name: &str, active: bool) -> dominet_domain::camera::Camera {
    use dominet_domain::camera::CameraDraft;
    use dominet_server::cameras::{CameraInput, SecretInput};
    let mut draft = CameraDraft::new(name);
    draft.ip = "192.168.1.29".into();
    draft.active = active;
    state
        .cameras
        .save(
            None,
            CameraInput {
                draft,
                rtsp_pass: SecretInput::Keep,
                onvif_pass: SecretInput::Keep,
                web_pass: SecretInput::Keep,
                web_api_key: SecretInput::Keep,
            },
        )
        .await
        .unwrap()
        .0
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_live_page_shows_a_player_for_any_account_and_nothing_for_an_unknown_camera(
    pool: PgPool,
) {
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let cam = live_camera(&state, "Entr\u{e9}e <b>", true).await;
    let off = live_camera(&state, "Cave", false).await;
    let path = format!("/surveillance/live/{}", cam.uuid);

    let anon = get(&app, &path, &[HTML]).await;
    assert_eq!(anon.status, StatusCode::FOUND);

    let page = get(&app, &path, &[("cookie", &user)]).await;
    assert_eq!(page.status, StatusCode::OK);
    for needle in [
        "vc-live",
        "data-live-state",
        "<video",
        "autoplay",
        "playsinline",
    ] {
        assert!(page.text.contains(needle), "{needle}");
    }
    assert!(
        page.text.contains("Entr\u{e9}e &lt;b&gt;"),
        "le nom est echappe"
    );
    // Rien de la configuration du flux n'est envoye.
    assert!(!page.text.contains("192.168.1.29") && !page.text.contains("rtsp://"));

    // La politique de contenu laisse la page appeler MediaMTX (reglage par defaut : http://localhost:8889).
    let csp = page.headers[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("connect-src 'self' http://localhost:8889"),
        "{csp}"
    );

    for unknown in [
        off.uuid.to_string(),
        "0".repeat(32),
        "pas-un-uuid".to_string(),
    ] {
        let r = get(
            &app,
            &format!("/surveillance/live/{unknown}"),
            &[("cookie", &user)],
        )
        .await;
        assert!(r.text.contains("live-not-found"), "{unknown}");
        assert!(!r.text.contains("<video"), "{unknown}");
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_live_function_builds_the_whep_address_from_the_settings_and_the_host(pool: PgPool) {
    // L'hote des requetes de ce harnais est `dominet.test`.
    let (app, state, _guard) = app_with_vars(
        pool,
        None,
        &[("ALLOWED_HOSTS", "dominet.local,dominet.test")],
    )
    .await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let cam = live_camera(&state, "Entree", true).await;
    let body = format!("uuid={}", cam.uuid);
    let ask = || async {
        send(
            &app,
            Method::POST,
            "/sf/cameras/live",
            &[("cookie", user.as_str()), FORM],
            Some(&body),
        )
        .await
    };

    // Domaine configure = un hote propre de l'instance : le navigateur appelle MediaMTX par l'hote par lequel il est arrive.
    state
        .settings
        .set("MEDIAMTX_WEBRTC_DOMAIN", "dominet.local:8889")
        .await
        .unwrap();
    let own = ask().await;
    assert!(
        own.text
            .contains(&format!("http://dominet.test:8889/{}/whep", cam.uuid)),
        "{}",
        own.text
    );
    // Domaine externe : garde tel quel.
    state
        .settings
        .set("MEDIAMTX_WEBRTC_DOMAIN", "cdn.exemple.com:8889")
        .await
        .unwrap();
    let external = ask().await;
    assert!(
        external
            .text
            .contains(&format!("http://cdn.exemple.com:8889/{}/whep", cam.uuid)),
        "{}",
        external.text
    );
    // Aucune adresse du flux source, aucun secret.
    assert!(!external.text.contains("192.168.1.29") && !external.text.contains("rtsp"));
    // Sans session : refuse.
    let anon = send(&app, Method::POST, "/sf/cameras/live", &[FORM], Some(&body)).await;
    assert_eq!(anon.status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_camera_widget_shows_the_live_player_and_an_unknown_camera_says_so(pool: PgPool) {
    use dominet_domain::dashboard::WidgetType as T;
    let (app, state, _guard) = app(pool).await;
    seed(&state).await;
    let cookie = session_for(&app, &state, "bob", Role::User).await;
    let b = dash::board(&state, "bob", "Maison").await;
    let cam = live_camera(&state, "Garage", true).await;
    let off = live_camera(&state, "Cave", false).await;
    for (title, uuid, row) in [
        ("Garage", cam.uuid.0, 0),
        ("Cave", off.uuid.0, 3),
        ("Inconnue", uuid::Uuid::from_u128(7), 6),
    ] {
        dash::widget(&state, b, T::Camera, title, (0, row, 4, 3), |d| {
            d.camera_uuid = Some(uuid);
        })
        .await;
    }
    let page = get(&app, "/", &[("cookie", &cookie)]).await;
    assert_eq!(page.status, StatusCode::OK);
    // Une seule camera est active : un seul lecteur, avec son flux WHEP ; les deux autres disent pourquoi il n'y en a pas.
    assert_eq!(
        page.text.matches("data-live-state").count(),
        1,
        "un lecteur"
    );
    assert!(page.text.contains(&format!("data-camera=\"{}\"", cam.uuid)));
    assert_eq!(
        page.text.matches("widget-camera-missing").count(),
        2,
        "{}",
        page.text.len()
    );
    assert!(!page.text.contains("rtsp://") && !page.text.contains("192.168.1.29"));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_night_function_is_for_administrators_and_reports_what_the_camera_did(pool: PgPool) {
    use dominet_domain::camera::NightApi;
    use dominet_surveillance::testing::night::FakeThingino;
    let fake = FakeThingino::start("cle").await;
    let (app, state, _guard) = app_with_key(pool, Some(TEST_MASTER_KEY)).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let mut cam = live_camera(&state, "Garage", true).await;
    let mut draft = cam.config.clone();
    draft.ip = "127.0.0.1".into();
    draft.night_mode_api = NightApi::Thingino;
    draft.web_port = i32::from(fake.port());
    cam = state
        .cameras
        .save(
            Some(cam.id),
            dominet_server::cameras::CameraInput {
                draft,
                rtsp_pass: dominet_server::cameras::SecretInput::Keep,
                onvif_pass: dominet_server::cameras::SecretInput::Keep,
                web_pass: dominet_server::cameras::SecretInput::Keep,
                web_api_key: dominet_server::cameras::SecretInput::Set("cle".into()),
            },
        )
        .await
        .unwrap()
        .0;
    let body = |action: &str| format!("uuid={}&action={action}", cam.uuid);

    for cookie in [None, Some(user.as_str())] {
        assert_eq!(
            camera_fn_form(&app, cookie, "night", &body("daynight"))
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }
    assert!(fake.bodies().is_empty(), "rien n'est parti vers la camera");

    let done = camera_fn_form(&app, Some(&admin), "night", &body("daynight")).await;
    assert!(
        done.text.contains("Done") && done.text.contains("night"),
        "{}",
        done.text
    );
    assert!(!done.text.contains("cle"), "la cle d'API ne revient jamais");
    assert_eq!(fake.bodies().len(), 1);
    assert!(
        camera_fn_form(&app, Some(&admin), "night", &body("danse"))
            .await
            .text
            .contains("InvalidAction")
    );
    assert!(
        camera_fn_form(&app, Some(&admin), "night", "uuid=pas-un-uuid&action=light")
            .await
            .text
            .contains("NotFound")
    );
}

async fn camera_fn_form(app: &Router, cookie: Option<&str>, name: &str, body: &str) -> Resp {
    let mut extra = vec![FORM];
    if let Some(c) = cookie {
        extra.push(("cookie", c));
    }
    send(
        app,
        Method::POST,
        &format!("/sf/cameras/{name}"),
        &extra,
        Some(body),
    )
    .await
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_camera_controls_are_shown_to_administrators_only_and_only_for_what_the_camera_can_do(
    pool: PgPool,
) {
    use dominet_domain::camera::{NightApi, OnvifInfo};
    use dominet_domain::repo::CameraRepo;
    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;

    // Une camera qui pivote et a une methode jour / nuit ; une autre qui ne fait ni l'un ni l'autre.
    let rich = live_camera(&state, "Salon", true).await;
    state
        .store
        .cameras()
        .apply_onvif(
            rich.id,
            &OnvifInfo {
                ptz: true,
                ..OnvifInfo::default()
            },
        )
        .await
        .unwrap();
    let mut draft = state.cameras.get(rich.id).await.unwrap().unwrap().config;
    draft.night_mode_api = NightApi::Onvif;
    state
        .cameras
        .save(
            Some(rich.id),
            dominet_server::cameras::CameraInput {
                draft,
                rtsp_pass: dominet_server::cameras::SecretInput::Keep,
                onvif_pass: dominet_server::cameras::SecretInput::Keep,
                web_pass: dominet_server::cameras::SecretInput::Keep,
                web_api_key: dominet_server::cameras::SecretInput::Keep,
            },
        )
        .await
        .unwrap();
    let poor = live_camera(&state, "Cave", true).await;

    let page = |uuid: String, cookie: String| {
        let app = app.clone();
        async move {
            get(
                &app,
                &format!("/surveillance/live/{uuid}"),
                &[("cookie", &cookie)],
            )
            .await
        }
    };
    let as_admin = page(rich.uuid.to_string(), admin.clone()).await;
    for needle in [
        "ptz-panel",
        "ptz-up",
        "ptz-left",
        "ptz-zoom-in",
        "ptz-preset-save",
        "night-panel",
        "night-daynight",
        "night-light",
    ] {
        assert!(as_admin.text.contains(needle), "{needle}");
    }
    assert!(
        as_admin.text.contains("Pilotage de la caméra") && as_admin.text.contains("Jour / nuit")
    );

    let as_user = page(rich.uuid.to_string(), user.clone()).await;
    assert!(as_user.text.contains("vc-live"), "le live reste visible");
    assert!(
        !as_user.text.contains("ptz-panel") && !as_user.text.contains("night-panel"),
        "ni PTZ ni jour/nuit pour un compte ordinaire"
    );

    let plain = page(poor.uuid.to_string(), admin).await;
    assert!(
        !plain.text.contains("ptz-panel") && !plain.text.contains("night-panel"),
        "rien a commander sur cette camera"
    );
}

// ---------------------------------------------------------------- snapshots (R6 lot 6)

fn snapshot_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dominet-ui-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const TEST_JPEG: [u8; 8] = [0xFF, 0xD8, 1, 2, 3, 4, 0xFF, 0xD9];

fn write_latest(dir: &std::path::Path, uuid: impl std::fmt::Display, bytes: &[u8]) {
    let latest = dir.join("latest");
    std::fs::create_dir_all(&latest).unwrap();
    std::fs::write(latest.join(format!("{uuid}.jpg")), bytes).unwrap();
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_snapshot_route_serves_the_latest_image_to_any_account_and_indexes_only_for_administrators(
    pool: PgPool,
) {
    let dir = snapshot_dir("route");
    let (app, state, _guard) =
        app_with_vars(pool, None, &[("SNAPSHOTS_DIR", dir.to_str().unwrap())]).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let cam = live_camera(&state, "Garage", true).await;
    let path = format!("/surveillance/camera/{}/snapshot/", cam.uuid);

    // Sans session : refus. Camera inconnue : 404. Pas d'image : 204.
    assert_eq!(get(&app, &path, &[]).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        get(
            &app,
            &format!("/surveillance/camera/{}/snapshot/", "0".repeat(32)),
            &[("cookie", &user)]
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(
            &app,
            "/surveillance/camera/pas-un-uuid/snapshot/",
            &[("cookie", &user)]
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, &path, &[("cookie", &user)]).await.status,
        StatusCode::NO_CONTENT
    );

    // Une image recente : servie, jamais mise en cache, avec son age.
    write_latest(&dir, cam.uuid, &TEST_JPEG);
    let r = get(&app, &path, &[("cookie", &user)]).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.headers[header::CONTENT_TYPE], "image/jpeg");
    assert_eq!(r.headers[header::CACHE_CONTROL], "no-store");
    assert!(r.headers.contains_key("x-frame-age"));

    // `?save=1` : administrateurs seulement ; la galerie recoit l'image.
    assert_eq!(
        get(&app, &format!("{path}?save=1"), &[("cookie", &user)])
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        state
            .snapshots
            .list(None, None, None, 10)
            .await
            .unwrap()
            .len(),
        0
    );
    let writer = {
        let (dir, uuid) = (dir.clone(), cam.uuid);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            write_latest(&dir, uuid, &[0xFF, 0xD8, 7, 7, 0xFF, 0xD9]);
        })
    };
    let saved = get(&app, &format!("{path}?save=1"), &[("cookie", &admin)]).await;
    assert_eq!(saved.status, StatusCode::OK);
    writer.await.unwrap();
    let listed = state.snapshots.list(None, None, None, 10).await.unwrap();
    assert_eq!(listed.len(), 1);

    // Le fichier du snapshot indexe.
    let file = get(
        &app,
        &format!("/surveillance/snapshots/{}/file/", listed[0].id.0),
        &[("cookie", &user)],
    )
    .await;
    assert_eq!(
        (
            file.status,
            file.headers[header::CONTENT_TYPE].to_str().unwrap()
        ),
        (StatusCode::OK, "image/jpeg")
    );
    assert_eq!(
        get(
            &app,
            "/surveillance/snapshots/999999/file/",
            &[("cookie", &user)]
        )
        .await
        .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        get(&app, "/surveillance/snapshots/1/file/", &[])
            .await
            .status,
        StatusCode::UNAUTHORIZED
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn snapshot_functions_follow_the_rights_and_filter_the_gallery(pool: PgPool) {
    let dir = snapshot_dir("fns");
    let (app, state, _guard) =
        app_with_vars(pool, None, &[("SNAPSHOTS_DIR", dir.to_str().unwrap())]).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let a = live_camera(&state, "Garage", true).await;
    let b = live_camera(&state, "Salon", true).await;
    write_latest(&dir, a.uuid, &TEST_JPEG);
    let body_a = format!("uuid={}", a.uuid);
    let ask = |who: Option<&str>, name: &str, body: &str| {
        let (app, who, name, body) = (
            app.clone(),
            who.map(str::to_string),
            name.to_string(),
            body.to_string(),
        );
        async move {
            let mut extra = vec![FORM];
            if let Some(c) = who.as_deref() {
                extra.push(("cookie", c));
            }
            send(
                &app,
                Method::POST,
                &format!("/sf/snapshots/{name}"),
                &extra,
                Some(&body),
            )
            .await
        }
    };

    // Capture et suppression : administrateurs ; liste : tout compte connecte.
    for who in [None, Some(user.as_str())] {
        assert_eq!(
            ask(who, "capture", &body_a).await.status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            ask(who, "delete", "id=1").await.status,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        ask(None, "list", "camera=&day=").await.status,
        StatusCode::UNAUTHORIZED
    );

    let saved = ask(Some(&admin), "capture", &body_a).await;
    assert!(
        saved.text.contains("Saved") && saved.text.contains("Garage"),
        "{}",
        saved.text
    );
    // Camera sans image : le dit ; inconnue : le dit aussi.
    assert!(
        ask(Some(&admin), "capture", &format!("uuid={}", b.uuid))
            .await
            .text
            .contains("NoImage")
    );
    assert!(
        ask(Some(&admin), "capture", "uuid=0000")
            .await
            .text
            .contains("NotFound")
    );

    let all = ask(Some(&user), "list", "camera=&day=").await;
    assert!(all.text.contains("Garage"), "{}", all.text);
    // Filtres : une autre camera, une camera inconnue, un autre jour, aujourd'hui.
    assert!(
        !ask(Some(&user), "list", &format!("camera={}&day=", b.uuid))
            .await
            .text
            .contains("Garage")
    );
    assert_eq!(
        ask(Some(&user), "list", "camera=0000&day=")
            .await
            .text
            .trim(),
        "[]"
    );
    assert_eq!(
        ask(Some(&user), "list", "camera=&day=2001-01-01")
            .await
            .text
            .trim(),
        "[]"
    );
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    assert!(
        ask(Some(&user), "list", &format!("camera=&day={today}"))
            .await
            .text
            .contains("Garage")
    );

    let id = serde_json::from_str::<serde_json::Value>(&saved.text).unwrap()["Saved"]["id"]
        .as_i64()
        .unwrap();
    assert!(
        ask(Some(&admin), "delete", &format!("id={id}"))
            .await
            .text
            .contains("Deleted")
    );
    assert!(
        ask(Some(&admin), "delete", &format!("id={id}"))
            .await
            .text
            .contains("NotFound")
    );
    assert_eq!(
        ask(Some(&user), "list", "camera=&day=").await.text.trim(),
        "[]"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_gallery_lists_snapshots_for_everyone_and_offers_deletion_to_administrators(
    pool: PgPool,
) {
    let dir = snapshot_dir("gallery");
    let (app, state, _guard) =
        app_with_vars(pool, None, &[("SNAPSHOTS_DIR", dir.to_str().unwrap())]).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let cam = live_camera(&state, "Garage <b>", true).await;
    assert_eq!(
        get(&app, "/surveillance/snapshots", &[HTML]).await.status,
        StatusCode::FOUND
    );

    let empty = get(&app, "/surveillance/snapshots", &[("cookie", &user)]).await;
    assert_eq!(empty.status, StatusCode::OK);
    assert!(
        empty.text.contains("snapshots-empty")
            && empty.text.contains("snap-camera")
            && empty.text.contains("snap-day")
    );

    let snap = state
        .snapshots
        .index(&cam, chrono::Utc::now(), &TEST_JPEG)
        .await
        .unwrap();
    let as_user = get(&app, "/surveillance/snapshots", &[("cookie", &user)]).await;
    assert!(
        as_user.text.contains("snapshots-grid")
            && as_user
                .text
                .contains(&format!("/surveillance/snapshots/{}/file/", snap.id.0))
    );
    assert!(
        as_user.text.contains("Garage &lt;b&gt;"),
        "le nom est echappe"
    );
    assert!(
        !as_user.text.contains("snapshot-delete"),
        "pas de suppression pour un compte ordinaire"
    );
    let as_admin = get(&app, "/surveillance/snapshots", &[("cookie", &admin)]).await;
    assert!(as_admin.text.contains("snapshot-delete"));

    // La page live propose la capture aux administrateurs seulement.
    let live = format!("/surveillance/live/{}", cam.uuid);
    assert!(
        get(&app, &live, &[("cookie", &admin)])
            .await
            .text
            .contains("snapshot-capture")
    );
    assert!(
        !get(&app, &live, &[("cookie", &user)])
            .await
            .text
            .contains("snapshot-capture")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_application_logs_are_listed_to_administrators_only(pool: PgPool) {
    use dominet_server::logbuf::{self, LogEntry};

    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    // Les lignes viennent du tampon du processus (la couche `tracing` n'est pas installee dans les tests : on y pose les notres).
    let marker = format!("essai-journaux-{}", std::process::id());
    for (rank, target, message) in [
        (3, "dominet_test::alpha", format!("information {marker}")),
        (2, "dominet_test::beta", format!("avertissement {marker}")),
        (1, "dominet_test::alpha", format!("erreur {marker}")),
    ] {
        logbuf::global().push(LogEntry {
            at_ms: logbuf::now_ms(),
            rank,
            target: target.into(),
            message,
        });
    }
    let body = |level: &str, target: &str, text: &str| {
        serde_json::json!({"filter": {"level": level, "target": target, "text": text, "minutes": 0, "limit": 50}}).to_string()
    };
    let call = |cookie: Option<String>, body: String| {
        let app = app.clone();
        async move {
            let mut extra = vec![JSON];
            if let Some(c) = cookie.as_deref() {
                extra.push(("cookie", c));
            }
            send(&app, Method::POST, "/sf/logs/list", &extra, Some(&body)).await
        }
    };

    for cookie in [None, Some(user.clone())] {
        let denied = call(cookie, body("info", "", "")).await;
        assert_eq!(denied.status, StatusCode::FORBIDDEN, "{}", denied.text);
    }
    let all = call(Some(admin.clone()), body("info", "", &marker)).await;
    assert_eq!(all.status, StatusCode::OK, "{}", all.text);
    let v: serde_json::Value = serde_json::from_str(&all.text).unwrap();
    assert_eq!(v["matched"], 3);
    // Les plus recentes d'abord : l'erreur a ete posee en dernier.
    assert!(
        v["lines"][0]["message"]
            .as_str()
            .unwrap()
            .starts_with("erreur")
    );
    assert_eq!(v["lines"][0]["level"], "ERROR");
    assert!(
        v["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "dominet_test::alpha")
    );

    let warn = call(Some(admin.clone()), body("warn", "", &marker)).await;
    let w: serde_json::Value = serde_json::from_str(&warn.text).unwrap();
    assert_eq!(w["matched"], 2, "erreurs et avertissements seulement");
    let beta = call(Some(admin), body("info", "BETA", &marker)).await;
    let b: serde_json::Value = serde_json::from_str(&beta.text).unwrap();
    assert_eq!(b["matched"], 1, "module sans casse");
    assert_eq!(b["lines"][0]["target"], "dominet_test::beta");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn mcp_tokens_are_managed_by_administrators_and_their_value_is_shown_once(pool: PgPool) {
    use dominet_domain::repo::McpTokenRepo;
    use dominet_server::mcp_tokens;

    let (app, state, _guard) = app(pool).await;
    let user = session_for(&app, &state, "bob", Role::User).await;
    let admin = session_for(&app, &state, "alice", Role::Admin).await;
    let call = |cookie: Option<String>, name: &'static str, body: Option<String>| {
        let app = app.clone();
        async move {
            let mut extra = vec![if body.as_deref().is_some_and(|b| b.starts_with('{')) {
                JSON
            } else {
                FORM
            }];
            if let Some(c) = cookie.as_deref() {
                extra.push(("cookie", c));
            }
            send(
                &app,
                Method::POST,
                &format!("/sf/mcp/{name}"),
                &extra,
                body.as_deref(),
            )
            .await
        }
    };
    let create = |name: &str, scopes: &[&str], days: u32| {
        serde_json::json!({"input": {"name": name, "scopes": scopes, "days": days}}).to_string()
    };
    let audit = || serde_json::json!({"filter": {"kind": "", "limit": 50}}).to_string();
    let token_count = || async { state.store.mcp_tokens().list().await.unwrap().len() };

    // Visiteur et compte simple : refus sur les quatre points, rien n'est cree.
    for cookie in [None, Some(user.clone())] {
        for (name, body) in [
            ("page", None),
            ("tokens/create", Some(create("x", &["lecture"], 30))),
            ("tokens/revoke", Some("id=1".to_string())),
            ("audit/list", Some(audit())),
        ] {
            let r = call(cookie.clone(), name, body).await;
            assert_eq!(r.status, StatusCode::FORBIDDEN, "{name}: {}", r.text);
        }
    }
    assert_eq!(token_count().await, 0);

    // Un formulaire invalide est refuse champ par champ, sans rien creer.
    let bad = call(
        Some(admin.clone()),
        "tokens/create",
        Some(create("", &[], 0)),
    )
    .await;
    assert_eq!(bad.status, StatusCode::OK, "{}", bad.text);
    let bad: serde_json::Value = serde_json::from_str(&bad.text).unwrap();
    let fields: Vec<_> = bad["Invalid"]
        .as_array()
        .expect("refus de validation")
        .iter()
        .map(|p| p["field"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(fields, ["name", "scopes", "lifetime_days"]);
    assert_eq!(token_count().await, 0);

    // Creation : la valeur est dans cette reponse, la commande aussi, et elle marche.
    let ok = call(
        Some(admin.clone()),
        "tokens/create",
        Some(create("Claude Code", &["lecture", "pilotage"], 90)),
    )
    .await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    let created: serde_json::Value = serde_json::from_str(&ok.text).unwrap();
    let saved = &created["Saved"];
    let value = saved["value"].as_str().unwrap().to_string();
    assert!(value.starts_with("dnm_") && value.len() == 47, "{value}");
    assert!(
        saved["command"]
            .as_str()
            .unwrap()
            .contains(&format!("Bearer {value}"))
    );
    assert_eq!(saved["token"]["state"], "active");
    assert_eq!(
        saved["token"]["scopes"],
        serde_json::json!(["lecture", "pilotage"])
    );
    let token = mcp_tokens::authenticate(&state, &value)
        .await
        .unwrap()
        .expect("le jeton cree authentifie");
    assert_eq!(token.name, "Claude Code");

    // Ni la liste, ni le journal d'audit ne revelent la valeur ; la liste ne montre que son debut.
    let page = call(Some(admin.clone()), "page", None).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    assert!(
        !page.text.contains(&value),
        "la liste ne doit pas contenir la valeur"
    );
    let page: serde_json::Value = serde_json::from_str(&page.text).unwrap();
    assert_eq!(page["enabled"], false, "serveur coupe par defaut");
    assert_eq!(page["rate_limit_per_min"], 120);
    assert_eq!(page["tokens"][0]["prefix"], value[..12]);
    let lines = call(Some(admin.clone()), "audit/list", Some(audit())).await;
    assert!(!lines.text.contains(&value));
    let lines: serde_json::Value = serde_json::from_str(&lines.text).unwrap();
    assert_eq!(lines[0]["kind"], "token_created");
    assert_eq!(lines[0]["actor"], "alice");
    assert_eq!(lines[0]["actor_kind"], "web");

    // Revocation : refusee des l'appel suivant, ecrite au journal, sans effet la seconde fois.
    let id = saved["token"]["id"].as_i64().unwrap();
    for expected in ["true", "false"] {
        let r = call(
            Some(admin.clone()),
            "tokens/revoke",
            Some(format!("id={id}")),
        )
        .await;
        assert_eq!(r.status, StatusCode::OK, "{}", r.text);
        assert_eq!(r.text.trim(), expected);
    }
    assert!(
        mcp_tokens::authenticate(&state, &value)
            .await
            .unwrap()
            .is_none()
    );
    let kinds = call(Some(admin), "audit/list", Some(audit())).await;
    let kinds: serde_json::Value = serde_json::from_str(&kinds.text).unwrap();
    let kinds: Vec<_> = kinds
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["kind"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(kinds, ["token_revoked", "token_created"]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_custom_style_sheet_is_served_empty_or_from_the_configured_file(pool: PgPool) {
    {
        // Sans CUSTOM_CSS : feuille vide, jamais une erreur (la page de connexion la charge aussi, sans session).
        // Bloc : `app_with_vars` prend un verrou global, libere ici avant le second appel.
        let (app, _state, _guard) = app_with_vars(pool.clone(), None, &[]).await;
        let none = get(&app, "/custom.css", &[]).await;
        assert_eq!(none.status, StatusCode::OK);
        assert_eq!(
            none.headers[header::CONTENT_TYPE],
            "text/css; charset=utf-8"
        );
        assert_eq!(none.headers[header::CACHE_CONTROL], "no-cache");
        assert_eq!(none.text, "");
        // L'interface la charge apres la feuille principale.
        let login = get(&app, "/login", &[]).await;
        let main = login
            .text
            .find("/pkg/dominet.css")
            .expect("feuille principale");
        let custom = login
            .text
            .find("/custom.css")
            .expect("feuille de personnalisation");
        assert!(
            main < custom,
            "la personnalisation doit venir apres la feuille principale"
        );
    }

    // Avec un fichier : son contenu, relu a chaque requete.
    let file = std::env::temp_dir().join(format!("dominet-custom-test-{}.css", std::process::id()));
    std::fs::write(&file, ":root { --accent: #ff9f1a; }").unwrap();
    let (app, _state, _guard) =
        app_with_vars(pool, None, &[("CUSTOM_CSS", file.to_str().unwrap())]).await;
    let first = get(&app, "/custom.css", &[]).await;
    assert_eq!(first.status, StatusCode::OK);
    assert!(first.text.contains("--accent: #ff9f1a"), "{}", first.text);
    std::fs::write(&file, ":root { --accent: #123456; }").unwrap();
    let second = get(&app, "/custom.css", &[]).await;
    assert!(second.text.contains("#123456"), "{}", second.text);
    // Fichier disparu : feuille vide.
    std::fs::remove_file(&file).unwrap();
    let gone = get(&app, "/custom.css", &[]).await;
    assert_eq!(gone.status, StatusCode::OK);
    assert_eq!(gone.text, "");
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_documentation_is_served_only_to_logged_in_users(pool: PgPool) {
    let root = std::env::temp_dir().join(format!("dominet-docs-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("en/page")).unwrap();
    std::fs::write(root.join("index.html"), "<h1>Accueil docs</h1>").unwrap();
    std::fs::write(root.join("en/page/index.html"), "<h1>English page</h1>").unwrap();
    let (app, state, _guard) =
        app_with_vars(pool, None, &[("DOCS_ROOT", root.to_str().unwrap())]).await;

    // Visiteur anonyme : renvoye vers la connexion, sans une ligne de la documentation.
    for path in ["/docs/", "/docs", "/docs/en/page/"] {
        let r = get(&app, path, &[]).await;
        assert_eq!(r.status, StatusCode::SEE_OTHER, "{path}: {}", r.text);
        assert!(
            r.headers[header::LOCATION]
                .to_str()
                .unwrap()
                .starts_with("/login?next=%2Fdocs")
        );
        assert!(!r.text.contains("docs"), "{path}");
    }

    state
        .auth
        .create_user("bob", "", "", Role::User, PASSWORD)
        .await
        .unwrap();
    let cookie = login_cookie(&app, "bob").await;
    let h = [("cookie", cookie.as_str())];
    let home = get(&app, "/docs/", &h).await;
    assert_eq!(home.status, StatusCode::OK, "{}", home.text);
    assert!(home.text.contains("Accueil docs"));
    let en = get(&app, "/docs/en/page/", &h).await;
    assert_eq!(en.status, StatusCode::OK);
    assert!(en.text.contains("English page"));
    let bare = get(&app, "/docs", &h).await;
    assert!(
        bare.status == StatusCode::OK
            || bare
                .headers
                .get(header::LOCATION)
                .is_some_and(|l| l.to_str().unwrap().starts_with("/docs")),
        "{} {:?}",
        bare.status,
        bare.headers.get(header::LOCATION)
    );
    assert_eq!(
        get(&app, "/docs/absent/", &h).await.status,
        StatusCode::NOT_FOUND
    );
    // Pas d'evasion hors du dossier.
    let r = get(&app, "/docs/../Cargo.toml", &h).await;
    assert_ne!(r.status, StatusCode::OK);
    assert!(!r.text.contains("[workspace]"));
    let _ = std::fs::remove_dir_all(&root);
}
