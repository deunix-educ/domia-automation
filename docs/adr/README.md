# Architecture decision records (ADR)

Format: `0000-template.md` (context, options, decision, consequences). A superseded record is not deleted: its status points to the one that replaces it. Every record exists in English (the reference) and in French (`*.fr.md`). For a reading by theme, start with [Decisions by theme](../decisions.md).

| ADR | Subject | Status |
|---|---|---|
| [0001](0001-global-architecture.md) | Global architecture (workspace, single binary, `x86-64-v2` target) | accepted |
| [0002](0002-iot-storage.md) | IoT storage: ReductStore (raw) + PostgreSQL (aggregates) | accepted |
| [0003](0003-video-decoding.md) | Video decoding: `ffmpeg` subprocess behind a `FrameSource` trait | accepted |
| [0004](0004-auth-sessions.md) | Authentication and sessions (own store on `sqlx` 0.9) | accepted |
| [0005](0005-mqtt-client-rumqttc.md) | MQTT client: `rumqttc` | accepted |
| [0006](0006-rhai-sandbox.md) | Automation scripts: sandboxed Rhai host | accepted |
| [0007](0007-leptos-ssr-frontend.md) | Front end: Leptos SSR + hydration, WebSocket, Fluent i18n | accepted |
| [0008](0008-minimal-onvif-client.md) | ONVIF: minimal hand-written SOAP client | accepted |
| [0009](0009-yolo-ort.md) | Object detection: YOLOv8n through `ort` | accepted (reservations) |
| [0010](0010-whep-live-wasm.md) | Camera live view: WHEP client in Rust/WASM towards MediaMTX | accepted |
| [0011](0011-secrets-encryption.md) | Encryption of sensitive values at rest (AES-256-GCM) | accepted |
| [0012](0012-interface-architecture.md) | Interface: Leptos server functions and three crates | accepted |
| [0013](0013-self-hosted-ci.md) | CI on a self-hosted runner | **partly replaced by 0015** |
| [0014](0014-generic-crud.md) | Generic CRUD component for administration pages | accepted |
| [0015](0015-local-gate.md) | Local gate (`pre-push` hook) instead of automatic CI | accepted |
| [0016](0016-csp-and-websocket-hub.md) | Nonce-based CSP and the base WebSocket hub | accepted |
| [0017](0017-docker-image-and-e2e.md) | Docker image with `cargo-leptos` and end-to-end tests on the image | accepted |
| [0018](0018-dashboard-schema.md) | Dashboard schema: overlap exclusion, references without foreign keys | accepted |
| [0019](0019-multiplexed-websocket-protocol.md) | Multiplexed WebSocket protocol: typed topics, last state kept | accepted |
| [0020](0020-system-statistics.md) | System statistics: `sysinfo`, published as the last state of the `system` topic | accepted |
| [0021](0021-page-render-guard.md) | Page render guard: abandon and retry a frozen render (Leptos 0.8.23 defect) | accepted (workaround) |
| [0022](0022-dashboard-read-view.md) | Dashboard read view: class-based placement, sanitised HTML, restricted Grafana iframes | accepted |
| [0023](0023-dashboard-editing.md) | Dashboard editing: administrators only, pointer drag, conditional-field form | accepted |
| [0024](0024-e2e-marionette.md) | Browser end-to-end tests: stay on Marionette/Firefox | accepted |
| [0025](0025-device-schema.md) | Device schema: replaced state, per-second history, images in the database | accepted |
| [0026](0026-supervised-mqtt-client.md) | Supervised MQTT client: immediate or refused publications, re-subscribe on every connection | accepted |
| [0027](0027-zigbee2mqtt-ingestion.md) | Zigbee2MQTT ingestion: same result as the reference implementation, replayed on recorded sessions | accepted |
| [0028](0028-mqtt-sources-and-raw-archive.md) | Dynamic MQTT sources, non-blocking raw ReductStore archive, history retention | accepted |
| [0029](0029-device-commands-and-pages.md) | Validated commands, device pages and images, Grafana dashboard as a file | accepted |
| [0030](0030-weather-and-solarmax-publishers.md) | Supervised weather and SolarMax publishers; protocol checked against reference vectors, inverter never contacted | accepted |
| [0031](0031-automation-schema.md) | Scene, automation, alert and energy schema: reference JSON shape, strong types, deliberate deviations | accepted |
| [0032](0032-automation-engine.md) | Automation engine: single queue, useful events only, batched states always seen, anti-loop 30/min | accepted |
| [0033](0033-scheduler.md) | Scheduler: `scheduler` role, no catch-up, cron semantics of the reference, solar computation checked against `ephem` | accepted |
| [0034](0034-rhai-scripts.md) | Rhai scripts: all or nothing, missing key = `()`, muted source code, other-language code refused, dry run | accepted |
| [0035](0035-alerts.md) | Alerts: cycle of the reference with its quirks (checked on 2,030 evaluations and 8 scenarios), `Notifier` port, `alerts` topic | accepted |
| [0036](0036-energy.md) | Energy balance: same computation as the reference (13 identical days), local day, server-side SVG, `seed-energy` / `aggregate-energy` | accepted |
| [0037](0037-android-relay-and-messaging.md) | Android relay and messaging: hashed token, `sms_outbox` queue (acks), call log, send states | accepted |
| [0038](0038-delivery-channels.md) | Delivery channels: `lettre` (mandatory STARTTLS, plaintext credentials refused off loopback), checked Free Mobile, fake servers, `notify-test` | accepted |
| [0039](0039-android-relay-server.md) | Android relay server side: exact endpoint, one connection per device, 30/90 s heartbeat, 60 s acks, `android:<id>` topic | accepted |
| [0040](0040-android-pages.md) | Android pages: per-account rights, token shown once, live reread, SMS send state, simulated phone for end-to-end tests | accepted |
| [0041](0041-asynchronous-messaging.md) | Messaging: asynchronous background sending, live per-recipient statuses (`message:<id>`), rights, resume after restart | accepted |
| [0042](0042-alerts-on-real-channels.md) | Alerts and automation notifications on real channels: the rule's three channels, failure isolation, `notified` after the attempt | accepted |
| [0043](0043-surveillance-schema.md) | Surveillance schema: seven tables, 32-hex public identifier, encrypted secrets, ports per aggregate | accepted |
| [0044](0044-mediamtx-and-cameras.md) | MediaMTX client (REST only, `replace`, managed paths = 32 hex), camera service (encrypted secrets, non-blocking MediaMTX failure), rights | accepted |
| [0045](0045-onvif-and-discovery.md) | ONVIF (client, service addresses, masked secrets), discovery by `LISTEN`/`NOTIFY` between the `discovery` and `web` roles, add by address | accepted |
| [0046](0046-whep-live-view.md) | Camera live view: Rust/WASM WHEP player, reconnection by `getStats`, no-H.264 message, host-adapted WHEP address, `connect-src` | accepted |
| [0047](0047-ptz-and-night-mode.md) | ONVIF PTZ (reference protocol, automatic stop, presets without orphans), four day/night methods behind `NightControl`, administrator rights | accepted |
| [0048](0048-decoded-frames-and-snapshots.md) | `FrameSource` (one supervised `ffmpeg` per camera, JPEG), `video` role, last frame on disk, capped indexed snapshots | accepted |
| [0049](0049-recording-and-replay.md) | Recording by MediaMTX (fMP4), scan of finished segments, ReductStore archive with labels, replay with `Range`, manual recording | accepted |
| [0050](0050-motion-and-detection.md) | Motion (reference algorithm in pure Rust, OpenCV parity measured), sliding window, YOLO through `ort` (17/17 parity), model not committed (`make fetch-model`), events page | accepted |
| [0051](0051-prometheus-metrics.md) | Prometheus metrics: separate `/metrics` listener (`METRICS_ADDR`), bounded labels | accepted |
| [0052](0052-prometheus-and-grafana-alert-rules.md) | Prometheus and Grafana alert rules in the database, atomic provisioning files, non-fatal reload, seed only on an empty table | accepted |
| [0053](0053-prometheus-alerts-and-grafana-dashboards.md) | Prometheus alert relay (state machine), Grafana dashboards, checks by execution and by real services | accepted |
| [0054](0054-backups.md) | Backups: gzipped `pg_dump` + ReductStore volume archive, SHA-256 sums, single-transaction restore, retention, scheduled, measured | accepted |
| [0055](0055-compose-base-and-overlays.md) | Compose: 8-service base + dev/prod/demo overlays, pinned images, `dominet healthcheck` probe, shared volumes | accepted (anonymous LAN listener replaced by 0057) |
| [0056](0056-mkdocs-documentation.md) | MkDocs documentation built into the image, served on `/docs/` behind the login (`DOCS_ROOT`) | accepted |
| [0057](0057-authenticated-mqtt.md) | Authenticated MQTT: one listener with a password, named accounts per client, rights per topic (`make prod-mqtt-user`) | accepted |
| [0058](0058-mcp-server.md) | MCP server built into the binary: pinned `rmcp`, session-less HTTP, `dominet-mcp` crate, script code in the `administration` scope | accepted |
| [0059](0059-mcp-access-tokens.md) | MCP access tokens: hashed, scoped, revocable, mandatory expiry, audit trail | accepted |
| [0060](0060-local-assistant.md) | Local assistant: `llama-server`, Qwen3 8B and 14B, bench measurements | accepted |
| [0061](0061-platform-alerts-in-the-alerts-page.md) | Prometheus and Grafana alerts in the alerts page: read-only, one badge | accepted |
| [0062](0062-published-images-with-local-build-fallback.md) | Published multi-architecture images, local build as the fallback | accepted |
| [0063](0063-arm64-support-levels.md) | arm64 and Raspberry Pi 5: best-effort support with explicit test status | accepted |

Decisions taken outside ADRs: languages, pinned toolchain, dark theme only, migrations by a separate command.
