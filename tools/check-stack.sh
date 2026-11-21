#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Controle de la pile Compose de production (R7 lots 5 et 6) sur la machine de DEV, sans toucher a rien d'autre : projet Compose a part
# (dominet-chk), ports decales, secrets jetables, certificat auto-signe, volumes detruits a la fin. Verifie : tous les services sains,
# nginx en TLS (application, WebSocket, Grafana, WebRTC, CA), MQTT (mot de passe / LAN), Prometheus qui scrute DomiNet, regles partagees,
# Grafana provisionne, sauvegarde. Aucun contact avec la production.
# Usage : tools/check-stack.sh [image]   (defaut dominet:local ; l'image doit exister : docker build -t dominet:local .)
# Prerequis : la stack de dev arretee (le port ICE 8189 est unique) -> make dev-down
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
IMAGE="${1:-dominet:local}"
TAG="${IMAGE#*:}"
P=dominet-chk
W="$(mktemp -d /tmp/dominet-chk-XXXXXX)"
HTTPS=18443; HTTP=18080; PG=15440; MQTT=11884
fails=0
ok() { echo "  OK    $1"; }
ko() { echo "  ECHEC $1"; fails=$((fails + 1)); }
check() { if eval "$2" >/dev/null 2>&1; then ok "$1"; else ko "$1"; fi; }
rand() { head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 24; }

docker image inspect "$IMAGE" >/dev/null 2>&1 || { echo "image $IMAGE absente" >&2; exit 2; }
if docker ps --format '{{.Ports}}' | grep -q ':8189->'; then echo "le port 8189 est pris (make dev-down d'abord)" >&2; exit 2; fi

mkdir -p "$W/certs"
openssl req -x509 -newkey rsa:2048 -nodes -keyout "$W/certs/key.pem" -out "$W/certs/cert.pem" -days 1 -subj "/CN=localhost" \
  -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" 2>/dev/null
cp "$W/certs/cert.pem" "$W/certs/dominet-ca.crt"
MK="$(docker run --rm "$IMAGE" gen-master-key | tail -1)"
GPW="$(rand)"; DPW="$(rand)"; MPW="$(rand)"
cat > "$W/env" <<ENV
DOMINET_TAG=$TAG
CERTS_DIR=$W/certs
LAN_DOMAIN=localhost
ALLOWED_HOSTS=localhost,127.0.0.1
CSRF_TRUSTED_ORIGINS=https://localhost:$HTTPS,https://127.0.0.1:$HTTPS
GRAFANA_PUBLIC_URL=https://localhost:$HTTPS
DB_PASSWORD=$DPW
REDUCTSTORE_TOKEN=$(rand)
MQTT_PASSWORD=$MPW
GRAFANA_PASSWORD=$GPW
EMAIL_RECIPIENT=controle@example.org
DOMINET_MASTER_KEY=$MK
POSTGRES_HOST_PORT=$PG
NGINX_HTTP_PORT=$HTTP
NGINX_HTTPS_PORT=$HTTPS
MQTT_LAN_PORT=$MQTT
ENV
C=(docker compose -p $P --env-file "$W/env" -f "$ROOT/deploy/docker-compose.yml" -f "$ROOT/deploy/docker-compose.prod.yml")
cleanup() { if [ -n "${KEEP:-}" ]; then echo "pile conservee : projet $P, dossier $W"; return; fi; "${C[@]}" down -v --remove-orphans >/dev/null 2>&1; rm -rf "$W"; }
trap cleanup EXIT

echo "== demarrage (projet $P)"
"${C[@]}" up -d --wait --wait-timeout 240 >"$W/up.log" 2>&1; check "docker compose up --wait : tous les services sont sains" "[ \$? -eq 0 ] && ! grep -qi 'unhealthy\|error' $W/up.log"
tail -3 "$W/up.log" | cut -c1-160
"${C[@]}" ps -a --format '{{.Service}} {{.Status}}' | sort > "$W/ps.txt"; cat "$W/ps.txt" | sed 's/^/    /'
check "migrate et init se sont termines (code 0)" "grep -q '^migrate Exited (0)' $W/ps.txt && grep -q '^init Exited (0)' $W/ps.txt"
for s in dominet postgres reductstore mosquitto mediamtx prometheus grafana nginx dominet-discovery; do
  check "service $s en marche" "grep -q '^$s Up' $W/ps.txt"
