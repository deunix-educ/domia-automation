# Architecture decisions: index by theme

The full records are the ADRs in [`adr/`](adr/README.md) (context, options, decision, consequences). Each exists in English and in French; this page is the map: what was decided in each area, which decisions were **reversed or abandoned** and why, and what was deliberately left for later.

## By theme

### Architecture and build
One binary with roles; server-rendered Leptos with a WebSocket and Fluent translations; server functions under `/sf/`; a generic CRUD component for administration pages; an image built with `cargo-leptos`; a guard that retries a frozen render; browser tests on Marionette/Firefox.

ADR: [0001](adr/0001-global-architecture.md), [0007](adr/0007-leptos-ssr-frontend.md), [0012](adr/0012-interface-architecture.md), [0014](adr/0014-generic-crud.md), [0017](adr/0017-docker-image-and-e2e.md), [0021](adr/0021-page-render-guard.md), [0024](adr/0024-e2e-marionette.md).

### Data and storage
ReductStore for raw history, PostgreSQL for current state and aggregates; one replaced state plus a per-second history; images in the database; secrets encrypted at rest with AES-256-GCM; `pg_dump` and a ReductStore archive for backups.

ADR: [0002](adr/0002-iot-storage.md), [0018](adr/0018-dashboard-schema.md), [0025](adr/0025-device-schema.md), [0028](adr/0028-mqtt-sources-and-raw-archive.md), [0011](adr/0011-secrets-encryption.md), [0054](adr/0054-backups.md).

### Security and access
Server-side sessions with argon2id; a nonce-based CSP; MQTT with one authenticated listener and per-client accounts; hashed, scoped, revocable MCP tokens with an audit trail; camera viewing authorised by DomiNet (viewing token in the browser, personal revocable password for external players). The optional VPN is wg-easy 15 (AGPL-3.0), set up once through its API.

ADR: [0004](adr/0004-auth-sessions.md), [0016](adr/0016-csp-and-websocket-hub.md), [0057](adr/0057-authenticated-mqtt.md), [0059](adr/0059-mcp-access-tokens.md), [0064](adr/0064-authenticated-camera-viewing.md), [0065](adr/0065-wg-easy-15.md).

### Real time and MQTT
`rumqttc`; typed WebSocket topics that keep the last state; a supervised client that publishes at once or refuses; Zigbee2MQTT ingestion; supervised weather and solar publishers.

ADR: [0005](adr/0005-mqtt-client-rumqttc.md), [0019](adr/0019-multiplexed-websocket-protocol.md), [0020](adr/0020-system-statistics.md), [0026](adr/0026-supervised-mqtt-client.md), [0027](adr/0027-zigbee2mqtt-ingestion.md), [0030](adr/0030-weather-and-solarmax-publishers.md).

### Automation, alerts and energy
Sandboxed Rhai; stored in the same JSON shape the scenarios use; one engine with one queue; a single scheduler with solar events; alert evaluation and lifecycle; the daily energy balance; alerts on real channels.

ADR: [0006](adr/0006-rhai-sandbox.md), [0031](adr/0031-automation-schema.md), [0032](adr/0032-automation-engine.md), [0033](adr/0033-scheduler.md), [0034](adr/0034-rhai-scripts.md), [0035](adr/0035-alerts.md), [0036](adr/0036-energy.md), [0042](adr/0042-alerts-on-real-channels.md).

### Messaging and the phone relay
SMTP and SMS channels in a crate of their own; the Android relay keeps its WebSocket contract; hashed device tokens shown once; asynchronous sending with live status.

ADR: [0037](adr/0037-android-relay-and-messaging.md), [0038](adr/0038-delivery-channels.md), [0039](adr/0039-android-relay-server.md), [0040](adr/0040-android-pages.md), [0041](adr/0041-asynchronous-messaging.md).

### Surveillance and video
`ffmpeg` as a supervised subprocess behind a `FrameSource` trait; a minimal hand-written ONVIF client; YOLOv8n through ONNX Runtime; WHEP live view in WASM through MediaMTX; MediaMTX records and the application indexes; pure-Rust motion detection.

ADR: [0003](adr/0003-video-decoding.md), [0008](adr/0008-minimal-onvif-client.md), [0009](adr/0009-yolo-ort.md), [0010](adr/0010-whep-live-wasm.md), [0043](adr/0043-surveillance-schema.md), [0044](adr/0044-mediamtx-and-cameras.md), [0045](adr/0045-onvif-and-discovery.md), [0046](adr/0046-whep-live-view.md), [0047](adr/0047-ptz-and-night-mode.md), [0048](adr/0048-decoded-frames-and-snapshots.md), [0049](adr/0049-recording-and-replay.md), [0050](adr/0050-motion-and-detection.md).

### Observability and operations
A `/metrics` endpoint; Prometheus and Grafana rules stored in the database; relay of external alerts; Compose base with overlays; MkDocs served by the application; the local pre-push gate; platform alerts read-only in the alerts page.

