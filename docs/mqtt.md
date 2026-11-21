# MQTT

The server's supervised MQTT client (R3). Decisions: [ADR 0005](adr/0005-mqtt-client-rumqttc.md) (choice of `rumqttc`) and [ADR 0026](adr/0026-supervised-mqtt-client.md) (behaviour), both in French.

## Configuration
`MQTT_HOST` (required to enable the client), `MQTT_PORT` (1883), `MQTT_USER`, `MQTT_PASSWORD`, `MQTT_CLIENT_ID` (`dominet`): see [configuration](configuration.md). The password never appears in logs.

## Broker (production Mosquitto)
A single listener, **no anonymous access** ([ADR 0057](adr/0057-authenticated-mqtt.md)). DomiNet and Zigbee2MQTT use the internal account (`MQTT_USER` / `MQTT_PASSWORD`). Each LAN client (Linky, an external SolarMax, Tasmota…) has its own account and rights: `make prod-mqtt-user NAME=linky [TOPIC=linky/#] [ACCESS=read|write|readwrite]` (generated password, shown once), `make prod-mqtt-users`, `make prod-mqtt-del NAME=linky`. No restart: connected clients are not cut. In the client: host `LAN_DOMAIN`, port 1883, user and password; it only writes under its own topic. A refusal shows in the Mosquitto log (`not authorised`).

**Per-client queue**: `max_queued_messages 100000` (production and development). Mosquitto keeps only 1,000 by default and then **drops** the QoS 1 messages of a subscriber slower than the flow (log: `Outgoing messages are being dropped for client …`). DomiNet deliberately slows its reading when loaded: this queue absorbs the burst. Found on 2026-11-13: the throughput test lost the final states on a loaded machine.

## Running
- `dominet serve`: web **and** the MQTT client when `MQTT_HOST` is set. `dominet serve --only mqtt`: the client alone. `--only web`: without MQTT.
- Log: `MQTT connecte` / `MQTT deconnecte, reconnexion en cours` (one line per state change, not one per attempt).

## Behaviour to know
- Automatic reconnection (250 ms up to 5 s); **all subscriptions are resumed on every connection**.
- A **publication is refused immediately** when the broker is unreachable (`NotConnected`): no queue of stale commands.
- Subscriptions change live (`MqttClient::set_subscriptions`), without a restart.
- Payloads are decoded as UTF-8 (else Latin-1), as JSON when possible, otherwise as text.

## Code
`crates/dominet-mqtt` (`start`, `MqttClient`, `topic_matches`, `decode_payload`); `crates/dominet-server/src/mqtt.rs` (configuration adapter, role).

## Tests
`cargo test -p dominet-mqtt`: unit, then integration on the dev Mosquitto (`make dev-up`, port 11883, `MQTT_TEST_PORT` to change it) and on a **throwaway** Mosquitto for outages (Docker required, image `eclipse-mosquitto:2.0`):
no accepted message is lost when the broker stops for 3 s and comes back, publications are refused during the outage, a broker that appears after the client is joined with the subscriptions already requested.

## Zigbee2MQTT device ingestion (lot 3, ADR 0027)
- The client subscribes to `<prefix>/#` (QoS 1). The **prefix** is the `Z2M_BASE_TOPIC` setting (`zigbee2mqtt` by default); changing it in Settings changes the subscription live.
- `<prefix>/bridge/devices`: devices are created or updated (type inferred from `exposes`, hidden key `update`); `<prefix>/bridge/info`: `permit_join`; `<prefix>/<name>/availability`: availability; `<prefix>/<name>`: state (JSON object) that **replaces** the active device's state, updates `last_seen`,
  feeds the tracked-key history and is **broadcast live** (topic `device:<id>`, last state kept). `bridge/…`, `…/set`, `…/get` topics and non-JSON or non-object payloads are ignored.
