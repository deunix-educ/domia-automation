# ADR 0063 — arm64 and Raspberry Pi 5: supported on a best-effort basis, with explicit test status

Status: accepted (P4, 2026-11-21). Reopens the "arm64 not built" point of [ADR 0017](0017-docker-image-and-e2e.md).

## Context
The first target is x86-64 (Intel Core i5 mini PC). A Raspberry Pi 5 (8 GB) is the cheapest machine a novice can buy. The development machine has no arm64 emulation and no arm64 hardware, so nothing arm64 has been built or run. Known facts: ONNX Runtime ships a prebuilt arm64 binary ([ADR 0009](0009-yolo-ort.md)); the Debian packages `intel-media-va-driver` and `vainfo` exist for amd64 only, which broke an arm64 build of the `Dockerfile`; the Pi has no VAAPI decoder usable by `ffmpeg` in the image.

## Options
- **A. Declare arm64 unsupported.** Honest today, but closes the door on the machine most people own.
- **B. Declare arm64 supported.** Promises what has not been verified.
- **C. Make the image buildable for arm64, say "untested" in the support table until a measurement exists, and give people the tool to produce that measurement.**

## Decision
Option C.
- The `Dockerfile` installs the Intel VAAPI packages on amd64 only; on arm64 video is decoded in software. `docker buildx build --check --platform linux/arm64` passes; no arm64 build has been run.
- `docs/machines.md` states a status per machine ("tested and measured", "not tested yet", "untested") and lists what is known and unknown for the Pi.
- The installer and `preflight` no longer refuse arm64: they warn, and warn again on an SD card or under 7 GB of memory.
- `tools/machine-card.sh` produces a read-only Markdown card (hardware, image architecture, memory and CPU per service, interface latency, Pi temperature and throttling) to paste into `docs/measures.md`.
- A machine moves to "tested" only with a card and a cold install done on that machine.

## Consequences
- The first arm64 build may reveal other blockers (a tool download without arm64 binary, memory during the WebAssembly link); they will be fixed then, not guessed now.
- The camera count on a Pi is unknown and probably well below the reference machine's; nothing in the documentation promises a figure.
- Publishing an arm64 image (ADR 0062) should wait for one successful native build and start-up on a Pi 5.

## Outcome (2026-11-23)
The condition was met on a Raspberry Pi 5 (8 GB, NVMe, Debian 13): native build in 67 minutes, stack started, interface answering. The arm64 image was merged with the amd64 one into a multi-architecture `:1.0.1` and `:latest` (the compiled sources had not changed since 1.0.1). The first run exposed a defect that had nothing to do with the build: on arm64 `docker pull` fetched the amd64 image without error and it replaced the local build; `prod.sh` now pulls only the machine's architecture. Camera count and detection load remain unmeasured.
