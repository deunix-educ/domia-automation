# SPDX-License-Identifier: AGPL-3.0-or-later
# Image de production (amd64 et arm64) : binaire serveur compile en release + fichiers de l'interface (WASM, JS, CSS) produits par
# `cargo leptos build --release`. ffmpeg est dans l'image (R6 lot 6) ; ONNX Runtime arrive au lot 8. arm64 : jamais construite a ce jour (ADR 0017, ADR 0062).
#
# Le serveur (HTML rendu) et le WASM (hydratation) doivent etre compiles avec les memes reglages, sinon l'interface s'affiche
# mais reste inerte : `.cargo/config.toml` les fixe, il est donc copie des la premiere etape (ne pas le retirer).
#
# Les dependances sont compilees dans des couches a part (cargo-chef), pour le serveur ET pour le WASM : tant que Cargo.lock et
# la liste des crates ne changent pas, un build ne recompile que le code du projet. L'image de base fournit deja Rust 1.99.1
# (celui de rust-toolchain.toml) : on ne copie donc pas ce fichier, pour eviter un telechargement par rustup.
FROM rust:1.99.1-slim-trixie AS chef
# `cargo leptos` telecharge au premier usage wasm-bindgen (version de Cargo.lock) et binaryen/wasm-opt : il faut le reseau et des
# certificats ; pkg-config et libssl servent a compiler cargo-leptos lui-meme.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates pkg-config libssl-dev curl g++ \
 && rm -rf /var/lib/apt/lists/*
RUN rustup target add wasm32-unknown-unknown
RUN cargo install cargo-chef --locked --version '^0.1' \
 && cargo install cargo-leptos --locked --version '=0.3.11'
WORKDIR /src

# 1. Plan : liste des dependances, sans le code.
FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY .cargo ./.cargo
COPY crates ./crates
RUN cargo chef prepare --recipe-path recipe.json

# 2. Dependances (couches mises en cache).
FROM chef AS build
# Metadonnees des requetes sqlx verifiees a la compilation : le build n'a pas de base de donnees.
ENV SQLX_OFFLINE=true
COPY .cargo ./.cargo
COPY --from=planner /src/recipe.json recipe.json
# Serveur.
RUN cargo chef cook --release --locked --recipe-path recipe.json -p dominet
# WebAssembly : meme cible, meme profil et meme dossier que `cargo leptos build --release`.
RUN cargo chef cook --locked --recipe-path recipe.json -p dominet-web \
      --target wasm32-unknown-unknown --target-dir target/front --profile wasm-release \
      --no-default-features --features hydrate

# 3. Code du projet : binaire + fichiers de l'interface dans target/site.
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY .sqlx ./.sqlx
# `cargo leptos` telecharge wasm-bindgen et wasm-opt depuis GitHub : ce CDN est parfois lent ou coupe la connexion. La compilation, deja faite, est
# reutilisee par les essais suivants (meme conteneur) : reessayer ne coute que le telechargement.
RUN for attempt in 1 2 3 4; do cargo leptos build --release && exit 0; echo "cargo leptos build : essai $attempt echoue, nouvel essai dans 20 s" >&2; sleep 20; done; exit 1

# 4. Documentation (MkDocs, ADR 0056) : etape jetee, seul le site statique est copie dans l'image finale.
FROM python:3.13-slim-trixie AS docs
WORKDIR /src
COPY requirements-docs.txt ./
RUN pip install --no-cache-dir -r requirements-docs.txt
COPY mkdocs.yml DEPLOY.md DEPLOY.fr.md README.md README.fr.md ./
COPY docs ./docs
RUN NO_MKDOCS_2_WARNING=1 mkdocs build --strict

FROM debian:trixie-slim
LABEL org.opencontainers.image.source="https://github.com/deunix-educ/domia-automation" \
      org.opencontainers.image.licenses="AGPL-3.0-or-later" \
      org.opencontainers.image.description="Self-hosted home automation hub"
# Certificats racine : le client HTTP de ReductStore (rustls) refuse de se construire sans eux, et les appels HTTPS (meteo) en ont besoin.
# postgresql-client : `dominet backup` / `dominet restore` (pg_dump, psql ; ADR 0054).
# ffmpeg : le role `video` decode les flux de MediaMTX (ADR 0003 et 0048). Le dossier des snapshots (volume en production) appartient a l'utilisateur de l'application.
# intel-media-va-driver + vainfo : decodage materiel VAAPI (Intel Gen8+, p. ex. HD 530) quand /dev/dri est passe au conteneur (VIDEO_DEVICE, voir DEPLOY.md).
# Ces deux paquets n'existent que pour amd64 dans Debian : sur arm64 (Raspberry Pi) ils sont omis et la video est decodee par le processeur.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg libstdc++6 postgresql-client \
 && if [ "$(dpkg --print-architecture)" = amd64 ]; then apt-get install -y --no-install-recommends intel-media-va-driver vainfo; fi \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --no-create-home dominet \
 && mkdir -p /data/snapshots /data/detections /recordings /backups /prometheus-rules /grafana-alerting /grafana-dashboards && chown -R dominet /data /recordings /backups /prometheus-rules /grafana-alerting /grafana-dashboards
COPY --from=build /src/target/release/dominet /usr/local/bin/dominet
COPY --from=build /src/target/site /app/site
COPY --from=docs /src/target/docs-site /app/docs
# Modele de detection (make fetch-model avant docker build, ADR 0050).
COPY models/yolov8n.onnx /models/yolov8n.onnx
# /recordings : volume commun avec MediaMTX (segments fMP4), en production.
ENV SITE_ROOT=/app/site DOCS_ROOT=/app/docs SNAPSHOTS_DIR=/data/snapshots RECORDINGS_DIR=/recordings YOLO_MODEL_PATH=/models/yolov8n.onnx DETECTIONS_DIR=/data/detections
USER 10001
EXPOSE 8000
ENTRYPOINT ["dominet"]
CMD ["serve"]
