# Prometheus and Grafana alert rules

Two administration pages (administrators only): **Prometheus rules** (`/admin/prometheus-rules`) and **Grafana rules** (`/admin/grafana-rules`). Rules live in the database; each change **rewrites the provisioning file** (`rules.yml` for Prometheus, `rules.yaml` for Grafana) and **asks the service to read it again**. These rules watch the **platform** (application, MQTT, MediaMTX…); rules on device state are elsewhere (`/admin/alert-rules`, see [alerts](alerts.md)).

## Shipped rules
`dominet seed` creates the default rules, written for DomiNet metrics ([`/metrics`](configuration.md), ADR 0051): failing scheduled tasks, application down, 5xx errors, p95 latency, MQTT disconnected, MediaMTX down or without streams; on the Grafana side: heat wave (SQL on `device`), application and MediaMTX down, 5xx errors. They are created **only in an empty table**: a deleted rule does not come back.

## Fields
- **Prometheus**: name (unique), group, severity, PromQL expression, delay before alert (`5m`), summary, description (variables `{{ $value }}`, `{{ $labels.job }}`), group evaluation interval (that of the group's **first** rule), active.
- **Grafana**: title, identifier (UID, derived from the title if empty), folder, group, interval, data source (`prometheus`: PromQL; other, e.g. `postgresql`: **raw SQL**), query, window (seconds), delay, severity, summary, description, "no data" and "execution error" states, active.
- Durations: `30s`, `5m`, `1h30m` (units `ms s m h d w y`).

## Files and reload
| Variable | Role |
|---|---|
| `PROMETHEUS_RULES_PATH`, `PROMETHEUS_URL` | `rules.yml` file; `POST /-/reload` |
| `GRAFANA_ALERTING_PATH`, `GRAFANA_URL`, `GRAFANA_USER`, `GRAFANA_PASSWORD` | `rules.yaml` file; `POST /api/admin/provisioning/alerting/reload` |

The file is written **atomically**. If the reload fails (service stopped, refusal), the file stays written and the page says so; the service will read it at its next start. With no path configured, the service is "not configured" (nothing written). `dominet sync-rules` does the same on the command line; writing also happens when the server starts.

## "No data" state of the shipped rules
The four shipped Grafana rules have their "no data" state set to **`OK`**. A query such as `up == 0` or "5xx rate" returns nothing while everything is fine: with `NoData` (the former system's setting), Grafana sent an anomaly alert at every evaluation, observed during the production installation (R8). If you create your own rules of this kind, choose `OK`. An installation already made with the old setting is fixed in `/admin/grafana-rules` (existing rules are never modified by `dominet seed`).

## Verification
The files produced are compared with frozen reference values. See [ADR 0052](adr/0052-prometheus-and-grafana-alert-rules.md).
