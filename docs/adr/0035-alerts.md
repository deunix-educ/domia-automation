# 0035 — Alerts: evaluation, lifecycle, send port

- Status: accepted
- Date: 2026-11-03

## Context
The reference implementation evaluates alert rules after every device state, creates an alert with a minimum delay between two (`cooldown`), resolves current alerts when the condition ceases, and lets the user acknowledge them (an alerts page, a sidebar badge). The requirement is to reproduce its rules **quirks included**, to verify them by **running the reference implementation**, to broadcast the alert to the interface and to provide a send port.

## Decisions
- **Evaluation.** After every recorded **state** message (Zigbee2MQTT and automatic devices; `Sink::device_state`, not the grouped states), `dominet_automation::alerts::AlertEngine` reads `state[key]` for each enabled rule of the device: key missing or `null` → nothing (neither firing nor resolving); condition true → an alert is created **unless** a *current* alert of the rule for this device is younger than `cooldown_minutes`; condition false → the rule's current alerts are resolved. Same structure as the automation engine: a handle that discards at the source what no rule watches, a bounded queue, a loop.
- **Quirks of the reference reproduced on purpose**, all verified: an *acknowledged* alert no longer counts in the cooldown (the next hot state creates one at once); an alert that stays true is recreated at every cooldown expiry; `cooldown = 0` creates an alert at every true state; the true / false / true toggle does not hold the resolved alert's cooldown; a non-numeric value with a numeric operator = false (`eq` / `neq` fall back to case- and space-insensitive text equality); `contains` searches the text form of the value (`True`, `None`).
- **Gap found by the reference run.** `float("1_000")` is 1000 in the reference (underscores between two digits); `f64::from_str` refuses it. Fixed in `float_from_text` (2,030 identical combinations since).
- **Verification against the reference implementation**: the rule evaluation run on 2,030 combinations; the alert cycle replayed with a controlled clock over 8 scenarios (cooldown, acknowledgement, missing value, text, fast toggle…), the state of alerts compared **after every step** with the Rust engine's on a throw-away database. The expected values are frozen in `tests/fixtures/automation/alerts.json`.
- **Efficiency.** Enabled rules are in memory per device (reloaded at every modification and every 60 s); the database is only read to decide a firing (last current alert); a resolution only writes if the (rule, device) pair *may* have a current alert (an in-memory set filled at start-up from the database).
- **`Notifier` port** (`fired(&AlertNotice)`, `changed()`). `UiNotifier` publishes on `notifications` the short text (`[DomiNet] rule — device: value`) with a level according to severity (info, warning, error) and logs the rule's three channels. `AlertNotice` also carries the long text (e-mail). **`notified` stays false** until a real channel (e-mail, SMS, Android push) has sent it.
- **Current alert count.** A new real-time topic `alerts` (`WsServerMsg::AlertCount`, last state kept, any signed-in account): the sidebar badge and the `/alerts` page update live, without polling. The count is published at start-up, at every firing / resolution and every acknowledgement.
- **Pages.** `/alerts` (any signed-in account: filters current / resolved / acknowledged / all, 200 alerts, acknowledge one alert, acknowledge all); `/admin/alert-rules` (administrators, generic CRUD: numeric threshold with a decimal comma accepted, three channels, delay 0 to 1,440 min). The "Alerts" link of the sidebar is **fixed** (like the reference's badge) and not a menu entry in the database: existing installations are not modified. Deleting a rule leaves its alerts (without a rule).

## Consequences
- States grouped by ingestion are not evaluated: under a burst, an isolated threshold crossing in the same second as a following state of the same device may be missed (the reference evaluated every state). Weather / energy values arrive much more slowly.
- E-mail, SMS and Android channels plug in behind `Notifier` without touching evaluation.
