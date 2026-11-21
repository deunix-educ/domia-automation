# domia-automation

A self-hosted home-automation and video-surveillance application written in Rust: Zigbee2MQTT devices, IP cameras, automations and scenes, alerts, energy balance, messaging, an MCP server and an optional local AI assistant. It runs on a small x86-64 machine (an Intel Core i5 mini PC holds 12 cameras in 1080p, see [Measurements](docs/measures.md)) and installs with Docker Compose.

**Status.** Part 1 (automations, video surveillance, observability, MCP server) and part 2 (local chat assistant) are built; part 3 (voice assistant) is planned. The first public release is being prepared.

## What it does

- **Devices and automations**: Zigbee2MQTT and generic MQTT sources, scenes, schedules, sunrise and sunset, sandboxed scripts, alert rules with cooldown and acknowledgement.
- **Video surveillance**: ONVIF discovery and PTZ, live view in the browser, continuous and motion recording, motion detection and object detection on your own hardware.
- **Energy**: daily balance and a Grafana dashboard with the self-consumption rate.
- **Notifications**: e-mail, SMS, and an Android relay.
- **Observability**: Prometheus and Grafana, with platform alerts shown in the application.
- **MCP server and local assistant**: control and configure the application from an assistant, with scoped tokens, an audit trail and confirmation of every change. Nothing leaves your network.

## Install

New to this? Clone the repository and run `./install.sh`: it checks your machine, asks four questions and does the rest ([quick install](docs/quick-install.md); `./install.sh --dry-run` changes nothing). Manual production install in three commands: see [`DEPLOY.md`](DEPLOY.md). The full procedure, step by step, is in [`docs/installation.md`](docs/installation.md).

## Documentation

The documentation is built with `make docs` and is also served by the application under `/docs/`. Start with:

- [Pitfalls and lessons](docs/pitfalls.md): every trap met while building, by area
- [Measurements](docs/measures.md): what the hardware holds, with how to redo each measurement
- [Decisions by theme](docs/decisions.md) and the [ADRs](docs/adr/): why it is built this way, including what was abandoned
- [Rebuilding from scratch](docs/rebuild.md): the order to build it in

## Development

```bash
make help   # available targets
make ci     # format, clippy, tests, WASM, deny, audit
make hooks  # enable the pre-push gate (runs make ci on a clean copy)
make e2e    # browser end-to-end tests (see docs/tests.md); make ci-full adds the Docker image
make docs   # MkDocs documentation in target/docs-site
```

Requirements: `rustup` (the toolchain is pinned in `rust-toolchain.toml`) and Docker. Details in [`docs/dev-machine.md`](docs/dev-machine.md).

There is no automatic CI: the gate is `make ci`, run locally and by the `pre-push` hook.

## License

AGPL-3.0-or-later. Français : [README.fr.md](README.fr.md). Third-party components and their licences, with how far each was verified: [Licences](docs/licenses.md). Weather data by [Open-Meteo.com](https://open-meteo.com/) (CC BY 4.0; the free API is for non-commercial use).
