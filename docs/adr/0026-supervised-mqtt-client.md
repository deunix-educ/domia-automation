# 0026 — Supervised MQTT client: immediate or refused publications, re-subscribe on every connection

- Status: accepted
- Date: 2026-11-02

## Context
The application listens to Zigbee2MQTT and MQTT sources, and publishes device commands, weather readings and SolarMax values. The client spike (ADR 0005) validated `rumqttc`. What remains is the client's long-term behaviour: what to do with a command when the broker is unreachable, how to resume after a drop, how to change subscriptions live, how to stop cleanly.

## Decisions
- **`dominet-mqtt` crate**, with no dependency on the server: settings (`MqttSettings`, password masked in `Debug`), client (`start`), payload decoding, topic matching. The server adapts its configuration (`mqtt::settings`). `rumqttc` without default features: clear TCP (a local-network broker, anonymous or with a password); TLS will be added when a remote broker needs it.
- **One task supervises the connection**: `poll()` in a loop, recovery after an error with a growing delay (250 ms to 5 s), **re-subscription of the whole wanted set on every `ConnAck`** (nothing is assumed about the broker session, ADR 0005), state exposed through a `watch` channel (`Connecting`, `Connected`, `Disconnected`). Persistent session (`clean_session = false`) and a stable client identifier.
- **A publication fails at once if the link is down** (`PublishError::NotConnected`) instead of being queued. A light or blind command executed ten minutes after the click is worse than a displayed error. A message accepted by the API is delivered **at least once** (QoS 1, resent after reconnection); what the broker had accepted without forwarding survives its stop only if it persists its data.
- **Wanted subscriptions**: a `filter → QoS` set replaced as a block (`set_subscriptions`); the task applies the difference at once if the link is up (`diff_subscriptions`, pure and tested), otherwise at the next connection. A QoS change re-subscribes. MQTT sources modified in the interface thus apply without a restart.
- **Received messages decoded**: UTF-8 (otherwise Latin-1), JSON if possible, otherwise text. An input queue of 8,192 messages: when full, reading from the network waits (the broker slows down, nothing is lost).
- **Stop**: `shutdown()` (a level, not an event: a pause in progress does not absorb it), clean disconnection, end of the task. If nobody consumes the messages or holds the handle any more, the task stops.
- **Roles**: `dominet serve` starts `web` and, if `MQTT_HOST` is set, `mqtt`; `--only mqtt` runs the client alone (the process then waits for the stop signal). Default identifier `dominet` (`MQTT_CLIENT_ID`).
- **Tests**: unit (matching, decoding, subscription difference, publication topics, masked password); integration on the development Mosquitto (round trip, retained messages, live subscriptions, stop) and on a **throw-away Mosquitto** (a named, persistent container removed at the end) for drops: the shared broker is never stopped.

## Consequences
- During an outage, commands are refused and the publishers' readings are lost for the current cycle; they resume at the next cycle. Device states come back from the broker (Zigbee2MQTT retained messages) at reconnection.
- Drop tests need Docker (already required by the development stack) and the `eclipse-mosquitto:2.0` image.
- QoS 1 messages may arrive twice: ingestion is idempotent (ADR 0025).
