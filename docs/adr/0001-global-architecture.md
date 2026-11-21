# 0001 — Global architecture

- Status: accepted
- Date: 2026-10-31

## Context
DomiNet is a Rust application for home automation and video surveillance. This record fixes its overall shape and the adjustments decided after the eight feasibility spikes (ADR 0002, 0003, 0005 to 0010).

## Decision
- **One `dominet` binary** (Axum + Tokio) with sub-commands and roles (`--only web|mqtt|scheduler|video|publishers|discovery`); supervised tasks with restart and backoff; `/healthz`, `/readyz`, `/metrics`. Only the `discovery` role is deployed apart (host network, WS-Discovery multicast).
- **Cargo workspace** of 10 crates (`domain` ← `automation`/`surveillance`/`notify`/`integrations` ← `store`/`mqtt` ← `server`; `proto` shared by server and WASM; `web` for Leptos). `domain` has no I/O and no framework.
- **Front end**: Leptos 0.8 SSR + hydration (ADR 0007). **Data**: PostgreSQL (`sqlx` 0.9) for configuration, users, aggregates and recent state; ReductStore for video, snapshots, detections and raw IoT (ADR 0002). **Bus**: MQTT `rumqttc` (ADR 0005) + Axum WebSocket. **Live view**: WHEP through MediaMTX (ADR 0010).
- **User scripts**: sandboxed Rhai, no direct side effects (ADR 0006). **Vision**: `ffmpeg` subprocess behind `FrameSource` (ADR 0003), YOLO through `ort` (ADR 0009), hand-written ONVIF (ADR 0008).
- **Authentication**: ADR 0004.
- **Deployment**: Docker Compose (the official route) and a release binary; **main target x86_64** (Intel Core i5-6500 mini PC), arm64 on a best-effort basis. **glibc** image (Debian slim): ONNX Runtime has no musl binary. Binaries are published at the **`x86-64-v2`** baseline (never `target-cpu=native`); the heavy computation (ONNX Runtime) picks its own AVX2 kernels at run time.
- **Observability**: structured `tracing`, Prometheus `/metrics`, Grafana.

## Adjustments from the spikes
- Storage: ReductStore = raw archive and threshold queries; **all charts** come from PostgreSQL.
- `ort` only has release candidates: the version is pinned exactly and a parity test is mandatory on every upgrade.
- Leptos stays on 0.8 (0.9.0-rc1 came out on 2026-10-31; `leptos-fluent` requires `< 0.9`).
- Docker: the `dominet` service gets `/dev/dri` and the `render` group (VAAPI); MediaMTX publishes its ICE port **unshifted**; ReductStore needs a non-empty token.
- ONVIF: adding a camera by IP is mandatory (some cameras do not answer multicast); OpenIPC has no ONVIF.

## Consequences
- Any departure from these choices goes through a new ADR that replaces this one.
