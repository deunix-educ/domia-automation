#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds the Docker image, starts it without a database and checks: /healthz answers, /readyz answers 503
# (the database is absent), the process runs as a non-root user, and the shutdown is clean (code 0).
set -euo pipefail

IMAGE="${IMAGE:-dominet:ci}"
NAME="dominet-smoke-$$"
tmp="$(mktemp -d)"
cleanup() { docker rm -f "$NAME" >/dev/null 2>&1 || true; rm -rf "$tmp"; }
trap cleanup EXIT

docker build -t "$IMAGE" .
docker run --rm "$IMAGE" --version

docker run -d --name "$NAME" -p 127.0.0.1::8000 \
  -e DATABASE_URL=postgres://u:p@127.0.0.1:1/none "$IMAGE" >/dev/null
port="$(docker port "$NAME" 8000/tcp | head -1 | sed 's/.*://')"

ok=0
for _ in $(seq 1 40); do
  if curl -fsS "http://127.0.0.1:${port}/healthz" >/dev/null 2>&1; then ok=1; break; fi
  sleep 0.5
done
if [ "$ok" != 1 ]; then echo "ERREUR : /healthz ne repond pas" >&2; docker logs "$NAME" >&2; exit 1; fi
echo "healthz : $(curl -fsS "http://127.0.0.1:${port}/healthz")"

code="$(curl -s -o /dev/null -w '%{http_code}' "http://127.0.0.1:${port}/readyz")"
[ "$code" = 503 ] || { echo "ERREUR : /readyz devrait repondre 503 sans base (obtenu ${code})" >&2; exit 1; }
echo "readyz sans base : ${code} (attendu)"

# Interface: the login page renders (without a database), the security policy is there, and the static files are served
# with the right type and compressed. Without them the image starts but the interface stays inert (see the pitfall in PROJECT_CONTEXT.md).
base="http://127.0.0.1:${port}"
page_headers="$(curl -sS -D- -o "$tmp/login.html" "$base/login")"
echo "$page_headers" | head -1 | grep -q ' 200' || { echo "ERREUR : /login ne repond pas 200 :" >&2; echo "$page_headers" | head -3 >&2; exit 1; }
echo "$page_headers" | grep -qi "^content-security-policy:.*'nonce-" || { echo "ERREUR : pas de CSP a nonce sur /login" >&2; exit 1; }
grep -q 'href="/pkg/dominet.wasm"' "$tmp/login.html" || { echo "ERREUR : la page ne reclame pas /pkg/dominet.wasm (compilation serveur / WASM differente ?)" >&2; exit 1; }
grep -q 'space<!>' "$tmp/login.html" || { echo "ERREUR : marqueurs d'hydratation absents du HTML rendu" >&2; exit 1; }
echo "login : 200, CSP a nonce, WASM et marqueurs d'hydratation attendus"
for f in dominet.js dominet.css dominet.wasm; do
  code="$(curl -s -o /dev/null -w '%{http_code}' "$base/pkg/$f")"
  [ "$code" = 200 ] || { echo "ERREUR : /pkg/$f repond $code" >&2; exit 1; }
done
wasm_headers="$(curl -sS -D- -o "$tmp/dominet.wasm.gz" -H 'Accept-Encoding: gzip' "$base/pkg/dominet.wasm")"
echo "$wasm_headers" | grep -qi '^content-type: application/wasm' || { echo "ERREUR : type MIME du WASM incorrect" >&2; exit 1; }
echo "$wasm_headers" | grep -qi '^content-encoding: gzip' || { echo "ERREUR : le WASM n'est pas compresse" >&2; exit 1; }
echo "$wasm_headers" | grep -qi '^cache-control: no-cache' || { echo "ERREUR : /pkg doit etre en no-cache" >&2; exit 1; }
wasm_size="$(gzip -dc "$tmp/dominet.wasm.gz" | wc -c)"
[ "$wasm_size" -gt 100000 ] || { echo "ERREUR : WASM trop petit (${wasm_size} octets)" >&2; exit 1; }
echo "statiques : JS, CSS et WASM (${wasm_size} octets, gzip, application/wasm, no-cache)"

uid="$(docker exec "$NAME" id -u)"
[ "$uid" != 0 ] || { echo "ERREUR : le processus tourne en root" >&2; exit 1; }
echo "utilisateur du conteneur : uid ${uid} (non root)"

docker stop "$NAME" >/dev/null
exit_code="$(docker inspect "$NAME" --format '{{.State.ExitCode}}')"
[ "$exit_code" = 0 ] || { echo "ERREUR : arret non propre (code ${exit_code})" >&2; exit 1; }
echo "arret propre (code ${exit_code})"
