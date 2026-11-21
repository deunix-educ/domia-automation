# Observability: metrics, alerts, Grafana dashboards

DomiNet exposes its metrics to Prometheus, relays its alerts by e-mail and SMS, and ships the platform's Grafana dashboards. End-to-end setup: see deployment (R7 lot 5).

## Metrics (`/metrics`)
A separate listener (`METRICS_ADDR`, default `0.0.0.0:9100`, `off` to disable), **never published** on the host: only Prometheus reaches it, over the Docker network. The public port answers 404 on `/metrics`. Signals: HTTP requests (method, route, status, latency), WebSocket, MQTT, scheduled tasks (result, duration), recording segments, motions and detections, PostgreSQL pool, firing Prometheus alerts, version. Labels are bounded (normalised route, never the raw URL) and carry no secret. [ADR 0051](adr/0051-prometheus-metrics.md).

## Alert rules
Edited in `/admin/prometheus-rules` and `/admin/grafana-rules`, written to provisioning files, services reloaded: see [Prometheus and Grafana rules](prometheus-grafana-rules.md).

## Platform alerts in the interface
The [alerts](alerts.md) page also shows, read-only, the alerts currently firing in Prometheus and Grafana (read every minute, [ADR 0061](adr/0061-platform-alerts-in-the-alerts-page.md)) and the badge counts them. This is display only: e-mail and SMS delivery is still done by the relay below (Prometheus) and by Grafana's contact point.

## Prometheus alert relay
With `PROMETHEUS_URL`, the `scheduler` role reads `/api/v1/alerts` every minute. A **new** `firing` alert sends an e-mail "[DomiNet] Alerte" (alert recipient) and, if `critical`, a Free Mobile SMS; it then stays quiet for 1 h (`PROMETHEUS_ALERT_COOLDOWN`). When it **disappears**, a "RESOLU" e-mail goes out (never an SMS). Each alert has its own identity (a single key for all would silence the second alert), and the "RESOLU" message actually goes out. [ADR 0053](adr/0053-prometheus-alerts-and-grafana-dashboards.md).

## Grafana dashboards
Seven generic dashboards in `deploy/grafana/provisioning/dashboards/`: overview (HTTP, tasks, database), automation and devices, surveillance (MediaMTX), camera detail (variable `camera_uuid`), weather (variable `station`), device history (variables `device_id` and `key`) and **energy** (see below). The first six are JSON files in the repository; the energy one is hand-written. **Per-device** dashboards are produced by the application (the device's "Grafana dashboard" option). Data sources `postgresql` and `prometheus`, contact point `dominet-email` (e-mail to `EMAIL_RECIPIENT`) and notification policy are in `deploy/grafana/provisioning/`.

## "DomiNet — Energy" dashboard
One dashboard (`energy.json`, 30 days by default) that mirrors the [energy](energy.md) page: production of the day (`qdy` key), live power (production `pac`, Linky `PAPP`, heat pump `power_ab`), **live net balance** (production − Linky − heat pump), a per-day histogram (solar, Linky, heat pump, net balance as a line, read from `energy_daily_log`) and **self-consumption**.

- **Self-consumption** = share of the solar production used by the house itself: `min(production, Linky + heat pump)` divided by production, in %. A day with no production has no rate (not 0 %). The "over the period" panel sums the days before dividing (it does not average the rates).
- Devices are found by their state keys (`pac`, `PAPP`, `power_ab`, `qdy`), not by name: the `ENERGY_*_DEVICE` settings need not be copied into Grafana.
- The "Live power" panel needs `pac`, `PAPP` and `power_ab` to be **tracked** in their devices' history (Devices page, tracked keys); `dominet seed-energy` adds them along with the daily-balance counters. Without them that panel stays empty, the rest of the dashboard works.

## Checks
- `cargo test -p dominet-server --test grafana_assets`: every SQL query of the dashboards runs on the DomiNet schema, every cited metric exists (DomiNet or MediaMTX 1.21.3), and the self-consumption rate is computed correctly on known days.
- `tools/check-observability.sh`: real throwaway Prometheus and Grafana (shifted ports, throwaway database) load rules, dashboards (seven), data sources, contact points and policies without error, run the energy dashboard's queries, and accept the reloads DomiNet asks for.
