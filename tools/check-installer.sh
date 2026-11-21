#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Tests the guided installer (install.sh) without installing anything: every scenario runs `./install.sh --dry-run` with
# stub programs placed first in PATH (no Docker, Docker down, old Compose, not Linux, port in use...). Fast, no network,
# nothing created: it also checks that a dry run leaves the working tree untouched.
set -uo pipefail
cd "$(dirname "$0")/.."
W="$(mktemp -d)"; trap 'rm -rf "$W"; [ -z "${HOLD_PID:-}" ] || kill "$HOLD_PID" 2>/dev/null || true' EXIT
fails=0; total=0
check() { # check NAME CONDITION-EXIT-CODE
  total=$((total + 1))
  if [ "$2" = 0 ]; then printf '  OK    %s\n' "$1"; else printf '  FAIL  %s\n' "$1"; fails=$((fails + 1)); fi
}
stub() { # stub NAME BODY : creates an executable in $W/bin
  mkdir -p "$W/bin"; printf '#!/bin/sh\n%s\n' "$2" > "$W/bin/$1"; chmod +x "$W/bin/$1"
}
good_docker() {
  stub docker 'case "$1 $2" in
  "info "*) echo /var/lib/docker; exit 0;;
  "version -f") echo 27.0.0; exit 0;;
  "compose version") echo v2.30.0; exit 0;;
esac
case "$1" in info) echo /var/lib/docker;; version) echo 27.0.0;; esac; exit 0'
}
run_install() { # run_install ARGS... -> output in $W/out, exit code in $RC
  PATH="$W/bin:$PATH" INSTALL_PORTS="${PORTS:-59998}" ENV_FILE="${ENVF:-$W/none.env}" DOMINET_ADMIN_PASSWORD="${PASS-correct-horse-battery}" \
    ./install.sh --dry-run --yes --lang en "$@" > "$W/out" 2>&1
  RC=$?
}
has() { grep -q -- "$1" "$W/out"; }

before="$(git status --porcelain 2>/dev/null | md5sum)"

echo "== installer scenarios"
good_docker
run_install --domain home.local;                         check "happy path: exit 0" "$RC"
has "Dry run finished" ;                                  check "happy path: says nothing was changed" $?
has "would run: LAN_DOMAIN=home.local make prod-init";    check "happy path: plans prod-init with the domain" $?
has "DOMINET_ADMIN_PASSWORD=\*\*\*";                      check "the password is never printed" $?
! grep -q "correct-horse-battery" "$W/out";               check "the password does not appear in the output" $?

run_install --lang fr --domain home.local; has "Essai terminé"; check "French output with --lang fr" $?

good_docker
stub docker 'exit 1'
run_install --domain home.local; [ "$RC" != 0 ];          check "Docker not reachable: blocks" $?
has "not reachable";                                      check "Docker not reachable: explains the docker group" $?

stub docker 'case "$1 $2" in "compose version") echo v2.20.0;; *) echo ok;; esac; exit 0'
run_install --domain home.local; [ "$RC" != 0 ];          check "old Compose: blocks" $?
has "too old";                                            check "old Compose: says 2.24 is needed" $?

good_docker
stub uname 'case "$1" in -s) echo Darwin;; -r) echo 1;; -m) echo arm64;; *) echo Darwin;; esac'
run_install --domain home.local; [ "$RC" != 0 ];          check "not Linux: blocks" $?
has "Linux machine is required";                          check "not Linux: explains" $?
rm -f "$W/bin/uname"

stub uname 'case "$1" in -s) echo Linux;; -r) echo 6.1;; -m) echo aarch64;; *) echo Linux;; esac'
run_install --domain home.local; [ "$RC" = 0 ];           check "arm64: only a warning" $?
has "best-effort";                                        check "arm64: says it is best effort" $?
rm -f "$W/bin/uname"

printf 'Raspberry Pi 5 Model B Rev 1.0\0' > "$W/model"
INSTALL_MODEL_FILE="$W/model" INSTALL_ROOT_DEV=mmcblk0 run_install --domain home.local; [ "$RC" = 0 ]; check "Raspberry Pi on SD card: warning only" $?
has "Raspberry Pi 5 Model B";                             check "Raspberry Pi: model named" $?
has "SD card";                                            check "SD card: advises an SSD" $?
INSTALL_ROOT_DEV=nvme0n1 run_install --domain home.local; ! has "SD card"; check "NVMe: no SD card warning" $?

python3 -m http.server 59997 --bind 127.0.0.1 >/dev/null 2>&1 & HOLD_PID=$!
sleep 1
PORTS=59997 run_install --domain home.local; [ "$RC" != 0 ]; check "port in use: blocks" $?
has "port 59997 is already used";                         check "port in use: names the port" $?
kill "$HOLD_PID" 2>/dev/null; HOLD_PID=""

