# Rebuilding from scratch

The order in which to build the application, what to write first, which traps to read before each step and how to know a step is done. It assumes you have read [Pitfalls and lessons](pitfalls.md), [Measurements](measures.md) and [Decisions by theme](decisions.md); each step below points at the parts that matter.

## Principles

- **One phase at a time, then stop and check.** A phase ends with its definition of done and a demonstration; the next one does not start before that is accepted.
- **Write the reference data first.** Recorded device sessions, expected alert and schedule results, protocol vectors and camera answers are cheap to produce early and expensive to guess later. Tests that compare against such files caught most of the subtle defects (see the pitfalls page, domain logic).
- **A throw-away spike before any architecture** for every risky technique, with a written verdict (go, go with reservations, no-go with the alternative) and an ADR.
- **Run the whole gate often.** `make ci` (format, clippy with warnings as errors, tests, WASM build, `cargo deny`, `cargo audit`, SQL metadata check) before every commit; `make ci-full` (adds the Docker image and the browser tests on it) before a milestone or when the Dockerfile or a native dependency changes.
- **Rehearse on the real machine** as early as the phase allows: decoding, certificates, mail and disk problems appear nowhere else.
- No secret, real address or personal image in the repository, logs, dumps or fixtures.

## Prerequisites

Rust 1.99.1 (edition 2024, pinned in `rust-toolchain.toml`) with the `wasm32-unknown-unknown` target; `cargo-leptos` 0.3.11, `sqlx-cli`, `cargo-deny`, `cargo-audit`; Docker with Compose; `ffmpeg` with the VAAPI driver on the target; a browser driver for the end-to-end tests. Build with a baseline CPU level (`x86-64-v2`), never `target-cpu=native`. Keep `.cargo/config.toml` as it is: it makes the server and the WASM compile alike.

## Workspace

One Cargo workspace, one binary with roles. Crates by responsibility, from the inside out:

| Crate | Role |
|---|---|
| `dominet-proto` | data types shared by the server and the browser |
| `dominet-domain` | pure rules and validation, no I/O |
| `dominet-store` | PostgreSQL repositories and migrations |
| `dominet-mqtt` | the supervised MQTT client |
| `dominet-automation` | automation engine, scheduler, script host, alerts, energy balance |
| `dominet-notify` | e-mail and SMS channels |
| `dominet-integrations` | weather and solar-inverter publishers |
| `dominet-surveillance` | cameras, ONVIF, MediaMTX client, motion, detection, frames |
| `dominet-server` | HTTP and WebSocket server, services shared by the interface and MCP |
| `dominet-web` | the Leptos interface |
| `dominet-mcp` | the MCP server |
| `dominet-agent` | the local assistant loop |
| `dominet` | the binary: sub-commands and roles (`web`, `mqtt`, `scheduler`, `video`, `publishers`, `discovery`) |

## Build order

