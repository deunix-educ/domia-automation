#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Publishes the application image (linux/amd64 and linux/arm64) to GHCR. Run BY HAND by the maintainer, after the repository is
# public (ADR 0062); nothing calls it automatically.
#
#   docker login ghcr.io -u <user>          (a token with write:packages)
#   VERSION=1.0.0 tools/publish-image.sh    pushes :1.0.0 and :latest
#   DRY=1 VERSION=1.0.0 tools/publish-image.sh   prints the command only
#
# Cross-building arm64 on amd64 needs QEMU (`docker run --privileged --rm tonistiigi/binfmt --install arm64`) and is very slow; building
# on a real arm64 machine and merging the manifests is faster (see docs/quick-install.md, "Images").
set -euo pipefail
cd "$(dirname "$0")/.."
IMAGE="${IMAGE:-ghcr.io/deunix-educ/domia-automation}"
VERSION="${VERSION:?set VERSION (for example 1.0.0)}"
PLATFORMS="${PLATFORMS:-linux/amd64,linux/arm64}"
cmd=(docker buildx build --platform "$PLATFORMS" -t "$IMAGE:$VERSION" -t "$IMAGE:latest" --push -f Dockerfile .)
if [ -n "${DRY:-}" ]; then printf '%q ' "${cmd[@]}"; echo; exit 0; fi
if [ -f models/yolov8n.onnx ]; then :; else echo "models/yolov8n.onnx missing: run make fetch-model first" >&2; exit 1; fi
"${cmd[@]}"
