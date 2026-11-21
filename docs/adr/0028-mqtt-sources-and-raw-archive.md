# 0028 — MQTT sources, history and the raw ReductStore archive

- Status: accepted
- Date: 2026-11-02

## Context
Zigbee2MQTT ingestion listens to `zigbee2mqtt/#`. The reference implementation also listens to every active `MqttSource` (Linky, SolarMax, weather…), can create a device at every message (`auto_device`), records tracked keys in history (PostgreSQL) and — decision of ADR 0002 — raw IoT must also go to ReductStore. This work adds these three things and the administration page for sources.

## Decisions
- **Dynamic sources.** Ingestion keeps the list of active sources (`Ingestor::set_sources`); a message outside the Zigbee2MQTT prefix is matched against each source's filter (`topic_matches`, MQTT 3.1.1). Modifying a source in the administration notifies the listener (`AppState.sources_changed`, an in-process `Notify`) which rereads the sources, updates the routing **and** the subscriptions (difference only: `set_subscriptions`). A safety reload every 60 s covers the case where administration and listener run in two processes (`--only web`, `--only mqtt`). A filter declared twice (or equal to `<prefix>/#`) is subscribed once, at the highest QoS.
- **Behaviour taken from the reference**: for each matching source — a log entry if `log_messages` (topic and beginning of the payload, 512 characters), an automatic device if `auto_device` **and** the payload is a JSON object; then, if at least one source matched, active devices whose `base_topic/name` or `base_topic` equals the topic are marked seen. The automatic device takes the topic without the source's prefix as its name (otherwise the whole topic), replaces its state, is **reactivated** if it was disabled, is broadcast in real time and recorded in history like a Zigbee2MQTT device.
- **`handler_code`** is kept, editable and displayed in the administration; it is run by the Rhai host (ADR 0006).
- **Raw ReductStore archive (`iot`)**, optional (`REDUCTSTORE_URL`). One record per message that writes at least one history row: body = the full JSON state, one label per numeric tracked key (booleans are 0/1), entry `dev_<id>`, timestamp in µs. The bucket name is `iot` (configurable: `REDUCTSTORE_IOT_BUCKET`, for tests), FIFO quota `IOT_QUOTA_GB` (default 5) applied **at bucket creation**. Writing goes through a **bounded queue (10,000)** and a task writing in batches of 50 (HTTP header limit found by the spike): ingestion never waits for ReductStore. Server unreachable: the bucket is recreated on the next attempt, records that do not go through are **dropped and counted** (a warning at the first outage, a message at recovery). Two messages of the same device in the same microsecond get shifted timestamps (ReductStore refuses duplicates).
- **PostgreSQL stays the source of charts.** `device_state_log` is purged once a day beyond `HISTORY_RETENTION_DAYS` (default **400**; ADR 0002 proposed 30 days provisionally). Environment variables rather than application settings: like `REDUCTSTORE_URL`, they describe the deployment, not the house's behaviour.
- **`/admin/mqtt-sources` page** (administrators): the generic CRUD component (ADR 0014), typed `list/save/delete` server functions, field-by-field validation by the domain (`MqttSource::problems`: a valid MQTT filter, QoS 0 to 2, lengths), prefix stripped of its trailing `/`.

## Deviations from the reference
1. **State at creation of the automatic device**: the reference writes `state`, `available` and `last_seen` in `defaults`, with an upsert whose defaults only apply at creation: a source's first message leaves **no state** in the database (it is only broadcast). Here the state is written at once (same defect as the one fixed in ADR 0027).
2. **Invalid names ignored**: a topic that would make an invalid device name (more than 128 characters, wildcard, control character) is ignored instead of causing a database error.
3. **No "retry once"** on database connection loss: the connection pool reconnects, a lost message is resent by the broker's QoS 1 as long as the session persists.

## Consequences
- During a ReductStore outage, PostgreSQL history continues; only the raw archive has a gap (counted in the log, in a metric later).
- Changing `IOT_QUOTA_GB` does not act on an existing bucket (set it in ReductStore or delete the bucket): a documented limit.
- The weather and SolarMax publishers publish on the broker: their topics are declared like any source (`meteo/#`, `solarmax/#`); nothing special on the ingestion side.
- ReductStore's HTTP client (rustls) cannot be built without root certificates, and `reduct-rs` panicked then: the Docker image installs `ca-certificates`, and the server uses `try_build` — on failure it starts **without an archive** (warning) instead of stopping. Discovered by the end-to-end test on the image (`make ci-full`), not by host tests.
