# Devices

Device pages, commands and administration (R3 lot 5). Decisions: [ADR 0025](adr/0025-device-schema.md) (schema), [ADR 0027](adr/0027-zigbee2mqtt-ingestion.md) and [ADR 0028](adr/0028-mqtt-sources-and-raw-archive.md) (where devices come from), [ADR 0029](adr/0029-device-commands-and-pages.md) (this document).

## For every account
- **`/automation/`** ("Devices" entry of the navigation): active devices as cards following the state **live**. Filters by group, location and type (a form: the address can be shared).
- **Commands** on the cards and on dashboard widgets: ON/OFF, brightness, colour temperature, covers (open, stop, close, position), thermostat set-point (0.5 °C steps), fan modes, lock. They are published on
  `<prefix>/<name>/set`; the device confirms by publishing its state again. Buttons are **disabled** ("source unavailable") when the device is offline or the server is not connected to the broker.
- **`/automation/scenes`**: scenes and their activation, see [scenes.md](scenes.md).
- **`/automation/devices/<id>`**: identity (type, group, location, IEEE address, model), state and command topics, last message, **last 24 h curves** of the tracked keys.

## For administrators
- Zigbee2MQTT **pairing**: button on `/automation/` (publishes `{"time": 254}` to open, `{"time": 0}` to close, on `<prefix>/bridge/request/permit_join`; Zigbee2MQTT 2.x format, pairing closes by itself after 254 s).
- **`/admin/devices`**, **`/admin/device-groups`**, **`/admin/device-labels`**: devices (type, group, location, tracked and hidden keys…), groups, state key labels (display name, icon, unit; `dominet seed-labels` creates the usual ones). Tracked keys are comma-separated; what Zigbee2MQTT
  discovers (IEEE address, model) and the state cannot be edited by hand.
- Device **image** (device page): JPEG, PNG, WebP or GIF up to 5 MB; it is reduced to 400 × 300 at most and stored in the database.
- Device **Grafana dashboard** (button on its page): one panel per tracked key, written to `GRAFANA_DASHBOARDS_DIR` (the folder Grafana watches). 
- **Widgets** (`/admin/widgets`): the device is picked from a list.
- **Simulator** (`/admin/settings`): injects a state for an **existing** device (broadcast only, nothing is written to the database).

## Tests
`cargo test -p dominet --test ui` (pages, permissions, commands, administration, Grafana), `-p dominet-server --test media` (images), `--test mqtt_role` (commands published on the dev broker), `-p dominet-server --lib media grafana`, `-p dominet-domain` (command validation,
labels, Grafana dashboard); E2E `devices` (full journey in Firefox: administration, cards, commands received by the broker, image, curve, Grafana).
