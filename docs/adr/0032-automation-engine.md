# 0032 — Automation engine (`mqtt_message` and `device_state` triggers)

- Status: accepted
- Date: 2026-11-02

## Context
Reading the reference implementation showed that `device_state` was never evaluated there, that `mqtt_message` was only evaluated for Zigbee2MQTT states and that `mqtt_publish` published nothing (no client). We implement the documented **intent**, keeping identical what works in the reference (topic wildcards, payload filter, condition). The engine must keep up with the ingestion rate (2,000 messages/s) without slowing it down.

## Decisions
- **One engine, one queue.** The ingestion (`Sink`) announces two things: *every useful message* (`message`) and *every device state* (`device_state`). An `EngineHandle` discards **at the source** whatever no automation waits for (a read of an in-memory copy of the active automations, with no database access) and queues the rest in a bounded queue (1,024). A single `Engine::run` consumes the queue, event by event, in order: never two automations at once, never a race on counters. Queue full: the event is dropped and counted (never a wait on the ingestion side).
- **Which messages trigger `mqtt_message`.** Device states and availabilities and **all messages outside the Zigbee2MQTT prefix** (among them those of a topic with no MQTT source: the client subscribes by itself to the topic of every active `mqtt_message` automation, QoS 1). Not `bridge/…` messages nor `…/set` / `…/get` commands: an automation that commands a device would see its own command go by.
- **Meaning of `device_state`.** Fires for **every state message** of the device whose `state[key]` equals `value`, by the text form of scalars (`28` = `"28"`) or numerically (`28.0` = `28`); never non-scalar values. Remote-control keys are events (`action: 1_single`): a repeated "ON" state fires again at every message.
- **Bursts: grouped states stay visible.** The ingestion (ADR 0027) only keeps the last state of a device received within the same second. A remote control sends `{"action":"1_single"}` then `{"action":""}` a few milliseconds apart: the first would be lost. The `Sink` therefore also receives `state_skipped` for every grouped state (not stored, not broadcast, but seen by automations, in its place in the order). Found by an intermittent test of a remote-control scenario.
- **Equalities of the reference.** `1 == 1.0`, `True == 1`, recursive equality of lists and objects; a key absent from the payload counts as `None` for the filter (a `null` filter lets it through); topic matching where `#` accepts the end of any level. Checked against **the reference implementation** on 396 cases (frozen in `tests/fixtures/automation/matching.json`).
- **Anti-loop guard.** At most **30 firings per automation per minute** (sliding window); the following ones are ignored and the overflow is logged **once a minute** (`limited`).
- **Effects through a port.** `Executor` (publish, activate a scene, notify): the server implements it with the `mqtt` role's client (QoS 1 publication, not retained), `scenes::activate_by_id` and the `notifications` real-time topic. Tests simulate it. A scene activated with a failure, an unknown scene and a refused publication are **failures** (`error`) in the log.
- **Log and counters.** Every firing that passed the filter and the condition writes an `automation_run` row (cause, status, duration, detail) and increments `trigger_count` / `last_triggered` — failures and no-effect actions included. A false filter or condition writes nothing (the log would drown). Purge after 30 days, once a day.
- **Hot reload.** Any change (administration) wakes the `mqtt` role's loop (`automations_changed`), which reloads the engine and recomputes subscriptions; a safety reread every 60 s.
- **Scripts and scheduling.** `script` actions go through the Rhai host (ADR 0034); `schedule` and `solar` triggers go through the scheduler (ADR 0033).

## Consequences
- The engine never waits for the ingestion and vice versa; its only constraint is the bounded queue (never reached in practice: an automation takes a few milliseconds).
- Deviations from the reference are all deliberate and tested.
