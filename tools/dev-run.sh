#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Lance DomiNet en developpement, en une commande, contre la stack de dev jetable (`make dev-up`).
#
#   tools/dev-run.sh                 serveur web + planificateur (+ MQTT de dev), sur http://127.0.0.1:8080
#   tools/dev-run.sh --discovery     idem, plus le role `discovery` (WS-Discovery multicast sur le reseau local)
#   tools/dev-run.sh --discovery-only  seulement le role `discovery`, a lancer dans un SECOND terminal (il ne sonde le reseau que
#                                    quand on clique sur « Lancer la decouverte » dans l'interface)
#   DEV_PORT=9000 tools/dev-run.sh   autre port
#   RELEASE=1 tools/dev-run.sh       interface optimisee (cargo leptos build --release) : ~4 Mo de WASM au lieu de ~48 Mo, la page
#                                    reagit en une ou deux secondes ; la premiere compilation prend plusieurs minutes
#
# Au premier lancement : cle maitresse et compte administrateur sont crees et gardes dans deploy/.env.dev-run (ignore par git, chmod 600).
# Aucun contact avec la production : PostgreSQL, MQTT, ReductStore et MediaMTX sont ceux de la stack de dev.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
DISCOVERY=0
DISCOVERY_ONLY=0
for arg in "$@"; do
  case "$arg" in
    --discovery) DISCOVERY=1 ;;
    --discovery-only) DISCOVERY_ONLY=1 ;;
    -h|--help) sed -n '3,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "option inconnue : $arg (voir --help)" >&2; exit 2 ;;
  esac
done

if [ -n "${RELEASE:-}" ]; then PROFILE=release; else PROFILE=debug; fi
BIN="${DOMINET_BIN:-target/$PROFILE/dominet}"
SITE="${DOMINET_SITE:-target/site}"
ENV_FILE="deploy/.env.dev-run"
PORT="${DEV_PORT:-8080}"
PG_PORT="${PG_PORT:-15432}"

# The dev stack must be running.
if ! (exec 3<>"/dev/tcp/127.0.0.1/$PG_PORT") 2>/dev/null; then
  echo "PostgreSQL de dev injoignable sur 127.0.0.1:$PG_PORT : lancer d'abord  make dev-up" >&2
  exit 1
fi

# Binary and interface (WASM): built if missing; `make front` rebuilds them after a change.
# The development WASM is huge (~48 MB unoptimised): as long as it is not loaded and compiled, the buttons
# do not react. RELEASE=1 brings it down to a few MB.
if [ -n "${RELEASE:-}" ]; then
  if [ ! -x "$BIN" ] || [ "$(stat -c %s "$SITE/pkg/dominet.wasm" 2>/dev/null || echo 0)" -gt 20000000 ]; then
    echo "dev-run : construction optimisee (cargo leptos build --release), plusieurs minutes la premiere fois..."
    cargo leptos build --release
  fi
elif [ ! -x "$BIN" ] || [ ! -f "$SITE/pkg/dominet.wasm" ]; then
  echo "dev-run : binaire ou interface absents, construction (make front)..."
  make front
fi

# Master key and administrator account: created once, kept afterwards.
if [ ! -f "$ENV_FILE" ]; then
  umask 077
  {
    echo "# Genere par tools/dev-run.sh : acces de DEVELOPPEMENT uniquement (ignore par git)."
    echo "DOMINET_MASTER_KEY=$("$BIN" gen-master-key 2>/dev/null | tail -1)"
    echo "DEV_ADMIN_USER=admin"
    echo "DEV_ADMIN_PASSWORD=$(head -c 18 /dev/urandom | base64 | tr -d '/+=' | head -c 20)"
  } > "$ENV_FILE"
  chmod 600 "$ENV_FILE"
  FIRST_RUN=1
else
  FIRST_RUN=0
fi
set -a
# shellcheck disable=SC1090
. "$ENV_FILE"
set +a