ADR: [0013](adr/0013-self-hosted-ci.md), [0015](adr/0015-local-gate.md), [0051](adr/0051-prometheus-metrics.md), [0052](adr/0052-prometheus-and-grafana-alert-rules.md), [0053](adr/0053-prometheus-alerts-and-grafana-dashboards.md), [0055](adr/0055-compose-base-and-overlays.md), [0056](adr/0056-mkdocs-documentation.md), [0061](adr/0061-platform-alerts-in-the-alerts-page.md).

### MCP and the local assistant
An MCP server built into the binary; scoped tokens; a local model that proposes and an application that decides.

ADR: [0058](adr/0058-mcp-server.md), [0059](adr/0059-mcp-access-tokens.md), [0060](adr/0060-local-assistant.md).

## Decisions reversed, replaced or abandoned

| Decision | What happened | Why |
|---|---|---|
| CI on GitHub Actions (initial) | Replaced by a self-hosted runner ([0013](adr/0013-self-hosted-ci.md)), then by a local gate: the `pre-push` hook reruns `make ci` on a clean copy ([0015](adr/0015-local-gate.md)). The public repository keeps the local gate only | The workflow only reran `make ci`, which the developer already ran on the same machine (also the development and production server): 8 to 10 minutes per push, a dedicated system user, rootless Docker, resource ceilings to maintain, and failures hard to read or reproduce. One developer gains nothing from a per-commit trace |
| Two Mosquitto listeners, one anonymous on the LAN ([0055](adr/0055-compose-base-and-overlays.md)) | One listener with passwords and per-client accounts ([0057](adr/0057-authenticated-mqtt.md)) | The new stack is installed fresh, with no existing MQTT client to spare; an anonymous listener, even disabled by default, reopens the broker to the LAN by one configuration slip |
| Two assistant models, 8B daily and 14B for configuration ([0060](adr/0060-local-assistant.md)) | A single Qwen3-8B | Measured 98 % against 96 %, three times faster |
| An assistant that renames and modifies existing elements | Dropped; the assistant reads, controls and creates | Unreliable with the 8B, and a device's name is also its MQTT topic; administration pages do it better |
| The local assistant phase (R12) | Set aside on 2026-11-18 after trials | Reading, control and creation were reliable; modification was not |
| Migrating `chrono` to `jiff` or `time` | Rejected on 2026-11-20 | `sqlx` 0.9 has no `jiff` support, and `rmcp` and `schemars` keep `chrono` in the tree anyway. Reopen only on a security advisory, `sqlx` `jiff` support, or a need `chrono` cannot meet |
| Leptos 0.9 ([0007](adr/0007-leptos-ssr-frontend.md)) | Stayed on 0.8.x | `leptos-fluent` required below 0.9 at the time; reconsider when 0.9 is stable and supported |
| A pure-Rust or linked H.264 decoder | Rejected: `ffmpeg` subprocess ([0003](adr/0003-video-decoding.md)) | No mature pure-Rust decoder exists (`retina` does not decode); `ffmpeg-next` needs development headers; `openh264` cost about twice the CPU and failed on two camera families. Measured: 4 cameras at 22 % of the machine in software, 8 % with VAAPI |
| Using an OpenCV binding for motion | Rejected: pure-Rust motion ([0050](adr/0050-motion-and-detection.md)) | No OpenCV dependency; the algorithm and OpenCV's own details are reproduced and parity is measured against it on synthetic image pairs |
| An ONVIF crate | Rejected: a minimal hand-written SOAP client ([0008](adr/0008-minimal-onvif-client.md)) | No mature ONVIF crate fits; only a handful of calls are needed, and camera families use different namespace prefixes, so answers are read without relying on them |
| Replacing MediaMTX with an all-Rust video stack | Deferred (optional phase R9) | Only worth it if measurements show a real gain |
| Alert acknowledgement of platform alerts | Not offered: read-only ([0061](adr/0061-platform-alerts-in-the-alerts-page.md)) | Choice of the owner (read-only, one single total in the badge); acknowledgement stays in the tools that own those alerts |

## Decided and not yet done

- An arm64 image was never built ([0017](adr/0017-docker-image-and-e2e.md)); required for the Raspberry Pi 5.
- Short-message sending is deliberately excluded from the MCP tools.
- Voice assistant, all-Rust video (R9) and hardware extensions (R10) are later phases.

## Conventions that cost something to learn

- Any non-obvious decision gets an ADR with the options rejected.
- Never publish with `-C target-cpu=native`; the baseline is `x86-64-v2` ([0001](adr/0001-global-architecture.md)).
- Nothing is included from outside `crates/` (the image copies only that and `.sqlx/`).
- A script can reach the system only through the `administration` scope ([0059](adr/0059-mcp-access-tokens.md)).
