# Configuration

`dominet` is configured through environment variables only (names reused from DomiNet's
`.env.example.prod` where they make sense). All errors are reported together at startup (exit code 2).
Secrets are never printed or logged.

| Variable | Default | Purpose |
|---|---|---|
| `DATABASE_URL` | — (**required**) | `postgres://user:password@host:5432/db` |
| `HTTP_ADDR` | `0.0.0.0:8000` | HTTP listen address |
| `BACKUP_DIR` | — | backup folder (`postgres/`, `reductstore/`); absent: scheduled backup disabled |
| `BACKUP_RETENTION_DAYS`, `BACKUP_HOUR` | 7, 3 | maximum age of backups (the newest always stays); local hour of the daily backup |
| `REDUCTSTORE_BACKUP_SRC` | — | ReductStore data folder, mounted read-only, to archive; absent: only the database is backed up |
| `PG_DUMP_BIN`, `PSQL_BIN` | `pg_dump`, `psql` | PostgreSQL clients |
| `PROMETHEUS_RULES_PATH` | — | `rules.yml` file that Prometheus reads (rules edited in `/admin/prometheus-rules`); absent: not written |
| `PROMETHEUS_ALERTS_POLL_SECS`, `PROMETHEUS_ALERT_COOLDOWN` | 60, 3600 | polling of `/api/v1/alerts` (`scheduler` role, active when `PROMETHEUS_URL` is set) and the silence of an alert already notified (seconds) |
| `PROMETHEUS_URL` | — | e.g. `http://prometheus:9090`: `POST /-/reload` after each write (Prometheus started with `--web.enable-lifecycle`) |
| `GRAFANA_ALERTING_PATH` | — | `rules.yaml` file of Grafana's alerting provisioning (`/admin/grafana-rules`) |
| `GRAFANA_URL`, `GRAFANA_USER`, `GRAFANA_PASSWORD` | —, `admin`, — | reload of Grafana's alerting provisioning (administrator account) |
| `METRICS_ADDR` | `0.0.0.0:9100` | `/metrics` listener for Prometheus (**never published** on the host); `off` turns it off; a busy port does not stop the server |
| `DOMINET_MASTER_KEY` | — | AES-256 key (32 bytes, base64) for encrypting secrets at rest (`dominet gen-master-key`); validated at startup; see [Settings](settings.md) |
| `SESSION_IDLE_MINUTES` | `480` | sign-out after this long without activity |
| `SESSION_MAX_DAYS` | `14` | maximum lifetime of a session |
| `MCP_ENABLED` | `false` | enables the [MCP server](mcp.md) on `/mcp` (R11); tokens are created in Administration › Assistants (MCP) |
| `MCP_RATE_LIMIT_PER_MIN` | `120` | calls per minute and per token; beyond it, 429 |
| `MCP_AUDIT_RETENTION_DAYS` | `365` | how long the MCP audit trail is kept |
| `WS_HEARTBEAT_SECS` | `25` | WebSocket heartbeat interval (1 to 300); the session is re-validated at every beat |
| `SYSTEM_STATS_SECS` | `5` | machine resources measurement interval (1 to 3600 s): sidebar gauges, `/api/system-stats/` |
| `TRUSTED_PROXIES` | — | IPs or CIDR networks of the proxies whose `X-Forwarded-For` is trusted (e.g. `172.18.0.1,10.0.0.0/8`) |
| `ALLOWED_HOSTS` | — | accepted hosts, comma separated |
| `CSRF_TRUSTED_ORIGINS` | — | trusted origins, comma separated |
| `COOKIE_SECURE` | `true` | `false` for plain HTTP without TLS on the LAN |
| `SITE_ROOT` | `target/site` (`/app/site` in the image) | folder of the interface files (WASM, JS, CSS) produced by `cargo leptos build` |
| `DOCS_ROOT` | `target/docs-site` (`/app/docs` in the image) | documentation site (MkDocs), served on `/docs/` behind the login |
| `CUSTOM_CSS` | — | appearance customisation file, served at `/custom.css` and read again on every load (in production: `/custom/custom.css`, mounted from `deploy/custom/`): see [appearance](appearance.md) |
| `TZ` | `Europe/Paris` | display time zone |
| `LOG_FORMAT` | `pretty` | `pretty` or `json`; the filter is set with `RUST_LOG` (default `info`) |
| `MQTT_HOST`, `MQTT_PORT`, `MQTT_USER`, `MQTT_PASSWORD` | port `1883` | MQTT broker (used from R3); without `MQTT_HOST` the `mqtt` role is inactive |
| `MQTT_CLIENT_ID` | `dominet` | MQTT client identifier, stable (persistent session) |
| `REDUCTSTORE_URL`, `REDUCTSTORE_TOKEN` | — | ReductStore: raw history archive (`iot` bucket, from R3), video (R6) |
| `IOT_QUOTA_GB`, `REDUCTSTORE_IOT_BUCKET` | `5`, `iot` | FIFO quota of the `iot` bucket (applied when it is created) and its name |
| `HISTORY_RETENTION_DAYS` | `400` | maximum age of the PostgreSQL device history (daily purge) |
| `GRAFANA_DASHBOARDS_DIR` | — | folder watched by Grafana where device dashboards are written (device page); without it, generation is refused with a message |
| `OPEN_METEO_URL`, `SOLARMAX_HOST`, `SOLARMAX_PORT`, `SOLARMAX_INVERTER_ID`, `SOLARMAX_TOPIC`, `SOLARMAX_INTERVAL_SECS` | public API, —, `12345`, `1`, default topic, `10` | weather and SolarMax publishers: see [publishers](publishers.md); **`SOLARMAX_HOST` has no default** |
| `MEDIAMTX_API_URL` | — | MediaMTX REST API (e.g. `http://mediamtx:9997`): one *path* per active camera is created, replaced or removed on the fly, and everything is resynchronised at startup; without it cameras can be saved but nothing is synchronised. Camera passwords are encrypted with `DOMINET_MASTER_KEY`: without a key a camera password is refused |
| `PTZ_DEAD_MAN_SECS` | `8` | a continuous PTZ move that nobody refreshes or stops (frozen tab, cut network) is stopped by the server after this delay, in addition to stopping when the link closes |
| `MEDIAMTX_RTSP_URL` | — | MediaMTX RTSP re-stream (e.g. `rtsp://mediamtx:8554`) decoded by the `video` role (`<url>/<uuid>`, 2 frames/s, `ffmpeg`); **when set, `dominet serve` also starts the `video` role**. Without it: no latest image, no snapshots, no fallback for browsers without H.264 |
| `SNAPSHOTS_DIR`, `SNAPSHOTS_MAX_PER_CAMERA`, `FFMPEG_HWACCEL` | `data/snapshots` (`/data/snapshots` in the image), `500`, `auto` | folder of snapshots (gallery) and of each camera's latest image (`latest/`) — **shared by the `video` and `web` roles** if they are two processes; beyond the cap the oldest snapshots of a camera are deleted; VAAPI decoding if available (`auto`) or software (`off`) |
| `RECORDINGS_DIR`, `RECORDINGS_SEGMENT_DURATION`, `RECORDINGS_SCAN_SECS`, `RECORDINGS_GRACE_SECS` | `/recordings`, `60`, `60`, `15` | folder where MediaMTX writes the fMP4 segments (`<folder>/<uuid>/<date>.mp4`, **volume shared with MediaMTX**), segment length (that of `recordSegmentDuration`), scan interval of the `video` role, idle time after which a segment is considered finished |
| `VIDEO_QUOTA_GB`, `REDUCTSTORE_VIDEO_BUCKET` | `50`, `surveillance` | FIFO quota (GB) and name of the ReductStore bucket for video segments; without `REDUCTSTORE_URL`, segments stay on disk |
| `MOTION_THRESHOLD`, `MOTION_INTERVAL_SECS`, `MOTION_ALERT_COOLDOWN`, `DETECTION_ALERT_COOLDOWN` | `0.01`, `30`, `600`, `300` | motion (cameras in `motion` mode): share of the image that must change, evaluation interval, minimum delays between two notifications of the same camera (motion, detection) |
| `YOLO_MODEL_PATH`, `YOLO_WORKERS`, `DETECTIONS_DIR` | `models/yolov8n.onnx` (`/models/yolov8n.onnx` in the image), `1`, `data/detections` (`/data/detections`) | detection model (`make fetch-model`, not committed), ONNX Runtime sessions with one thread each, folder of annotated images. The detector is switched on in the interface (`/surveillance/events`) |
| `EMAIL_HOST`, `EMAIL_PORT`, `EMAIL_HOST_USER`, `EMAIL_HOST_PASSWORD`, `EMAIL_USE_TLS`, `EMAIL_USE_SSL`, `EMAIL_TIMEOUT_SECS`, `SMS_FREE_URL` | —, `587` (`465` with SSL), —, —, `true`, `false`, `10`, official API | e-mail sending (usual names) and the Free Mobile API address: see [notifications](notifications.md). **Without `EMAIL_HOST`, no e-mail.** Credentials without TLS are only allowed towards the loopback |

## Commands

```text
dominet serve [--only web,mqtt,...]   starts the server (only `web` exists for now)
dominet migrate | seed [--dry-run] | gen-master-key | createsuperuser | passwd
dominet healthcheck [--addr h:p]      probes GET /healthz (exit 0 on 200): Docker healthcheck, needs neither the database nor the full configuration
```

## Probes

- `GET /healthz`: 200 as long as the process answers (no dependency).
- `GET /readyz`: 200 if PostgreSQL answers within 2 s, otherwise 503 (no detail on the cause).
- Every response carries an `x-request-id` header (taken from the request if present), also in the logs.
