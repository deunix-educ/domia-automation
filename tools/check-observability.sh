#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# End-to-end check of observability (R7 lot 3) with REAL throw-away Prometheus and Grafana (images already
# present: prom/prometheus:v2.52.0, grafana/grafana:11.0.0), on ports shifted on the machine:
#  - throw-away database (CREATE DATABASE in the dev PostgreSQL), migrations, `dominet seed`, `dominet sync-rules`;
#  - `promtool check rules` on the written rules.yml, then loading by Prometheus (/api/v1/rules);
#  - Grafana loads the provisioning (dashboards, sources, contact points, policies, alert rules) WITHOUT error, the
#    PostgreSQL source connects, a dashboard's SQL query runs in Grafana, and the reload requested by DomiNet is accepted.
# Nothing is sent outside, no production is touched. Usage: tools/check-observability.sh (dev stack: make dev-up)
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${DOMINET_BIN:-$ROOT/target/debug/dominet}"
W="$(mktemp -d /tmp/dominet-obs-XXXXXX)"
DB="dominet_obs_$$"
PROM_PORT="${OBS_PROM_PORT:-19090}"
GRAF_PORT="${OBS_GRAFANA_PORT:-13000}"
PG_CT="${PG_CONTAINER:-dominet-dev-postgres-1}"
fails=0
ok() { echo "  OK    $1"; }
ko() { echo "  ECHEC $1"; fails=$((fails + 1)); }
check() { if eval "$2" >/dev/null 2>&1; then ok "$1"; else ko "$1"; fi; }

cleanup() {
  docker rm -f "dominet-obs-prom-$$" "dominet-obs-graf-$$" >/dev/null 2>&1
  docker exec "$PG_CT" psql -q -U dominet -d postgres -c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" >/dev/null 2>&1
  rm -rf "$W"
}
trap cleanup EXIT

[ -x "$BIN" ] || { echo "binaire absent : cargo build -p dominet" >&2; exit 2; }
docker exec "$PG_CT" psql -q -U dominet -d postgres -c "CREATE DATABASE $DB" >/dev/null || { echo "stack de dev absente : make dev-up" >&2; exit 2; }

mkdir -p "$W/prom-rules" "$W/graf-alerting" "$W/graf-ds" "$W/graf-dash"
export DATABASE_URL="postgres://dominet:dominet-dev@127.0.0.1:15432/$DB" METRICS_ADDR=off
export PROMETHEUS_RULES_PATH="$W/prom-rules/rules.yml" PROMETHEUS_URL="http://127.0.0.1:$PROM_PORT"
export GRAFANA_ALERTING_PATH="$W/graf-alerting/rules.yaml" GRAFANA_URL="http://127.0.0.1:$GRAF_PORT" GRAFANA_USER=admin GRAFANA_PASSWORD=admin
KEY="$("$BIN" gen-master-key | tail -1)"; export DOMINET_MASTER_KEY="$KEY"
"$BIN" migrate >/dev/null && "$BIN" seed | tail -1

echo "== Prometheus"
# 1) first write of the files without services (reload expected to fail)
"$BIN" sync-rules >"$W/sync1.log" 2>&1; check "dominet sync-rules ecrit les deux fichiers" "[ -s $W/prom-rules/rules.yml ] && [ -s $W/graf-alerting/rules.yaml ]"
check "rechargement signale en echec quand les services sont absents" "grep -q ECHOUE $W/sync1.log"
docker run --rm --entrypoint promtool -v "$W/prom-rules:/r:ro" prom/prometheus:v2.52.0 check rules /r/rules.yml >"$W/promtool.log" 2>&1
check "promtool check rules : $(grep -m1 SUCCESS "$W/promtool.log" | cut -c1-90)" "grep -q SUCCESS $W/promtool.log"

