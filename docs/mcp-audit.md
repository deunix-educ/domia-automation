# R11 — M0: server function audit and framing (2026-11-15)

Work package M0 of the MCP roadmap (not kept): find out **where the interface logic lives**, how much of it the assistant can reuse, and settle the technology (SDK, mounting, transport, tokens). Decisions: [ADR 0058](adr/0058-mcp-server.md) (server) and [ADR 0059](adr/0059-mcp-access-tokens.md) (tokens), written in French like all ADRs. The full table is in `tests/mcp/coverage.tsv`; **a `make ci` test fails if a server function is not classified there**.

## What the audit measured
- **107 server functions** in 20 files of `dominet-web` (3,482 body lines in total): 75 restricted to administrators, 25 open to any signed-in account, 7 tied to the session or the current account (sign in, sign out, password, personal dashboard).
- **The pattern is uniform**: `expect_context::<AppState>()`, an access check (`require_admin` / `require_user`), then logic that calls `AppState` and the repositories directly. Only 7 functions call `leptos_axum` directly: the session and current-account ones. No deep coupling: **the logic can be extracted**.
- **Control is already reusable**: `send_device_command` and `set_permit_join` call `dominet_server::control`. **Configuration is not yet**: the `save_*` functions (up to 98 lines) hold validation, writing and the reload signal. They will have to move into `dominet-server` as `service(&AppState, &Actor, input)` so the interface and MCP share one code path (work packages M2 to M5).
- **Technology**: `rmcp` 3.10.0 (official SDK, Apache-2.0, Rust 1.88 minimum) passes `cargo deny` and adds no `cargo audit` warning. The server is **session-less** (plain JSON responses), protected against DNS rebinding (`Host` header limited to `localhost` and `ALLOWED_HOSTS`) and works with a client on the previous protocol revision (tested).

