# ADR 0062 — Published multi-architecture images, with local build as the fallback

Status: accepted (P3 lot 2, decisions of the user, 2026-11-21).

## Context
Building the image takes 15 to 40 minutes (Rust, WebAssembly, YOLO runtime) and needs several GB of memory; on a Raspberry Pi 5 it is the main obstacle for a novice. The target machines are x86-64 (mini PC) and arm64 (Raspberry Pi 5). The repository is not public yet, so an image cannot be pulled before publication (P6).

## Options
- **A. Always build locally.** Simple and offline, but slow and fragile on small machines.
- **B. Published image only.** Fast, but the installation fails when the registry or the tag is not reachable, and nothing can be tried before publication.
- **C. Try the published image, fall back to a local build.** Fast when available, still works offline, from a fork or before publication.

## Decision
Option C. `tools/prod.sh pull-image` (called by `make prod-up`, therefore by `./install.sh` and `make prod-update`) pulls `ghcr.io/deunix-educ/domia-automation:latest`, tags it `dominet:<DOMINET_TAG>` (the name Compose already uses, so the compose files do not change) and the caller builds locally when the pull fails. `.env` can change the source (`DOMINET_REGISTRY_IMAGE`, `DOMINET_IMAGE_VERSION`) or force the build (`DOMINET_PULL=0`). Images are published by `tools/publish-image.sh` (`docker buildx`, `linux/amd64` and `linux/arm64`), run by hand by the maintainer; there is no automatic CI (ADR 0015).

## Consequences
- Until the first publication every installation silently falls back to the build (a few seconds lost on the failed pull, and a message saying so).
- A downloaded image is trusted as published: it is not verified against the source on the machine. Anyone who wants to can set `DOMINET_PULL=0`.
- The arm64 image has never been built yet; its first build is part of the Raspberry Pi 5 test (P4).
- `latest` moves with each publication; pin `DOMINET_IMAGE_VERSION` to a release tag to avoid surprises on `make prod-update`.