done
check "dominet est « healthy »" "grep -q '^dominet Up.*healthy' $W/ps.txt"
check "non-root : dominet tourne sous l'uid 10001" "[ \"\$(${C[*]} exec -T dominet id -u)\" = 10001 ]"

echo "== nginx (TLS)"
B="https://127.0.0.1:$HTTPS"
check "HTTPS /healthz via nginx" "curl -fsk $B/healthz"
check "HTTP redirige vers HTTPS" "[ \"\$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:$HTTP/)\" = 301 ]"
check "/dominet-ca.crt servi en HTTPS" "curl -fsk $B/dominet-ca.crt | grep -q 'BEGIN CERTIFICATE'"
check "/dominet-ca.crt servi aussi en HTTP (appareil sans CA)" "curl -fs http://127.0.0.1:$HTTP/dominet-ca.crt | grep -q 'BEGIN CERTIFICATE'"
check "page d'accueil de l'application (redirige vers la connexion ou la sert)" "curl -fsLk $B/ | grep -qi '<html'"
check "Grafana derriere /grafana/" "curl -fsk $B/grafana/api/health | grep -q ok"
check "Grafana integrable (pas de X-Frame-Options)" "! curl -skI $B/grafana/login | grep -qi '^x-frame-options: deny'"
check "WebRTC : /webrtc/ atteint MediaMTX" "[ \"\$(curl -sk -o /dev/null -w '%{http_code}' $B/webrtc/unknown/whep)\" != 502 ]"

echo "== MQTT"
MQ="${C[*]} exec -T mosquitto mosquitto_pub -h localhost -t t/x -m 1"
check "1883 (Docker) refuse l'anonyme" "! $MQ -p 1883"
check "1883 accepte le mot de passe" "$MQ -p 1883 -u dominet -P $MPW"
check "il n'y a plus de listener anonyme (1884 ferme)" "! $MQ -p 1884"
check "le port LAN est publie sur l'hote ($MQTT)" "(exec 3<>/dev/tcp/127.0.0.1/$MQTT)"
check "LAN depuis l'hote : l'anonyme est refuse" "! ${C[*]} exec -T mosquitto mosquitto_pub -h 127.0.0.1 -p 1883 -t t/x -m 1"
SUB="${C[*]} exec -T mosquitto mosquitto_sub -h localhost -p 1883"
PUB="${C[*]} exec -T mosquitto mosquitto_pub -h localhost -p 1883"
check "compte externe : creation (sujet linky/#, ecriture seule)" "${C[*]} exec -T mosquitto sh /deploy/accounts.sh add linky Essai-linky-123 'linky/#' write"
check "compte externe : publie dans son sujet" "$PUB -u linky -P Essai-linky-123 -t linky/compteur -m 42"
check "compte externe : mauvais mot de passe refuse" "! $PUB -u linky -P faux -t linky/compteur -m 42"
check "compte externe : hors de son sujet, rien n'arrive au compte interne" "( $SUB -u dominet -P $MPW -t 'zigbee2mqtt/#' -C 1 -W 4 >$W/acl.out & sleep 1; $PUB -u linky -P Essai-linky-123 -t zigbee2mqtt/intrus -m x; wait ) ; [ ! -s $W/acl.out ]"
check "compte externe : ecriture seule, la lecture ne livre rien" "$PUB -u dominet -P $MPW -t linky/retour -m y; [ -z \"\$($SUB -u linky -P Essai-linky-123 -t 'linky/#' -C 1 -W 3 2>/dev/null)\" ]"
check "compte externe : suppression, puis refus" "${C[*]} exec -T mosquitto sh /deploy/accounts.sh del linky && ! $PUB -u linky -P Essai-linky-123 -t linky/compteur -m 1"
check "dominet est connecte au broker" "${C[*]} logs dominet 2>&1 | grep -qi 'mqtt'"

