#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# End-to-end rehearsal of tools/migrate-pack.sh on THIS machine, with two throw-away production stacks (Compose projects mig-src and
# mig-dst, shifted ports, throw-away secrets, a throw-away mkcert authority, volumes and folders destroyed at the end):
#   source: administrator, data in the database, a ReductStore record, an MQTT account, Zigbee2MQTT files -> export;
#   refusals: wrong passphrase, damaged archive, dry run, existing installation without --yes;
#   target (empty): import -> same accounts, same rows, same ReductStore record, same keys and authority, stack healthy.
# Nothing else is touched. The development stack must be stopped first (the WebRTC port is unique): make dev-down ... make dev-up.
# Usage: tools/check-migrate.sh [image]   (default dominet:dev, built by make image)
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
IMAGE="${1:-dominet:dev}"
TAG="${IMAGE#*:}"
SRC=mig-src; DST=mig-dst
W="$(mktemp -d /tmp/dominet-mig-XXXXXX)"
HTTPS=18543; HTTP=18180; PG=15540; MQTT=11984
fails=0
ok() { echo "  OK    $1"; }
ko() { echo "  FAIL  $1"; fails=$((fails + 1)); }
check() { if eval "$2" >/dev/null 2>&1; then ok "$1"; else ko "$1"; fi; }
rand() { head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 24; }
PASS="rehearsal-passphrase-$(rand)"; ADMIN_PW="Admin-$(rand)"

docker image inspect "$IMAGE" >/dev/null 2>&1 || { echo "image $IMAGE missing (make image)" >&2; exit 2; }
if docker ps --format '{{.Ports}}' | grep -q ':8189->'; then echo "port 8189 is taken (make dev-down first)" >&2; exit 2; fi

mkdir -p "$W/certs-src" "$W/certs-dst" "$W/z2m-src" "$W/z2m-dst" "$W/caroot-src" "$W/caroot-dst" "$W/out"
# A throw-away mkcert authority (created, never installed) and a certificate for localhost.
CAROOT="$W/caroot-src" mkcert -cert-file "$W/certs-src/cert.pem" -key-file "$W/certs-src/key.pem" localhost 127.0.0.1 >/dev/null 2>&1 || { echo "mkcert is needed for this rehearsal" >&2; exit 2; }
cp "$W/caroot-src/rootCA.pem" "$W/certs-src/dominet-ca.crt"; chmod 600 "$W/certs-src/key.pem"
printf 'mqtt:\n  server: mqtt://mosquitto\nserial:\n  port: /dev/ttyUSB0\nadvanced:\n  network_key: [1, 2, 3, 4]\n' > "$W/z2m-src/configuration.yaml"
head -c 4096 /dev/urandom > "$W/z2m-src/database.db"; echo '{"coordinator":"fake"}' > "$W/z2m-src/coordinator_backup.json"

MK="$(docker run --rm "$IMAGE" gen-master-key | tail -1)"
cat > "$W/src.env" <<ENV
DOMINET_TAG=$TAG
DOMINET_PULL=0
LAN_DOMAIN=localhost
LOCAL_DOMAIN=127.0.0.1
ALLOWED_HOSTS=localhost,127.0.0.1
CSRF_TRUSTED_ORIGINS=https://localhost:$HTTPS,https://127.0.0.1:$HTTPS
WEBRTC_ADDITIONAL_HOSTS=localhost,127.0.0.1
GRAFANA_PUBLIC_URL=https://localhost:$HTTPS
DB_PASSWORD=$(rand)
REDUCTSTORE_TOKEN=$(rand)
MQTT_PASSWORD=$(rand)
GRAFANA_PASSWORD=$(rand)
ZIGBEE_AUTH_TOKEN=$(rand)
EMAIL_RECIPIENT=controle@example.org
DOMINET_MASTER_KEY=$MK
ZIGBEE_DEVICE=
VIDEO_DEVICE=
RENDER_GID=
POSTGRES_HOST_PORT=$PG
NGINX_HTTP_PORT=$HTTP
NGINX_HTTPS_PORT=$HTTPS
MQTT_LAN_PORT=$MQTT
ENV
chmod 600 "$W/src.env"