sed "s#dominet:9100#127.0.0.1:1#; s#mediamtx:9998#127.0.0.1:2#" "$ROOT/deploy/prometheus/prometheus.yml" > "$W/prometheus.yml"
docker run -d --name "dominet-obs-prom-$$" --network host -v "$W/prometheus.yml:/etc/prometheus/prometheus.yml:ro" -v "$W/prom-rules:/etc/prometheus/rules" \
  prom/prometheus:v2.52.0 --config.file=/etc/prometheus/prometheus.yml --web.listen-address=127.0.0.1:$PROM_PORT --web.enable-lifecycle --storage.tsdb.path=/tmp/tsdb >/dev/null
for _ in $(seq 1 40); do curl -fs "http://127.0.0.1:$PROM_PORT/-/ready" >/dev/null 2>&1 && break; sleep 0.5; done
GROUPS_WRITTEN="$(grep -c '^  - name:' "$W/prom-rules/rules.yml")"
check "Prometheus charge les groupes de regles ($GROUPS_WRITTEN attendus : ceux du fichier ecrit)" "[ \"\$(curl -s http://127.0.0.1:$PROM_PORT/api/v1/rules | python3 -c 'import sys,json; print(len(json.load(sys.stdin)[\"data\"][\"groups\"]))')\" = $GROUPS_WRITTEN ]"

# 2) modification in the database then reload requested by DomiNet
docker exec "$PG_CT" psql -q -U dominet -d "$DB" -c "UPDATE prometheus_rule SET summary = 'Resume modifie par le controle' WHERE name = 'DominetDown'" >/dev/null
"$BIN" sync-rules >"$W/sync2.log" 2>&1
check "Prometheus accepte le rechargement demande par DomiNet" "grep -q 'Prometheus : $GROUPS_WRITTEN groupe(s) ecrit(s), rechargement demande' $W/sync2.log"
check "la regle modifiee est visible dans Prometheus" "curl -s http://127.0.0.1:$PROM_PORT/api/v1/rules | grep -q 'Resume modifie par le controle'"