echo "== Prometheus / Grafana"
sleep 20
check "Prometheus scrute DomiNet (cible up)" "${C[*]} exec -T prometheus wget -qO- 'http://localhost:9090/api/v1/query?query=up%7Bjob%3D%22dominet%22%7D' | grep -q '\"1\"\\]'"
check "Prometheus scrute MediaMTX (cible up)" "${C[*]} exec -T prometheus wget -qO- 'http://localhost:9090/api/v1/query?query=up%7Bjob%3D%22mediamtx%22%7D' | grep -q '\"1\"\\]'"
"${C[@]}" exec -T dominet dominet seed >"$W/seed.log" 2>&1
"${C[@]}" exec -T dominet dominet sync-rules >"$W/sync.log" 2>&1
check "sync-rules : regles ecrites et rechargement Prometheus demande" "grep -q 'Prometheus : .* rechargement demande' $W/sync.log"
check "Prometheus voit les regles ecrites par DomiNet (volume partage)" "${C[*]} exec -T prometheus wget -qO- http://localhost:9090/api/v1/rules | grep -q DominetDown"
check "sync-rules : rechargement Grafana demande" "grep -q 'Grafana' $W/sync.log && ! grep -q 'ECHOUE' $W/sync.log"
GA="admin:$GPW"
check "Grafana : source PostgreSQL saine" "curl -fsk -u $GA -X GET $B/grafana/api/datasources/uid/postgresql/health | grep -qi ok"
check "Grafana : tableaux provisionnes (6)" "[ \"\$(curl -sk -u $GA '$B/grafana/api/search?type=dash-db' | python3 -c 'import sys,json; print(len(json.load(sys.stdin)))')\" = 6 ]"

echo "== ReductStore et sauvegarde"
"${C[@]}" exec -T dominet dominet backup >"$W/backup.log" 2>&1; check "dominet backup (base + ReductStore)" "[ -s $W/backup.log ] && ! grep -qi 'erreur' $W/backup.log"
check "les sauvegardes sont dans le volume backups-data" "${C[*]} exec -T dominet dominet backups | grep -q 'sql.gz'"

echo "== utilisateur et connexion"
"${C[@]}" exec -T -e DOMINET_PASSWORD=essai-Chk-123456 dominet dominet createsuperuser --username chk >/dev/null 2>&1
LOGIN="$(curl -sk -c "$W/jar" -H 'Content-Type: application/json' -H "Origin: $B" -d '{"username":"chk","password":"essai-Chk-123456"}' -o /dev/null -w '%{http_code}' $B/api/auth/login)"
check "connexion par nginx (cookie Secure, Origin verifiee) : 200" "[ \"$LOGIN\" = 200 ]"
check "cookie de session marque Secure" "grep -q 'TRUE.*dominet\|#HttpOnly' $W/jar && grep -qi secure <(curl -sk -i -H 'Content-Type: application/json' -H 'Origin: $B' -d '{\"username\":\"chk\",\"password\":\"essai-Chk-123456\"}' $B/api/auth/login)"

curl -sk --http1.1 -i --max-time 3 -b "$W/jar" -H "Origin: $B" -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==' "$B/ws/notifications/" > "$W/ws.txt" 2>/dev/null
check "WebSocket /ws/notifications/ a travers nginx : 101 Switching Protocols" "head -1 $W/ws.txt | grep -q 101"
echo
if [ "$fails" -eq 0 ]; then echo "pile de production : tous les controles passent"; else echo "$fails controle(s) en echec ; journaux :"; "${C[@]}" logs --tail 15 2>&1 | tail -60; fi
exit "$fails"
