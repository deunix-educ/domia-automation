// SPDX-License-Identifier: AGPL-3.0-or-later
//! Ecriture et rechargement des regles d'alerte (R7 lot 2) : faux Prometheus et faux Grafana, base isolee, dossier temporaire.

use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use dominet_domain::alert::Severity;
use dominet_domain::observability::{
    PrometheusRuleDraft, default_grafana_rules, default_prometheus_rules,
};
use dominet_domain::repo::{GrafanaRuleRepo, PrometheusRuleRepo};
use dominet_server::commands::seed_data;
use dominet_server::config::Config;
use dominet_server::rules::{RulesService, SyncStatus};
use dominet_server::settings::SettingsService;
use dominet_store::PgStore;
use sqlx::PgPool;

/// Ce que le faux service a recu : (chemin, en-tete Authorization).
type Seen = Arc<Mutex<Vec<(String, Option<String>)>>>;

struct Fake {
    url: String,
    seen: Seen,
}

async fn fake(status: StatusCode) -> Fake {
    let seen: Seen = Arc::default();
    let app = Router::new()
        .route(
            "/{*path}",
            post(
                move |State(seen): State<Seen>,
                      axum::extract::Path(path): axum::extract::Path<String>,
                      headers: HeaderMap| async move {
                    let auth = headers
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(str::to_string);
                    seen.lock().unwrap().push((format!("/{path}"), auth));
                    status
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Fake { url, seen }
}

fn config(env: &[(&str, String)]) -> Config {
    let env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect();
    Config::from_lookup(|k| {
        if k == "DATABASE_URL" {
            return Some("postgres://u:p@localhost/db".into());
        }
        env.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone())
    })
    .unwrap()
}

fn tmp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("dominet-rules-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn prom(name: &str) -> PrometheusRuleDraft {
    PrometheusRuleDraft {
        name: name.into(),
        group: "g".into(),
        severity: Severity::Critical,
        expr: "up == 0".into(),
        for_duration: "1m".into(),
        summary: "Resume".into(),
        description: String::new(),
        eval_interval: String::new(),
        active: true,
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn prometheus_rules_are_written_then_prometheus_is_asked_to_reload(pool: PgPool) {
    let store = PgStore::new(pool);
    store.prometheus_rules().create(&prom("Une")).await.unwrap();
    let dir = tmp("prom");
    let file = dir.join("sub/rules.yml");
    let prometheus = fake(StatusCode::OK).await;
    let cfg = config(&[
        ("PROMETHEUS_RULES_PATH", file.display().to_string()),
        ("PROMETHEUS_URL", format!("{}/", prometheus.url)),
    ]);
    let svc = RulesService::new(store, &cfg);
    assert_eq!(
        svc.sync_prometheus().await,
        SyncStatus::Done {
            groups: 1,
            reloaded: Some(true)
        }
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("- alert: \"Une\""), "{text}");
    assert!(
        !dir.join("sub/.rules.yml.tmp").exists(),
        "pas de fichier temporaire oublie"
    );
    assert_eq!(
        prometheus.seen.lock().unwrap().clone(),
        [("/-/reload".to_string(), None)]
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn grafana_reload_uses_the_configured_account(pool: PgPool) {
    let store = PgStore::new(pool);
    let dir = tmp("graf");
    let file = dir.join("rules.yaml");
    let grafana = fake(StatusCode::OK).await;
    let cfg = config(&[
        ("GRAFANA_ALERTING_PATH", file.display().to_string()),
        ("GRAFANA_URL", grafana.url.clone()),
        ("GRAFANA_USER", "admin".into()),
        ("GRAFANA_PASSWORD", "pw-de-grafana".into()),
    ]);
    let svc = RulesService::new(store.clone(), &cfg);
    let rule = default_grafana_rules().remove(0);
    store.grafana_rules().create(&rule).await.unwrap();
    let status = svc.sync_grafana().await;
    assert_eq!(
        status,
        SyncStatus::Done {
            groups: 1,
            reloaded: Some(true)
        }
    );
    let seen = grafana.seen.lock().unwrap().clone();
    assert_eq!(seen[0].0, "/api/admin/provisioning/alerting/reload");
    // « admin:pw-de-grafana » en Basic.
    assert_eq!(
        seen[0].1.as_deref(),
        Some("Basic YWRtaW46cHctZGUtZ3JhZmFuYQ==")
    );
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("apiVersion: 1") && text.contains("rawSql"),
        "{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn a_refused_or_unreachable_reload_is_reported_but_the_file_is_kept(pool: PgPool) {
    let store = PgStore::new(pool);
    let dir = tmp("fail");
    let refusing = fake(StatusCode::INTERNAL_SERVER_ERROR).await;
    let file = dir.join("a.yml");
    let svc = RulesService::new(
        store.clone(),
        &config(&[
            ("PROMETHEUS_RULES_PATH", file.display().to_string()),
            ("PROMETHEUS_URL", refusing.url.clone()),
        ]),
    );
    let status = svc.sync_prometheus().await;
    assert_eq!(
        status,
        SyncStatus::Done {
            groups: 0,
            reloaded: Some(false)
        }
    );
    assert!(!status.is_ok() && file.exists());

    // Rien n'ecoute sur le port : meme resultat, sans faire echouer l'ecriture.
    let svc = RulesService::new(
        store.clone(),
        &config(&[
            ("PROMETHEUS_RULES_PATH", file.display().to_string()),
            ("PROMETHEUS_URL", "http://127.0.0.1:1".into()),
        ]),
    );
    assert_eq!(
        svc.sync_prometheus().await,
        SyncStatus::Done {
            groups: 0,
            reloaded: Some(false)
        }
    );

    // Sans adresse : fichier ecrit, rechargement « a faire a la main ». Sans chemin : rien du tout.
    let svc = RulesService::new(
        store.clone(),
        &config(&[("PROMETHEUS_RULES_PATH", file.display().to_string())]),
    );
    assert_eq!(
        svc.sync_prometheus().await,
        SyncStatus::Done {
            groups: 0,
            reloaded: None
        }
    );
    assert!(svc.sync_prometheus().await.is_ok());
    let none = RulesService::new(store, &config(&[]));
    assert_eq!(none.sync_prometheus().await, SyncStatus::Disabled);
    assert!(!none.is_configured());
    let _ = std::fs::remove_dir_all(&dir);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn an_unwritable_destination_is_a_failure_not_a_panic(pool: PgPool) {
    let store = PgStore::new(pool);
    // Le « dossier parent » est un fichier : impossible de creer le fichier de regles.
    let blocker =
        std::env::temp_dir().join(format!("dominet-rules-blocker-{}", std::process::id()));
    std::fs::write(&blocker, "x").unwrap();
    let svc = RulesService::new(
        store,
        &config(&[(
            "PROMETHEUS_RULES_PATH",
            blocker.join("rules.yml").display().to_string(),
        )]),
    );
    assert!(matches!(svc.sync_prometheus().await, SyncStatus::Failed(_)));
    let _ = std::fs::remove_file(&blocker);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn seed_creates_the_shipped_rules_once_and_never_brings_back_a_deleted_one(pool: PgPool) {
    let store = PgStore::new(pool);
    let settings = SettingsService::new(store.settings(), None);
    let first = seed_data(&settings, &store, |_| None, false).await.unwrap();
    assert_eq!(
        first.rules_created,
        default_prometheus_rules().len() + default_grafana_rules().len()
    );
    let second = seed_data(&settings, &store, |_| None, false).await.unwrap();
    assert_eq!(
        second.rules_created, 0,
        "table non vide : rien n'est recree"
    );

    let rules = store.prometheus_rules();
    let all = rules.list().await.unwrap();
    rules.delete(all[0].id).await.unwrap();
    let third = seed_data(&settings, &store, |_| None, false).await.unwrap();
    assert_eq!(
        third.rules_created, 0,
        "une regle supprimee par l'administrateur ne revient pas"
    );
    assert_eq!(rules.list().await.unwrap().len(), all.len() - 1);

    // Simulation : compte sans ecrire.
    let dry = PgStore::new(store.pool().clone());
    for r in dry.prometheus_rules().list().await.unwrap() {
        dry.prometheus_rules().delete(r.id).await.unwrap();
    }
    let simulated = seed_data(&settings, &dry, |_| None, true).await.unwrap();
    assert_eq!(simulated.rules_created, default_prometheus_rules().len());
    assert_eq!(dry.prometheus_rules().list().await.unwrap().len(), 0);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_context_lists_active_grafana_rules_like_reference_does(pool: PgPool) {
    use dominet_server::commands::context_document;

    let store = PgStore::new(pool);
    let mut rules = default_grafana_rules();
    rules[1].active = false;
    for r in &rules {
        store.grafana_rules().create(r).await.unwrap();
    }
    let doc = context_document(&store).await.unwrap();
    let listed = doc["grafana_rules"].as_array().unwrap();
    assert_eq!(
        listed.len(),
        rules.len() - 1,
        "les regles coupees ne sont pas dans le contexte"
    );
    // Tri par source (postgresql avant prometheus) puis identifiant.
    let order: Vec<_> = listed.iter().map(|r| r["uid"].as_str().unwrap()).collect();
    assert_eq!(
        order,
        ["meteo-canicule", "dominet-high-error-rate", "mediamtx-down"]
    );
    assert_eq!(listed[0]["datasource"], "postgresql");
    assert_eq!(listed[0]["no_data_state"], "OK");
    assert_eq!(listed[1]["for_duration"], "2m");
}