echo "== Grafana"
cp "$ROOT"/deploy/grafana/provisioning/alerting/*.yaml "$W/graf-alerting/"
cp "$ROOT"/deploy/grafana/provisioning/datasources/*.yml "$W/graf-ds/"
sed -i "s#http://prometheus:9090#http://127.0.0.1:$PROM_PORT#" "$W/graf-ds/prometheus.yml"
docker run -d --name "dominet-obs-graf-$$" --network host \
  -e GF_SERVER_HTTP_ADDR=127.0.0.1 -e GF_SERVER_HTTP_PORT=$GRAF_PORT -e GF_SECURITY_ADMIN_USER=admin -e GF_SECURITY_ADMIN_PASSWORD=admin \
  -e GF_DB_HOST=127.0.0.1 -e GF_DB_PORT=15432 -e GF_DB_NAME="$DB" -e GF_DB_USER=dominet -e GF_DB_PASSWORD=dominet-dev -e EMAIL_RECIPIENT=controle@example.org \
  -e GF_ANALYTICS_REPORTING_ENABLED=false -e GF_ANALYTICS_CHECK_FOR_UPDATES=false \
  -v "$ROOT/deploy/grafana/provisioning/dashboards:/etc/grafana/provisioning/dashboards:ro" \
  -v "$W/graf-ds:/etc/grafana/provisioning/datasources:ro" -v "$W/graf-alerting:/etc/grafana/provisioning/alerting" \
  -v "$W/graf-dash:/var/lib/dominet-dashboards:ro" \
  grafana/grafana:11.0.0 >/dev/null
for _ in $(seq 1 120); do curl -fs "http://127.0.0.1:$GRAF_PORT/api/health" >/dev/null 2>&1 && break; sleep 0.5; done
A="admin:admin"; G="http://127.0.0.1:$GRAF_PORT"
check "Grafana demarre" "curl -fs $G/api/health"
docker logs "dominet-obs-graf-$$" >"$W/grafana.log" 2>&1
check "aucune erreur de provisionnement dans le journal de Grafana" "! grep -E 'level=error' $W/grafana.log | grep -iE 'provision|dashboard|datasource|alert|contact'"
check "7 tableaux provisionnes" "[ \"\$(curl -s -u $A '$G/api/search?type=dash-db' | python3 -c 'import sys,json; print(len(json.load(sys.stdin)))')\" = 7 ]"
check "source PostgreSQL : connexion a la base jetable" "curl -s -u $A $G/api/datasources/uid/postgresql/health | grep -q 'Database Connection OK'"
check "source Prometheus : connexion" "curl -s -u $A $G/api/datasources/uid/prometheus/health | grep -qi 'successfully queried'"
check "4 regles d'alerte provisionnees" "[ \"\$(curl -s -u $A $G/api/v1/provisioning/alert-rules | python3 -c 'import sys,json; print(len(json.load(sys.stdin)))')\" = 4 ]"
check "point de contact dominet-email" "curl -s -u $A $G/api/v1/provisioning/contact-points | grep -q dominet-email"
check "politique de notification vers dominet-email" "curl -s -u $A $G/api/v1/provisioning/policies | grep -q dominet-email"
check "API d'alertes de Grafana lue par DomiNet (liste)" "curl -fs -u $A '$G/api/alertmanager/grafana/api/v2/alerts?active=true&silenced=false&inhibited=false' | python3 -c 'import sys,json; assert isinstance(json.load(sys.stdin), list)'"
check "API d'alertes de Prometheus lue par DomiNet" "curl -fs http://127.0.0.1:$PROM_PORT/api/v1/alerts | python3 -c 'import sys,json; assert json.load(sys.stdin)[\"data\"][\"alerts\"] is not None'"
# A dashboard's SQL query, run BY Grafana against the database.
cat > "$W/q.json" <<'JSON'
{"queries":[{"refId":"A","datasource":{"type":"postgres","uid":"postgresql"},"rawSql":"SELECT COUNT(*) AS value FROM device WHERE active = true","format":"table"}],"from":"now-1h","to":"now"}
JSON
check "Grafana execute la requete SQL du tableau « appareils actifs »" "curl -s -u $A -H 'Content-Type: application/json' -d @$W/q.json $G/api/ds/query | grep -q '\"value\"'"
# The energy dashboard: each query is run BY Grafana (its $__timeFilter macros expanded by it) on the throw-away database.
python3 - "$ROOT/deploy/grafana/provisioning/dashboards/energy.json" > "$W/energy-queries.json" <<'PY'
import json, sys
doc = json.load(open(sys.argv[1]))
queries = [{"refId": f"{p['id']}{t['refId']}", "datasource": t["datasource"], "rawSql": t["rawSql"], "format": "table"}
           for p in doc["panels"] for t in p["targets"]]
print(json.dumps({"queries": queries, "from": "now-30d", "to": "now"}))
PY
check "Grafana execute les requetes SQL du tableau energie (sans erreur)" "curl -s -u $A -H 'Content-Type: application/json' -d @$W/energy-queries.json $G/api/ds/query | python3 -c 'import sys,json; r=json.load(sys.stdin)[\"results\"]; assert r and not [k for k,v in r.items() if \"error\" in v], r'"
# Reload requested by DomiNet: the entry point exists in Grafana 11 and accepts our credentials.
"$BIN" sync-rules >"$W/sync3.log" 2>&1
check "Grafana accepte le rechargement du provisionnement d'alertes" "grep -q 'Grafana : 4 groupe(s) ecrit(s), rechargement demande' $W/sync3.log || grep -q 'Grafana : 1 groupe(s) ecrit(s), rechargement demande' $W/sync3.log"

echo
if [ "$fails" -eq 0 ]; then echo "Observabilite : tout est vert"; else echo "Observabilite : $fails echec(s)"; echo "(journaux : $W)"; trap - EXIT; docker rm -f "dominet-obs-prom-$$" "dominet-obs-graf-$$" >/dev/null 2>&1; fi
exit "$fails"
