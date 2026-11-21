# 0033 — Scheduler (cron, sun) and solar computation

- Status: accepted
- Date: 2026-11-02

## Context
The reference implementation relied on an external scheduler: a `CrontabSchedule` (zone `Europe/Paris`) or a `SolarSchedule` per `schedule` / `solar` automation, and `ephem` for the sun. We want to do without it, with a computation **checked against the reference**.

## Decisions
- **A `scheduler` role**, in the default `serve` process (with `web`), the single instance of an installation. It reads the `schedule_job` table; the next run is persisted (`next_run_at`) for the administration but **recomputed** at start-up and on every definition change. It sleeps until the next run, at most 30 s, and wakes up when the administration edits an automation (`schedule_changed`).
- **No catch-up.** A run discovered more than 60 s after its time (server stopped, slow database) is not replayed: the job notes "missed (N s late, no catch-up)" and moves on.
- **Synchronisation with automations.** Every active automation with a `schedule` or `solar` trigger has an `automation-<id>` job (task `run_automation`) that follows its definition; disabled, deleted or turned into `mqtt_message`, its job disappears. An unchanged definition keeps its next run. The system job `compute_daily_energy` (`5 0 * * *`, zone `TZ`) is created if missing and never overwritten afterwards (it can be edited).
- **Cron.** Classic five-field syntax (ADR 0031), wall-clock time of the job's zone (`TZ`, `Europe/Paris` by default). The day of month **and** the day of week must both match (not Vixie cron). **Daylight-saving changes as in the reference**: a non-existent hour (spring jump, 02:30) is read with the offset from before the jump (so 03:30 summer time), a repeated hour (autumn fall-back) is taken at its first occurrence and its second occurrence fires nothing. Each behaviour is covered by the frozen reference values.
- **Sun without a dependency.** `dominet_domain::solar`: Meeus series for the sun's position, equation of time, hour angle. Same conventions as the reference: sunrise and sunset at the upper limb at −34′ (centre ≈ −0.83°), dawns and dusks at the centre at −6°, −12°, −18°, solar noon = meridian transit. Midnight sun or polar night: no event within the next three days, so no event; the job retries 6 h later.
- **Interval.** Supported by the table and the computation (`after + n × unit`); no automation produces one today.
- **Execution.** The scheduler fires the automation through the **same engine** as messages (`Engine::fire`: guard, log, counters), so scene, publication and notification behave identically. An `--only scheduler` process opens its own MQTT client (no listening) if a broker is configured.

## Verification against the reference
- 119 sequences of six cron runs (17 expressions × 7 starting points: daylight-saving nights, year end, 29 February): **identical to the second**.
- 1,036 solar events (9 events × 5 places × 12 months × 2 starting hours, including Reykjavik and Sydney): **maximum gap 13 s** (tolerance 90 s). Cases where the reference itself fails (midnight sun) are set aside. Expected values are frozen in `tests/fixtures/automation/schedule.json`.

## Consequences
- No external scheduler and no Redis: one process, one table.
- Solar accuracy depends on series good to about 0.01°: plenty for blinds, to revisit only beyond the polar circles.
- `chrono-tz` (embedded IANA database) enters the server binary, never the WASM (the domain does not depend on it).
