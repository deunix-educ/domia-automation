# ADR 0053 — Prometheus alert relay and Grafana dashboards

Status: accepted.

## Context
Alerts raised by Prometheus must reach the owner by e-mail (and by SMS for critical ones), and Grafana needs dashboards written for the metrics of this application. The relay itself had two defects, and some dashboards cited metrics that do not exist.

## Decisions
- **Relay** (`prometheus_alerts`, `scheduler` role, active only if `PROMETHEUS_URL` is set): every `PROMETHEUS_ALERTS_POLL_SECS` (60 s), `GET /api/v1/alerts`. A new `firing` alert → e-mail "[DomiNet] Alert" + a Free Mobile SMS if `critical` (the channels of ADR 0038: never a real send outside an explicit request); silence of `PROMETHEUS_ALERT_COOLDOWN` (3,600 s). The core is a **state machine without input-output** (clock supplied), hence testable.
- **Two deliberate deviations from the previous behaviour**, which were defects:
  1. the cache was indexed by `fingerprint`, a field that **Prometheus's alerts API does not provide** (it is Alertmanager's): all alerts shared the same key and the second stayed silent for an hour after the first. Rust draws the alert's identity from **all its labels**.
  2. a "RESOLVED" message was planned for the `resolved` state, which Prometheus never returns (it removes the alert from the list). Rust sends "RESOLVED" (e-mail, never SMS) when a notified alert **disappears**, or turns `resolved`. It is one more e-mail per ending alert.
- **Messages** keep the format (`[DomiNet — FIRING] name (SEVERITY)`, summary, description), checked pass by pass (pending alert ignored, silence, resolution, missing severity, silence expiry).
- **Grafana dashboards**: the six generic dashboards are ported (SQL table and variable names, PromQL expression by expression). Device dashboards (`device_<name>`) stay produced by the application (ADR 0029). Old MediaMTX names (`paths_bytes_received`…) become `paths_inbound_bytes`… (MediaMTX 1.21.3 declares the former obsolete). The weather station is recognised by the state key `weather_code`. Server function routes (`/sf/<point>`) become an exact `route` label when the server knows them (status ≠ 404), which makes the "commands" and "scene activations" panels possible (completes ADR 0051). `dominet_scheduler_task_duration_seconds` is added for the task duration panel.
- **Provisioning files** in `deploy/`: `grafana/provisioning/{datasources,dashboards,alerting}` (PostgreSQL and Prometheus sources, e-mail contact point, policy) and `prometheus/prometheus.yml` (jobs `dominet:9100` and `mediamtx:9998`). Compose mounts them.

## Checks
- Integration test `grafana_assets`: every SQL query of the dashboards (macros and variables replaced) passes `EXPLAIN` on DomiNet's schema; **every metric cited** by a dashboard or shipped rule exists, in `metrics::names()` or in the **real MediaMTX 1.21.3 listing** (`tests/fixtures/observability/mediamtx-1.21.3.names`). This test found `mediamtx_paths`, a non-existent metric in the `MediaMTXNoStreams` rule (ADR 0052 corrected).
- `tools/check-observability.sh`: real throw-away Prometheus 2.52 and Grafana 11.0 on shifted ports, throw-away database: `promtool check rules`, rule loading, reload requested by DomiNet accepted by both services, Grafana loads dashboards, sources (real PostgreSQL connection), contact points, policies and alert rules without error, and runs a dashboard SQL query.

## Consequences
- Rule files must be on a volume shared between `dominet` (write) and Prometheus / Grafana (read).
