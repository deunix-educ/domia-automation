// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alert rules of **Prometheus** and **Grafana** (R7 lot 2, ADR 0052): the inputs (edited in the administration), their
//! validation, and the provisioning documents derived from them (`rules.yml`, `rules.yaml`). Same construction rules as
//! the reference rules (grouping, interval of the first one, SQL or `PromQL` query...).

use serde_json::{Map, Value, json};

use crate::alert::Severity;
use crate::error::Problem;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PrometheusRuleId(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrafanaRuleId(pub i64);

/// Prometheus / Grafana duration: `0` or a series of `<n><unit>` (`30s`, `5m`, `1h30m`), units `ms s m h d w y`.
#[must_use]
pub fn is_duration(s: &str) -> bool {
    if s == "0" {
        return true;
    }
    let mut rest = s;
    let mut any = false;
    while !rest.is_empty() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 {
            return false;
        }
        rest = &rest[digits..];
        let unit = ["ms", "s", "m", "h", "d", "w", "y"]
            .into_iter()
            .find(|u| rest.starts_with(u));
        let Some(unit) = unit else { return false };
        // `m` must not swallow the start of `ms`.
        rest = &rest[unit.len()..];
        any = true;
    }
    any
}

/// Stable identifier derived from a title (like the reference's `slugify`, limited to 50 characters): lower case, `a-z0-9`, hyphens.
#[must_use]
pub fn slugify(title: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::new();
    let mut dash = false;
    for c in title.nfkd().filter(char::is_ascii) {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else if c == '-' || c.is_whitespace() {
            dash = true;
        }
    }
    out.chars()
        .take(50)
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

fn too_long(out: &mut Vec<(&'static str, Problem)>, field: &'static str, value: &str, max: usize) {
    if value.chars().count() > max {
        out.push((field, Problem::TooLong { max }));
    }
}

fn required(out: &mut Vec<(&'static str, Problem)>, field: &'static str, value: &str, max: usize) {
    if value.trim().is_empty() {
        out.push((field, Problem::Required));
    } else {
        too_long(out, field, value, max);
    }
}

// ---------------------------------------------------------------------------------------------------------------- Prometheus

/// Prometheus rule to create or replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrometheusRuleDraft {
    pub name: String,
    pub group: String,
    pub severity: Severity,
    pub expr: String,
    pub for_duration: String,
    pub summary: String,
    pub description: String,
    /// Evaluation interval of the group; empty = Prometheus's.
    pub eval_interval: String,
    pub active: bool,
}

impl PrometheusRuleDraft {
    /// All the problems, field by field.
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        required(&mut out, "name", &self.name, 128);
        required(&mut out, "group", &self.group, 64);
        required(&mut out, "expr", &self.expr, 4000);
        required(&mut out, "summary", &self.summary, 256);
        too_long(&mut out, "description", &self.description, 2000);
        if !is_duration(&self.for_duration) {
            out.push(("for_duration", Problem::Invalid));
        }
        if !self.eval_interval.is_empty() && !is_duration(&self.eval_interval) {
            out.push(("eval_interval", Problem::Invalid));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrometheusRule {
    pub id: PrometheusRuleId,
    pub draft: PrometheusRuleDraft,
}

/// `rules.yml` document: the **active** rules, in the order received (group then name), grouped by group. As in the reference,
/// the group's interval is that of its **first** rule.
#[must_use]
pub fn prometheus_rules_document(rules: &[PrometheusRule]) -> Value {
    let mut groups: Vec<(String, String, Vec<Value>)> = vec![];
    for rule in rules.iter().filter(|r| r.draft.active) {
        let d = &rule.draft;
        let mut annotations = Map::new();
        annotations.insert("summary".into(), json!(d.summary));
        if !d.description.is_empty() {
            annotations.insert("description".into(), json!(d.description));
        }
        let entry = json!({
            "alert": d.name,
            "expr": d.expr,
            "for": d.for_duration,
            "labels": {"severity": d.severity.as_str()},
            "annotations": Value::Object(annotations),
        });
        match groups.iter_mut().find(|(name, _, _)| *name == d.group) {
            Some((_, _, list)) => list.push(entry),
            None => groups.push((d.group.clone(), d.eval_interval.clone(), vec![entry])),
        }
    }
    let groups: Vec<Value> = groups
        .into_iter()
        .map(|(name, interval, rules)| {
            let mut g = Map::new();
            g.insert("name".into(), json!(name));
            g.insert("rules".into(), Value::Array(rules));
            if !interval.is_empty() {
                g.insert("interval".into(), json!(interval));
            }
            Value::Object(g)
        })
        .collect();
    json!({"groups": groups})
}

// -------------------------------------------------------------------------------------------------------------------- Grafana

pub const NO_DATA_STATES: [&str; 3] = ["NoData", "Alerting", "OK"];
pub const EXEC_ERR_STATES: [&str; 3] = ["Error", "Alerting", "OK"];

/// Grafana rule to create or replace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrafanaRuleDraft {
    /// Empty at creation: derived from the title (see [`slugify`]) and made unique by the service.
    pub uid: String,
    pub title: String,
    pub folder: String,
    pub group: String,
    pub group_interval: String,
    pub datasource_uid: String,
    pub expr: String,
    pub query_window: i32,
    pub for_duration: String,
    pub severity: Severity,
    pub summary: String,
    pub description: String,
    pub no_data_state: String,
    pub exec_err_state: String,
    pub active: bool,
}

impl GrafanaRuleDraft {
    #[must_use]
    pub fn problems(&self) -> Vec<(&'static str, Problem)> {
        let mut out = vec![];
        let uid_ok = self.uid.len() <= 64
            && self
                .uid
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
        if !self.uid.is_empty() && !uid_ok {
            out.push(("uid", Problem::Invalid));
        }
        required(&mut out, "title", &self.title, 128);
        required(&mut out, "folder", &self.folder, 64);
        required(&mut out, "group", &self.group, 64);
        required(&mut out, "datasource_uid", &self.datasource_uid, 64);
        required(&mut out, "expr", &self.expr, 4000);
        required(&mut out, "summary", &self.summary, 256);
        too_long(&mut out, "description", &self.description, 2000);
        if !is_duration(&self.group_interval) {
            out.push(("group_interval", Problem::Invalid));
        }
        if !is_duration(&self.for_duration) {
            out.push(("for_duration", Problem::Invalid));
        }
        if !(1..=86_400).contains(&self.query_window) {
            out.push((
                "query_window",
                Problem::OutOfRange {
                    min: 1,
                    max: 86_400,
                },
            ));
        }
        if !NO_DATA_STATES.contains(&self.no_data_state.as_str()) {
            out.push(("no_data_state", Problem::Invalid));
        }
        if !EXEC_ERR_STATES.contains(&self.exec_err_state.as_str()) {
            out.push(("exec_err_state", Problem::Invalid));
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrafanaRule {
    pub id: GrafanaRuleId,
    pub draft: GrafanaRuleDraft,
}

/// Grafana provisioning `rules.yaml` document: **active** rules, ordered by folder, group, title; grouped by
/// (folder, group, interval). A source other than `prometheus` is read as **raw SQL** (`rawSql`), otherwise as an instant `PromQL`.
#[must_use]
pub fn grafana_rules_document(rules: &[GrafanaRule]) -> Value {
    type Key = (String, String, String);
    let mut buckets: Vec<(Key, Vec<Value>)> = vec![];
    for rule in rules.iter().filter(|r| r.draft.active) {
        let d = &rule.draft;
        let (model, window) = if d.datasource_uid == "prometheus" {
            (
                json!({"expr": d.expr, "instant": true, "refId": "A"}),
                d.query_window,
            )
        } else {
            // Grafana requires `from` > 0; the window is not used by the SQL query itself but must be >= 60 s.
            (
                json!({"rawSql": d.expr, "format": "table", "refId": "A"}),
                d.query_window.max(60),
            )
        };
        let mut annotations = Map::new();
        annotations.insert("summary".into(), json!(d.summary));
        if !d.description.is_empty() {
            annotations.insert("description".into(), json!(d.description));
        }
        let entry = json!({
            "uid": d.uid,
            "title": d.title,
            "condition": "A",
            "data": [{
                "refId": "A",
                "datasourceUid": d.datasource_uid,
                "relativeTimeRange": {"from": window, "to": 0},
                "model": model,
            }],
            "for": d.for_duration,
            "labels": {"severity": d.severity.as_str()},
            "annotations": Value::Object(annotations),
            "noDataState": d.no_data_state,
            "execErrState": d.exec_err_state,
            "isPaused": false,
        });
        let key = (d.folder.clone(), d.group.clone(), d.group_interval.clone());
        match buckets.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(entry),
            None => buckets.push((key, vec![entry])),
        }
    }
    let groups: Vec<Value> = buckets
        .into_iter()
        .map(|((folder, name, interval), rules)| {
            json!({
                "orgId": 1,
                "name": name,
                "folder": folder,
                "interval": if interval.is_empty() { "1m".to_string() } else { interval },
                "rules": rules,
            })
        })
        .collect();
    json!({"apiVersion": 1, "groups": groups})
}

// ------------------------------------------------------------------------------------------------------------ shipped rules

#[allow(clippy::too_many_arguments)]
fn shipped_prom(
    name: &str,
    group: &str,
    severity: Severity,
    expr: &str,
    for_duration: &str,
    summary: &str,
    description: &str,
    interval: &str,
) -> PrometheusRuleDraft {
    PrometheusRuleDraft {
        name: name.into(),
        group: group.into(),
        severity,
        expr: expr.into(),
        for_duration: for_duration.into(),
        summary: summary.into(),
        description: description.into(),
        eval_interval: interval.into(),
        active: true,
    }
}

/// Shipped Prometheus rules (seed of `dominet seed`). Taken from the reference's `monitoring/prometheus/rules.yml`, with DomiNet's metric
/// names (ADR 0051): no more reference (the scheduler replaces it) nor reference.
#[must_use]
pub fn default_prometheus_rules() -> Vec<PrometheusRuleDraft> {
    use Severity::{Critical, Info, Warning};
    vec![
        shipped_prom(
            "SchedulerTaskFailureRate",
            "dominet_scheduler",
            Warning,
            r#"rate(dominet_scheduler_tasks_total{result="error"}[10m]) > 0.05"#,
            "5m",
            "High failure rate of scheduled tasks",
            r#"{{ $value | printf "%.3f" }} échecs/s — tâche : {{ $labels.task }}"#,
            "60s",
        ),
        shipped_prom(
            "DominetDown",
            "dominet_app",
            Critical,
            r#"up{job="dominet"} == 0"#,
            "1m",
            "Application DomiNet inaccessible",
            "The Prometheus scrape of DomiNet has failed for 1 min.",
            "30s",
        ),
        shipped_prom(
            "DominetHighErrorRate",
            "dominet_app",
            Warning,
            r#"sum(rate(dominet_http_requests_total{status=~"5.."}[5m])) > 0.1"#,
            "2m",
            "High HTTP 5xx error rate",
            r#"{{ $value | printf "%.2f" }} erreurs 5xx/s depuis 2 min."#,
            "",
        ),
        shipped_prom(
            "DominetHighLatency",
            "dominet_app",
            Warning,
            "histogram_quantile(0.95, sum by (le) (rate(dominet_http_request_duration_seconds_bucket[5m]))) > 2.0",
            "5m",
            "DomiNet p95 latency > 2 s",
            r#"p95 = {{ $value | printf "%.2f" }} s sur 5 min."#,
            "",
        ),
        shipped_prom(
            "DominetMqttDown",
            "dominet_app",
            Warning,
            "dominet_mqtt_connected == 0",
            "5m",
            "DomiNet is no longer connected to the MQTT broker",
            "No device data has arrived for 5 min.",
            "",
        ),
        shipped_prom(
            "BackupTooOld",
            "dominet_backup",
            Warning,
            "time() - dominet_backup_last_success_timestamp_seconds > 129600",
            "5m",
            "Last successful backup too old",
            "No successful backup for {{ $value | humanizeDuration }} (expected every night).",
            "",
        ),
        shipped_prom(
            "BackupFailing",
            "dominet_backup",
            Warning,
            r#"increase(dominet_backup_runs_total{result="error"}[1d]) > 0"#,
            "1m",
            "A backup failed",
            "See the DomiNet log (backup failed).",
            "",
        ),
        shipped_prom(
            "MediaMTXDown",
            "dominet_mediamtx",
            Critical,
            r#"up{job="mediamtx"} == 0"#,
            "1m",
            "MediaMTX inaccessible",
            "The Prometheus scrape of MediaMTX has failed for 1 min.",
            "30s",
        ),
        shipped_prom(
            "MediaMTXNoStreams",
            "dominet_mediamtx",
            Info,
            r#"(sum(paths{job="mediamtx",state="ready"}) or vector(0)) == 0"#,
            "10m",
            "No active stream on MediaMTX",
            "No camera has streamed for 10 min.",
            "",
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn shipped_grafana(
    uid: &str,
    title: &str,
    datasource: &str,
    expr: &str,
    for_duration: &str,
    severity: Severity,
    no_data: &str,
    summary: &str,
    description: &str,
) -> GrafanaRuleDraft {
    GrafanaRuleDraft {
        uid: uid.into(),
        title: title.into(),
        folder: "DomiNet Alerts".into(),
        group: "DomiNet".into(),
        group_interval: "1m".into(),
        datasource_uid: datasource.into(),
        expr: expr.into(),
        query_window: 60,
        for_duration: for_duration.into(),
        severity,
        summary: summary.into(),
        description: description.into(),
        no_data_state: no_data.into(),
        exec_err_state: "Error".into(),
        active: true,
    }
}

/// Shipped Grafana rules: the four of `context.yaml` / `alerting/rules.yaml` of the reference, on DomiNet's tables and metrics
/// (`device` instead of `automation_device`).
/// All four have the "no data" state set to `OK`: a query that returns nothing (no `up == 0`, no 5xx error) means everything is
/// fine. With `NoData` (the reference's setting), Grafana sent an anomaly alert at every evaluation (observed at the R8 installation).
#[must_use]
pub fn default_grafana_rules() -> Vec<GrafanaRuleDraft> {
    use Severity::{Critical, Warning};
    vec![
        shipped_grafana(
            "meteo-canicule",
            "Heat wave — temperature > 35°C",
            "postgresql",
            "SELECT (state->>'temperature')::float AS temperature\nFROM device\nWHERE friendly_name = 'meteo_locale'\n  AND (state->>'temperature')::float > 35",
            "5m",
            Warning,
            "OK",
            "Outdoor temperature > 35°C",
            "Heat wave detected on the local weather station.",
        ),
        shipped_grafana(
            "dominet-down",
            "DomiNet inaccessible",
            "prometheus",
            r#"up{job="dominet"} == 0"#,
            "1m",
            Critical,
            "OK",
            "Application DomiNet inaccessible",
            "The Prometheus scrape has failed for 1 min.",
        ),
        shipped_grafana(
            "dominet-high-error-rate",
            "High DomiNet error rate",
            "prometheus",
            r#"sum(rate(dominet_http_requests_total{status=~"5.."}[5m])) > 0.1"#,
            "2m",
            Warning,
            "OK",
            "High HTTP 5xx error rate",
            "",
        ),
        shipped_grafana(
            "mediamtx-down",
            "MediaMTX inaccessible",
            "prometheus",
            r#"up{job="mediamtx"} == 0"#,
            "1m",
            Critical,
            "OK",
            "MediaMTX inaccessible",
            "The Prometheus scrape of MediaMTX has failed for 1 min.",
        ),
    ]
}

// ------------------------------------------------------------------------------------------------------------------- YAML

fn yaml_key(k: &str) -> String {
    let simple = !k.is_empty()
        && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !k.starts_with(|c: char| c.is_ascii_digit());
    if simple {
        k.to_string()
    } else {
        Value::String(k.into()).to_string()
    }
}

fn yaml_scalar(v: &Value) -> String {
    match v {
        // A JSON string = a quoted YAML string (same escapes `\n`, `\"`, `\\`, `\uXXXX`).
        Value::String(_) | Value::Number(_) | Value::Bool(_) => v.to_string(),
        Value::Null => "null".into(),
        Value::Array(_) => "[]".into(),
        Value::Object(_) => "{}".into(),
    }
}

fn put(out: &mut String, pad: &str, text: &str) {
    out.push_str(pad);
    out.push_str(text);
}

fn yaml_into(v: &Value, indent: usize, out: &mut String) {
    let pad = " ".repeat(indent);
    match v {
        Value::Object(map) if !map.is_empty() => {
            for (k, val) in map {
                let nested = matches!(val, Value::Object(m) if !m.is_empty())
                    || matches!(val, Value::Array(a) if !a.is_empty());
                if nested {
                    put(out, &pad, &(yaml_key(k) + ":\n"));
                    yaml_into(val, indent + 2, out);
                } else {
                    put(out, &pad, &(yaml_key(k) + ": " + &yaml_scalar(val) + "\n"));
                }
            }
        }
        Value::Array(items) if !items.is_empty() => {
            for item in items {
                let nested = matches!(item, Value::Object(m) if !m.is_empty())
                    || matches!(item, Value::Array(a) if !a.is_empty());
                if nested {
                    // `- key: value` then the other keys aligned under the first one.
                    let mut inner = String::new();
                    yaml_into(item, indent + 2, &mut inner);
                    put(
                        out,
                        &pad,
                        &("- ".to_string() + inner.trim_start_matches(' ')),
                    );
                } else {
                    put(out, &pad, &("- ".to_string() + &yaml_scalar(item) + "\n"));
                }
            }
        }
        other => put(out, &pad, &(yaml_scalar(other) + "\n")),
    }
}

/// YAML text (blocks, quoted strings) of a JSON document: JSON is a subset of YAML, this rendering only changes
/// its presentation so that a human can re-read the file.
#[must_use]
pub fn to_yaml(v: &Value) -> String {
    let mut out = String::new();
    yaml_into(v, 0, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prom(name: &str, group: &str, interval: &str, active: bool) -> PrometheusRule {
        PrometheusRule {
            id: PrometheusRuleId(1),
            draft: PrometheusRuleDraft {
                name: name.into(),
                group: group.into(),
                severity: Severity::Warning,
                expr: "up == 0".into(),
                for_duration: "1m".into(),
                summary: "S".into(),
                description: String::new(),
                eval_interval: interval.into(),
                active,
            },
        }
    }

    #[test]
    fn shipped_rules_are_valid_and_unique() {
        let prom = default_prometheus_rules();
        for r in &prom {
            assert!(r.problems().is_empty(), "{}: {:?}", r.name, r.problems());
        }
        let graf = default_grafana_rules();
        for r in &graf {
            assert!(r.problems().is_empty(), "{}: {:?}", r.uid, r.problems());
        }
        let mut names: Vec<_> = prom.iter().map(|r| r.name.clone()).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), prom.len());
        let mut uids: Vec<_> = graf.iter().map(|r| r.uid.clone()).collect();
        uids.sort();
        uids.dedup();
        assert_eq!(uids.len(), graf.len());
    }

    #[test]
    fn durations_follow_the_prometheus_syntax() {
        for ok in ["0", "30s", "5m", "1h", "1h30m", "250ms", "2d", "1w", "1y"] {
            assert!(is_duration(ok), "{ok}");
        }
        for bad in ["", "m", "5", "5 m", "1.5h", "-1m", "5min", "h5", "5mm"] {
            assert!(!is_duration(bad), "{bad}");
        }
    }

    #[test]
    fn slugs_match_reference_for_plain_titles() {
        assert_eq!(
            slugify("Heat wave — temperature > 35°C"),
            "heat-wave-temperature-35c"
        );
        assert_eq!(slugify("  Service   inaccessible "), "service-inaccessible");
        assert_eq!(slugify("a_b-c"), "a_b-c");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn prometheus_document_groups_rules_and_the_first_interval_wins() {
        let rules = [
            prom("A", "g1", "30s", true),
            prom("B", "g1", "60s", true),
            prom("C", "g2", "", true),
            prom("off", "g3", "", false),
        ];
        let doc = prometheus_rules_document(&rules);
        let groups = doc["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "le groupe sans regle active disparait");
        assert_eq!(groups[0]["interval"], "30s");
        assert_eq!(groups[0]["rules"].as_array().unwrap().len(), 2);
        assert!(groups[1].get("interval").is_none());
        assert_eq!(groups[0]["rules"][0]["labels"]["severity"], "warning");
        assert!(
            groups[0]["rules"][0]["annotations"]
                .get("description")
                .is_none()
        );
    }

    #[test]
    fn grafana_sql_sources_use_raw_sql_and_a_window_of_at_least_a_minute() {
        let base = GrafanaRuleDraft {
            uid: "x".into(),
            title: "T".into(),
            folder: "F".into(),
            group: "G".into(),
            group_interval: "1m".into(),
            datasource_uid: "postgresql".into(),
            expr: "SELECT 1".into(),
            query_window: 5,
            for_duration: "5m".into(),
            severity: Severity::Critical,
            summary: "S".into(),
            description: "D".into(),
            no_data_state: "OK".into(),
            exec_err_state: "Error".into(),
            active: true,
        };
        let sql = grafana_rules_document(&[GrafanaRule {
            id: GrafanaRuleId(1),
            draft: base.clone(),
        }]);
        let data = &sql["groups"][0]["rules"][0]["data"][0];
        assert_eq!(data["model"]["rawSql"], "SELECT 1");
        assert_eq!(data["relativeTimeRange"]["from"], 60);
        let mut prom_rule = base;
        prom_rule.datasource_uid = "prometheus".into();
        let doc = grafana_rules_document(&[GrafanaRule {
            id: GrafanaRuleId(2),
            draft: prom_rule,
        }]);
        let data = &doc["groups"][0]["rules"][0]["data"][0];
        assert_eq!(data["model"]["expr"], "SELECT 1");
        assert_eq!(data["model"]["instant"], true);
        assert_eq!(data["relativeTimeRange"]["from"], 5);
        assert_eq!(
            doc["groups"][0]["rules"][0]["annotations"]["description"],
            "D"
        );
    }

    #[test]
    fn problems_are_reported_per_field() {
        let mut d = prom("", "", "x", true).draft;
        d.for_duration = "bientot".into();
        d.summary = " ".into();
        let fields: Vec<&str> = d.problems().iter().map(|(f, _)| *f).collect();
        assert_eq!(
            fields,
            ["name", "group", "summary", "for_duration", "eval_interval"]
        );
    }

    #[test]
    fn yaml_output_keeps_unicode_and_escapes() {
        let v = json!({"a": [{"k": "é \"q\" \\ \n x", "n": 1, "t": true}], "empty": [], "m": {"z": null}});
        let text = to_yaml(&v);
        assert!(text.contains("- k: \"é \\\"q\\\" \\\\ \\n x\""), "{text}");
        assert!(text.contains("  n: 1\n"), "{text}");
        assert!(text.contains("empty: []"));
        assert!(text.contains("m:\n  z: null"));
    }
}
