// SPDX-License-Identifier: AGPL-3.0-or-later
//! Depots des regles d'alerte Prometheus et Grafana (R7 lot 2) : base isolee par test (`#[sqlx::test]`).

use dominet_domain::RepoError;
use dominet_domain::alert::Severity;
use dominet_domain::observability::{
    GrafanaRuleDraft, GrafanaRuleId, PrometheusRuleDraft, PrometheusRuleId,
};
use dominet_domain::repo::{GrafanaRuleRepo, PrometheusRuleRepo};
use dominet_store::PgStore;
use sqlx::PgPool;

fn prom(name: &str, group: &str) -> PrometheusRuleDraft {
    PrometheusRuleDraft {
        name: name.into(),
        group: group.into(),
        severity: Severity::Critical,
        expr: "up == 0".into(),
        for_duration: "1m".into(),
        summary: "Resume".into(),
        description: String::new(),
        eval_interval: String::new(),
        active: true,
    }
}

fn graf(uid: &str, title: &str) -> GrafanaRuleDraft {
    GrafanaRuleDraft {
        uid: uid.into(),
        title: title.into(),
        folder: "DomiNet Alerts".into(),
        group: "DomiNet".into(),
        group_interval: "1m".into(),
        datasource_uid: "prometheus".into(),
        expr: "up == 0".into(),
        query_window: 300,
        for_duration: "5m".into(),
        severity: Severity::Warning,
        summary: "R".into(),
        description: String::new(),
        no_data_state: "NoData".into(),
        exec_err_state: "Error".into(),
        active: true,
    }
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn prometheus_rules_are_listed_by_group_then_name_and_can_be_changed(pool: PgPool) {
    let repo = PgStore::new(pool).prometheus_rules();
    repo.create(&prom("B", "g2")).await.unwrap();
    repo.create(&prom("Z", "g1")).await.unwrap();
    let a = repo.create(&prom("A", "g1")).await.unwrap();
    let names: Vec<_> = repo
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.draft.name)
        .collect();
    assert_eq!(names, ["A", "Z", "B"]);

    let mut d = a.draft.clone();
    d.severity = Severity::Info;
    d.description = "Variables : {{ $value }}".into();
    d.eval_interval = "30s".into();
    let saved = repo.update(a.id, &d).await.unwrap();
    assert_eq!(saved.draft, d);
    assert_eq!(
        repo.get(a.id).await.unwrap().unwrap().draft.severity,
        Severity::Info
    );

    // Le nom est unique ; une regle absente est « introuvable ».
    assert!(matches!(
        repo.create(&prom("A", "g9")).await,
        Err(RepoError::Conflict { .. })
    ));
    assert!(matches!(
        repo.update(PrometheusRuleId(999_999), &d).await,
        Err(RepoError::NotFound)
    ));
    repo.delete(a.id).await.unwrap();
    assert!(matches!(repo.delete(a.id).await, Err(RepoError::NotFound)));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn seeding_a_prometheus_rule_never_overwrites_an_edited_one(pool: PgPool) {
    let repo = PgStore::new(pool).prometheus_rules();
    assert!(repo.insert_if_absent(&prom("Livree", "g")).await.unwrap());
    let mut edited = repo.list().await.unwrap().remove(0);
    edited.draft.expr = "up == 1".into();
    repo.update(edited.id, &edited.draft).await.unwrap();
    assert!(
        !repo.insert_if_absent(&prom("Livree", "g")).await.unwrap(),
        "deja la"
    );
    assert_eq!(
        repo.list().await.unwrap()[0].draft.expr,
        "up == 1",
        "la modification de l'administrateur est gardee"
    );
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn grafana_uids_come_from_the_title_and_stay_unique(pool: PgPool) {
    let repo = PgStore::new(pool).grafana_rules();
    let first = repo
        .create(&graf("", "Heat wave — temperature > 35°C"))
        .await
        .unwrap();
    let second = repo
        .create(&graf("", "Heat wave — temperature > 35°C"))
        .await
        .unwrap();
    let third = repo.create(&graf("mon-uid", "Autre")).await.unwrap();
    assert_eq!(first.draft.uid, "heat-wave-temperature-35c");
    assert_eq!(second.draft.uid, "heat-wave-temperature-35c-1");
    assert_eq!(third.draft.uid, "mon-uid");
    // Un uid vide a la mise a jour est recalcule sans se heurter a lui-meme.
    let mut d = first.draft.clone();
    d.uid = String::new();
    assert_eq!(
        repo.update(first.id, &d).await.unwrap().draft.uid,
        "heat-wave-temperature-35c"
    );
    assert!(matches!(
        repo.update(GrafanaRuleId(999_999), &d).await,
        Err(RepoError::NotFound)
    ));
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn grafana_rules_are_listed_by_folder_group_title_and_seeded_once(pool: PgPool) {
    let repo = PgStore::new(pool).grafana_rules();
    let mut other = graf("b", "B");
    other.folder = "Autre".into();
    repo.create(&graf("z", "Zeta")).await.unwrap();
    repo.create(&other).await.unwrap();
    repo.create(&graf("a", "Alpha")).await.unwrap();
    let titles: Vec<_> = repo
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.draft.title)
        .collect();
    assert_eq!(titles, ["B", "Alpha", "Zeta"]);
    assert!(
        !repo
            .insert_if_absent(&graf("a", "Alpha bis"))
            .await
            .unwrap()
    );
    assert!(repo.insert_if_absent(&graf("n", "Nouvelle")).await.unwrap());
    let id = repo.list().await.unwrap()[0].id;
    repo.delete(id).await.unwrap();
    assert!(repo.get(id).await.unwrap().is_none());
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_database_refuses_invalid_values_even_without_the_domain(pool: PgPool) {
    let repo = PgStore::new(pool.clone()).grafana_rules();
    // uid hors `[a-z0-9_-]`, etat inconnu : refuses par les contraintes de la table.
    assert!(repo.create(&graf("Majuscule", "T")).await.is_err());
    let mut bad = graf("ok", "T");
    bad.no_data_state = "Peut-etre".into();
    assert!(repo.create(&bad).await.is_err());
    let prom_repo = PgStore::new(pool).prometheus_rules();
    let mut p = prom("x", "g");
    p.expr = String::new();
    assert!(prom_repo.create(&p).await.is_err());
}