## Coverage: 80 covered, 27 excluded
| Server function | File | Guard | MCP tool | Scope | Status | Note |
|---|---|---|---|---|---|---|
| `list_alert_rules` | alert_rules_admin.rs | admin | `config_list(alert_rule)` | read | covered |  |
| `save_alert_rule` | alert_rules_admin.rs | admin | `config_upsert(alert_rule)` | configuration | covered |  |
| `delete_alert_rule` | alert_rules_admin.rs | admin | `config_delete(alert_rule)` | configuration | covered | `confirm` and `dry_run` |
| `alerts_page` | alerts.rs | account | `alerts_list` | read | covered |  |
| `acknowledge_alert` | alerts.rs | account | `alert_ack` | control | covered |  |
| `acknowledge_all_alerts` | alerts.rs | account | `alert_ack(all)` | control | covered | acknowledge all: one card, one click on the assistant side |
| `android_overview` | android.rs | account | — | — | excluded | Android: messaging channel and tokens |
| `android_devices` | android.rs | account | — | — | excluded | list of phones and their tokens |
| `add_android_device` | android.rs | account | — | — | excluded | creates an Android token (secret) |
| `delete_android_device` | android.rs | account | — | — | excluded | Android tokens |
| `regenerate_android_token` | android.rs | account | — | — | excluded | Android tokens (secret) |
| `android_sms_page` | android.rs | account | — | — | excluded | SMS messaging excluded (decision of 2026-11-13) |
| `android_calls_page` | android.rs | account | — | — | excluded | call log: personal data, messaging excluded |
| `send_android_sms` | android.rs | account | — | — | excluded | SMS messaging excluded (decision of 2026-11-13) |
| `list_automations` | automations_admin.rs | admin | `automations_list` | read | covered |  |
| `save_automation` | automations_admin.rs | admin | `config_upsert(automation)` | configuration | covered | with a Rhai script: `administration` scope (see findings) |
| `delete_automation` | automations_admin.rs | admin | `config_delete(automation)` | configuration | covered | `confirm` and `dry_run` |
| `automation_runs` | automations_admin.rs | admin | `automation_runs` | read | covered |  |
| `list_cameras` | camera_fns.rs | admin | `cameras_list` | read | covered | no credentials |
| `list_camera_cards` | camera_fns.rs | account | `cameras_list` | read | covered |  |
| `save_camera` | camera_fns.rs | admin | `config_upsert(camera)` | administration | covered | credentials write-only (M5) |
| `delete_camera` | camera_fns.rs | admin | `config_delete(camera)` | administration | covered | `confirm` and `dry_run` |
| `sync_cameras` | camera_fns.rs | admin | `cameras_sync` | administration | covered | MediaMTX synchronisation (decided 2026-11-16) |
| `sync_camera_onvif` | camera_fns.rs | admin | `cameras_sync` | administration | covered | ONVIF profiles (decided 2026-11-16) |
| `add_camera_by_ip` | camera_fns.rs | admin | — | — | excluded | enters credentials from an address: interface only (decided 2026-11-16) |
| `start_discovery` | camera_fns.rs | admin | — | — | excluded | network scan: human trigger only |
| `camera_live` | camera_fns.rs | account | — | — | excluded | video stream: meaningless for an assistant |
| `camera_night` | camera_fns.rs | admin | `camera_night` | control | covered | night mode (decided 2026-11-16) |
| `capture_snapshot` | camera_fns.rs | admin | `camera_snapshot` | control | covered | returns an image |
| `list_snapshots` | camera_fns.rs | account | `camera_snapshot(list)` | read | covered | list of captures (decided 2026-11-16) |
| `delete_snapshot` | camera_fns.rs | admin | — | — | excluded | deletion: interface only (decided 2026-11-16) |
| `list_recordings` | camera_fns.rs | account | `recordings_list` | read | covered |  |
| `delete_recording` | camera_fns.rs | admin | — | — | excluded | deletion: interface only (decided 2026-11-16) |
| `set_manual_record` | camera_fns.rs | admin | `camera_record` | control | covered | manual recording (decided 2026-11-16) |
| `list_camera_events` | camera_fns.rs | account | `detections_list` | read | covered |  |
| `get_detector` | camera_fns.rs | admin | `config_get(detector)` | read | covered |  |
| `set_detector` | camera_fns.rs | admin | `config_upsert(detector)` | configuration | covered |  |
| `list_dashboards_admin` | dashboard_fns.rs | admin | `config_list(dashboard)` | read | covered |  |
| `save_dashboard` | dashboard_fns.rs | admin | `config_upsert(dashboard)` | configuration | covered |  |
| `delete_dashboard` | dashboard_fns.rs | admin | `config_delete(dashboard)` | configuration | covered | `confirm` and `dry_run` |
| `list_widgets` | dashboard_fns.rs | admin | `dashboard_get` | read | covered |  |
| `widgets_page` | dashboard_fns.rs | admin | `dashboard_get` | read | covered |  |
| `save_widget` | dashboard_fns.rs | admin | `widget_upsert` | configuration | covered |  |
| `delete_widget` | dashboard_fns.rs | admin | `widget_delete` | configuration | covered | `confirm` and `dry_run` |
| `move_widget` | dashboard_fns.rs | admin | `dashboard_set_layout` | configuration | covered |  |
| `send_device_command` | device_fns.rs | account | `device_command` | control | covered | active devices only |
| `set_permit_join` | device_fns.rs | admin | `zigbee_pairing` | configuration | covered | opens the Zigbee network to new devices (decided 2026-11-16) |
| `permit_join_state` | device_fns.rs | account | `system_status` | read | covered | `zigbee_pairing_open` field |
| `generate_device_dashboard` | device_fns.rs | admin | `config_upsert(dashboard)` | configuration | covered | generates a device's Grafana board (decided 2026-11-16) |
| `list_devices_admin` | devices_admin.rs | admin | `devices_list` | read | covered |  |
| `save_device_admin` | devices_admin.rs | admin | `config_upsert(device)` | configuration | covered |  |
| `delete_device_admin` | devices_admin.rs | admin | `config_delete(device)` | configuration | covered | `confirm` and `dry_run`: also deletes its automations |
| `device_choices` | devices_admin.rs | admin | `config_schema` | read | covered | form choices = schema |
| `list_device_groups` | devices_admin.rs | admin | `config_list(device_group)` | read | covered |  |
| `save_device_group` | devices_admin.rs | admin | `config_upsert(device_group)` | configuration | covered |  |
| `delete_device_group` | devices_admin.rs | admin | `config_delete(device_group)` | configuration | covered | `confirm` and `dry_run` |
| `list_device_labels` | devices_admin.rs | admin | `config_list(device_label)` | read | covered |  |
| `save_device_label` | devices_admin.rs | admin | `config_upsert(device_label)` | configuration | covered |  |
| `delete_device_label` | devices_admin.rs | admin | `config_delete(device_label)` | configuration | covered | `confirm` and `dry_run` |
| `devices_page` | devices_pages.rs | account | `devices_list` | read | covered |  |
| `device_detail` | devices_pages.rs | account | `device_get` | read | covered | and `device_history` |
| `energy_page` | energy.rs | account | `energy_summary` | read | covered |  |
| `list_logs` | logs_admin.rs | admin | `logs_query` | read | covered |  |
| `compose_form` | messaging.rs | admin | — | — | excluded | messaging excluded (decision of 2026-11-13) |
| `send_message` | messaging.rs | admin | — | — | excluded | messaging excluded (decision of 2026-11-13); `notify_test` covers testing the channels |
| `sent_messages` | messaging.rs | admin | — | — | excluded | messaging excluded (decision of 2026-11-13) |
| `inbox_messages` | messaging.rs | account | — | — | excluded | messaging excluded (decision of 2026-11-13) |
| `message_detail` | messaging.rs | account | — | — | excluded | messaging excluded (decision of 2026-11-13) |
| `list_mqtt_sources` | mqtt_sources.rs | admin | `config_list(mqtt_source)` | read | covered |  |
| `save_mqtt_source` | mqtt_sources.rs | admin | `config_upsert(mqtt_source)` | configuration | covered | with Rhai handler code: `administration` scope (see findings) |
| `delete_mqtt_source` | mqtt_sources.rs | admin | `config_delete(mqtt_source)` | configuration | covered | `confirm` and `dry_run` |
| `list_prometheus_rules` | rules_admin.rs | admin | `config_list(prometheus_rule)` | read | covered |  |
| `save_prometheus_rule` | rules_admin.rs | admin | `config_upsert(prometheus_rule)` | configuration | covered |  |
| `delete_prometheus_rule` | rules_admin.rs | admin | `config_delete(prometheus_rule)` | configuration | covered | `confirm` and `dry_run` |
| `list_grafana_rules` | rules_admin.rs | admin | `config_list(grafana_rule)` | read | covered |  |
| `save_grafana_rule` | rules_admin.rs | admin | `config_upsert(grafana_rule)` | configuration | covered |  |
| `delete_grafana_rule` | rules_admin.rs | admin | `config_delete(grafana_rule)` | configuration | covered | `confirm` and `dry_run` |
| `sync_rules` | rules_admin.rs | admin | `config_upsert(prometheus_rule / grafana_rule)` | configuration | covered | rule files written automatically after saving (decided 2026-11-16) |
| `list_scenes` | scenes.rs | account | `scenes_list` | read | covered |  |
| `activate_scene` | scenes.rs | account | `scene_run` | control | covered |  |
| `list_scenes_admin` | scenes_admin.rs | admin | `config_list(scene)` | read | covered |  |
| `save_scene` | scenes_admin.rs | admin | `config_upsert(scene)` | configuration | covered |  |
| `delete_scene` | scenes_admin.rs | admin | `config_delete(scene)` | configuration | covered | `confirm` and `dry_run` |
| `dry_run_script` | scripts.rs | admin | `script_check / automation_test` | administration | covered | dry run: no effect |
| `whoami` | server_fns.rs | session | — | — | excluded | web session; the assistant has no account |
| `layout_data` | server_fns.rs | session | — | — | excluded | web session (the account's navigation bar) |
| `logout` | server_fns.rs | session | — | — | excluded | web session |
| `login` | server_fns.rs | session | — | — | excluded | web session: tokens replace signing in |
| `change_password` | server_fns.rs | session | — | — | excluded | accounts and passwords out of scope |
| `list_locations` | server_fns.rs | admin | `config_list(location)` | read | covered |  |
| `save_location` | server_fns.rs | admin | `config_upsert(location)` | configuration | covered |  |
| `delete_location` | server_fns.rs | admin | `config_delete(location)` | configuration | covered | `confirm` and `dry_run` |
| `list_navbar` | server_fns.rs | admin | `config_list(navbar)` | read | covered |  |
| `save_navbar` | server_fns.rs | admin | `navbar_set` | configuration | covered |  |
| `delete_navbar` | server_fns.rs | admin | `navbar_set` | configuration | covered | `confirm` and `dry_run` |
| `list_settings` | server_fns.rs | admin | `settings_list` | administration | covered | secrets: "set" / "not set" only |
| `save_setting` | server_fns.rs | admin | `setting_set` | administration | covered | secrets write-only |
| `list_users` | server_fns.rs | admin | — | — | excluded | accounts out of scope |
| `save_user` | server_fns.rs | admin | — | — | excluded | accounts out of scope |
| `delete_user` | server_fns.rs | admin | — | — | excluded | accounts out of scope |
| `broadcast_notification` | server_fns.rs | admin | `notify_broadcast` | control | covered | banner to every signed-in account (decided 2026-11-16) |
| `inject_device_state` | server_fns.rs | admin | — | — | excluded | forges a device state (test tool): must not be offered to an assistant |
| `dashboard_page` | server_fns.rs | session | `dashboard_get` | read | covered | the signed-in account's dashboard: to define for a token |
| `create_dashboard` | server_fns.rs | session | `config_upsert(dashboard)` | configuration | covered | personal dashboard: to define for a token |
| `list_weather_stations` | weather_admin.rs | admin | `weather_current / config_list(weather_station)` | read | covered |  |
| `save_weather_station` | weather_admin.rs | admin | `config_upsert(weather_station)` | configuration | covered |  |
| `delete_weather_station` | weather_admin.rs | admin | `config_delete(weather_station)` | configuration | covered | `confirm` and `dry_run` |

## The 12 functions outside the catalogue: decisions of 2026-11-16 (validated by the user)
- `sync_cameras` : tool `cameras_sync`, administration scope
- `sync_camera_onvif` : tool `cameras_sync`, administration scope
- `add_camera_by_ip` : excluded (interface only)
- `camera_night` : tool `camera_night`, control scope
- `list_snapshots` : tool `camera_snapshot(list)`, read scope
- `delete_snapshot` : excluded (interface only)
- `delete_recording` : excluded (interface only)
- `set_manual_record` : tool `camera_record`, control scope
- `set_permit_join` : tool `zigbee_pairing`, configuration scope
- `generate_device_dashboard` : tool `config_upsert(dashboard)`, configuration scope
- `sync_rules` : tool `config_upsert(prometheus_rule / grafana_rule)`, configuration scope
- `broadcast_notification` : tool `notify_broadcast`, control scope

## Findings that change the roadmap
1. **Rhai scripts get in through the back door.** `save_automation` ("script" action) and `save_mqtt_source` (handler code) accept Rhai code. With a `config_upsert` at `configuration` scope, a `configuration` token would write scripts, bypassing the 2026-11-13 choice "scripts = `administration`". **Proposed rule**: any `config_upsert` whose payload contains a script requires the `administration` scope.
2. **Reading configuration requires `admin` today.** All 75 administrator-only functions, configuration listings included, require that role. For MCP, `config_list` and `config_get` at `read` scope (like `automations_list`) allow a diagnostic token with no right to change anything; secrets stay masked. **Validated on 2026-11-16.**
3. **The built-in assistant (R12) has no token**: it acts on behalf of the session. The common service must therefore receive an **actor** (`Token{scopes}` or `Session{account}`), not just an account. To plan from M1 to avoid redoing the extraction.
4. **`dry_run` on control.** R12 requires a dry run for every action; `ROADMAP-mcp.md` only plans it for deletions and configuration. To add to `device_command` and `scene_run` (M3).
5. **No way back.** `dominet dump-context` exports the configuration but nothing re-imports it: R12's `rollback` tool needs a reliable restore that R11's catalogue does not have.
6. **Error messages.** Server functions answer "internal error" or "access denied": fine for a browser, poor for an assistant. Domain validation already returns an error **per field**; the tools must pass it on as is.
7. **30 s timeout** on the whole router: enough for tools that answer in milliseconds; to watch for `automation_test` and `logs_query`.

## Delivered in this work package
`dominet-mcp` crate (`system_status` tool, temporary `Bearer` authentication `MCP_SPIKE_TOKEN`, off by default `MCP_ENABLED=false`), mounting through `serve(.., dominet_mcp::routes)`, end-to-end tests with a real MCP client (`cargo test -p dominet-mcp`), the coverage guard, ADR 0058 and 0059. See [MCP server](mcp.md) for the trial with Claude Code.
