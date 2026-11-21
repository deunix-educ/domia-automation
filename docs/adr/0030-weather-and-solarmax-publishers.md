# 0030 — Weather and SolarMax publishers, supervised inside the server

- Status: accepted
- Date: 2026-11-02

## Context
The reference implementation ran two separate processes (a weather publisher and a SolarMax publisher) that read an external source and publish on MQTT; the MQTT-source pipeline turns them into devices. The SolarMax was contacted at a **hard-coded** address, the production inverter's. Project rule: no contact with the inverter without explicit agreement.

## Decisions
- **Server tasks**: a `publishers` role (active by default when `MQTT_HOST` is set) starts **supervised** tasks (`supervise`: a panic restarts the task with a 1 to 60 s pause). It publishes through the `mqtt` role's client if it runs, otherwise through its own (identifier `…-publishers`, no subscription). `--only publishers` allows a separate process. The loops live in `dominet-integrations` and only know the `Publisher` trait: they are tested without a broker.
- **Weather**: the task always runs but only queries the API for **enabled** stations (no station, no request). A cycle goes through the enabled stations; a failing station does not stop the others. The pause is the shortest `refresh_minutes` of the enabled stations (30 s without a station: a database reread, no API request, to see quickly a station created by `dominet seed-weather` in another process) and the task is **woken** when a station is created or modified in the administration (`AppState.stations_changed`). The API address is `OPEN_METEO_URL` (default: the public API): tests and CI talk to a **simulated** API.
- **SolarMax**: the protocol is ported to Rust (`solarmax::protocol`, pure) and checked **against frozen vectors from the reference**. Byte-identical requests, same scaling, same rounding (half-even: `round(115.25, 1)` is 115.2). The asynchronous TCP client is time-bounded, drops the connection at the slightest error and resumes at the next cycle. An inverter alarm (`SAL` ≠ 0) is an error: nothing is published. The publication topic is a setting (`SOLARMAX_TOPIC`).
- **No default inverter address.** Without `SOLARMAX_HOST` there is no SolarMax publisher and `dominet publish solarmax` refuses to run. Tests use a **simulated inverter** (a TCP server on `127.0.0.1`).
- **Diagnostic**: `dominet publish weather|solarmax [--print-only]` runs **one** cycle, prints the result and publishes (unless `--print-only`).
- **Administration**: `/admin/weather-stations` (generic CRUD, decimal comma accepted, all form problems reported together). `dominet seed-weather [--lat --lon …]` creates the "Weather" source (`meteo/#`, automatic devices), the `meteo_locale` device and the station.
- **`dominet dump-context`**: a YAML snapshot of active devices (type, state schema deduced from current values, numeric keys, command topic, place) and reference schemas. The YAML is written by an emitter of a few dozen lines (no maintained YAML library needed): strings always quoted, reserved keys quoted.
- `serde_json` switches to `preserve_order`: the key order of a state is the database's.

## Deviations from the reference
1. No fake initial state for the weather device: it waits for the first real reading.
2. The weather device's manufacturer and model are not filled in: a hand-created device does not carry these discovery fields.
3. The weather topic is `meteo/<suffix>`.
4. A publication no longer creates an MQTT connection per message (the reference connected, published and disconnected at every cycle): the server's client stays connected.

## Consequences
- A broker outage interrupts publications (refused at once, logged); they resume at the next cycle without intervention.
- A second `--only publishers` process and another `mqtt` role must not run at the same time with the same client identifier (the `-publishers` suffix avoids it).
- Real contact with the inverter and with the Open-Meteo API remains reserved to explicit demonstration with agreement.
