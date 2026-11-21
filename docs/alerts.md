# Alerts

Rules, evaluation, alerts page and sidebar badge (R4 lot 6). Decisions: [ADR 0035](adr/0035-alerts.md), R4 plan (D8, D10, D12).

A **rule** watches a key of a device's state (`temperature`, `state`, `power`…); when its condition is true, an **alert** is created. It stays "firing" until the condition ends ("resolved") or an account **acknowledges** it.

## For every account
- **`/alerts`** (the "Alerts" link of the sidebar, with a **badge** of the number of firing alerts, updated live): the most recent alerts (200) filtered by status — firing (default), resolved, acknowledged, all. Each row gives the severity, the rule, the device, the triggering value, the date and the status (and who acknowledged, when). **Acknowledge** (one firing alert) and **Acknowledge all** buttons.
- Every fired alert shows a **banner** in the interface of all signed-in accounts: `[DomiNet] Heatwave — meteo_locale : 36.2` (level according to severity).

## Platform alerts (Prometheus and Grafana)
The sidebar badge also counts the alerts **currently firing** in Prometheus and Grafana (application offline, 5xx errors, MQTT disconnected, heat wave…), read every minute and read-only: they have no "Acknowledge" button and disappear when their cause does. No new setting (`PROMETHEUS_URL`, `GRAFANA_URL`, `GRAFANA_USER`, `GRAFANA_PASSWORD`). [ADR 0061](adr/0061-platform-alerts-in-the-alerts-page.md). The `/alerts` page lists them in a **Platform** section (severity, source, name, summary, since when) above the device alerts, for the "Firing" and "All" filters. If Prometheus or Grafana does not answer, the page says so ("Prometheus cannot be reached") instead of showing "no alerts". With no address configured for either, the section does not appear.

## For administrators
- **`/admin/alert-rules`**: device, state key, operator (`>`, `≥`, `<`, `≤`, `=`, `≠`, contains), numeric (decimal comma accepted) or text threshold, severity (info, warning, critical), delay between two alerts (0 to 1,440 min), three channels (e-mail, SMS, Android), enabled. A change applies immediately. Deleting a rule keeps its alerts.

## How a rule is evaluated
- On **every state** of the device (Zigbee2MQTT or automatic device). Missing or `null` key: skipped, nothing fired or resolved.
- **Condition**: numbers (numeric text and booleans included: `"40"` or `true` are read as numbers); if the value or the threshold is not a number, `=` and `≠` compare texts ignoring case and spaces and other operators are false; "contains" searches the text threshold in the value's text, ignoring case.
- **True**: an alert is created, unless a **firing** alert of the rule is younger than the delay. **False**: the rule's firing alerts are resolved.
- Quirks kept on purpose: an **acknowledged** alert no longer counts in the delay (the next hot state creates one right away); a condition that stays true recreates an alert each time the delay expires; a delay of 0 creates an alert on every true state.

## Delivery
Each fired alert first shows a **banner** in the interface, then a background task serves the rule's **channels** (R5 lot 6, [ADR 0042](adr/0042-alerts-on-real-channels.md)):

| Channel | What goes out |
|---|---|
| **Email** | to the alert recipient (`EMAIL_RECIPIENT`, else the sender), subject `[DomiNet Alerte <severity>] <rule>`, long text (rule, device, value, date) |
| **SMS** | Free Mobile SMS, short text (160 characters at most); skipped without `SMS_FREE_USER` / `SMS_FREE_KEY` |
| **Android** | `alert_notification` pushed to **all** online Android devices. The current app **ignores** this message ("Message ignored"): it is sent for a future version |

Each channel is tried even if another fails (failures go to the server log, never the secret); an unconfigured channel is simply skipped. The alert becomes `notified` after the attempt, **failures or not** and is not sent again: the rule's repeat delay decides the next attempt. Channels and their settings: [notifications](notifications.md). Tests and the demonstration never send anything "for real".

## For developers
- `dominet_automation::alerts`: `AlertEngine`, `AlertHandle`, `Notifier`, `AlertNotice`; wired in `dominet_server::mqtt::spawn` (`AlertNotifier` in `dominet_server::alerts`). Domain: `AlertRuleDraft::evaluate`, `alert_messages`.
- Real-time topic `alerts` (`WsServerMsg::AlertCount`): last state retained; published at startup and on every change.
- Server functions: `alerts/{page,acknowledge,acknowledge-all}` (any signed-in account), `alert-rules/{list,save,delete}` (administrators).

## Tests
`cargo test -p dominet-domain --test alerts_oracle` (**2,030 evaluations compared with frozen reference values**), `-p dominet-automation --test alerts` (**8 scenarios of the `alerting.py` cycle compared with frozen reference values after every step**, texts, restart, disabled rules), `-p dominet --test ui alert` (permissions, field-by-field validation, page, badge), `-p dominet-server --test alerts` (the production heatwave rule end to end on the dev broker); E2E `alerts`.
