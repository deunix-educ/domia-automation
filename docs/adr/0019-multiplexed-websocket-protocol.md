# 0019 — Multiplexed WebSocket protocol: typed topics, last state kept

- Status: accepted
- Date: 2026-11-01

## Context
A base WebSocket hub exists (ADR 0016): one connection per tab, a single topic (`notifications`), messages broadcast to all. Device state must now flow through it (then system statistics, cameras…): the dashboard follows dozens of devices, each interesting only the tabs that display it. A tab that subscribes must also receive the current state at once, without waiting for the device's next message (which may never come for a quiet sensor).

## Options
**Connections.** (a) One WebSocket connection per topic — multiplies connections (limit of 8 per account) and heartbeats. (b) **One multiplexed connection**, the client subscribing and unsubscribing by message — chosen.

**Topics.** (a) Free strings — a typo becomes a phantom topic. (b) **A `Topic` type** in `dominet-proto` (`Notifications`, `System`, `Device(i64)`, `Camera(Uuid)`), with a strict canonical text form (`device:7`, never `device:07` nor `device:+7`) — chosen: server and browser share the same type, an invalid topic is refused at parsing.

**Initial state.** (a) The client queries a REST API on subscription — two paths to keep consistent, and a race window between the answer and the first message. (b) **The hub keeps the last message of each topic** ("retained", as in MQTT) and sends it after `Subscribed` — chosen.

## Decision
- Client → server messages: `subscribe {topic}`, `unsubscribe {topic}`. Server → client: `subscribed {topic}`, `refused {topic, reason}` with `reason` ∈ `invalid` (unreadable topic), `forbidden` (insufficient right), `limit` (128 subscriptions per connection), then the topic's messages (`device_state`, `notification`, …), in a flat `{"type": …}` format.
- **Authorisation per topic** (`authorize` in `web/ws.rs`): any signed-in account reads `notifications`, `system` and `device:*`; `camera:*` is decided with the camera table. The `notifications` subscription is automatic and cannot be removed.
- **Last state**: `Hub::publish_retained` records the message *before* broadcasting it, so a subscriber arriving during the publication receives the snapshot or the message, never neither. Memory is bounded by the number of devices; bursts are grouped per device by the ingestion.
- **Server side**, each connection keeps its subscriptions in a table; each is a task relaying the topic's channel to the connection's mailbox and **aborted on close** (test: subscriptions are released). Subscribing twice has no effect (idempotent); an unknown or malformed client message is ignored, capped at 1 KiB.
- **Browser side**, components subscribe through `Realtime::subscribe(&Topic)` / `unsubscribe`, which **reference-counts**: two widgets on the same device produce one subscription, removed when the last disappears. An effect (`layout.rs`) sends the difference between wanted and sent state (`subscription_diff`). On every (re)connection, on receiving `hello`, the client forgets what it believed subscribed and sends everything again: reconnection after a server restart is therefore transparent (end-to-end test).
- **Simulation tool** (Settings → Device state simulator, administrators): a server function `inject_device_state` that publishes a state under `device:<id>` (a JSON object of at most 4 KB). It serves as the first real consumer of the protocol before MQTT and as a test bench for widgets.

## Consequences
- The dashboard only has to subscribe to the devices it displays; the ingestion publishes with `publish_retained` without touching the protocol.
- One more topic = one `Topic` variant (+ its `Display`/`FromStr`, its canonical-form tests) and a rule in `authorize`.
- Camera stream access remains to be decided; the right to read devices for all accounts is a decision to revisit if per-device rights appear.
- The last state stays in memory: a server restart empties it until the next message (the ingestion reloads it from the database).
- `WsServerMsg` is no longer `Eq` (it carries a `serde_json::Value`); `PartialEq` is enough for the tests.
