# 0025 — Device schema: replaced state, per-second history, images in the database

- Status: accepted
- Date: 2026-11-02

## Context
Devices (Zigbee2MQTT, generic MQTT sources), their history, images, MQTT sources and weather stations are introduced. The acceptance criterion is a **replay**: the same MQTT messages must give the same `Device`s, states and history rows as the reference implementation. The schema and domain rules therefore take over its semantics, quirks included, while taking advantage of PostgreSQL.

## Decisions
- **Tables**: `device_group`, `device_label`, `device`, `device_image`, `device_state_log`, `mqtt_source`, `weather_station`. `BIGINT` identity primary keys; `friendly_name` unique and case-sensitive.
- **State replaced, not merged**: `device.state` (a JSON object) is **replaced** at every message, `last_seen = now()`, `available = true`; availability (`…/availability`) does not touch `last_seen`. The MQTT listener's operations are repository methods (`apply_state`, `set_available`, `upsert_discovered`, `upsert_auto`, `touch_by_topic`): one SQL statement each, hence **idempotent** (the QoS 1 duplicates of ADR 0005 have no effect but `last_seen`).
- **Key lists as `TEXT[]`** (`tracked_keys`, `hidden_keys`) rather than JSON: the type guarantees the shape (a list of texts); `exposes` and `state` stay `JSONB`.
- **History**: `device_state_log(device_id, key, ts, value_num, value_str)` per second, `UNIQUE(device_id, key, ts)`, batch writes (`UNNEST`) with **replacement** of an existing row of the same instant. An index `(device_id, key, ts DESC)` for charts and `(ts)` for purging (configurable retention).
- **Discovery** (`bridge/devices`): creation with the `zigbee2mqtt` prefix, the hidden key `update` and a type deduced from `exposes`; an existing device **keeps** its type, tracked keys, icon and state (only the discovered metadata is refreshed). **Quirk of the reference kept**: only `composite` elements of `exposes` are flattened to deduce the type, so a Zigbee2MQTT `light`, `switch` or `cover` (which carry `features` but are not `composite`) stay `generic`. The replay against the reference checks this equivalence; fixing the deduction is an improvement to decide separately. Units, on the other hand, flatten all `features`.
- **`auto_device` sources**: the device is created or updated **and reactivated** (`active = true`), without its type changing.
- **Images**: a `device_image` table (reduced JPEG, 5 MiB at most, types checked) served by an `ETag` route: no volume to back up separately. `image_url` (an external URL) remains possible.
- **Names and topics**: a device name is used to build topics; the domain refuses control characters, MQTT wildcards (`+`, `#`), leading or trailing `/` and `//` (`is_valid_friendly_name`). A discovery with an invalid name is ignored rather than failing. A subscription filter (`mqtt_source.topic_pattern`) must be a valid MQTT filter (`+` on a whole level, `#` last level).
- **Foreign key `widget.device_id`** (announced by ADR 0018): `ON DELETE SET NULL`; orphan references are set to `NULL` before adding the constraint. A reference to a non-existent device is refused when saving the widget.
- **`handler_code`** of sources: kept as is (≤ 20,000 characters) and displayed, executed by Rhai; code in another language is not portable.
- **Labels**: `KNOWN_LABELS` (25 keys) is in the domain; `dominet seed-labels [--dry-run]` creates the missing labels of keys present on devices, never overwriting.

## Consequences
- 21 tables; the end-to-end scripts that used fictitious device numbers now create their devices.
- The replay against the reference result settles any semantic gap; the type deduction of `light`/`switch`/`cover` is the first quirk to watch.
- No production data migration here.
