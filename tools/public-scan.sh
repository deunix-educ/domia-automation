#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Sweep of the tracked files before publication (roadmap ROADMAP-depot-public.md, P0).
# Looks, by category, for: private addresses, identities, hosts, references to the old system, probable secrets.
# Usage: tools/public-scan.sh            summary per category
#         tools/public-scan.sh --files    + list of the files per category
#         tools/public-scan.sh --strict   exit code 1 if a finding remains (to wire in before publication)
# Read-only: never reads a real .env (only the files tracked by git are examined, and .env is ignored).
set -euo pipefail
cd "$(dirname "$0")/.."

files=0 strict=0
for a in "$@"; do
  case "$a" in
    --files) files=1 ;;
    --strict) strict=1 ;;
    *) echo "option inconnue : $a" >&2; exit 2 ;;
  esac
done

# This script and the roadmap quote the patterns: we exclude them from the sweep.
EXCL=(':!tools/public-scan.sh' ':!ROADMAP-depot-public.md' ':!INVENTAIRE-depot-public.md' ':!Cargo.lock' ':!.sqlx')

# category|extended regular expression (case-insensitive unless stated)
CATS=(
  "adresses-privees|(^|[^0-9.])(192\\.168|10\\.[0-9]{1,3}|172\\.(1[6-9]|2[0-9]|3[01]))\\.[0-9]{1,3}\\.[0-9]{1,3}"
  "identite|mirace[t]i|deunix-educ/DomiNe[t]-rs"
  "hotes|dominet\\.local|elitedesk|eliteDesk|\\.local\\b"
  "ancien-systeme|django|celery|python-django|DomiNet Python|depot Python|dépôt Python|stack Python|ancien système|framework précédent|update_or_create|create_defaults|dumpdata|manage\\.py|core\\.mail|WebsocketCommunicator|previous (system|implementation)|old system|legacy system"
  "secrets-probables|BEGIN (RSA |EC |OPENSSH )?PRIVATE KEY|ghp_[A-Za-z0-9]{20,}|xox[bp]-|AKIA[0-9A-Z]{16}|sk-[A-Za-z0-9]{20,}|(password|passwd|secret|token)[\"']?[[:space:]]*[:=][[:space:]]*[\"'][^\"' ]{8,}"
)

total=0
for entry in "${CATS[@]}"; do
  name="${entry%%|*}"; re="${entry#*|}"
  # -I: ignores binaries; -i: case-insensitive; -E: extended regexes
  list="$(git grep -IilE -e "$re" -- . "${EXCL[@]}" 2>/dev/null || true)"
  n=0; if [ -n "$list" ]; then n="$(printf "%s\n" "$list" | wc -l)"; fi
  hits="$(git grep -IiE -c -e "$re" -- . "${EXCL[@]}" 2>/dev/null | awk -F: '{s+=$NF} END{print s+0}' || true)"
  printf '%-18s %5d fichiers %6d lignes\n' "$name" "$n" "$hits"
  total=$((total + n))
  if [ "$files" -eq 1 ] && [ "$n" -gt 0 ]; then printf '%s\n' "$list" | sed 's/^/    /'; fi
done

if [ "$strict" -eq 1 ] && [ "$total" -gt 0 ]; then
  echo "public-scan : $total fichier(s) a traiter avant publication" >&2
  exit 1
fi
