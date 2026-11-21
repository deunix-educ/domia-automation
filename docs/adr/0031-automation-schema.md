# 0031 — Scene, automation, alert and energy schema

- Status: accepted
- Date: 2026-11-02

## Context
Scenes, automations, alert rules and the energy balance follow the reference implementation. Reading its code showed that `device_state` was never evaluated there and that `mqtt_publish` published nothing: we implement the data's **intent**, not all of the mechanics. This work lays down the schema, the domain and the repositories; later work plugs the engine into them.

## Decisions
- **JSON shape of the reference in the database, strong types in memory.** `automation.trigger_type` + `trigger_config`, `condition`, `action_type` + `action_config` keep the reference's shape (real scenarios load as they are, without conversion). The domain reads them into enums (`Trigger`, `Condition`, `Action`) that validate field by field; a database row that can no longer be read back is flagged as corrupt data rather than silently ignored. Plugins are read as `script` (Rhai, code in the `script` column).
- **Scenes.** `scene` + `scene_action` (device, JSON command, `position`). The **name is unique** (the script API activates a scene by name; the reference imposed nothing and crashed on a duplicate). Commands follow the same rules as widget commands (non-empty JSON object, 1 KB, 3 levels), at most 100 per scene. Creating and replacing actions is **one transaction**: never a half-written scene. `widget.scene_id` becomes a foreign key (`ON DELETE SET NULL`, orphan references set to `NULL` before the constraint).
- **Execution log** `automation_run` (one row per firing: instant, cause, status `ok|error|skipped|limited`, duration, message bounded to 1,000 characters), deleted with the automation, purged by age. The reference only had `last_triggered` and `trigger_count` (kept).
- **Scheduler.** `schedule_job` gains `automation_id` (unique, deleted with the automation) and accepts the **nine** solar events (the original constraint only admitted two).
- **Alerts.** `alert_rule` (device, key, operator, numeric or text threshold, severity, cooldown in minutes, three channels) and `alert` (status `firing|resolved|acknowledged`, triggering value, dates, acknowledgement by an account). `AlertRuleDraft::evaluate` is **the reference's computation**: value and threshold read as numbers (numeric text and booleans included, `float(True) = 1.0`), fallback to case- and space-insensitive text equality for `eq` / `neq` only, `contains` on the text form of the value (`True`, `None`). An alert whose rule is deleted **stays** (without a rule); those of a deleted device go with it. `latest_firing` only looks at **current** alerts: this is the reference's quirk (an acknowledged alert leaves the cooldown computation), reproduced on purpose.
- **Energy.** `energy_daily_log` (one local day per row, four integers in Wh). `recent(n)` returns the **last n** days from oldest to newest: the reference wrongly returned the first n (the page froze after thirty days).
- **Cron.** `CronSpec::parse` takes the syntax of a classic crontab (lists, ranges, steps, a reversed range that wraps around the field, named weekdays, Sunday = 0); the next-run computation is described in ADR 0033.
- **Real scenarios.** A conversion of a real installation's context into `tests/fixtures/automation/prod-context.json`; `dominet_domain::scenarios` reads it (feature `scenarios`, never in the binary). **No import**: these are test data sets.

## Consequences
- Administration forms validate through the same `problems()` as the database: a single source of rules.
- The execution log grows with activity: purge tied to the history retention.
