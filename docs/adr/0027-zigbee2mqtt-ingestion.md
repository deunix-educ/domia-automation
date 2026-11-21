# 0027 — Zigbee2MQTT ingestion: same result as the reference implementation, checked on recorded sessions

- Status: accepted
- Date: 2026-11-02

## Context
The acceptance criterion is a replay: the same MQTT messages must produce the same devices, states and history rows as the reference implementation. Reading its code to reproduce it is not enough (a quirk of the flattening of `exposes` had already been found). The reference implementation had to be **executed** on the same messages and compared.

## Decisions
- **Frozen reference result**: the reference message handler was run once on a message file, on a throw-away PostgreSQL, with a virtual clock (the time comes from the file) so that the per-second history is reproducible. The result (devices and history, as JSON) is versioned next to the message file (`session-r3.golden.json`): the Rust tests need neither Docker nor the reference. The scripts that produced it are not kept; the file is.
- **Replay session** (`tests/fixtures/z2m/session-r3.jsonl`, 64 lines): 11 devices of varied shapes (sensor, light, blind, sockets, contact, presence, unsupported device, a name containing a `/` or spaces), coordinator, re-discovery with changed metadata, added device, states repeated within the same second or not, booleans, texts, nested objects, `null`, accented characters, availabilities in all their forms, unknown device, device deactivated then reactivated, noise to ignore (`/set`, `/get`, `bridge/*`, non-JSON or non-object payloads).
- **`dominet-automation::ingest` module**, generic over the storage ports and over a `Sink` (broadcast; later automations and alerts): it knows neither PostgreSQL nor the hub, so it can be replayed on a file. Routing (`route`) follows the **order of the reference**: `bridge/devices`, `bridge/info` (object), `…/availability` **before** the exclusion of `bridge/…` topics, then `…/set`/`…/get`, then the state.
- **One ingestion task, in batches**: messages are taken in batches of 256; a device state is ignored if the **next message about the same device** is also a state received within the same second (`bridge/devices` breaks any grouping). The final result is identical (replaced state, per-second history where the last wins; checked by the "one-second batches" replay), the intermediate broadcast is lost. Outside bursts each batch holds one message: no grouping. Measured: 4,000 messages (50 devices × 80) in 1.7 s, 832 broadcasts. **Consequence for automations**: under a burst, a "device state" trigger only sees the last state of each device per second (see ADR 0032).
- **Server sink** (`HubSink`): every change is published on `device:<id>` with **last state retained** (ADR 0019); at start-up, the known database state of every active device is republished, so widgets show it even after a restart. `permit_join` is remembered in `AppState`.
- **Zigbee2MQTT prefix = the `Z2M_BASE_TOPIC` setting**: changing it in the interface changes the subscription (`<prefix>/#`, QoS 1) and the routing live, without a restart. `upsert_discovered` receives this prefix.

## Deviations from the reference
1. **Metadata at creation (fixed)**: the reference puts `ieee`, `vendor`, `model_id`, `description` and `exposes` in `defaults`, but an upsert whose defaults only apply at creation: a discovered device stays without metadata until the next `bridge/devices` republication. Here they are written immediately. The replay test does not hide the gap: it requires the device to carry exactly the metadata of the last discovery in the file.
2. **Invalid names ignored**: a discovery whose name contains an MQTT wildcard or a control character is ignored (the reference would accept it); texts longer than their column are truncated (the reference would fail with a database error).
3. **Quirk kept**: only `composite` elements are flattened to deduce the type (ADR 0025) — the reference result confirms it (a classic Zigbee2MQTT lamp stays `generic`).
4. Messages outside the Zigbee2MQTT prefix are handled by the generic sources (ADR 0028).

## Consequences
- Every extension of the replay (`auto_device` sources) uses the same mechanism: a message file, a reference result, a comparison.
- The Rust tests compare with the versioned file and need nothing else.
