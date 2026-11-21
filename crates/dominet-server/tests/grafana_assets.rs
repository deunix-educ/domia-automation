// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tableaux et fichiers de provisionnement Grafana / Prometheus (R7 lot 3, ADR 0053) : chaque requete SQL des tableaux **s'execute**
//! contre le schema de DomiNet (les variables et macros de Grafana sont remplacees), et chaque metrique citee par un tableau ou
//! une regle livree existe — metriques de DomiNet (`metrics::names()`) ou de MediaMTX 1.21.3 (noms releves sur le vrai binaire).

use std::collections::BTreeSet;
use std::path::PathBuf;

use dominet_domain::observability::{default_grafana_rules, default_prometheus_rules};
use serde_json::Value;
use sqlx::PgPool;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn dashboards() -> Vec<(String, Value)> {
    let dir = root().join("deploy/grafana/provisioning/dashboards");
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .map(|e| {
            let text = std::fs::read_to_string(e.path()).unwrap();
            (
                e.file_name().to_string_lossy().into_owned(),
                serde_json::from_str(&text).unwrap(),
            )
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Toutes les requetes d'un tableau : (nature, texte) ; `sql` pour PostgreSQL, `promql` pour Prometheus.
fn queries(doc: &Value) -> Vec<(&'static str, String)> {
    let mut out = vec![];
    for panel in doc["panels"].as_array().into_iter().flatten() {
        for target in panel["targets"].as_array().into_iter().flatten() {
            if let Some(sql) = target["rawSql"].as_str() {
                out.push(("sql", sql.to_string()));
            }
            if let Some(expr) = target["expr"].as_str() {
                out.push(("promql", expr.to_string()));
            }
        }
    }
    for var in doc["templating"]["list"].as_array().into_iter().flatten() {
        if var["type"] == "query" {
            out.push(("sql", var["query"].as_str().unwrap().to_string()));
        }
    }
    out
}

/// Rend une requete executable : macros de Grafana et variables remplacees par des valeurs.
fn concrete(sql: &str) -> String {
    sql.replace("$__timeFilter(ts)", "ts >= now() - interval '1 hour'")
        .replace("$__timeFilter(day)", "day >= current_date - 30")
        .replace(
            "$__timeFilter(last_seen)",
            "last_seen >= now() - interval '1 hour'",
        )
        .replace("${device_id}", "1")
        .replace("${station}", "1")
        .replace("${key}", "temperature")
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn every_sql_query_of_the_dashboards_runs_against_the_rust_schema(pool: PgPool) {
    let mut count = 0;
    for (file, doc) in dashboards() {
        for (kind, text) in queries(&doc) {
            if kind != "sql" {
                continue;
            }
            let sql = concrete(&text);
            assert!(
                !sql.contains("$__") && !sql.contains("${"),
                "{file} : macro ou variable non remplacee dans {sql}"
            );
            // `EXPLAIN` fait analyser la requete par PostgreSQL (tables, colonnes, types) sans lire de donnees.
            sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN {sql}")))
                .execute(&pool)
                .await
                .unwrap_or_else(|e| panic!("{file} : requete refusee ({e}) :\n{sql}"));
            count += 1;
        }
    }
    assert!(
        count >= 15,
        "les tableaux portes contiennent des requetes SQL ({count})"
    );
}

/// La requete SQL d'un panneau du tableau « Energie » (par son titre), macros remplacees.
fn energy_sql(title: &str) -> String {
    let (_, doc) = dashboards()
        .into_iter()
        .find(|(f, _)| f == "energy.json")
        .expect("tableau energie");
    let panel = doc["panels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["title"] == title)
        .unwrap_or_else(|| panic!("panneau {title}"));
    concrete(panel["targets"][0]["rawSql"].as_str().unwrap())
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_energy_dashboard_computes_the_self_consumption_rate(pool: PgPool) {
    use sqlx::Row;
    // Jour 1 : 10 kWh produits, 6 kWh consommes (3 + 1 Linky, 2 PAC) -> 6 kWh autoconsommes, surplus 4.
    // Jour 2 : 2 kWh produits, 7 kWh consommes -> tout est autoconsomme (2), deficit 5. Jour 3 : pas de soleil.
    for (day, hp, hc, solar, pac) in [
        ("2026-11-10", 3000, 1000, 10_000, 2000),
        ("2026-11-11", 5000, 0, 2000, 2000),
        ("2026-11-12", 4000, 0, 0, 1000),
    ] {
        sqlx::query(
            "INSERT INTO energy_daily_log (day, linky_hp_wh, linky_hc_wh, solar_wh, pac_wh) VALUES ($1::date, $2, $3, $4, $5)",
        )
        .bind(day)
        .bind(hp)
        .bind(hc)
        .bind(solar)
        .bind(pac)
        .execute(&pool)
        .await
        .unwrap();
    }
    // Sur la periode : (6000 + 2000) / (10000 + 2000) = 66,7 %.
    // (PostgreSQL rend du NUMERIC : on le convertit pour le lire en f64.)
    let sql = format!(
        "SELECT v::float8 FROM ({}) t(v)",
        energy_sql("Autoconsommation sur la période")
    );
    let rate: f64 = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_one(&pool)
        .await
        .unwrap()
        .get::<Option<f64>, _>(0)
        .unwrap_or(f64::NAN);
    assert!((rate - 66.666).abs() < 0.01, "taux de la periode : {rate}");
    // Par jour : 60 %, 100 %, et pas de valeur (et non 0 %) un jour sans production.
    let sql = format!(
        "SELECT rate::float8 FROM ({}) t(time, rate)",
        energy_sql("Autoconsommation par jour (%)")
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(&pool)
        .await
        .unwrap();
    let rates: Vec<Option<f64>> = rows.iter().map(|r| r.get(0)).collect();
    assert_eq!(rates.len(), 3);
    assert!((rates[0].unwrap() - 60.0).abs() < 0.01);
    assert!((rates[1].unwrap() - 100.0).abs() < 0.01);
    assert_eq!(rates[2], None);
    // Le bilan net par jour est celui de la page : production - (Linky + PAC), en kWh.
    let sql = format!(
        "SELECT net::float8 FROM ({}) t(time, solar, linky, pac, net)",
        energy_sql("Énergie par jour (kWh)")
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .fetch_all(&pool)
        .await
        .unwrap();
    let net: Vec<f64> = rows.iter().map(|r| r.get(0)).collect();
    assert_eq!(net, [4.0, -5.0, -5.0]);
}

#[sqlx::test(migrator = "dominet_store::MIGRATOR")]
async fn the_shipped_grafana_sql_rule_runs_too(pool: PgPool) {
    for rule in default_grafana_rules()
        .iter()
        .filter(|r| r.datasource_uid == "postgresql")
    {
        sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN {}", rule.expr)))
            .execute(&pool)
            .await
            .unwrap_or_else(|e| panic!("{} : {e}", rule.uid));
    }
}

/// Noms de metriques cites par une expression `PromQL` : les identifiants en `snake_case` qui ne sont ni des fonctions, ni des
/// etiquettes, ni des mots-cles.
fn metric_names(expr: &str) -> BTreeSet<String> {
    const WORDS: [&str; 25] = [
        "sum",
        "rate",
        "increase",
        "histogram_quantile",
        "by",
        "or",
        "and",
        "vector",
        "up",
        "without",
        "avg",
        "max",
        "min",
        "count",
        "le",
        "job",
        "name",
        "state",
        "status",
        "route",
        "method",
        "task",
        "result",
        "absent",
        "time",
    ];
    let mut out = BTreeSet::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut braces = 0;
    for c in expr.chars().chain(std::iter::once(' ')) {
        match c {
            '"' => quoted = !quoted,
            '{' if !quoted => braces += 1,
            '}' if !quoted => braces -= 1,
            _ => {}
        }
        if !quoted && braces == 0 && (c.is_ascii_alphanumeric() || c == '_') {
            word.push(c);
        } else {
            if word.len() > 2
                && word.chars().next().unwrap().is_ascii_lowercase()
                && !WORDS.contains(&word.as_str())
                && !word.chars().all(|x| x.is_ascii_digit())
            {
                out.insert(word.clone());
            }
            word.clear();
        }
    }
    out
}

fn mediamtx_names() -> BTreeSet<String> {
    let text =
        std::fs::read_to_string(root().join("tests/fixtures/observability/mediamtx-1.21.3.names"))
            .unwrap();
    text.lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn every_metric_cited_by_a_dashboard_or_a_shipped_rule_exists() {
    let ours: BTreeSet<String> = dominet_server::metrics::names()
        .into_iter()
        .map(str::to_string)
        .collect();
    let mediamtx = mediamtx_names();
    assert!(
        mediamtx.contains("paths_inbound_bytes"),
        "releve MediaMTX 1.21.3 attendu"
    );
    let mut exprs: Vec<(String, String)> = vec![];
    for (file, doc) in dashboards() {
        exprs.extend(
            queries(&doc)
                .into_iter()
                .filter(|(k, _)| *k == "promql")
                .map(|(_, e)| (file.clone(), e)),
        );
    }
    exprs.extend(
        default_prometheus_rules()
            .into_iter()
            .map(|r| (format!("regle {}", r.name), r.expr)),
    );
    exprs.extend(
        default_grafana_rules()
            .into_iter()
            .filter(|r| r.datasource_uid == "prometheus")
            .map(|r| (format!("regle Grafana {}", r.uid), r.expr)),
    );
    assert!(exprs.len() >= 30);
    for (origin, expr) in exprs {
        for metric in metric_names(&expr) {
            // Les series histogramme de DomiNet se terminent par `_bucket` / `_sum` / `_count`.
            let base = ["_bucket", "_sum", "_count"]
                .iter()
                .find_map(|s| metric.strip_suffix(s))
                .unwrap_or(&metric);
            assert!(
                ours.contains(&metric) || ours.contains(base) || mediamtx.contains(&metric),
                "{origin} : la metrique `{metric}` n'existe ni dans DomiNet ni dans MediaMTX 1.21.3 :\n{expr}"
            );
        }
    }
}

#[test]
fn the_provisioning_files_declare_the_expected_datasources_and_scrape_jobs() {
    let ds = |name: &str| {
        std::fs::read_to_string(
            root()
                .join("deploy/grafana/provisioning/datasources")
                .join(name),
        )
        .unwrap()
    };
    assert!(
        ds("postgresql.yml").contains("uid: postgresql")
            && ds("postgresql.yml").contains("type: postgres")
    );
    assert!(
        ds("prometheus.yml").contains("uid: prometheus")
            && ds("prometheus.yml").contains("http://prometheus:9090")
    );
    let prom = std::fs::read_to_string(root().join("deploy/prometheus/prometheus.yml")).unwrap();
    assert!(prom.contains("job_name: dominet") && prom.contains("dominet:9100"));
    assert!(prom.contains("job_name: mediamtx") && prom.contains("mediamtx:9998"));
    assert!(prom.contains("/etc/prometheus/rules/rules.yml"));
    let contacts = std::fs::read_to_string(
        root().join("deploy/grafana/provisioning/alerting/contactpoints.yaml"),
    )
    .unwrap();
    assert!(contacts.contains("dominet-email") && contacts.contains("${EMAIL_RECIPIENT}"));
}
