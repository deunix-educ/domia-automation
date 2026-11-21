# ADR 0055 — Compose: base and overlays, probes, volumes, Mosquitto

Status: accepted. **The anonymous LAN listener (1884, `MQTT_LAN_ANONYMOUS`) is replaced by ADR 0057** (a single listener with a password).

## Context
The repository first had only a standalone development stack (4 services). A deployable stack needs a base of 8 services with overlays (development, production, demonstration), pinned images (no `latest`), an authenticated Mosquitto and a separate `migrate` step.

## Decisions
- **Four files** in `deploy/`: `docker-compose.yml` (base: services and links, secrets only through the environment, no published port except PostgreSQL on loopback), `.dev`, `.prod`, `.demo` (projects `dominet-dev`, `dominet`, `dominet-demo`: separate volumes and networks). A single network `dominet-net`; only `dominet-discovery` and `wg-easy` are in host network.
- **Base services**: `dominet`, `dominet-discovery`, `postgres`, `reductstore`, `mosquitto`, `mediamtx`, `prometheus`, `grafana`, plus **two one-shot services**: `migrate` (`dominet migrate` before any application, blank database included) and `init` (gives the shared volumes to user 10001: a named volume is created by root and the first container to mount it does not always own it).
- **Pinned images**: `postgres:16-alpine`, `reduct/store:v1.21.5`, `eclipse-mosquitto:2.0`, `bluenviron/mediamtx:1.21.3`, `prom/prometheus:v2.52.0`, `grafana/grafana:11.0.0`, `nginx:1.27-alpine`, `koenkk/zigbee2mqtt:2.1.1`, `wg-easy:14`, `adminer:4.8.1`. A single image for all DomiNet roles (`dominet:${DOMINET_TAG}`), built by the `dominet` service alone; `pull_policy: never` for the others: never a pull of an image that would not exist.
- **Probes**: the image has no `curl`; a `dominet healthcheck` command (GET `/healthz` on the `HTTP_ADDR` port, 3 s, exit code 0/1, **without reading the database or the full configuration**) serves as healthcheck. `depends_on` uses `service_healthy` for `postgres`, `reductstore`, `mosquitto`, `dominet` (nginx).
- **Mosquitto**: `per_listener_settings true`; listener 1883 (Docker network: DomiNet, Zigbee2MQTT) **with a password**; the LAN listener is described by ADR 0057. Configuration and password file are produced at start-up by `deploy/mosquitto/entrypoint.sh` from the environment: no secret in the repository. Observed: without `per_listener_settings`, a LAN `allow_anonymous true` also applied to the secured listener.
- **MediaMTX**: user 10001 (owner of the segments DomiNet rereads then **deletes** after archiving — the volume is therefore writable by `dominet`); reading is open, but publishing, the API and metrics are only accepted from the machine and private Docker networks (cameras are pulled by MediaMTX, they do not publish); SRT and MoQ off.
- **Shared volumes**: `prometheus-rules` (DomiNet writes, Prometheus reads), `grafana-alerting` (same for Grafana rules; the repository's contact point and policy files are mounted on top), `grafana-dashboards` (generated device dashboards, a second dashboard provider in `dashboards.yml`), `backups-data`, `recordings-data`, `app-data` (snapshots, annotated images), `reductstore-data` (read-only for `dominet`: backup).
- **ONVIF discovery**: `dominet serve --only discovery` in host network, `METRICS_ADDR=off` (9100 would be the host's), PostgreSQL reached through `127.0.0.1:${POSTGRES_HOST_PORT}` (a port published on loopback).
- **Development overlay**: starts the infrastructure only (the code runs through `cargo`), shifted ports (15432, 18383, 11883, 18554, 18889, 19997, 8189); Prometheus and Grafana in the `observability` profile (19090, 13000).
- **Production overlay**: nginx (mkcert TLS, 80/443, `/grafana/`, `/webrtc/`, `/dominet-ca.crt` — also served over HTTP, a new device not yet having the CA —, WebSocket), Mosquitto LAN, RTSP 8554, ICE 8189, `restart: unless-stopped`, profiles `zigbee`, `vpn`, `adminer` (loopback 8181). Variables of inactive profiles are not required by Compose (it interpolates even inactive profiles): `make prod-guard` checks them.
- **Demo overlay**: HTTP, shifted ports, LAN MQTT off, a fake camera (the image's ffmpeg → MediaMTX).
- **Secrets**: `${VAR:?}` in the production and demo overlays (start-up fails with the variable's name), never a value in a Compose file or an image.

## Consequences
- Scripts that named the development network use `dominet-dev_dominet-net`.
- `make dev-up` never builds the image (no development service asks for it): integration tests stay fast.
- A full trial (blank install, three commands) and a trial on real hardware are separate checks.
