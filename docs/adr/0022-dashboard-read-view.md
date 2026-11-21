# 0022 — Dashboard read view: class-based placement, sanitised HTML, restricted Grafana iframes

- Status: accepted
- Date: 2026-11-02

## Context
The read-only dashboard replaces the provisional home page (editing comes afterwards). Three security and robustness constraints drive the design: the CSP forbids the `style` attribute (placing each widget by `style="grid-column: …"` is not possible), the `text` widget's content is HTML typed by an administrator, and Grafana is displayed in an iframe that the CSP blocks by default.

## Decision
- **Placement by classes**: a widget carries `vc-c-N` (column N from 1 to 12), `vc-r-N` (row 1 to 100), `vc-w-N` and `vc-h-N` (width and height, 1 to 12). These classes set CSS variables (`--c`, `--r`, `--w`, `--h`, plus `--w8` and `--w4`, the width reduced to 8 then 4 columns) and a single `.vc-widget` rule derives `grid-column` / `grid-row` from them. On medium (≤ 1200 px) and narrow (≤ 768 px, ≤ 480 px) screens widgets are placed one after another (`auto / span …`) instead of keeping their original column, which would fall outside the reduced grid. **The domain's maximum row goes from 999 to 99** (the CSS covers 100 rows of 120 px, 12,000 px: plenty); the SQL constraint stays at 999 (the domain is stricter, which needs no migration).
- **`text` widget HTML sanitised at display time** (`dominet_server::html::sanitize_widget_html`, the `ammonia` crate): formatting, lists, tables, links and images; no script, iframe, form, `style` or event handler; `http`, `https`, `mailto`, `tel` links only, with `rel="noopener noreferrer"`; `id` and `class` attributes removed. The server returns already-safe HTML (`content_html`): the browser has no security decision to make. The same sanitising serves at save time: the database is not a trusted source (hand-written SQL, import). The "plain text of the configuration" fallback becomes an escaped text with line breaks.
- **Grafana iframes limited to one origin**: the address is computed on the server (`widget_view::grafana_src`: `panel_url`, device + `key`, `dashboard_uid` + `panel_id`) and **is only loaded if its origin is the one of the `GRAFANA_PUBLIC_URL` setting**; otherwise the widget shows "address refused". The CSP gains `frame-src <origin>` (read from the settings cache, `SettingsService::cached_string`, loaded at start-up so the header does not have to query the database). `uid` and `key` are filtered (alphanumeric characters, `-`, `_`): they cannot inject parameters. The iframe carries `sandbox="allow-scripts allow-same-origin"`.
- **Visibility**: an account only sees **its own** dashboards. `?d=<id>` of another account, absurd or non-existent falls back to the default dashboard, revealing nothing (tested). Administration of all dashboards comes with editing.
- **Devices**: a `device` widget renders from the hub's **last retained state** (server rendering), then follows `device:<id>` live (reference-counted subscription, ADR 0019). The automatic presentation is deduced from the state's keys (`widget_view::resolve_presentation`: light, blind, climate, fan, lock, weather, switch, binary sensor, sensor, generic); units come from a table of Zigbee2MQTT keys.
- **Dependencies**: `dominet-domain` becomes usable in WASM (`uuid` without the `v4` feature, never used); `ammonia` on the server side.

## Consequences
- A row beyond 99 is refused by the domain; a widget wider than the screen is placed in sequence rather than at its original column.
- The CSP now contains `frame-src` on every page (the login included): a minimal surface (one origin, configurable), but not zero; it disappears if the setting is empty or invalid.
- A Grafana widget whose address changes origin afterwards (setting modified) shows "address refused" instead of loading an iframe the browser blocks.
- The dashboard selector is a native `<details>`: no script, keyboard accessible.
