# 0005 — MQTT client: rumqttc

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go**

## Context
The integration bus is MQTT (Zigbee2MQTT, Linky, SolarMax, weather). A reliable Rust client is needed for `dominet-mqtt`: automatic reconnection, QoS 1, `+`/`#` wildcards, throughput well above the real load (about 50 devices at 1 Hz, i.e. a few tens of msg/s). Spike: `rumqttc` 0.25.1, development Mosquitto 2.0, Intel Core i5-6500.

## Measurements
| Test | Result |
|---|---|
| Topic matching, 19 cases (`zigbee2mqtt/#`, `+/availability`, `solarmax/+/+`, `$SYS`, empty levels…) | own implementation: 19/19; `rumqttc::matches` diverges on `$SYS/#` (harmless here) |
| 50 devices × 1 Hz, QoS 1, 10 s | 500/500, no loss |
| 2,000 msg/s, QoS 1, 10 s | 20,000/20,000, no loss |
| Maximum burst QoS 1 (43,000 msg/s sent) | 49,251/50,000 received (749 lost), 33,800 msg/s received |
| Maximum burst QoS 0 (285,000 msg/s sent) | 50,000/50,000 received |
| Broker down 5 s during 100 msg/s QoS 1 (persistent sessions) | 3,000 sent, **0 missing**, 1 duplicate; reconnection 0.6 s after the broker returned; re-subscribe on every `ConnAck` |

## Decision
- `dominet-mqtt` is built on **`rumqttc`** (QoS 1 by default, `clean_session = false`, stable client identifier).
- Supervised event loop: on error, wait 250 ms then `poll()` again (automatic reconnection); **re-subscribe on every `ConnAck`** so as not to depend on the broker-side session.
- `topic_matches` is implemented in `dominet-mqtt` with its case table as unit tests; `rumqttc::matches` is not used.
- Handle QoS 1 duplicates: consumers (device state, history) must be idempotent (per-second deduplication is already planned for `device_state_log`).

## Consequences
- At a heavy burst (≫ 1,000 msg/s) the broker may drop QoS 1 messages (Mosquitto's per-client queue): irrelevant at the real rate, to watch in load tests.
- Pitfalls noted: detect inactivity on received `Publish` packets, not on `poll()` (`PingResp` restarts a naive timeout); `docker compose exec` needs `</dev/null` inside a `while read` loop.
