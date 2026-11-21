#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Rejoue une session de fixtures Z2M (.jsonl) vers le Mosquitto de dev.
# Usage : tools/replay-z2m.sh [fichier.jsonl] [delai_s]
set -euo pipefail
cd "$(dirname "$0")/.."
file="${1:-tests/fixtures/z2m/session-basic.jsonl}"
delay="${2:-0.2}"
compose=(docker compose -f deploy/docker-compose.yml -f deploy/docker-compose.dev.yml)
while IFS= read -r line; do
  [ -z "$line" ] && continue
  topic=$(jq -r '.topic' <<<"$line")
  payload=$(jq -c '.payload | if type == "string" then . else tostring end' <<<"$line" | jq -r '.')
  retain=()
  [ "$(jq -r '.retain // false' <<<"$line")" = "true" ] && retain=(-r)
  "${compose[@]}" exec -T mosquitto mosquitto_pub -t "$topic" -m "$payload" "${retain[@]}" </dev/null
  echo "-> $topic"
  sleep "$delay"
done < "$file"
