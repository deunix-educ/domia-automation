# ADR 0051 — Prometheus metrics (`/metrics`)

Status: accepted.

## Context
Prometheus must scrape the application (15 s) with rules on `up`, 5xx errors, p95 latency and task failures, without exposing data to visitors: `/metrics` is therefore not served on the public port.

## Options
1. **A `/metrics` route on the public port**, protected by a token: one more secret, and one more page behind nginx.
2. **A separate listener** (`METRICS_ADDR`), reached by Prometheus on the Docker network, never published on the host nor proxied.
3. Library: `metrics` + `metrics-exporter-prometheus` (without its built-in listener) or `prometheus-client`.

## Decision
- Option 2: **`METRICS_ADDR`**, default `0.0.0.0:9100`, `off` to disable; a port already taken is reported and does not stop the server (several processes on a development machine). The public router does not know `/metrics` (404, tested).
- **`metrics` + `metrics-exporter-prometheus`** without listener: a global recorder installed once (`metrics::handle()`), the listener is ours.
- `dominet_*` names (Prometheus convention): `dominet_http_requests_total{method,route,status}`, `dominet_http_request_duration_seconds` (histogram 5 ms to 30 s), `dominet_http_requests_in_flight`, `dominet_ws_connections`, `dominet_mqtt_messages_received_total`, `dominet_mqtt_disconnections_total`, `dominet_mqtt_connected`, `dominet_scheduler_tasks_total{task,result}`, `dominet_recordings_total{outcome}`, `dominet_motion_events_total{camera}`, `dominet_detection_events_total{camera}`, `dominet_db_pool_connections{state}`, `dominet_build_info{version}`, `dominet_process_start_time_seconds`.
- **Bounded labels, nothing sensitive**: the route is the route *template* (`MatchedPath`) or a fixed family (`/sf/*`, `/pkg/*`, `/api/*`, `ui`), never the raw URL (identifiers, Android token); no account identifier; the camera is identified by its public UUID. Tested.
- Pool and MQTT gauges are refreshed every 15 s and just before each read.

## Consequences
- Prometheus rules are written on these names (5xx rate → `dominet_http_requests_total{status=~"5.."}`, p95 latency → `histogram_quantile` on `dominet_http_request_duration_seconds_bucket`, failed tasks → `dominet_scheduler_tasks_total{result="error"}`).
- Each role's process has its own metrics (the `dominet-discovery` service, in host network, must use `METRICS_ADDR=off` to expose nothing on the LAN).
- The `metrics` library is a simple facade (`dominet-mqtt` uses it too); without an installed recorder, calls cost almost nothing.
