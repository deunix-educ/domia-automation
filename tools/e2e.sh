#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# E2E navigateur reproductible : demarre un serveur sur une base JETABLE (creee puis supprimee), un Firefox headless a
# profil temporaire, et rejoue les scripts de tests/e2e/ (Marionette).
#
#   tools/e2e.sh                       serveur = target/debug/dominet + target/site (apres `make front`)
#   tools/e2e.sh --image dominet:dev   serveur = l'image Docker (le test qui compte pour livrer)
#   tools/e2e.sh home auth             seulement ces scripts (defaut : tous)
#   SITE_ROOT=/vide tools/e2e.sh --image I home   verifie que le test echoue bien avec une image sans fichiers statiques
#
# Prerequis : stack de dev (`make dev-up`, PostgreSQL 15432) et Firefox. Aucun contact avec la production : ni Mosquitto, ni
# cameras, ni Zigbee. Tout ce qui est cree (base, profil, conteneur, processus) est supprime a la fin, meme en cas d'echec.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
IMAGE=""
SCRIPTS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --image) IMAGE="$2"; shift 2 ;;
    *) SCRIPTS+=("$1"); shift ;;
  esac
done
[ ${#SCRIPTS[@]} -gt 0 ] || SCRIPTS=(home auth layout locations admin users realtime system dashboard dashboard_edit r2 mqtt mqtt_sources devices scenes automations alerts energy weather android messaging cameras onvif ptz live recordings events rules mcp mqtt_outage)

PG_CONTAINER="${PG_CONTAINER:-dominet-dev-postgres-1}"
PG_PORT="${PG_PORT:-15432}"
DB="dominet_e2e_$$"
NAME="dominet-e2e-$$"
WORK="$(mktemp -d)"
chmod 700 "$WORK"
PORT="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
BASE="http://127.0.0.1:$PORT/"
export DATABASE_URL="postgres://dominet:dominet-dev@127.0.0.1:${PG_PORT}/${DB}"
# Commande qui lit du SQL sur son entree standard, sur la base jetable (pour preparer des donnees que l'interface ne sait pas encore creer).
export E2E_DB_SHELL="docker exec -i $PG_CONTAINER psql -q -U dominet -d $DB -v ON_ERROR_STOP=1"
PASSWORD="$(head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 16)"
ADMIN=e2eadmin
SERVER_PID=""
FF_PID=""

psql_admin() { docker exec "$PG_CONTAINER" psql -q -U dominet -d postgres -v ON_ERROR_STOP=1 -c "$1" >/dev/null; }

cleanup() {
  set +e
  [ -n "$FF_PID" ] && kill "$FF_PID" 2>/dev/null
  for f in "$WORK/sim-meteo.pid" "$WORK/sim-sun.pid" "$WORK/sim-smtp.pid" "$WORK/sim-onvif.pid" "$WORK/sim-prom.pid"; do [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null; done
  [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
  [ -n "$IMAGE" ] && docker rm -f "$NAME" >/dev/null 2>&1
  curl -s -X DELETE -H "Authorization: Bearer ${REDUCTSTORE_TOKEN}" "${REDUCTSTORE_URL}/api/v1/b/${REDUCTSTORE_IOT_BUCKET}" >/dev/null 2>&1
  curl -s -X DELETE -H "Authorization: Bearer ${REDUCTSTORE_TOKEN}" "${REDUCTSTORE_URL}/api/v1/b/${REDUCTSTORE_VIDEO_BUCKET}" >/dev/null 2>&1
  psql_admin "DROP DATABASE IF EXISTS ${DB} WITH (FORCE)" 2>/dev/null
  rm -rf "$WORK"
}
trap cleanup EXIT

docker exec "$PG_CONTAINER" pg_isready -U dominet >/dev/null 2>&1 || { echo "stack de dev absente : make dev-up" >&2; exit 1; }
if python3 -c 'import socket,sys; s=socket.socket(); sys.exit(0 if s.connect_ex(("127.0.0.1",2828))==0 else 1)'; then
  echo "le port Marionette 2828 est deja pris (un autre Firefox pilote ?) : l'arreter d'abord" >&2
  exit 1
fi

# Binaire et fichiers de l'interface (DOMINET_BIN=target-rel/release/dominet pour essayer un autre profil de compilation).
BIN="${DOMINET_BIN:-target/debug/dominet}"
SITE="${DOMINET_SITE:-$(dirname "$(dirname "$BIN")")/site}"
[ -n "$IMAGE" ] || export SITE_ROOT="$SITE"

# Cle maitresse jetable (les scripts enregistrent un secret).
if [ -n "$IMAGE" ]; then
  KEY="$(docker run --rm "$IMAGE" gen-master-key 2>/dev/null | tail -1)"
else
  [ -x "$BIN" ] && [ -f "$SITE/pkg/dominet.wasm" ] || { echo "binaire ou interface absents : make front" >&2; exit 1; }
  KEY="$($BIN gen-master-key 2>/dev/null | tail -1)"
fi
export DOMINET_MASTER_KEY="$KEY" COOKIE_SECURE=false HTTP_ADDR="127.0.0.1:${PORT}" SYSTEM_STATS_SECS="${SYSTEM_STATS_SECS:-1}"
# Serveur MCP allume pour le script `mcp` (coupe par defaut ; sans jeton cree dans l'administration, personne n'y entre).
export MCP_ENABLED=true
export RUST_BACKTRACE=1
# Le serveur ecoute le Mosquitto de dev (stack `make dev-up`, port 11883) sous un identifiant de client propre a ce lancement.
export MQTT_HOST="${MQTT_HOST:-127.0.0.1}" MQTT_PORT="${MQTT_PORT:-11883}" MQTT_CLIENT_ID="dominet-e2e-$$"
# Archive brute : bucket jetable du ReductStore de dev (port 18383), supprime a la fin.
export REDUCTSTORE_URL="${REDUCTSTORE_URL:-http://127.0.0.1:18383}" REDUCTSTORE_TOKEN="${REDUCTSTORE_TOKEN:-dominet-dev-token}" REDUCTSTORE_IOT_BUCKET="e2e-iot-$$" REDUCTSTORE_VIDEO_BUCKET="e2e-video-$$"
# API Open-Meteo et onduleur SolarMax SIMULES (boucle locale, ports libres) : aucun appel reel, jamais l'onduleur de la production.
free_port() { python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'; }
METEO_PORT="$(free_port)"; SUN_PORT="$(free_port)"
python3 tools/sim-open-meteo.py "$METEO_PORT" > "$WORK/sim-meteo.log" 2>&1 & echo $! > "$WORK/sim-meteo.pid"
python3 tools/sim-solarmax.py "$SUN_PORT" > "$WORK/sim-sun.log" 2>&1 & echo $! > "$WORK/sim-sun.pid"
# Serveur SMTP SIMULE : les e-mails de la messagerie y sont ecrits (une ligne JSON chacun) et ne sortent jamais de la machine.
SMTP_PORT="$(free_port)"; : > "$WORK/mails.jsonl"
python3 tools/sim-smtp.py "$SMTP_PORT" "$WORK/mails.jsonl" > "$WORK/sim-smtp.log" 2>&1 & echo $! > "$WORK/sim-smtp.pid"
# Prometheus SIMULE, seulement pour le script `platform_alerts` (R13) : sinon la pastille compterait ses alertes dans les autres scripts.
if printf '%s\n' "${SCRIPTS[@]}" | grep -qx platform_alerts; then
  PROM_PORT="$(free_port)"
  python3 tools/sim-prometheus.py "$PROM_PORT" > "$WORK/sim-prom.log" 2>&1 & echo $! > "$WORK/sim-prom.pid"
  export PROMETHEUS_URL="http://127.0.0.1:${PROM_PORT}" PROMETHEUS_ALERTS_POLL_SECS=2 E2E_PROM_MODE_URL="http://127.0.0.1:${PROM_PORT}/__mode"
fi
# Camera ONVIF simulee (famille NVT : service sur un port quelconque, URL de flux avec mot de passe en clair).
ONVIF_PORT="$(free_port)"
python3 tools/sim-onvif.py "$ONVIF_PORT" nvt admin e2e-onvif-secret "$WORK/onvif-commands.log" > "$WORK/sim-onvif.log" 2>&1 & echo $! > "$WORK/sim-onvif.pid"
export E2E_ONVIF_PORT="$ONVIF_PORT" E2E_ONVIF_PASSWORD=e2e-onvif-secret E2E_ONVIF_LOG="$WORK/onvif-commands.log"
export EMAIL_HOST=127.0.0.1 EMAIL_PORT="$SMTP_PORT" EMAIL_USE_TLS=false E2E_SMTP_FILE="$WORK/mails.jsonl"
export OPEN_METEO_URL="http://127.0.0.1:${METEO_PORT}/v1/forecast" E2E_METEO_COUNT_URL="http://127.0.0.1:${METEO_PORT}/__count"
export SOLARMAX_HOST=127.0.0.1 SOLARMAX_PORT="$SUN_PORT" SOLARMAX_TOPIC="e2e-sun-$$/inv/prod" SOLARMAX_INTERVAL_SECS=2
export E2E_SOLARMAX_TOPIC="$SOLARMAX_TOPIC"
# Tableaux Grafana des appareils : dossier jetable (monte tel quel dans le conteneur en mode image).
export GRAFANA_DASHBOARDS_DIR="$WORK/grafana" E2E_GRAFANA_DIR="$WORK/grafana"
mkdir -p "$WORK/grafana" && chmod 777 "$WORK/grafana"
# Regles d'alerte (R7 lot 2) : fichiers de provisionnement jetables, sans adresse de service (rechargement « a faire a la main »).
# /metrics n'est pas expose (le port 9100 pourrait servir a un autre processus de la machine).
export PROMETHEUS_RULES_PATH="$WORK/rules/rules.yml" GRAFANA_ALERTING_PATH="$WORK/rules/rules.yaml" METRICS_ADDR=off
export E2E_PROMETHEUS_RULES="$PROMETHEUS_RULES_PATH" E2E_GRAFANA_RULES="$GRAFANA_ALERTING_PATH"
mkdir -p "$WORK/rules" && chmod 777 "$WORK/rules"
# Role video : relit le re-stream du MediaMTX de dev (publie sur 18554 de la machine) ; images et snapshots dans un dossier jetable.
export MEDIAMTX_RTSP_URL="${MEDIAMTX_RTSP_URL:-rtsp://127.0.0.1:18554}" SNAPSHOTS_DIR="$WORK/snapshots" E2E_SNAPSHOTS_DIR="$WORK/snapshots"
mkdir -p "$WORK/snapshots" && chmod 777 "$WORK/snapshots"
# Enregistrements : le dossier ou le MediaMTX de dev ecrit ses segments (make dev-up) ; passages rapides, segments de 10 s.
# Mouvement (evalue toutes les 5 s) et detection : modele de `make fetch-model` (dans l'image : /models), images annotees dans un dossier jetable.
export MOTION_INTERVAL_SECS=5 DETECTIONS_DIR="$WORK/detections"
mkdir -p "$WORK/detections" && chmod 777 "$WORK/detections"
if [ -n "$IMAGE" ]; then export YOLO_MODEL_PATH=/models/yolov8n.onnx; else export YOLO_MODEL_PATH="$ROOT/models/yolov8n.onnx"; fi
export E2E_YOLO_MODEL="$ROOT/models/yolov8n.onnx" E2E_BUS_IMAGE="$ROOT/crates/dominet-surveillance/fixtures/yolo/bus.jpg"
export RECORDINGS_DIR="${RECORDINGS_DIR:-$ROOT/deploy/recordings-dev}" RECORDINGS_SEGMENT_DURATION=10 RECORDINGS_SCAN_SECS=5 E2E_RECORDINGS_DIR="${RECORDINGS_DIR:-$ROOT/deploy/recordings-dev}"
# MediaMTX de la stack de dev (jamais celui de la production) : les cameras de test y creent leurs paths, puis les retirent.
export MEDIAMTX_API_URL="${MEDIAMTX_API_URL:-http://127.0.0.1:19997}"
export RUST_LOG="${RUST_LOG:-info}"

# CLI : le binaire local, ou l'image sur le reseau de l'hote (pour joindre la base de dev).
dom() {
  if [ -n "$IMAGE" ]; then
    docker run --rm --network host -e DATABASE_URL -e DOMINET_MASTER_KEY -e DOMINET_PASSWORD -e COOKIE_SECURE -e OPEN_METEO_URL -e SOLARMAX_HOST -e SOLARMAX_PORT -e SOLARMAX_TOPIC -e EMAIL_HOST -e EMAIL_PORT -e EMAIL_USE_TLS "$IMAGE" "$@"
  else
    "$BIN" "$@"
  fi
}

start_server() {
  if [ -n "$IMAGE" ]; then
    docker rm -f "$NAME" >/dev/null 2>&1 || true
    docker run -d --name "$NAME" --network host -e DATABASE_URL -e DOMINET_MASTER_KEY -e COOKIE_SECURE -e MCP_ENABLED -e HTTP_ADDR -e SYSTEM_STATS_SECS -e RUST_BACKTRACE -e RUST_LOG -e MQTT_HOST -e MQTT_PORT -e MQTT_CLIENT_ID -e REDUCTSTORE_URL -e REDUCTSTORE_TOKEN -e REDUCTSTORE_IOT_BUCKET -e REDUCTSTORE_VIDEO_BUCKET -e RECORDINGS_DIR -e RECORDINGS_SEGMENT_DURATION -e RECORDINGS_SCAN_SECS -e MOTION_INTERVAL_SECS -e DETECTIONS_DIR -e YOLO_MODEL_PATH -e GRAFANA_DASHBOARDS_DIR -e PROMETHEUS_RULES_PATH -e GRAFANA_ALERTING_PATH -e METRICS_ADDR -e OPEN_METEO_URL -e SOLARMAX_HOST -e SOLARMAX_PORT -e SOLARMAX_TOPIC -e SOLARMAX_INTERVAL_SECS -e EMAIL_HOST -e EMAIL_PORT -e EMAIL_USE_TLS -e MEDIAMTX_API_URL -e MEDIAMTX_RTSP_URL -e SNAPSHOTS_DIR -e PROMETHEUS_URL -e PROMETHEUS_ALERTS_POLL_SECS -v "$WORK/snapshots:$WORK/snapshots" -v "$RECORDINGS_DIR:$RECORDINGS_DIR" -v "$WORK/detections:$WORK/detections" -v "$ROOT/models:/models:ro" -v "$WORK/grafana:$WORK/grafana" -v "$WORK/rules:$WORK/rules" ${SITE_ROOT:+-e SITE_ROOT} "$IMAGE" serve >/dev/null
  else
    ("$BIN" serve > "$WORK/server.log" 2>&1 & echo $! > "$WORK/server.pid")
    SERVER_PID="$(cat "$WORK/server.pid")"
  fi
  for _ in $(seq 1 100); do curl -fs "${BASE}healthz" >/dev/null 2>&1 && return 0; sleep 0.2; done
  echo "le serveur ne repond pas" >&2
  [ -n "$IMAGE" ] && docker logs "$NAME" >&2 || cat "$WORK/server.log" >&2
  return 1
}

psql_admin "CREATE DATABASE ${DB}"
dom migrate >/dev/null
dom seed | tail -1
DOMINET_PASSWORD="$PASSWORD" dom createsuperuser --username "$ADMIN" >/dev/null
start_server

# Commande de redemarrage donnee a realtime.py (les sessions sont en base : elles survivent).
cat > "$WORK/restart.sh" <<RESTART
#!/usr/bin/env bash
set -e
if [ -n "$IMAGE" ]; then
  docker restart "$NAME" >/dev/null
else
  kill \$(cat "$WORK/server.pid") 2>/dev/null || true
  sleep 1
  (cd "$ROOT" && DATABASE_URL="$DATABASE_URL" DOMINET_MASTER_KEY="$DOMINET_MASTER_KEY" COOKIE_SECURE=false MCP_ENABLED=true HTTP_ADDR="$HTTP_ADDR" SYSTEM_STATS_SECS=1 MQTT_HOST="$MQTT_HOST" MQTT_PORT="$MQTT_PORT" MQTT_CLIENT_ID="$MQTT_CLIENT_ID" \
     REDUCTSTORE_URL="$REDUCTSTORE_URL" REDUCTSTORE_TOKEN="$REDUCTSTORE_TOKEN" REDUCTSTORE_IOT_BUCKET="$REDUCTSTORE_IOT_BUCKET" REDUCTSTORE_VIDEO_BUCKET="$REDUCTSTORE_VIDEO_BUCKET" RECORDINGS_DIR="$RECORDINGS_DIR" RECORDINGS_SEGMENT_DURATION=10 RECORDINGS_SCAN_SECS=5 GRAFANA_DASHBOARDS_DIR="$GRAFANA_DASHBOARDS_DIR" OPEN_METEO_URL="$OPEN_METEO_URL" SOLARMAX_HOST="$SOLARMAX_HOST" SOLARMAX_PORT="$SOLARMAX_PORT" SOLARMAX_TOPIC="$SOLARMAX_TOPIC" SOLARMAX_INTERVAL_SECS=2 EMAIL_HOST="$EMAIL_HOST" EMAIL_PORT="$EMAIL_PORT" EMAIL_USE_TLS=false MEDIAMTX_API_URL="$MEDIAMTX_API_URL" MEDIAMTX_RTSP_URL="$MEDIAMTX_RTSP_URL" SNAPSHOTS_DIR="$SNAPSHOTS_DIR" PROMETHEUS_URL="${PROMETHEUS_URL:-}" PROMETHEUS_ALERTS_POLL_SECS="${PROMETHEUS_ALERTS_POLL_SECS:-}" \
     "$BIN" serve >> "$WORK/server.log" 2>&1 & echo \$! > "$WORK/server.pid")
fi
for _ in \$(seq 1 100); do curl -fs "${BASE}healthz" >/dev/null 2>&1 && exit 0; sleep 0.2; done
exit 1
RESTART
chmod +x "$WORK/restart.sh"

mkdir -p "$WORK/ff"
(firefox --headless --marionette --profile "$WORK/ff" -no-remote about:blank > "$WORK/ff.log" 2>&1 & echo $! > "$WORK/ff.pid")
FF_PID="$(cat "$WORK/ff.pid")"
for _ in $(seq 1 100); do
  python3 -c 'import socket,sys; s=socket.socket(); sys.exit(0 if s.connect_ex(("127.0.0.1",2828))==0 else 1)' && break
  sleep 0.3
done

echo "E2E sur $([ -n "$IMAGE" ] && echo "l'image $IMAGE" || echo "$BIN") : $BASE (base $DB)"
failed=()
for s in "${SCRIPTS[@]}"; do
  args=("$BASE" "$ADMIN" "$PASSWORD")
  [ "$s" = realtime ] && args+=("$WORK/restart.sh")
  if python3 "tests/e2e/$s.py" "${args[@]}" > "$WORK/$s.out" 2>&1 && grep -q '=> 0 echec' "$WORK/$s.out"; then
    echo "  OK    $s ($(grep -c '^PASS' "$WORK/$s.out") verifications)"
  else
    echo "  ECHEC $s"
    grep -E 'FAIL|Traceback|Error|DIAGNOSTIC' -A3 "$WORK/$s.out" | head -20
    echo "  --- fin du journal du serveur ---"
    if [ -n "$IMAGE" ]; then docker logs "$NAME" > "$WORK/server.log" 2>&1; fi
    # Un panic serveur est la cause la plus probable d'une page qui ne repond plus : on montre son debut, pas la queue.
    if grep -q panicked "$WORK/server.log"; then grep -m2 -A70 panicked "$WORK/server.log" | cut -c1-240; else tail -25 "$WORK/server.log" | cut -c1-300; fi
    [ -n "${KEEP_SERVER_LOG:-}" ] && cp "$WORK/server.log" "$KEEP_SERVER_LOG"
    failed+=("$s")
  fi
done
# Commandes de diagnostic contre les simulateurs : un cycle SolarMax sans publier, et l'instantane de contexte.
echo "  --- diagnostics (simulateurs) ---"
if dom publish solarmax --print-only 2>&1 | grep -q '"pac": 632.5'; then echo "  OK    dominet publish solarmax --print-only"; else echo "  ECHEC publish solarmax"; failed+=("publish-solarmax"); fi
if dom dump-context 2>&1 | grep -q '^devices:'; then echo "  OK    dominet dump-context"; else echo "  ECHEC dump-context"; failed+=("dump-context"); fi
if [ ${#failed[@]} -gt 0 ]; then
  echo "E2E : ECHEC (${failed[*]})"
  exit 1
fi
echo "E2E : tout est vert"
