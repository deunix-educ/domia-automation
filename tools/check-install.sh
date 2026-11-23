#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Fresh installation on the DEV machine (R7 lot 6): `prod-init` then `prod-deploy` with an environment file, a Compose project
# (dominet-chk2) and separate ports, non-interactive administrator; then `prod-update` and backup. Destroys everything at the end.
# Prerequisite: dev stack stopped (make dev-down, unique ICE port 8189). Usage: tools/check-install.sh
set -uo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"; cd "$ROOT"
T="$(mktemp -d /tmp/dominet-inst-XXXXXX)"
export ENV_FILE="$T/env" PROJECT=dominet-chk2 CERTS_DIR="$T/certs" DOMINET_ADMIN_USER=chk DOMINET_ADMIN_PASSWORD=Essai-install-123456
docker ps --format '{{.Ports}}' | grep -q ':8189->' && { echo "port 8189 pris : make dev-down d'abord" >&2; exit 2; }
fails=0; check() { if eval "$2" >/dev/null 2>&1; then echo "  OK    $1"; else echo "  ECHEC $1"; fails=$((fails + 1)); fi; }
cleanup() { docker compose -p "$PROJECT" --env-file "$ENV_FILE" -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml down -v --remove-orphans >/dev/null 2>&1; rm -rf "$T" deploy/zigbee2mqtt/configuration.yaml; }
trap cleanup EXIT
check "prod-init cree l'environnement (droits 600)" "tools/prod.sh init && [ \"\$(stat -c %a $ENV_FILE)\" = 600 ]"
check "prod-init refuse d'ecraser un .env existant" "! tools/prod.sh init"
printf 'NGINX_HTTP_PORT=18080\nNGINX_HTTPS_PORT=18443\nPOSTGRES_HOST_PORT=15441\nMQTT_LAN_PORT=11885\nEMAIL_RECIPIENT=essai@example.org\nALLOWED_HOSTS=localhost,127.0.0.1\nCSRF_TRUSTED_ORIGINS=https://127.0.0.1:18443\n' >> "$ENV_FILE"
check "preflight : aucun point bloquant sur la machine d'essai (ports decales par le .env)" "ENV_FILE=$ENV_FILE PROJECT=$PROJECT tools/preflight.sh"
check "prod-guard accepte l'environnement genere" "tools/prod.sh guard"
sed 's/^DB_PASSWORD=.*/DB_PASSWORD=/' "$ENV_FILE" > "$T/vide"; check "prod-guard refuse un mot de passe vide" "! ENV_FILE=$T/vide tools/prod.sh guard"
check "prod-deploy (certificats, build, demarrage, admin, donnees, verification)" "tools/prod.sh deploy"
check "https: /healthz via nginx" "curl -fsk https://127.0.0.1:18443/healthz"
check "connexion de l'administrateur cree" "[ \"\$(curl -sk -H 'Content-Type: application/json' -H 'Origin: https://127.0.0.1:18443' -d '{\"username\":\"chk\",\"password\":\"$DOMINET_ADMIN_PASSWORD\"}' -o /dev/null -w '%{http_code}' https://127.0.0.1:18443/api/auth/login)\" = 200 ]"
check "prod-deploy rejoue sans erreur (idempotent)" "tools/prod.sh deploy"
check "prod-update (sans git pull) : sauvegarde, reconstruction, verification" "NOPULL=1 tools/prod.sh update"
check "prod-backup" "tools/prod.sh backup"
C="docker compose -p $PROJECT --env-file $ENV_FILE -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml"
psql_() { $C exec -T postgres sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -tA -v ON_ERROR_STOP=1 -c "$0"' "$1"; }
check "/docs/ renvoie vers la connexion sans session" "[ \"\$(curl -sk -o /dev/null -w '%{http_code}' https://127.0.0.1:18443/docs/)\" = 303 ]"
check "/docs/ (FR et EN) servi apres connexion" "for _ in 1 2 3 4 5; do curl -sk -c $T/jar -H 'Content-Type: application/json' -H 'Origin: https://127.0.0.1:18443' -d '{\"username\":\"chk\",\"password\":\"$DOMINET_ADMIN_PASSWORD\"}' https://127.0.0.1:18443/api/auth/login && curl -skf -b $T/jar https://127.0.0.1:18443/docs/ | grep -q DomiNet && curl -skf -b $T/jar https://127.0.0.1:18443/docs/en/ | grep -q 'Where to start'&& break; sleep 3; done; [ -f $T/jar ] && curl -skf -b $T/jar https://127.0.0.1:18443/docs/en/ | grep -q 'Where to start'"
check "/metrics n'est pas servi par nginx" "[ \"\$(curl -sk -o /dev/null -w '%{http_code}' https://127.0.0.1:18443/metrics)\" != 200 ]"
check "restauration : marqueur cree, sauvegarde, marqueur supprime" "psql_ 'CREATE TABLE restore_marker AS SELECT 42 AS v' && tools/prod.sh backup && psql_ 'DROP TABLE restore_marker'"
check "restauration : dominet restore --latest postgres rend le marqueur" "$C exec -T dominet dominet restore --latest postgres --yes && [ \"\$(psql_ 'SELECT v FROM restore_marker')\" = 42 ]"
check "restauration : l'application repond apres coup" "curl -skf https://127.0.0.1:18443/readyz"
check "prod-mqtt-user cree un compte externe (mot de passe affiche une fois)" "NAME=linky tools/prod.sh mqtt-user | grep -q 'Mot de passe'"
check "prod-mqtt-users liste le compte et ses droits" "tools/prod.sh mqtt-users | grep -q 'linky : readwrite linky/#'"
check "prod-mqtt-del le supprime" "NAME=linky tools/prod.sh mqtt-del && ! tools/prod.sh mqtt-users | grep -q linky"
check "prod-dump-context" "tools/prod.sh dump-context | grep -q '^devices:'"
[ "$fails" -eq 0 ] && echo "installation vierge : tous les controles passent" || echo "$fails echec(s)"
exit "$fails"
