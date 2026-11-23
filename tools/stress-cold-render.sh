#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Cold restarts in a loop: a few concurrent requests on a page right after the server starts (container).
# Used to measure Leptos's rendering defect that `src/web/render_guard.rs` works around (ADR 0021):
#   - with the safeguard, "anomalies" must stay at 0 (retried renders are counted separately);
#   - to test a new Leptos version, remove the safeguard, rebuild the image and check that the anomalies
#     and the retried renders stay at 0 over several hundred starts.
#
# Usage: tools/stress-cold-render.sh [image] [starts] [concurrent_requests] [page]
#   defaults: dominet:dev 100 3 /admin/settings
# Prerequisites: dev stack (`make dev-up`) and built image (`make image`). Throw-away database, deleted at the end.
set -u
IMG="${1:-dominet:dev}"; N="${2:-100}"; PAR="${3:-3}"; PAGE="${4:-/admin/settings}"
PG_CONTAINER="${PG_CONTAINER:-dominet-dev-postgres-1}"; PG_PORT="${PG_PORT:-15432}"
DB="dominet_cold_$$"; NAME="dominet-cold-$$"
WORK="$(mktemp -d)"; chmod 700 "$WORK"
PORT="$(python3 -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')"
export DATABASE_URL="postgres://dominet:dominet-dev@127.0.0.1:${PG_PORT}/${DB}" COOKIE_SECURE=false HTTP_ADDR="127.0.0.1:${PORT}" SYSTEM_STATS_SECS=1 RUST_BACKTRACE=1
DOMINET_MASTER_KEY="$(docker run --rm "$IMG" gen-master-key 2>/dev/null | tail -1)"; export DOMINET_MASTER_KEY
DOMINET_PASSWORD="$(head -c 24 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 16)"; export DOMINET_PASSWORD

psql_admin() { docker exec "$PG_CONTAINER" psql -q -U dominet -d postgres -v ON_ERROR_STOP=1 -c "$1" >/dev/null; }
cleanup() { set +e; docker rm -f "$NAME" >/dev/null 2>&1; psql_admin "DROP DATABASE IF EXISTS ${DB} WITH (FORCE)" 2>/dev/null; rm -rf "$WORK"; }
trap cleanup EXIT
dom() { docker run --rm --network host -e DATABASE_URL -e DOMINET_MASTER_KEY -e DOMINET_PASSWORD -e COOKIE_SECURE "$IMG" "$@"; }

docker exec "$PG_CONTAINER" pg_isready -U dominet >/dev/null 2>&1 || { echo "stack de dev absente : make dev-up" >&2; exit 1; }
psql_admin "CREATE DATABASE ${DB}"
dom migrate >/dev/null; dom seed >/dev/null; dom createsuperuser --username root >/dev/null
docker run -d --name "$NAME" --network host -e DATABASE_URL -e DOMINET_MASTER_KEY -e COOKIE_SECURE -e HTTP_ADDR -e SYSTEM_STATS_SECS -e RUST_BACKTRACE "$IMG" serve >/dev/null
for _ in $(seq 1 100); do curl -fs "http://127.0.0.1:${PORT}/healthz" >/dev/null 2>&1 && break; sleep 0.2; done

python3 - "$PORT" "$DOMINET_PASSWORD" "$N" "$PAR" "$NAME" "$PAGE" <<'PY'
import concurrent.futures as cf
import http.cookiejar
import json
import subprocess
import sys
import time
import urllib.request

port, password, n, par, name, page = sys.argv[1], sys.argv[2], int(sys.argv[3]), int(sys.argv[4]), sys.argv[5], sys.argv[6]
base = f"http://127.0.0.1:{port}"
jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
login = urllib.request.Request(base + "/api/auth/login", json.dumps({"username": "root", "password": password}).encode(),
                               {"Content-Type": "application/json", "Origin": base})
opener.open(login, timeout=10)
cookie = "; ".join(f"{c.name}={c.value}" for c in jar)  # les sessions sont en base : elles survivent aux redemarrages


def hit(_):
    t = time.time()
    try:
        with urllib.request.urlopen(urllib.request.Request(base + page, headers={"Cookie": cookie}), timeout=20) as x:
            x.read()
            return x.status, time.time() - t
    except Exception as e:  # noqa: BLE001
        return repr(e)[:60], time.time() - t


anomalies = slow = 0
for i in range(n):
    subprocess.run(["docker", "restart", name], capture_output=True, check=False)
    for _ in range(100):
        try:
            urllib.request.urlopen(base + "/healthz", timeout=1)
            break
        except Exception:  # noqa: BLE001
            time.sleep(0.1)
    with cf.ThreadPoolExecutor(par) as ex:
        results = list(ex.map(hit, range(par)))
    if any(status != 200 for status, _ in results):
        anomalies += 1
        print(f"demarrage {i} : {results}", flush=True)
    elif any(elapsed > 3 for _, elapsed in results):
        slow += 1  # reponse correcte mais tardive : un rendu a ete relance par le garde-fou
print(f"{n} demarrages a froid ({par} requetes sur {page}) : {anomalies} anomalies, {slow} reponses tardives")
PY
LOG="$(docker logs "$NAME" 2>&1 | sed 's/\x1b\[[0-9;]*m//g')"
echo "journal du serveur : $(printf '%s\n' "$LOG" | grep -c 'abandonne et relance') rendus relances, $(printf '%s\n' "$LOG" | grep -c panicked) panics"