- `<prefix>/bridge/logging`: **errors reported by Zigbee2MQTT** (`error` level only). A published command always succeeds from the broker's point of view; if Zigbee2MQTT cannot deliver it (unreachable device, ZCL error), it says so here: `Publish 'set' 'state' to 'Volet-salon' failed…`. DomiNet writes the line to its log (warning "erreur signalee par Zigbee2MQTT", visible in `/admin/logs` and counted by `make prod-report`) and shows a **banner** to connected accounts; the same error is not shown again within 60 s. The automation or scene that sent the command still counts as successful: the error comes afterwards and Zigbee2MQTT does not say which command it belongs to.
- At startup the last known state of every active device is republished to the widgets.
- Messages that pile up (bursts) are processed in batches: a device's state is written at most once per second within a batch (the last one wins).

### Replay of recorded sessions
`tests/fixtures/z2m/session-r3.jsonl` is a message session; its expected result `session-r3.golden.json` is a frozen reference file. The Rust tests (`cargo test -p dominet-automation --test replay`) replay the same session and require the same result — except for one documented deviation (ADR 0027: metadata written at creation).
To add a case: extend the `.jsonl` file and the expected result in the `.golden.json` file, checking it by hand.

## MQTT sources, history and raw archive (lot 4, ADR 0028)
- **Sources**: administration page `/admin/mqtt-sources` (administrators). Each active source adds its **filter** (`linky/#`, `solarmax/+/prod`…) and QoS to the subscriptions; adding, editing or deleting one takes effect **live**, without a restart
  (a safety reload also happens every 60 s).
- A message outside the Zigbee2MQTT prefix whose topic matches a source: **logged** if "Log messages" is set (topic and start of the payload); **automatic device** if that box is checked and the payload is a JSON object (name = topic without the source prefix, chosen type, state replaced,
  device reactivated if it was deactivated, broadcast live, recorded in the history); devices whose `prefix/name` is that topic are marked as seen. The source's **handler code** is a Rhai script run on every message (see [scripts.md](scripts.md)); Python code is refused when saving.
- **Tick "Automatic device" when creating the source.** A device that publishes a **retained** message (Linky's `report`, for instance) only delivers it on subscription: if the box is ticked later, the message has already gone by and the device only appears at the next message or the next resubscription (`dominet` restart, broker reconnection).
- **History**: PostgreSQL (`device_state_log`, one row per tracked key per second) feeds the charts; daily purge beyond `HISTORY_RETENTION_DAYS` (400 days by default).
- **Raw archive**: when `REDUCTSTORE_URL` is set, every message that writes a history row adds one record to the `iot` bucket (entry `dev_<id>`, body = full state, one label per numeric tracked key), FIFO quota `IOT_QUOTA_GB` (5 GB by default, applied when the bucket is created).
  It never slows ingestion down: bounded queue, batches of 50 written by a background task; ReductStore unreachable → records are dropped (warning in the log) and writing resumes on its own.

| Variable | Default | Role |
|---|---|---|
| `HISTORY_RETENTION_DAYS` | `400` | maximum age of `device_state_log` rows |
| `IOT_QUOTA_GB` | `5` | FIFO quota of the `iot` bucket (at creation) |
| `REDUCTSTORE_IOT_BUCKET` | `iot` | bucket name (trials on a shared stack) |

Tests: `cargo test -p dominet-automation --test sources` (sources and automatic devices), `-p dominet-store --test reduct` and `-p dominet-server --test mqtt_role` (ReductStore and dev broker), E2E `mqtt` (archive) and `mqtt_sources` (administration page).

### Replay of sources
`tests/fixtures/mqtt/sources-r3.jsonl` (+ `.golden.json`) replays **MQTT sources** (automatic devices, logging, "seen" marking, reactivation, inactive sources): `cargo test -p dominet-automation --test replay_sources`. A single, deliberate deviation: the first message of an automatic device does not write its state (ADR 0028).
Throughput: `cargo test --release -p dominet-server --test throughput -- --nocapture`.

