#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Verification en lecture seule d'un serveur MCP DomiNet (aucune ecriture, aucune commande) :
#
#   MCP_TOKEN=dnm_... tools/check-mcp.sh [URL]            URL par defaut : http://127.0.0.1:8080/mcp
#   CACERT=dominet-ca.crt MCP_TOKEN=dnm_... tools/check-mcp.sh https://dominet.local/mcp
#
# Controle : refus sans jeton (401), liste des outils et des prompts propres a la portee du jeton, `system_status`.
# Le jeton n'est jamais affiche.
set -euo pipefail

URL="${1:-http://127.0.0.1:8080/mcp}"
: "${MCP_TOKEN:?MCP_TOKEN (valeur dnm_... du jeton) est obligatoire}"
CURL=(curl -sS --max-time 20)
[ -n "${CACERT:-}" ] && CURL+=(--cacert "$CACERT")
HDR=(-H 'accept: application/json, text/event-stream' -H 'content-type: application/json')
ok=0 ko=0
check() { if [ "$2" = ok ]; then echo "  ok   $1"; ok=$((ok + 1)); else echo "  ECHEC $1 ($2)"; ko=$((ko + 1)); fi; }
rpc() { "${CURL[@]}" "${HDR[@]}" -H "Authorization: Bearer $MCP_TOKEN" -X POST "$URL" -d "$1"; }

echo "MCP : $URL"
code=$("${CURL[@]}" -o /dev/null -w '%{http_code}' "${HDR[@]}" -X POST "$URL" -d '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' || echo 000)
[ "$code" = 401 ] && check "sans jeton : refuse (401)" ok || check "sans jeton : refuse (401)" "code $code"

tools=$(rpc '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' || true)
n=$(printf %s "$tools" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["tools"]))' 2>/dev/null || echo 0)
[ "$n" -gt 0 ] && check "$n outil(s) offerts a ce jeton" ok || check "liste des outils" "reponse inattendue"

prompts=$(rpc '{"jsonrpc":"2.0","id":3,"method":"prompts/list"}' || true)
p=$(printf %s "$prompts" | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["result"]["prompts"]))' 2>/dev/null || echo 0)
echo "  --   $p prompt(s) (0 sans la portee lecture)"

if printf %s "$tools" | grep -q '"system_status"'; then
  st=$(rpc '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"system_status","arguments":{}}}' || true)
  line=$(printf %s "$st" | python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]["structuredContent"]; print("version %s, base %s, appareils actifs %s, alertes %s" % (r["version"], r["database"], r["devices_active"], r["alerts_firing"]))' 2>/dev/null || true)
  [ -n "$line" ] && check "system_status : $line" ok || check "system_status" "reponse inattendue"
else
  echo "  --   system_status absent : jeton sans la portee lecture"
fi
echo "$ok ok, $ko echec(s)"
[ "$ko" = 0 ]
