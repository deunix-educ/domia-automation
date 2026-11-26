# Changelog

All notable changes are listed here. Format based on [Keep a Changelog](https://keepachangelog.com/); versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- **Watching a camera needs an account** (ADR 0064). MediaMTX asks DomiNet before every RTSP and WebRTC read. The browser needs nothing: the live view sends a short-lived token tied to the session. VLC and phone applications use the account's user name and a personal password per device, created on **My account → Camera access** (shown once, revocable, with its last use; revoking closes the account's open sessions). Every account sees every camera. See [Camera access](docs/camera-access.md).
- `make prod-init` generates `MEDIAMTX_READ_USER` and `MEDIAMTX_READ_PASSWORD` (DomiNet's own account for decoding the video); `make prod-up` adds them to an existing `.env`, and `make prod-doctor` blocks if they are missing.
- `tools/e2e.sh live_auth` replays the live view against the shipped MediaMTX configuration.
- The weather widget shows the Open-Meteo attribution ("Weather data by Open-Meteo.com", CC BY 4.0), linked to open-meteo.com.
- Video load measured on a Raspberry Pi 5 (8 GB, NVMe, software decoding): about 0.25 core per camera, 6 cameras in 1080p at 40 % of the machine in full mode (`docs/measures.md`, section 2). Nothing above 6 cameras was measured.

### Changed
- **Breaking, viewers**: a player that opened `rtsp://<host>:8554/<uuid>` without a password is now refused until its owner creates a password on the Camera access page (see above). The production MediaMTX no longer allows anonymous reads; the API and metrics ports stay unpublished, and nginx answers 404 on `/internal/`.
- Code comments, `make help` and the `--help` texts are in English.
- `SECRET_KEY` is removed: nothing used it (CSRF protection relies on the `Origin` header, ADR 0004). The installer no longer generates it, the configuration no longer reads it, and a `SECRET_KEY` line left in an existing `.env` is ignored.
- **VPN: wg-easy 15** (AGPL-3.0) replaces version 14, which was licensed for non-commercial use only (ADR 0065, `docs/licenses.md`). The profile follows `WG_ADMIN_PASSWORD` (user `admin`); `make prod-up` sets once, through the wg-easy API, the firewall rules (`iptables-nft`), the NAT interface and the keepalive. `make prod-export` and `make prod-import` carry the VPN (administrator, settings, server keys, devices). **Breaking, installations that used the VPN with wg-easy 14**: `WG_ADMIN_HASH` is no longer used; the first `make prod-up` generates `WG_ADMIN_PASSWORD` in `.env` and starts wg-easy 15 on a new empty volume, so the devices must be created again (the old volume is left in place). A migration archive that holds wg-easy 14 data is reported and its VPN part is not restored.
- `make certs` reloads nginx when the stack is running. Before, a renewed certificate (a new name, such as the VPN address) was served only after the next restart, and browsers kept receiving the old one.
- **The Android application is in the repository** (`android/`, ADR 0066), renamed **DomoComm** (package `com.domoconf.comm`: a new application for Android, the former VideoComm is uninstalled and its servers entered again). `make android-apk` builds and signs `dist/DomoComm-<version>.apk` with a key kept outside the repository. Fixed: the dot of a configured server never turned green when it was connected; the service refused to start unless every permission was granted (it now starts with the granted ones, only the SMS permissions are required). The personal certificate authority is no longer built in: the authority installed on the phone is trusted. See [Android relay](docs/android.md). DomoComm has its own icon (a phone with a house and signal waves) with a monochrome layer for themed icons.

## [1.0.2] - 2026-11-23

### Added
- The published image is now multi-architecture (amd64 and arm64): a Raspberry Pi 5 downloads it instead of building for about 67 minutes. Tested on a Pi 5 (8 GB, NVMe): installs, starts and answers; camera count not measured. `docs/machines.md` says how to enable the memory cgroup so the machine card shows memory.
- The installer and `make prod-up` warn when a Zigbee dongle is plugged in but the Zigbee2MQTT folder is empty (a new network would replace the one on the dongle), and `make prod-doctor` reports a failed network creation and says to stop the retries first.

### Fixed
- On a 64-bit ARM machine (Raspberry Pi), `make prod-up` no longer replaces the image built locally with the published amd64 one ("exec format error", empty master key): the download is limited to the machine's architecture and the image is checked before it is used. An empty master key is now an error instead of being written to `.env`.
- The Zigbee2MQTT example configuration generates its own network identifiers (`GENERATE`) and uses channel 25 instead of the default values shared by every installation and the channel that overlaps Wi-Fi channel 1.
- `make prod-import` writes the Zigbee2MQTT folder even when the container left root-owned files in it, refuses (without `--yes`) when a database volume or services of the project already exist, removes them with `--yes`, and has `--no-zigbee` for a rehearsal next to a running production.

## [1.0.1] - 2026-11-22

### Changed
- The local assistant (`make agent-install`) now defaults to port 8081, so it no longer collides with Zigbee2MQTT and the demo on 8080. Set `AGENT_LLM_URL` accordingly.
- `make fetch-model` explains clearly that a different checksum is not an error and points to the `yolo_parity` test.

### Added
- The installer and `make prod-doctor` detect a PostgreSQL volume left by an earlier installation (a new `.env` cannot open it) and say how to keep or erase it.

### Fixed
- The published image now carries the `org.opencontainers.image.source` label, so GHCR links the package to the repository by itself.
- The installer no longer builds the alert sender address from an IP address (`dominet@192.168.1.104`): Grafana refused to start with it. `make prod-doctor` reports an existing `.env` with that problem.

## [1.0.0] - 2026-11-21

First public release.

### Added
- **Devices and automations**: Zigbee2MQTT and generic MQTT sources, scenes, schedules (interval, cron, sunrise and sunset), sandboxed scripts, alert rules with cooldown and acknowledgement, dashboards and widgets.
- **Video surveillance**: ONVIF discovery and PTZ, live view in the browser (WebRTC), continuous and motion recording, replay, motion detection and object detection (YOLO, on the CPU).
- **Energy**: daily balance, Linky, solar inverter and heat pump readings, Grafana dashboard with the self-consumption rate.
- **Notifications**: e-mail, SMS (Free Mobile) and an Android relay with messaging.
- **Observability**: Prometheus metrics, shared alert rules for Prometheus and Grafana, platform alerts shown in the alerts page, scheduled backups with checksums and restore.
- **MCP server** (off by default) with scoped, expiring, audited tokens, and an optional **local assistant** that proposes changes and never applies one without confirmation.
- **Guided installation**: `./install.sh` (checks, four questions, deployment; `--dry-run`), published multi-architecture images with a local build as fallback, `make prod-doctor` and `dominet doctor`, troubleshooting page.
- **Moving to another machine**: `make prod-export` and `make prod-import` carry the database, ReductStore data and, encrypted, the keys, certificate authority, Zigbee2MQTT pairing and MQTT accounts; `make check-migrate` rehearses it.
- **Machines**: arm64 image build prepared, per-machine test status, `tools/machine-card.sh`.
- Documentation in English and French (MkDocs, served by the application under `/docs/`), 63 architecture decision records, pitfalls, measurements, licences and security pages.

### Known limits
- arm64 and the Raspberry Pi 5 have **not been tested on real hardware** yet (see [Supported machines](docs/machines.md)).
- Camera viewing (RTSP, WebRTC) has no password on the LAN; never forward those ports (see [Security](docs/security.md)).
- The assistant is French-only; scope identifiers of MCP tokens are French (see [To improve](docs/to-improve.md)).
- The optional VPN profile pins wg-easy 14, reported as non-commercial (see [Licences](docs/licenses.md)).
