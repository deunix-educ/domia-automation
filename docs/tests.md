# Tests

637 Rust tests + 22 browser E2E scripts (604 checks). Everything runs locally: there is no automatic CI any more
([ADR 0015](adr/0015-local-gate.md), in French).

| Command | Contents | Duration |
|---|---|---|
| `make ci` | formatting, clippy (server **and** WASM), Rust tests, `cargo deny`, `cargo audit`, `sqlx` metadata | ≈ 2 min warm |
| `make e2e` | browser E2E on `target/debug/dominet` (after `make front`) | ≈ 2.5 min |
| `make ci-full` | `make ci` + Docker image (build, smoke test) + **E2E on the image** | depends on the Docker cache |
| `tools/stress-cold-render.sh [image] [n] [concurrency] [page]` | cold starts in a loop with concurrent requests: measures the Leptos stalled-render defect the guard works around ([ADR 0021](adr/0021-page-render-guard.md), in French); expected: 0 anomalies | ≈ 3 s per start |
| `make hooks` | `pre-push` guard: `make ci` on a clean copy of the pushed commit | |

## Rust tests (`cargo test --workspace`)

Database tests use `#[sqlx::test]`: one isolated database per test, created on the dev stack's PostgreSQL (`make dev-up`).

| Level | Where | Tests | Covers |
|---|---|---|---|
| Unit | `dominet-domain` | 59 | validated types, password policy, lockout, per-field refusal reasons, grid and widgets |
| | `dominet-proto` | 9 | message, topic and DTO format (server / browser contract) |
| | `dominet-server` (lib) | 85 | configuration, errors, secret encryption, WebSocket hub, CSRF |
| | `dominet-automation` (lib) | 20 | topic routing, burst coalescing, **Rhai host** (API, effects in order, all or nothing, **hostile scripts**: loops, recursion, giant strings / arrays / maps, `pad`, `eval`, nesting, endless effects; time limit alone; another language refused) |
| | `dominet-mqtt` (lib) | 12 | topic matching, payload decoding, subscription diff, publish topics, masked password, immediate failure without a broker |
| | `dominet-integrations` (lib) | 9 | SolarMax protocol: requests byte-identical to the reference ones, answers, scaling, half-even rounding, derived values (frozen reference vectors); Open-Meteo request and normalization |
| Publishers | `dominet-integrations/tests` | 10 | SolarMax client against a **simulated** inverter (cycle, alarms, unknown model, silence, noise, recovery, closed port); weather against a simulated API (a failing station does not stop the others, pause = shortest interval), supervision of a panicking task |
| | `dominet-web` (lib) | 21 | return addresses (`next`), reconnection, counted subscriptions, CSP, Lucide icons, form pairs editor |
| Repositories and schema | `dominet-store/tests` | 83 | schema (18, including the widget overlap exclusion), devices, history, images, sources and stations (14: constraints, MQTT-listener semantics, widget foreign key), repositories (27, including the last-administrator rule, the races on the default dashboard and on widget placement), sessions and attempts (12) |
| API and services | `dominet-server/tests` | 77 | authentication and lockout (20), accounts (6), settings and secrets (17), health (6), database errors (4), **automation engine end to end** on the dev broker (3: the production remote control moves the shutter, the production sunset / sunrise activates its scenes through the scheduler, topic without source, scene, hot disable), **production heatwave rule end to end** on the dev broker (1: alert, banner, badge, cooldown, resolution), **production Rhai plugins and source code end to end** on the dev broker (1), **scene activation** on the dev broker (3: command order, a failure does not stop the next, the 3 production scenes: 6 / 4 / 4 commands on the right topics) |
| Session replay | `dominet-automation/tests/replay.rs` | 4 | 64-message session replayed one by one, in one-second batches and twice in a row: same devices, states and history as the reference result; 4,000-message burst |
| Sources replay | `dominet-automation/tests/replay_sources.rs` | 1 | 36-item session (sources, automatic devices, reactivation, "seen" marking, prefixes, inactive sources, non-object payloads): same devices, states and history as the reference result — a single documented deviation (first message of an automatic device) |
| Throughput | `dominet-server/tests/throughput.rs` | 1 | 2,000 messages/s for 5 s, QoS 1, end to end (dev broker → database → broadcast): last state of 50 devices kept, final lag < 5 s (measured ≈ 0.1 s) |
| MQTT sources | `dominet-automation/tests/sources.rs` | 7 | automatic device (name without prefix, type, state replaced, reactivation), history and archive of tracked keys, inactive sources, non-object payloads, devices marked as seen |
| Scenes, automations, alerts, energy | `dominet-store/tests/automation.rs` | 10 | scenes (action order, transactional replacement, unique name, missing device, widget foreign key), **production scenarios** (3 scenes of 6 / 4 / 4 actions, 7 automations read back unchanged), counters and run journal, scheduler job, 9 solar events, alerts (firing / resolved / acknowledged), energy balance |
| Automation engine | `dominet-automation/tests` (`matching`, `engine`, `hooks`) | 3 + 7 + 2 | **396 cases compared with frozen reference values** (topic matching, condition, payload filter: les fichiers de référence); `device_state` / `mqtt_message` triggers, condition, actions, log and counters, 30/min guard, bounded queue, **production automations**; messages and coalesced states announced to the engine |
| Scripts in the engine | `dominet-automation/tests/engine.rs` | (within the 10) | script seeing trigger, devices and effects in order; failing or hostile script: nothing applied, journaled, engine carries on; **the two production plugins ported to Rhai**; source code muted after a failure |
| Energy | `dominet-automation/tests/energy.rs` | 2 | **13 days compared with frozen reference values real `compute_daily_energy`**` truncation); local day length (23 / 24 / 25 h) |
| R4 acceptance | `dominet-server/tests/acceptance_r4.rs` | 1 | **the whole production `context.yaml` at once** (29 devices, 3 scenes, 7 automations including the 2 Rhai plugins, the heatwave rule) on the dev Mosquitto: remote, plugin, alert and banner, sunset → the scene's six commands, without getting in each other's way |
| Alerts | `dominet-domain/tests/alerts_oracle.rs`, `dominet-automation/tests/alerts.rs` | 1 + 4 | **2,030 rule evaluations compared with frozen reference values**; **8 alert-cycle scenarios** (cooldown, acknowledgement leaving the cooldown, missing value, text, quick toggle) compared with frozen reference values **after every step** on a throwaway database; reference texts, restart, disabled rules, handle |
| Android relay | `dominet-domain/tests/android_fixtures.rs`, `dominet-store/tests/android.rs`, `dominet/tests/android_ws.rs`, `dominet/tests/android_oracle.rs` | 2 + 7 + 15 + 1 + 2 + E2E | frames taken from the **Kotlin code**; repositories (hashed token, acks, timeout); endpoint on a real TCP server (token, presence, replacement, heartbeat, SMS, calls, acks, 60 s timeout, messaging, alerts, rights); **21 sessions replayed against frozen reference exchanges** |
| Alert channels | `dominet/tests/alert_channels.rs` | 5 + E2E | the **real alert engine** fires: email (sender, recipient, subject `[DomiNet Alerte …]`, long text), SMS (credentials, short text), push received by a connected phone, the rule's channels only, one channel failing does not affect the others, unconfigured channel skipped, automation notification (SMS only if asked) |
| Messaging | `dominet/tests/messaging.rs`, `dominet-server` (unit `resolve_audience`) | 6 + 1 + E2E | emails to a **fake SMTP** (sender, subject, text), SMS handed to a phone that speaks like the app, statuses before / after the ack, failures with their reason, SMS with no ack failed at 60 s, `message:<id>` events until done, subscription rights, recovery after restart |
| Scheduler | `dominet-domain/tests/schedule_oracle.rs`, `dominet-automation/tests` (`schedule`, `scheduler`) | 1 + 2 + 4 | **cron and sun compared with frozen reference values**: 119 sequences of six cron runs in `Europe/Paris` (daylight-saving nights, 29 February) identical to `TzAwareCrontab`, 1,036 solar events within 13 s of `ephem`; synchronisation with automations, on-time runs, no catch-up, failures, **production sunset / sunrise** |
| ReductStore archive | `dominet-store/tests/reduct.rs` | 2 | records with numeric labels and distinct timestamps (disposable bucket on the dev ReductStore); unreachable server: never blocking, records counted |
| End-to-end publishers | `dominet-server/tests/publishers.rs` | 2 | simulated weather API and inverter → publisher → dev Mosquitto → MQTT sources → devices; without a station or `SOLARMAX_HOST` nothing is requested |
| Context | `dominet-server/tests/context.rs` | 1 | `dump-context`: active devices sorted by type then name, state schema, YAML |
| MQTT broker | `dominet-mqtt/tests` | 6 | dev Mosquitto (decoded round trip, retained messages, live subscription changes, shutdown) and a **throwaway Mosquitto** (broker appearing after the client; 3 s outage: no accepted message lost, publications refused during the outage) — Docker required |
| Whole application | `dominet/tests/ui.rs` | 84 | FR/EN server rendering, guards, sign-in, password, CRUD of the four pages, permissions, secrets, CSP, cache, compression, **no sensitive data in logs** |
| WebSocket | `dominet/tests/ws.rs` | 15 | real TCP server and client: session, foreign origin, limit, heartbeats, cut on sign-out, subscriptions, last state, refusals, connection independence |
| Translations | `dominet-web/tests/i18n.rs` | 3 | same keys in FR and EN, no used key missing, no dead key |

R1 critical paths and their tests: **sign-in and lockout** (`auth.rs`, `ui.rs`), **sessions** (revocation, fixation: `auth_store.rs`, `auth.rs`),
**administrator permissions** (`ui.rs`, every server function tested anonymous and as an ordinary account), **secrets** (never returned: `settings.rs`, `ui.rs`),
**last administrator** (`repos.rs`, `ui.rs`: race between two administrators, checked by disabling the lock), **WebSocket** (`ws.rs`),
**CSP and nonce** (`ui.rs`), **hydration** (WASM name and markers in the HTML: `ui.rs`).

## Browser E2E (`tools/e2e.sh`)

Headless Firefox driven by Marionette (`tests/e2e/marionette.py`, no external Python dependency). The runner creates a **throwaway database**, a temporary
Firefox profile and a server, replays the scripts, then deletes everything (even on failure):

| Script | Checks |
|---|---|
| `home.py` | server rendering, **WASM hydration** (menu collapse), live translations, session |
| `r2.py` | **R2 journey through the UI only (no SQL)**: dashboard created from the invitation, one widget of each type added through administration, test message and device state, live follow, pointer move, reload, occupied position refused |
| `dashboard_edit.py` | editing: pointer move (persistent, occupied position refused), widget administration (fields by type, overlap, sanitized HTML, deletion) and dashboards |
| `dashboard.py` | dashboard: creation from the invitation, class-based placement, sanitized text, Grafana (iframe, refusal, CSP), device followed live, selector without reload, small screen; prepares its widgets in SQL (`E2E_DB_SHELL`) |
| `auth.py` | guards, autofill, failure then success, translation, open redirect refused, password change, sign-out |
| `layout.py` | server-rendered sidebar, active link, remembered collapse, language, 404, sign-out |
| `locations.py`, `admin.py`, `users.py` | generic CRUD, navigation (the sidebar follows), settings (secret never shown), users (protections) |
| `mqtt.py` | MQTT → database → real time → widget: discovery (inferred type), state replacing the previous one, history, availability, ignored noise, live-changeable prefix; on the dev Mosquitto |
| `mqtt_sources.py` | `/admin/mqtt-sources` page: add (invalid filter refused), device created by a message, state replaced, non-object payloads ignored, live deactivation and deletion |
| `devices.py` | administration (group, device, dropdown), `/automation/` (cards, filters), state published on the broker, commands (button, slider) received by the broker, offline, pairing, device page (topics, uploaded image reduced then deleted, curve, Grafana dashboard) |
| `scenes.py` | scenes (R4): administration (ordered commands, device dropdown, invalid JSON refused, order reversed), `/automation/scenes` page, activation publishing **in order** on the dev broker, ✓ / ✗ feedback (inactive device skipped, the rest goes out), scene widget (dropdown, button), deletion |
| `automations.py` | automations (R4): form by type, invalid JSON refused, state published → command published, topic without source (filter, condition), notification shown, log, 30/min guard, scheduler (every-minute cron, sunset, next run), hot disable and delete |
| (in `automations.py`) | Rhai scripts: syntax refused with the line, code in another language refused, dry run (effects, log, infinite loop interrupted), execution on a message, infinite loop while the server keeps answering, engine still alive afterwards |
| `alerts.py` | alerts (R4): rule created from the administration (unreadable threshold refused, decimal comma), state published → alert, live banner and badge, `/alerts` page (filters, acknowledge, acknowledge all), acknowledged alert = out of the cooldown, resolution, deleted rule: alerts stay |
| `energy.py` | energy (R4): counter history, **the real scheduler** computes yesterday (same Wh as the reference computation), `/automation/energy` page (script-free SVG, ranges, table), `/api/energy/` API, live state (production, Linky, heat pump, net balance) without reload |
| `weather.py` | weather stations (administration, decimal comma, latitude out of range), immediate reading on creation against a **simulated** API, simulated inverter → devices (MQTT source) visible on `/automation/`, disabling = no more requests; diagnostics `publish solarmax --print-only` and `dump-context` |
| `android.py` | Android pages with a **simulated phone** (`tools/sim_android.py`): device added (token shown once, hashed in the database), presence, SMS and calls live, SMS written in the browser (pending / sent / failed), regenerated token (the old one cut with 4001), deletion |
| `messaging.py` | messaging: empty form refused, asynchronous sending, live statuses (emails to a **simulated SMTP** `tools/sim-smtp.py`, SMS acknowledged by hand by the simulated phone), recipient's inbox without the others' table, restricted pages, failure with no phone and its reason |
| `mcp.py` | MCP server (R11 M1): token created in `/admin/mcp` (form refused without a name or a scope), **value shown once**, real `/mcp` call with that token (`system_status`), 401 without a token or with an unknown one, revocation with confirmation then 401 from the next call, audit trail without the value, live translation |
| `mqtt_outage.py` | recovery after an outage: the dev Mosquitto is **stopped then restarted**; during the outage a command is refused immediately with a message, the server stays reachable; afterwards, without a restart, states and commands resume and the refusal message clears |
| `realtime.py` | CSP enforced (nonce-less script blocked), WebSocket, broadcast, device state simulator, reconnection and resubscription after a server restart |

`tools/e2e.sh --image dominet:dev` (or `make e2e-image`) replays everything **on the Docker image**: the only test that sees the production binary (built
in release mode) together with its WASM files. A server and a WASM compiled differently give an inert interface (see `PROJECT_CONTEXT.md`, "Pièges connus");
only tests that really hydrate the page in a browser catch it.

## Adding a test

- A business rule → unit test in `dominet-domain`. A SQL query → repository test (`dominet-store/tests`).
- A server function or a page → `dominet/tests/ui.rs` (always: anonymous, ordinary account, administrator).
- Behaviour that only exists in the browser (hydration, click, WebSocket, CSP) → a script in `tests/e2e/`, added to the list in `tools/e2e.sh`.
  Forms are also tested without an `input` event (autofill).
