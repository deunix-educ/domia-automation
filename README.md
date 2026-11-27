# domia-automation

**Français : [README.fr.md](README.fr.md)**

A self-hosted home-automation and video-surveillance application written in Rust, with an **MCP server** and an optional **local AI assistant**. It runs on a small x86-64 machine (an Intel Core i5 mini PC holds 12 cameras in 1080p, see [Measurements](docs/measures.md)) and installs with Docker Compose.

**Origin.** DomiNet started as the author's personal home-automation application, built with Django, Channels and Redis. This repository is its rewrite in Rust (Axum, Leptos, PostgreSQL); the product name inside the code is still `dominet`.

**Status.** Released: the current version is 1.1.0 (see the [changelog](CHANGELOG.md)). Part 1 (automations, video surveillance, observability, MCP server) and part 2 (local chat assistant) are built; part 3 (voice assistant) is planned.

## What it does

- **MCP server**: an assistant (Claude Code, Claude Desktop, any MCP client) reads, drives and configures the application under the same rules as the interface, with scoped tokens, an audit trail and no secret ever shown. [MCP server](docs/mcp.md)
- **Local AI assistant**: ask in the interface ("Which shutters are still open?", "Close them"). A model you host yourself (llama.cpp) proposes actions that you confirm with one click; every change is audited and can be undone. Nothing leaves your network. It answers in French. [Assistant](docs/assistant.md)
- **Devices and automations**: Zigbee2MQTT and generic MQTT sources, scenes, schedules, sunrise and sunset, sandboxed scripts, alert rules with cooldown and acknowledgement.
- **Video surveillance**: ONVIF discovery and PTZ, live view in the browser, continuous and motion recording, motion detection and object detection on your own hardware. Each viewer has their own credentials for players such as VLC ([Camera access](docs/camera-access.md)).
- **Energy**: daily balance and a Grafana dashboard with the self-consumption rate.
- **Notifications and Android**: e-mail, SMS, and an Android relay: the **DomoComm** application sends the SMS and the calls of a phone to the server and sends SMS on request. Download the APK from the [releases](https://github.com/deunix-educ/domia-automation/releases) (tags `android-*`), see [Android relay](docs/android.md).
- **Remote access**: an optional WireGuard VPN (wg-easy 15) to use the application away from home.
- **Observability**: Prometheus and Grafana, with platform alerts shown in the application.
- **Moving house**: export and import of a whole installation, to change machine ([Migration](docs/migration.md)).

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
