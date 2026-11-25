# SPDX-License-Identifier: AGPL-3.0-or-later
# Production image (amd64 and arm64): server binary compiled in release + the interface files (WASM, JS, CSS) produced by
# `cargo leptos build --release`. ffmpeg is in the image (R6 lot 6); ONNX Runtime arrives in lot 8. arm64: never built to date (ADR 0017, ADR 0062).
#
# The server (rendered HTML) and the WASM (hydration) must be compiled with the same settings, otherwise the interface displays
# but stays inert: `.cargo/config.toml` sets them, so it is copied from the first stage (do not remove it).
#
# Dependencies are compiled in separate layers (cargo-chef), for the server AND for the WASM: as long as Cargo.lock and
# the list of crates do not change, a build only recompiles the project's code. The base image already provides Rust 1.99.1
# (that of rust-toolchain.toml): we therefore do not copy that file, to avoid a download by rustup.
FROM rust:1.99.1-slim-trixie AS chef
# `cargo leptos` downloads wasm-bindgen (version of Cargo.lock) and binaryen/wasm-opt on first use: it needs the network and
# certificates; pkg-config and libssl are used to compile cargo-leptos itself.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev curl g++ \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN cargo install cargo-chef --locked --version '^0.1' \
 && cargo install cargo-leptos --locked --version '=0.3.11'
WORKDIR /src

# 1. Plan: list of the dependencies, without the code.
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

# 2. Dependencies (cached layers).
FROM chef AS build
# Metadata of the sqlx queries checked at compile time: the build has no database.
ENV SQLX_OFFLINE=true
COPY .cargo ./.cargo
COPY --from=planner /src/recipe.json recipe.json
# Server.
RUN cargo chef cook --release --locked --recipe-path recipe.json -p dominet
# WebAssembly: same target, same profile and same folder as `cargo leptos build --release`.
RUN cargo chef cook --locked --recipe-path recipe.json -p dominet-web \
      --target wasm32-unknown-unknown --target-dir target/front --profile wasm-release \
      --no-default-features --features hydrate

# 3. Project code: binary + interface files in target/site.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY .sqlx ./.sqlx
# `cargo leptos` downloads wasm-bindgen and wasm-opt from GitHub: this CDN is sometimes slow or drops the connection. The compilation, already done, is
# reused by the following attempts (same container): retrying only costs the download.
RUN for attempt in 1 2 3 4; do cargo leptos build --release && exit 0; echo "cargo leptos build : essai $attempt echoue, nouvel essai dans 20 s" >&2; sleep 20; done; exit 1

# 4. Documentation (MkDocs, ADR 0056): throw-away stage, only the static site is copied into the final image.
FROM python:3.13-slim-trixie AS docs
WORKDIR /src
COPY requirements-docs.txt ./
RUN pip install --no-cache-dir -r requirements-docs.txt
COPY mkdocs.yml DEPLOY.md DEPLOY.fr.md README.md README.fr.md ./
COPY docs ./docs
# Served on /docs/: the language switch of the theme needs the path (see mkdocs.yml).
RUN DOCS_SITE_URL=http://dominet.local/docs/ NO_MKDOCS_2_WARNING=1 mkdocs build --strict

FROM debian:trixie-slim
LABEL org.opencontainers.image.source="https://github.com/deunix-educ/domia-automation" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.description="Self-hosted home automation hub"
# Root certificates: ReductStore's HTTP client (rustls) refuses to build without them, and the HTTPS calls (weather) need them.
# postgresql-client: `dominet backup` / `dominet restore` (pg_dump, psql; ADR 0054).
# ffmpeg: the `video` role decodes the MediaMTX streams (ADR 0003 and 0048). The snapshots folder (a volume in production) belongs to the application's user.
# intel-media-va-driver + vainfo: VAAPI hardware decoding (Intel Gen8+, e.g. HD 530) when /dev/dri is passed to the container (VIDEO_DEVICE, see DEPLOY.md).
# These two packages only exist for amd64 in Debian: on arm64 (Raspberry Pi) they are omitted and the video is decoded by the processor.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg libstdc++6 postgresql-client \
 && if [ "$(dpkg --print-architecture)" = amd64 ]; then apt-get install -y --no-install-recommends intel-media-va-driver vainfo; fi \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home dominet \
 && mkdir -p /data/snapshots /data/detections /recordings /backups /prometheus-rules /grafana-alerting /grafana-dashboards && chown -R dominet /data /recordings /backups /prometheus-rules /grafana-alerting /grafana-dashboards
COPY --from=build /src/target/release/dominet /usr/local/bin/dominet
COPY --from=build /src/target/site /app/site
COPY --from=docs /src/target/docs-site /app/docs
# Detection model (make fetch-model before docker build, ADR 0050).
COPY models/yolov8n.onnx /models/yolov8n.onnx
# /recordings: volume shared with MediaMTX (fMP4 segments), in production.
ENV SITE_ROOT=/app/site DOCS_ROOT=/app/docs SNAPSHOTS_DIR=/data/snapshots RECORDINGS_DIR=/recordings YOLO_MODEL_PATH=/models/yolov8n.onnx DETECTIONS_DIR=/data/detections
USER 10001
EXPOSE 8000
ENTRYPOINT ["dominet"]
CMD ["serve"]
