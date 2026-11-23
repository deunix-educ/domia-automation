#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Prepares a DomiNet DEVELOPMENT workstation (repository already cloned): pinned Rust, local-gate tools, pre-push hook,
# throw-away dev stack. Idempotent: re-running it breaks nothing. Writes nothing outside the home folder (no sudo); what
# needs root (system packages, docker group) is only displayed.
#
#   tools/setup-dev-host.sh          installs / checks everything
#   tools/setup-dev-host.sh --ci     ... then runs `make ci` (first build: long)
#
# No contact with production: the dev stack is local (ports 1xxxx on 127.0.0.1). See docs/dev-machine.md.
set -euo pipefail
cd "$(dirname "$0")/.."

RUN_CI=0
[ "${1:-}" = "--ci" ] && RUN_CI=1
say() { printf '\n\033[1m== %s\033[0m\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

say "1/5 Prerequis systeme"
missing=()
for c in git curl make gcc pkg-config python3 firefox docker; do have "$c" || missing+=("$c"); done
if [ ${#missing[@]} -gt 0 ]; then
  echo "Manquent : ${missing[*]}"
  echo "Debian / Ubuntu (a lancer en root) :"
  echo "  apt-get install -y git curl make build-essential pkg-config libssl-dev python3 firefox-esr   # (firefox sur Ubuntu)"
  echo "  Docker : https://docs.docker.com/engine/install/ puis  usermod -aG docker \$USER  (se reconnecter)"
  exit 1
fi
docker info >/dev/null 2>&1 || { echo "Docker ne repond pas pour cet utilisateur (groupe docker ? service demarre ?)" >&2; exit 1; }
docker compose version >/dev/null 2>&1 || { echo "Le plugin « docker compose » est requis" >&2; exit 1; }
echo "ok"

say "2/5 Rust (version epinglee par rust-toolchain.toml)"
if ! have rustup; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none --profile minimal
fi
# shellcheck disable=SC1091
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"
rustup toolchain install   # reads rust-toolchain.toml: version, components (rustfmt, clippy) and wasm32 target
rustc --version

say "3/5 Outils de la barriere locale (versions de tools/check-tools.sh)"
install_tool() {  # install_tool <binary> <version pattern> <cargo install args...>
  local bin="$1" want="$2"; shift 2
  if "$bin" --version 2>/dev/null | head -1 | grep -qE "$want"; then echo "  ok   $bin"; return; fi
  echo "  installation de $bin (compilation, quelques minutes)..."
  cargo install --locked "$@"
}
install_tool cargo-deny '0\.20' cargo-deny --version '^0.20'
install_tool cargo-audit '0\.22' cargo-audit --version '^0.22'
install_tool sqlx '0\.9' sqlx-cli --version '^0.9' --no-default-features --features rustls,postgres
if ! cargo leptos --version >/dev/null 2>&1 && ! pkg-config --exists openssl 2>/dev/null; then
  echo "cargo-leptos se compile avec OpenSSL : installer libssl-dev (root : apt-get install -y libssl-dev) puis relancer." >&2
  exit 1
fi
install_tool cargo-leptos '0\.3' cargo-leptos --version '=0.3.11'

say "4/5 Garde-fou pre-push"
make hooks

say "5/5 Stack de dev jetable (PostgreSQL 15432, Mosquitto 11883, ReductStore 18383, MediaMTX)"
make dev-up
make db-migrate

echo
tools/check-tools.sh || true
echo
if [ "$RUN_CI" = 1 ]; then
  make ci
else
  echo "Poste pret. Verifier : make ci   (ou relancer ce script avec --ci)"
fi
