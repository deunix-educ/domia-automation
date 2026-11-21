# Automations

Engine and administration of automations (R4 lot 3). Decisions: [ADR 0031](adr/0031-automation-schema.md) (schema, domain), [ADR 0032](adr/0032-automation-engine.md) (engine), R4 plan (D1 to D3).

An automation has a **trigger**, an optional **condition** and an **action**. Administrators create, edit, enable / disable and delete them in **`/admin/automations`**.

## Triggers
| Type | Fires when | Status |
|---|---|---|
| `device_state` | a **state message** of the device contains the key with the given value (`28` = `"28"`) | active |
| `mqtt_message` | a message arrives on a topic matching the filter (`+`, `#`) and its content contains the **message filter** (JSON object, value equality) | active |
| `schedule` | the five-field cron expression (local time, `TZ` setting) | active (scheduler) |
| `solar` | one of the nine solar events at the given place (latitude, longitude; decimal comma accepted) | active (scheduler) |

- **Condition** (optional): if the message contains the key it must equal the value; otherwise the condition is true. Values are typed as JSON (`100`, `true`, `"ON"`) or plain text (`ON`).
- A `device_state` fires on **every matching state message**: a repeating "ON" state fires again. Remote-control keys (`action: 1_single`) are events.
- An `mqtt_message` waits for neither `bridge/…` messages nor `…/set` / `…/get` commands. **The MQTT client subscribes by itself to the topic of every active `mqtt_message` automation**: no source to create.
- A state coalesced by ingestion (two states of the same device within the same second) stays visible to automations: a remote sending `action: 1_single` then `action: ""` does not lose its key press.

## Actions
| Type | Effect |
|---|---|
| `mqtt_publish` | publishes the JSON object on the topic (QoS 1, not retained) |
| `scene` | activates the scene (commands go out in order, see [scenes.md](scenes.md)); the scene must exist when saving |
| `notification` | log + banner in the interface of all signed-in accounts **+ email** to the alert recipient, and **Free Mobile SMS if the "SMS" box is ticked** (default message "Automation <name> triggered"; R5 lot 6, [ADR 0042](adr/0042-alerts-on-real-channels.md)) |
| `script` | Rhai script run in a sandbox, see [scripts.md](scripts.md); its effects (publish, scene, history) are applied afterwards, only if it finishes without error |

## Scheduler
`schedule` and `solar` automations are run by the `scheduler` role ([ADR 0033](adr/0033-scheduler.md)), present by default in `dominet serve`. Each has a job in `schedule_job`; the **Next run** column of `/admin/automations` shows it, in the **server's local time** (`TZ`, `Europe/Paris` by default), like every date in the interface; the REST API stays in UTC. Editing, disabling or deleting the automation updates the job immediately.
- **No catch-up**: a missed run (server stopped) is not replayed, unless it is discovered less than 60 s after its time.
- **Cron**: five fields, Sunday = 0, day of month **and** day of week; local time with daylight-saving changes (nonexistent hour: shifted by one hour; repeated hour: once).
- **Sun**: sunrise, sunset, solar noon, civil / nautical / astronomical dawns and dusks, computed without a library (within 13 s of `ephem` on our 1,036 cases). Beyond the polar circles, with no rise or set, the automation waits for the day they return.
- The system job **`compute_daily_energy`** (00:05 local) already exists; its calculation comes with lot 7.

## Guard and log
- **At most 30 firings per automation per minute** (anti-loop); beyond that they are ignored and the overrun is logged once per minute (`limited`).
- The **log** (below the table of `/admin/automations`, last 50 rows) gives the cause, the result (`ok`, `failed`, `no effect`, `limited`) and detail; the table shows the number of firings and the last one. A false filter or condition writes nothing. Retention: 30 days.
- A change is applied **immediately** (engine and subscriptions are reloaded), no restart.

## Notable behaviors
- `device_state` works; `mqtt_publish` really publishes; `mqtt_message` sees all useful messages. `schedule` and `solar` need neither Redis nor an external scheduler.
- Anti-loop protection and an execution log.
- **At cut-over**, the five "inert" production automations will become active: three for the remote control (`Tele-commande` will really command `Volet-salon`) and two `Volet-SB-est` plugins (rewritten in Rhai, see [scripts.md](scripts.md)).

## For developers
- `dominet_automation::engine`: `Engine`, `EngineHandle`, `Executor`, `fires_on` (pure), `topic_matches_py`. The server wires the engine in `dominet_server::mqtt::spawn` (`ServerExecutor` in `dominet_server::automation`).
- Domain: `loose_equal`, `payload_filter_holds`, `device_state_holds`, `value_from_text` (`dominet_domain::automation`).
- Server functions: `automations/{list,save,delete,runs}` (administrators).

## Tests
`cargo test -p dominet-domain --lib automation` (equalities, filters, meaning of `device_state`), `-p dominet-automation --test matching` (**396 cases compared with frozen reference values**), `--test engine` (triggers, condition, actions, log, guard, queue, **production automations**), `--test hooks` (announced messages, coalesced states),
`-p dominet-automation --test schedule` and `-p dominet-domain --test schedule_oracle` (**cron and sun compared with frozen reference values**), `--test scheduler` (synchronisation, on-time runs, no catch-up, failures, the two production solar automations), `-p dominet-server --test automations` (end to end on the dev broker: the production remote control moves the shutter, the production sunset / sunrise activates its scenes, topic without source, scene, hot disable), `-p dominet --test ui automation` (permissions, field-by-field validation); E2E `automations`.