printf 'DB_PASSWORD=x\n' > "$W/existing.env"
ENVF="$W/existing.env" run_install --domain home.local; [ "$RC" != 0 ]; check "existing .env: refuses to overwrite" $?
has "rerun with --reuse";                                 check "existing .env: suggests --reuse" $?
ENVF="$W/existing.env" run_install --reuse;               check "existing .env with --reuse: accepted" "$RC"

PASS=short run_install --domain home.local; [ "$RC" = 2 ]; check "short password: refused with --yes" $?
PASS="" run_install --domain home.local; [ "$RC" = 2 ];   check "missing password: refused with --yes" $?
PASS=1234567890123 run_install --domain home.local; [ "$RC" = 2 ]; check "numeric-only password: refused with --yes" $?
PASS=admin12345 run_install --domain home.local --admin admin12345; [ "$RC" = 2 ]; check "password equal to the account name: refused" $?

echo
echo "== image source (prod.sh pull-image)"
pull_run() { # pull_run ENV-CONTENT -> output in $W/out, exit code in $RC, docker calls in $W/calls
  printf '%b' "$1" > "$W/pull.env"; : > "$W/calls"
  PATH="$W/bin:$PATH" ENV_FILE="$W/pull.env" tools/prod.sh pull-image > "$W/out" 2>&1
  RC=$?
}
stub docker 'echo "$*" >> '"$W"'/calls; exit 0'
pull_run 'DOMINET_TAG=local\n'
[ "$RC" = 0 ];                                            check "pull works: exit 0 (no build needed)" $?
grep -q "pull -q ghcr.io/deunix-educ/domia-automation:latest" "$W/calls"; check "pull works: default image and version" $?
grep -q "tag ghcr.io/deunix-educ/domia-automation:latest dominet:local" "$W/calls"; check "pull works: tagged for Compose as dominet:<tag>" $?
pull_run 'DOMINET_REGISTRY_IMAGE=example.org/me/app\nDOMINET_IMAGE_VERSION=1.2\n'
grep -q "pull -q example.org/me/app:1.2" "$W/calls";      check "registry and version come from .env" $?
stub docker 'echo "$*" >> '"$W"'/calls; case "$1" in pull) exit 1;; esac; exit 0'
pull_run 'DOMINET_TAG=local\n'
[ "$RC" = 1 ];                                            check "pull fails: exit 1 so the caller builds" $?
has "local build" ;                                       check "pull fails: says it builds locally" $?
! grep -q "^tag " "$W/calls";                             check "pull fails: nothing is tagged" $?
pull_run 'DOMINET_PULL=0\n'
[ "$RC" = 1 ] && ! grep -q "^pull" "$W/calls";            check "DOMINET_PULL=0: no network attempt" $?

echo
echo "== diagnosis (prod.sh doctor)"
printf 'DB_PASSWORD=x\nLAN_DOMAIN=home.local\nDOMINET_MASTER_KEY=abc\n' > "$W/doc.env"; chmod 600 "$W/doc.env"
stub docker 'case "$*" in
  *"config --services"*) printf "dominet\\npostgres\\nmosquitto\\n";;
  *"--status restarting"*) echo mosquitto;;
  *"--status running"*) printf "dominet\\npostgres\\n";;
  *"exec -T dominet true"*) exit 1;;
  *"compose"*"ps"*"--format"*) printf "dominet healthy\\npostgres unhealthy\\n";;
esac; exit 0'
PATH="$W/bin:$PATH" ENV_FILE="$W/doc.env" CERTS_DIR="$W/nocerts" tools/prod.sh doctor > "$W/out" 2>&1; RC=$?
[ "$RC" != 0 ];                                           check "doctor: exit code is non-zero when something blocks" $?
has "BLOCKING  mosquitto restarts in a loop";             check "doctor: names a service that restarts in a loop" $?
has "BLOCKING  postgres is unhealthy";                    check "doctor: names an unhealthy service" $?
has "OK        dominet running";                          check "doctor: lists a healthy service as OK" $?
has "make prod-logs S=mosquitto";                         check "doctor: gives the command to read the log" $?
! grep -q "DB_PASSWORD\|=abc" "$W/out";                  check "doctor: shows no secret" $?

after="$(git status --porcelain 2>/dev/null | md5sum)"
[ "$before" = "$after" ];                                 check "a dry run leaves the working tree untouched" $?

echo
if [ "$fails" -gt 0 ]; then echo "check-installer: $fails of $total checks FAILED"; exit 1; fi
echo "check-installer: $total checks passed"
