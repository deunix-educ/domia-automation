// SPDX-License-Identifier: AGPL-3.0-or-later
//! The alert-rule documents produced by Rust are those of the **real reference implementation** for the same
//! inputs; the YAML files written by Rust (`golden.*`) can be read back as those documents.

use std::path::PathBuf;

use dominet_domain::alert::Severity;
use dominet_domain::observability::{
    GrafanaRule, GrafanaRuleDraft, GrafanaRuleId, PrometheusRule, PrometheusRuleDraft,
    PrometheusRuleId, grafana_rules_document, prometheus_rules_document, to_yaml,
};
use serde_json::Value;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/observability")
}

fn text(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap().to_string()
}

fn oracle() -> Value {
    serde_json::from_str(&std::fs::read_to_string(fixtures().join("oracle.json")).unwrap()).unwrap()
}

fn prometheus(oracle: &Value) -> Vec<PrometheusRule> {
    oracle["inputs"]["prometheus"]
        .as_array()
        .unwrap()
        .iter()
        .zip(1..)
        .map(|(r, id)| PrometheusRule {
            id: PrometheusRuleId(id),
            draft: PrometheusRuleDraft {
                name: text(r, "name"),
                group: text(r, "group"),
                severity: Severity::parse(r["severity"].as_str().unwrap()).unwrap(),
                expr: text(r, "expr"),
                for_duration: text(r, "for_duration"),
                summary: text(r, "summary"),
                description: text(r, "description"),
                eval_interval: text(r, "eval_interval"),
                active: r["active"].as_bool().unwrap(),
            },
        })
        .collect()
}

fn grafana(oracle: &Value) -> Vec<GrafanaRule> {
    oracle["inputs"]["grafana"]
        .as_array()
        .unwrap()
        .iter()
        .zip(1..)
        .map(|(r, id)| GrafanaRule {
            id: GrafanaRuleId(id),
            draft: GrafanaRuleDraft {
                uid: text(r, "uid"),
                title: text(r, "title"),
                folder: text(r, "folder"),
                group: text(r, "group"),
                group_interval: text(r, "group_interval"),
                datasource_uid: text(r, "datasource_uid"),
                expr: text(r, "expr"),
                query_window: i32::try_from(r["query_window"].as_i64().unwrap()).unwrap(),
                for_duration: text(r, "for_duration"),
                severity: Severity::parse(r["severity"].as_str().unwrap()).unwrap(),
                summary: text(r, "summary"),
                description: text(r, "description"),
                no_data_state: text(r, "no_data_state"),
                exec_err_state: text(r, "exec_err_state"),
                active: r["active"].as_bool().unwrap(),
            },
        })
        .collect()
}

#[test]
fn prometheus_document_is_the_one_reference_writes() {
    let o = oracle();
    assert_eq!(prometheus_rules_document(&prometheus(&o)), o["prometheus"]);
}

#[test]
fn grafana_document_is_the_one_reference_writes() {
    let o = oracle();
    assert_eq!(grafana_rules_document(&grafana(&o)), o["grafana"]);
}

/// The `golden.*` files are what Rust writes; `UPDATE_GOLDEN=1` regenerates them.
#[test]
fn yaml_files_are_stable_and_reference_reads_them_back_identically() {
    let o = oracle();
    for (name, doc) in [
        (
            "golden.rules.yml",
            prometheus_rules_document(&prometheus(&o)),
        ),
        ("golden.rules.yaml", grafana_rules_document(&grafana(&o))),
    ] {
        let path = fixtures().join(name);
        let text = to_yaml(&doc);
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            text,
            "{name} : relancer avec UPDATE_GOLDEN=1"
        );
    }
}