# The tools take their settings from the environment (see tools/prod.sh); one function per machine.
as_src() { ENV_FILE="$W/src.env" PROJECT=$SRC CERTS_DIR="$W/certs-src" ZIGBEE_DIR="$W/z2m-src" CAROOT="$W/caroot-src" MIGRATE_PASSPHRASE="${PP:-$PASS}" "$@"; }
as_dst() { ENV_FILE="$W/dst.env" PROJECT=$DST CERTS_DIR="$W/certs-dst" ZIGBEE_DIR="$W/z2m-dst" CAROOT="$W/caroot-dst" MIGRATE_PASSPHRASE="${PP:-$PASS}" "$@"; }
psql_() { # psql_ PROJECT ENVFILE SQL
  docker compose -p "$1" --env-file "$2" -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml exec -T postgres psql -qAt -U dominet -d dominet -c "$3"
}
rs() { # rs PROJECT ENVFILE put|get NAME [BODY] : raw HTTP to ReductStore from inside the stack network (the image has no curl)
  local token; token="$(grep '^REDUCTSTORE_TOKEN=' "$2" | cut -d= -f2)"
  docker run --rm --network "$1_dominet-net" -e TOKEN="$token" -e ACT="$3" -e NAME="$4" -e BODY="${5:-}" --entrypoint bash "$IMAGE" -c '
    req() { exec 3<>/dev/tcp/reductstore/8383; printf "%b" "$1" >&3; cat <&3; }
    case "$ACT" in
      put)
        req "POST /api/v1/b/migtest HTTP/1.0\r\nAuthorization: Bearer $TOKEN\r\nContent-Length: 2\r\n\r\n{}" >/dev/null
        req "POST /api/v1/b/migtest/$NAME?ts=1000000 HTTP/1.0\r\nAuthorization: Bearer $TOKEN\r\nContent-Type: text/plain\r\nContent-Length: ${#BODY}\r\n\r\n$BODY" | head -1 ;;
      get) req "GET /api/v1/b/migtest/$NAME?ts=1000000 HTTP/1.0\r\nAuthorization: Bearer $TOKEN\r\n\r\n" | tr -d "\r" | sed "1,/^\$/d" ;;
    esac'
}
cleanup() {
  if [ -n "${KEEP:-}" ]; then echo "kept: projects $SRC and $DST, folder $W"; return; fi
  for p in "$SRC:$W/src.env" "$DST:$W/dst.env"; do
    [ -f "${p#*:}" ] && docker compose -p "${p%%:*}" --env-file "${p#*:}" -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml down -v --remove-orphans >/dev/null 2>&1
  done
  docker run --rm -v "$W:/w" --entrypoint sh debian:trixie-slim -c "rm -rf /w/*" >/dev/null 2>&1
  rm -rf "$W"
}
trap cleanup EXIT

echo "== source machine (project $SRC)"
as_src tools/prod.sh up >"$W/src-up.log" 2>&1; check "source stack up and healthy" "[ \$? -eq 0 ] && grep -q . $W/src-up.log"
tail -3 "$W/src-up.log" | sed 's/^/    /' | cut -c1-140
DOMINET_ADMIN_USER=admin DOMINET_ADMIN_PASSWORD="$ADMIN_PW" as_src tools/prod.sh superuser >"$W/su.log" 2>&1
check "administrator created" "[ \"\$(psql_ $SRC $W/src.env \"SELECT count(*) FROM users WHERE username='admin'\")\" = 1 ]"
as_src tools/prod.sh seed >/dev/null 2>&1
psql_ $SRC "$W/src.env" "INSERT INTO location (name, description) VALUES ('Salon', 'Pièce « principale » — étage 1')" >/dev/null
NAME=linkytest PASSWORD="mqtt-$(rand)" TOPIC='linky/#' as_src tools/prod.sh mqtt-user >/dev/null 2>&1
check "MQTT account created" "as_src tools/prod.sh mqtt-users | grep -q linkytest"
RECORD="record-$(rand)"
rs $SRC "$W/src.env" put entry1 "$RECORD" >/dev/null 2>&1
check "ReductStore record written and read back" "[ \"\$(rs $SRC $W/src.env get entry1)\" = \"$RECORD\" ]"
sleep 3   # lets ReductStore write its blocks to disk before the archive of the volume
fp() { psql_ "$1" "$2" "SELECT md5(string_agg(x::text, E'\n' ORDER BY x::text)) FROM $3 x"; }
U_SRC="$(fp $SRC $W/src.env users)"; L_SRC="$(fp $SRC $W/src.env location)"; M_SRC="$(psql_ $SRC $W/src.env 'SELECT count(*) FROM _sqlx_migrations')"
check "fingerprints taken (users, location, migrations)" "[ -n '$U_SRC' ] && [ -n '$L_SRC' ] && [ '$M_SRC' -gt 10 ]"

