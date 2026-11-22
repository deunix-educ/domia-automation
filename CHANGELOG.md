# Changelog

All notable changes are listed here. Format based on [Keep a Changelog](https://keepachangelog.com/); versions follow [Semantic Versioning](https://semver.org/).

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
