# ADR 0061 — Prometheus and Grafana alerts in the alerts page

Status: accepted.

## Context
Three alert systems coexist: **device** rules (DomiNet's database, the `/alerts` page, the sidebar badge), **Prometheus** rules (relayed by e-mail and SMS, ADR 0053) and **Grafana** rules (contact point `dominet-email`). Only the first were visible in the interface: a platform alert (MQTT offline, 5xx errors, heat wave) only appeared in a message.

## Options
- **A. Store everything in the database** (history, acknowledgement). A lot of code (migration, re-open rule) for a read need.
- **B. Read at every page display.** No state, but the badge does not move without a page visit, and every open page calls Prometheus and Grafana.
- **C. Read in the background, keep the last snapshot in memory.** One read every `PROMETHEUS_ALERTS_POLL_SECS` (60 s), badge broadcast at every change.

## Decision (C)
- `external_alerts` module of the server, started by the `web` role (the one that serves the page and the badge WebSocket; the hub is in process memory). Prometheus: `GET /api/v1/alerts`, `firing` state only. Grafana: `GET /api/alertmanager/grafana/api/v2/alerts?active=true&silenced=false&inhibited=false` with `GRAFANA_USER` / `GRAFANA_PASSWORD` (already configured for rule reloading). No new setting.
- **Read-only**, decided by the owner: no "Acknowledge" button (Prometheus has no such notion), the alert disappears when its cause disappears.
- **A single badge**: firing device alerts + firing Prometheus and Grafana alerts.
- Each source has a state: not configured, **unreachable** (the page must say so rather than display "no alerts"), or read. A source that no longer answers counts for nothing in the badge.
- No new notification: e-mail and SMS stay handled by the relay of ADR 0053 (Prometheus) and by Grafana's contact point. This module only shows.
- Severity: the `severity` label (`info`, `warning`, `critical`), `warning` if missing, as in the relay.

## Consequences
- A restart loses the snapshot; it is reread at once (up to a minute before the full badge; the first count at start-up only holds device alerts).
- No history of platform alerts in DomiNet: Grafana and Prometheus keep theirs.
- With roles split across several processes, only the `web` process reads the services; the e-mail relay of the `scheduler` role carries on alone (two reads a minute, negligible).