| # | Phase | What to build | Read first | Done when |
|---|---|---|---|---|
| 0 | Spikes | Eight throw-away experiments: server-rendered interface with WebSocket and translations; raw time-series store at one million records; WHEP live view in WASM; H.264 decoding cost; object detection latency; ONVIF against real cameras; sandboxed scripts; MQTT client under load | Measurements §1; decisions on video | each spike has a verdict and an ADR |
| 1 | Foundations | Typed configuration from environment variables, tracing, health endpoints, migrations, login with server-side sessions, hashed passwords and lockout, typed settings with masked secrets, seed data, the interface shell (layout, dark theme, navigation, language switch, generic CRUD), administration pages | Pitfalls §1 to §4 | login works, CRUD pages work, authentication tests pass (including lockout and races), the image builds and starts |
| 2 | Real time and dashboard | WebSocket hub with typed topics that keep the last state; dashboard and widgets with drag and drop and persistence; system statistics | Pitfalls §2; decision on the frozen-render guard | a message updates a widget without reload; browser tests green |
| 3 | MQTT and devices | Supervised listener, Zigbee2MQTT ingestion, dynamic MQTT sources, history in the database and the raw store, device commands, pairing, groups and labels, weather and solar publishers, context dump | Pitfalls §5 | a recorded session replays into the same devices, states and history rows; broker outage tested; sustained throughput validated |
| 4 | Automations, scenes, alerts, energy | Engine (message, schedule, solar and device-state triggers; publish, script, notification and scene actions), in-process scheduler, script host with limits, scenes, alert rules with lifecycle and acknowledgement, daily energy balance | Pitfalls §5, §7; decisions on automation | real scenarios run identically; cooldowns hold; a hostile script is stopped without harming the process |
| 5 | Notifications, messaging, phone relay | SMTP and SMS channels, internal messaging, the Android relay with its frozen WebSocket contract | Decisions on messaging | the unmodified phone app connects, sends and receives, acknowledges; contract tests pass |
| 6 | Surveillance | Cameras and MediaMTX path sync, live view, ONVIF discovery and PTZ, night-mode backends, recording and replay, snapshots, motion detection, object detection, the frame-source abstraction | Pitfalls §6; Measurements §2 | live under 1 s on the LAN; one hour of continuous recording with no lost segment; detection matches reference; CPU and memory measured on the target |
| 7 | Observability and operations | Metrics endpoint, Prometheus and Grafana rules and dashboards, backups with tested restore, Compose base and overlays, reverse proxy with TLS, Makefile, documentation site, deployment guide | Pitfalls §8, §9 | a blank machine installs in three commands; a restore is validated |
| 8 | Acceptance and production | Parity checklist run on the real machine, load tests, blank-install rehearsal, switch-over procedure, 48-hour watch | Measurements §3, §4; Pitfalls §9 | checklist validated by the owner, installation reproducible, rollback rehearsed |
| 9 | MCP server | Server built into the binary, scoped hashed tokens, audit written before execution, read then control then configuration then administration tools, a shared configuration service used by the interface too | Pitfalls §11; decisions on MCP | every tool has a dry run where it changes something; scope matrix tested |
| 10 | Local assistant | Agent loop calling the MCP tools in memory and a self-hosted model; action cards confirmed by the user; snapshot, undo and audit | Pitfalls §11; Measurements §5 | evaluation set passes its threshold with no wrong card |
| 11 | Platform alerts | Read-only view of Prometheus and Grafana alerts in the alerts page and the badge | Pitfalls §8 | both sources, unreachable-source state and the badge covered by tests |

Dependencies: 0 to 1 to 2 to 3 to 4 to 7 to 8 is the critical path. 5 and 6 can run beside 3 and 4 once 2 is done. 9 to 11 come after 8.

## Definition of done for every phase

- Every feature is delivered and reachable from the interface.
- Unit, integration and contract tests pass; critical paths are covered.
- Format, lints as errors, `cargo deny` and `cargo audit` are clean.
- The Docker image builds and answers its health check.
- Documentation (both languages) and ADRs are updated.
- No sensitive data in logs, dumps or fixtures.
- A step-by-step demonstration is written, then you wait for validation.

## Risks to keep in view

| Risk | Mitigation |
|---|---|
| No mature pure-Rust H.264 decoder | `ffmpeg` subprocess behind a frame-source trait; revisit only with measurements |
| The raw store is not a time-series database | aggregates in PostgreSQL, raw history in the store |
| Replacing a generated admin site (about thirty CRUD screens) | one generic CRUD component from phase 1 |
| WebRTC and drag and drop in WASM | minimal, isolated JavaScript interop; spike first |
| ONVIF variety across camera families | minimal client, real-camera tests, recorded SOAP fixtures |
| ARM images | build natively on an ARM machine, not by emulation |
| Scope for one developer | short phases, parity per module, optional phases kept optional |
