# ADR 0052 — Prometheus and Grafana alert rules

Status: accepted.

## Context
Prometheus alert rules (`AlertRule`) and Grafana rules (`GrafanaAlertRule`) are database rows edited in the administration; the provisioning files (`rules.yml`, `rules.yaml`) are derived from them, then the services are asked to reread them. An `alert_rule` table already exists in Rust: those are rules on **device state**, unrelated.

## Options
1. **Rules in the database, editable**, files regenerated and services reloaded.
2. Static versioned files: simpler, but no more online editing.
3. A single model for both services: the fields differ (raw SQL, no-data / error states, folder).

## Decision
- Option 1, two tables (`prometheus_rule`, `grafana_alert_rule`): constraints are repeated as `CHECK`s (severity, states, uid in `[a-z0-9_-]`), and also validated by the domain field by field.
- **Documents**: `prometheus_rules_document` / `grafana_rules_document` (domain, pure functions) build exactly the documents: grouping by group (Prometheus) or (folder, group, interval) (Grafana), **group interval = that of the first rule**, a source other than `prometheus` read as **raw SQL** with a window of at least 60 s. A home-made YAML rendering (blocks, quoted strings): the file reads back like the document.
- **Writing and reloading** (`RulesService`): temporary file then rename (never a half-written file); `POST {PROMETHEUS_URL}/-/reload`; `POST {GRAFANA_URL}/api/admin/provisioning/alerting/reload` with the `GRAFANA_USER` / `GRAFANA_PASSWORD` account. A refused or unreachable reload is **reported, not fatal**: the file stays written. Paths and addresses are optional (`PROMETHEUS_RULES_PATH`, `PROMETHEUS_URL`, `GRAFANA_ALERTING_PATH`, `GRAFANA_URL`); absent, the service is "not configured".
- **When**: after every save or deletion in the administration, at start-up of the web role, on demand (page button, `dominet sync-rules`).
- **Shipped rules**: `dominet seed` creates the default rules (DomiNet's metric names, ADR 0051), **only in an empty table**: a rule the administrator deleted does not come back. Addition: `DominetMqttDown` (MQTT client disconnected for 5 min). `SchedulerTaskFailureRate` covers failed scheduled tasks.
- `dump-context` lists the active Grafana rules (sorted by source then uid).

## Consequences
- Documents built from varied inputs are frozen in `tests/fixtures/observability/`; Rust tests replay the same inputs and the YAML files read back identically.
- `MediaMTXNoStreams`: an earlier expression (`mediamtx_paths == 0`) cited a metric that **does not exist** (observed on MediaMTX 1.21.3, ADR 0053) and so never fired; it becomes `(sum(paths{job="mediamtx",state="ready"}) or vector(0)) == 0`.
- Grafana rules on PostgreSQL read `device.state`: their execution against the schema is checked by the observability test.
