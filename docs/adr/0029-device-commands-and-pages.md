# 0029 — Device commands, pages and images

- Status: accepted
- Date: 2026-11-02

## Context
Devices have been brought from MQTT into the database and live. This work makes them usable: command them, see them (list, detail, widgets), administer them, give them an image, generate their Grafana dashboard. The reference implementation publishes any JSON (`device_command`), puts everything in a generated administration site and stores an uploaded image as is.

## Decisions
- **Commands** (`dominet_server::control`). Any signed-in account may command (the family turns on a light); opening pairing (`permit_join`), uploading an image, generating a Grafana dashboard and configuring devices are **reserved to administrators**. A command is a **non-empty JSON object**, 1 KB at most, 3 levels at most (`dominet_domain::command`) — the interface only sends flat objects, Zigbee2MQTT accepts a few nested objects (`color`), nothing else makes sense. It is published at QoS 1, not retained, on `<prefix>/<name>/set`, for an **active** device only. With no link to the broker (no `mqtt` role, broker down) it is refused at once (`Unavailable`), never queued: a command replayed later would be a surprise. The effect shows when the device republishes its state; the interface anticipates nothing.
- **Device widgets** (`device_widget`). The type, labels (name, unit) and hidden keys come from the device table (`DeviceMetaDto`, carrying only the labels of keys present in the state). Automatic presentation follows the **type** when it is precise, otherwise the state (ADR 0025: a Zigbee2MQTT lamp stays `generic`, so the state decides). Commands are `{"state":"ON"}`, `brightness`, `color_temp`, `position`, `OPEN/STOP/CLOSE`, `LOCK/UNLOCK`, the `occupied_heating_setpoint` set point in steps of 0.5, `fan_mode`. Buttons are enabled if the server has an MQTT client **and** the device is available; otherwise disabled with the tooltip "source unavailable".
- **Pages.** `/automation/` (list) and `/automation/devices/<id>` — both spellings of the address are served. Filters (group, place, type) through a **GET** form: no state in the browser, the address is shareable. Cards reuse the widget body (same code, same commands, same real time). The detail shows identity, topics, the last 24 hours of tracked keys (SVG curves computed in the browser, no library) and, for an administrator, the tools: image, Grafana dashboard, pairing (on the list).
- **Administration**: `/admin/devices`, `/admin/device-groups`, `/admin/device-labels` with the generic CRUD (ADR 0014). A new field type `Choice`: a drop-down whose choices come from the database, supplied by a context (`Choices`) — `Select` types stay static and translated. A dashboard widget chooses its device from a list instead of a number.
- **Images**: PostgreSQL (`device_image`), served by `/media/devices/<id>` with `ETag` (`private, no-cache`: revalidated, `304` without body). Upload by `multipart` form to `/api/devices/<id>/image` (administrators, redirect to the page): 5 MB and 8,000 pixels per side at most, format recognised by content (JPEG, PNG, WebP, GIF), then **decoded, reduced to at most 400 × 300 and re-encoded as JPEG** (transparency composited on white). What is stored is therefore always a sane, small image, whatever file was sent. The `image` crate without default features (jpeg, png, webp, gif).
- **Grafana dashboard** (`dominet_domain::grafana`, `dominet_server::grafana`): one panel per tracked key, written **as a file** in `GRAFANA_DASHBOARDS_DIR` (never through the API: a dashboard loaded from a file becomes "provisioned" and the API refuses to modify it). Atomic write (temporary file then rename), incremented version, old file deleted if the device is renamed, sanitised file name. Without a configured folder, the page says so.
- **Simulator**: only acts on an existing device, with its real name; it broadcasts without writing anything to the database.

## Deviations from the reference
1. A validated command (object, size, depth) instead of publishing anything.
2. An image re-encoded and reduced instead of kept as is; formats refused by content and not by the `Content-Type` header sent by the browser.
3. Pairing is reserved to administrators (the reference: any signed-in account).
4. No editing page of a generated site: editing is in the `/admin/…` pages.

## Consequences
- A browser without WebAssembly cannot command (buttons are interface actions) but sees all pages; image upload and filtering work without JavaScript.
- A device's detail only shows numeric keys; text history (e.g. `action`) stays in the database and in Grafana.
- There is no optimistic state (the interface does not show the command before the device's echo): the price is a slight delay, the gain is that a displayed state is always a real state.
