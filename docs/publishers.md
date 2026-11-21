# Publishers (weather, SolarMax) and context

Integrations that read an external source and **publish to MQTT** (R3 lot 6). Decisions: [ADR 0030](adr/0030-weather-and-solarmax-publishers.md). The messages become devices through the [MQTT sources](mqtt.md) (`meteo/#`, `solarmax/#`).

## Running
- `dominet serve` runs the `publishers` role when `MQTT_HOST` is set; `--only publishers` for a separate process (MQTT client `…-publishers`). Tasks are supervised (restarted after a panic).
- **Weather**: enabled stations of `/admin/weather-stations` (Open-Meteo, no key). Without an enabled station no request is made (the database is re-read every 30 s). Creating or editing a station triggers an immediate reading; then one reading every `refresh_minutes` (the shortest among stations). Published on `meteo/<suffix>`.
  `dominet seed-weather --lat 43.3 --lon 5.4` creates the "Météo" source, the `meteo_locale` device (tracked keys: temperature, humidity, precipitation, wind) and the station.
- **SolarMax**: only if `SOLARMAX_HOST` is set — **it has no default value** (the production inverter is never contacted by mistake). One cycle every `SOLARMAX_INTERVAL_SECS` s (10): detection, values (`pac`, `eac`, `pdc`, `edc`, `qdy`, `qt0`, `tmpr`, `status`), status and alarms;
  an inverter alarm is not published. Published on `SOLARMAX_TOPIC` (default `solarmax/0x6f0930a64c8b8803/prod`, the default topic).

| Variable | Default | Role |
|---|---|---|
| `OPEN_METEO_URL` | `https://api.open-meteo.com/v1/forecast` | API address (a simulated API for tests) |
| `SOLARMAX_HOST` | — | inverter to query; without it, no SolarMax publisher |
| `SOLARMAX_PORT`, `SOLARMAX_INVERTER_ID` | `12345`, `1` | TCP port, inverter number (1 to 99) |
| `SOLARMAX_TOPIC` | `solarmax/0x6f0930a64c8b8803/prod` | publication topic |
| `SOLARMAX_INTERVAL_SECS` | `10` | interval between two readings (1 to 3600) |

## Diagnostics
- `dominet publish weather [--print-only]`: one reading per enabled station; prints the JSON and publishes it (unless `--print-only`).
- `dominet publish solarmax [--print-only]`: one cycle; refuses to run without `SOLARMAX_HOST`.
  **The inverter accepts a single TCP client at a time**: run while the `publishers` role is active (the normal case in production), the command fails ("did not answer in time") although the service publishes normally. To check the reading without stopping the service, look at the SolarMax device in the interface or at the Logs page (`/admin/logs`, text `solarmax`); for a manual reading, stop the stack first (`make prod-down`), then start it again (`make prod-up`).
- `dominet dump-context [--output context.yaml]`: YAML snapshot of active devices (type, state schema inferred from current values, numeric keys, command topic, location), active scenes, automations and device alert rules (R7 lot 8), Grafana rules and reference schemas.

## Simulators (tests, demonstration)
`tools/sim-open-meteo.py PORT` (weather API, fixed reading, `/__count` counter) and `tools/sim-solarmax.py PORT` (inverter, full protocol) only listen on `127.0.0.1`. Rust tests have their own simulated inverter (`dominet_integrations::solarmax::sim`). `tools/e2e.sh` starts them and points the server at them.

## Checking with reference values
- `tests/fixtures/solarmax/vectors.json` holds frozen reference vectors (requests, answers, scaling, rounding), produced **without network**: the constructor, which connects to the inverter, is never called. The Rust tests require the same results.
- `tests/fixtures/context/schemas.json` holds the reference state schemas (`_state_schema`, `_numeric_keys`).

## Tests
`cargo test -p dominet-integrations` (protocol, client against the simulated inverter, weather against the simulated API, supervision), `-p dominet-server --test publishers` (simulated API and inverter → publisher → dev Mosquitto → source → device), `--test context`, `-p dominet --test ui` (stations), E2E `weather`.
