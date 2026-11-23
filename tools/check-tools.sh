#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Checks that the workstation has the tools of the local gate (make ci) (to install before building) and prints their
# versions. Fails with a clear message if one is missing: better than an obscure error in the middle of a job.
set -uo pipefail
fail=0
need() {  # need <command> <expected version pattern>
  local cmd="$1" want="$2" out
  if ! out="$("$cmd" --version 2>&1 | head -1)"; then out="(absent)"; fi
  if printf '%s' "$out" | grep -qE "$want"; then printf '  ok   %-14s %s\n' "$cmd" "$out"
  else printf '  MANQUE %-12s %s (attendu : %s)\n' "$cmd" "$out" "$want"; fail=1; fi
}
echo "Outils de la barriere locale :"
need rustc '^rustc 1\.99'
need cargo '^cargo 1\.99'
need cargo-deny '^cargo-deny 0\.20'
need cargo-audit '^cargo-audit.* 0\.22'
need sqlx '^sqlx-cli 0\.9'
need cargo-leptos '^cargo-leptos 0\.3'
need docker '^Docker version'
need make '^GNU Make'
if ! rustup target list --installed 2>/dev/null | grep -q '^wasm32-unknown-unknown$'; then
  echo "  MANQUE cible Rust wasm32-unknown-unknown"; fail=1
fi
if ! docker info >/dev/null 2>&1; then echo "  MANQUE acces a Docker (DOCKER_HOST=${DOCKER_HOST:-non defini})"; fail=1; fi
[ "$fail" = 0 ] && echo "Tous les outils sont presents." || { echo "Des outils manquent : lancer tools/setup-dev-host.sh." >&2; exit 1; }