export DATABASE_URL="${DATABASE_URL:-postgres://dominet:dominet-dev@127.0.0.1:$PG_PORT/dominet}"
export COOKIE_SECURE=false
export HTTP_ADDR="127.0.0.1:$PORT"
export SITE_ROOT="$SITE"
export MEDIAMTX_API_URL="${MEDIAMTX_API_URL:-http://127.0.0.1:19997}"
# Video role: re-reads the dev MediaMTX's re-stream; images and snapshots in target/ (ignored by git).
export MEDIAMTX_RTSP_URL="${MEDIAMTX_RTSP_URL:-rtsp://127.0.0.1:18554}" SNAPSHOTS_DIR="${SNAPSHOTS_DIR:-$ROOT/target/dev-snapshots}"
export MQTT_HOST="${MQTT_HOST:-127.0.0.1}" MQTT_PORT="${MQTT_PORT:-11883}" MQTT_CLIENT_ID="${MQTT_CLIENT_ID:-dominet-dev-run}"
# Recordings: the folder where the dev MediaMTX writes its segments (make dev-up).
export RECORDINGS_DIR="${RECORDINGS_DIR:-$ROOT/deploy/recordings-dev}" RECORDINGS_SEGMENT_DURATION="${RECORDINGS_SEGMENT_DURATION:-10}" RECORDINGS_SCAN_SECS="${RECORDINGS_SCAN_SECS:-10}"
export REDUCTSTORE_URL="${REDUCTSTORE_URL:-http://127.0.0.1:18383}" REDUCTSTORE_TOKEN="${REDUCTSTORE_TOKEN:-dominet-dev-token}"
export RUST_LOG="${RUST_LOG:-info}"

if [ "$DISCOVERY_ONLY" = 1 ]; then
  [ "$FIRST_RUN" = 0 ] || { echo "lancer d'abord  make dev-run  (premier lancement : base et compte a creer)" >&2; exit 1; }
  echo "dev-run : role discovery seul (Ctrl+C pour arreter). Cliquer sur « Lancer la decouverte » dans l'interface pour sonder le reseau."
  exec "$BIN" serve --only discovery
fi

"$BIN" migrate > /dev/null
"$BIN" seed | tail -1
# The browser calls MediaMTX (live) on the MEDIAMTX_WEBRTC_DOMAIN setting's domain: the default (localhost:8889) is not the dev stack's port
# (18889). Only replaces the default value; a setting chosen in the interface is not touched.
docker exec -i "${PG_CONTAINER:-dominet-dev-postgres-1}" psql -q -U dominet -d dominet -c \
  "UPDATE app_setting SET value = '127.0.0.1:18889' WHERE key = 'MEDIAMTX_WEBRTC_DOMAIN' AND value = 'localhost:8889'" > /dev/null 2>&1 || true
# The account may already exist (re-run): this is not an error.
if DOMINET_PASSWORD="$DEV_ADMIN_PASSWORD" "$BIN" createsuperuser --username "$DEV_ADMIN_USER" > /dev/null 2>&1; then
  echo "dev-run : compte administrateur \"$DEV_ADMIN_USER\" cree."
fi

echo
echo "  DomiNet (developpement)  http://127.0.0.1:$PORT"
echo "  Compte : $DEV_ADMIN_USER   Mot de passe : voir $ENV_FILE (DEV_ADMIN_PASSWORD)"
[ "$FIRST_RUN" = 1 ] && echo "  (premier lancement : acces crees dans $ENV_FILE)"
echo "  MediaMTX : $MEDIAMTX_API_URL    Arret : Ctrl+C"
[ -z "${RELEASE:-}" ] && echo "  Version de developpement : la page met 10 a 30 s a reagir (WASM de ~48 Mo). RELEASE=1 make dev-run pour l'accelerer."
echo

if [ "$DISCOVERY" = 1 ]; then
  # `discovery` role in a second process (it listens on no port); stopped with the server.
  "$BIN" serve --only discovery &
  DISCOVERY_PID=$!
  trap 'kill "$DISCOVERY_PID" 2>/dev/null || true' EXIT INT TERM
  "$BIN" serve
else
  exec "$BIN" serve
fi