echo "== export"
as_src tools/migrate-pack.sh export --out "$W/out" >"$W/export.log" 2>&1; check "export succeeds" "[ \$? -eq 0 ]"
ARCH="$(ls "$W"/out/dominet-migration-*.tar 2>/dev/null | head -1)"
check "one archive and its checksum" "[ -s '$ARCH' ] && [ -s '$ARCH.sha256' ]"
check "the archive contains no readable secret (.env, master key, CA key)" "! tar xOf '$ARCH' . 2>/dev/null | grep -aq '$MK' && ! tar xOf '$ARCH' . 2>/dev/null | grep -aq 'PRIVATE KEY'"
check "verify with the right passphrase" "as_dst tools/migrate-pack.sh verify '$ARCH'"
check "verify refuses a wrong passphrase" "! PP=wrong-passphrase-0000 as_dst tools/migrate-pack.sh verify '$ARCH'"
cp "$ARCH" "$W/damaged.tar"; cp "$ARCH.sha256" "$W/damaged.tar.sha256"; sed -i "s/  .*/  damaged.tar/" "$W/damaged.tar.sha256"
printf 'X' | dd of="$W/damaged.tar" bs=1 seek=2000 conv=notrunc 2>/dev/null
check "verify refuses a damaged archive" "! as_dst tools/migrate-pack.sh verify '$W/damaged.tar'"

echo "== stopping the source (frees the ports)"
docker compose -p $SRC --env-file "$W/src.env" -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml down -v --remove-orphans >/dev/null 2>&1
check "source stack gone, its volumes with it" "[ -z \"\$(docker volume ls -q --filter name=${SRC}_)\" ]"

echo "== target machine (project $DST), empty"
as_dst tools/migrate-pack.sh import "$ARCH" --dry-run >"$W/dry.log" 2>&1; check "dry run succeeds" "[ \$? -eq 0 ] && grep -q 'Dry run' $W/dry.log"
check "dry run changed nothing (no .env, no stack)" "[ ! -e $W/dst.env ] && [ -z \"\$(docker volume ls -q --filter name=${DST}_)\" ]"
PP=wrong-passphrase-0000 as_dst tools/migrate-pack.sh import "$ARCH" >"$W/wrong.log" 2>&1
check "import refuses a wrong passphrase and changes nothing" "[ \$? -ne 0 ] && grep -q 'wrong passphrase' $W/wrong.log && [ ! -e $W/dst.env ]"
as_dst tools/migrate-pack.sh import "$ARCH" >"$W/import.log" 2>&1; rc=$?
check "import succeeds" "[ $rc -eq 0 ]"
[ "$rc" -eq 0 ] || tail -25 "$W/import.log" | sed 's/^/    /' | cut -c1-160
tail -6 "$W/import.log" | sed 's/^/    /' | cut -c1-160

echo "== what the target now has"
check "DOMINET_MASTER_KEY identical" "[ \"\$(grep '^DOMINET_MASTER_KEY=' $W/dst.env)\" = 'DOMINET_MASTER_KEY=$MK' ]"
check "the .env is readable by its owner only" "[ \"\$(stat -c %a $W/dst.env)\" = 600 ]"
check "users table identical (same accounts and password hashes)" "[ \"\$(fp $DST $W/dst.env users)\" = '$U_SRC' ]"
check "location table identical (accents intact)" "[ \"\$(fp $DST $W/dst.env location)\" = '$L_SRC' ]"
check "same number of migrations" "[ \"\$(psql_ $DST $W/dst.env 'SELECT count(*) FROM _sqlx_migrations')\" = '$M_SRC' ]"
check "ReductStore record identical" "[ \"\$(rs $DST $W/dst.env get entry1)\" = \"$RECORD\" ]"
check "MQTT account restored" "as_dst tools/prod.sh mqtt-users | grep -q linkytest"
check "Zigbee2MQTT files identical (pairing kept)" "diff -r $W/z2m-src $W/z2m-dst"
check "mkcert authority key restored (devices keep trusting)" "cmp $W/caroot-src/rootCA-key.pem $W/caroot-dst/rootCA-key.pem && cmp $W/caroot-src/rootCA.pem $W/caroot-dst/rootCA.pem"
check "certificate present and signed by that authority" "openssl verify -CAfile $W/caroot-dst/rootCA.pem $W/certs-dst/cert.pem"
check "ZIGBEE_DEVICE is empty (no dongle here)" "[ -z \"\$(grep '^ZIGBEE_DEVICE=' $W/dst.env | cut -d= -f2)\" ]"
check "the application answers over HTTPS" "curl -fsk --resolve localhost:$HTTPS:127.0.0.1 https://localhost:$HTTPS/healthz"
check "make prod-doctor has nothing blocking" "as_dst tools/prod.sh doctor"

echo "== an existing installation is not overwritten"
as_dst tools/migrate-pack.sh import "$ARCH" >"$W/again.log" 2>&1
check "second import without --yes is refused" "[ \$? -ne 0 ] && grep -q -- '--yes' $W/again.log"
check "the data is still there" "[ \"\$(fp $DST $W/dst.env location)\" = '$L_SRC' ]"

echo
if [ "$fails" -eq 0 ]; then echo "Migration rehearsal: all green"; else echo "Migration rehearsal: $fails failure(s)"; echo "(rerun with KEEP=1 to keep the stacks and the logs for inspection)"; fi
exit "$fails"
