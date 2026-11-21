#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Machine card: what this machine is and how the installed stack behaves on it, as Markdown to paste into docs/measures.md or an issue.
# READ-ONLY (it only reads and times a few HTTP requests). The one exception, `--with-backup`, runs a normal backup and times it.
# Contains no password and no address other than the host name. Run it after `make prod-deploy`, once the stack has been idle for a few minutes.
#
#   tools/machine-card.sh                   > card.md
#   tools/machine-card.sh --with-backup     > card.md   also times `make prod-backup`
#   REQUESTS=100 tools/machine-card.sh                  number of timed requests (default 50)
set -uo pipefail
cd "$(dirname "$0")/.."
ENV_FILE="${ENV_FILE:-.env}"
PROJECT="${PROJECT:-dominet}"
REQUESTS="${REQUESTS:-50}"
BACKUP=0; [ "${1:-}" = "--with-backup" ] && BACKUP=1
val() { grep -s "^$1=" "$ENV_FILE" | tail -1 | cut -d= -f2- | sed 's/[[:space:]]*#.*//' | tr -d ' "' || true; }
row() { printf '| %s | %s |\n' "$1" "$2"; }

model_file="${CARD_MODEL_FILE:-/proc/device-tree/model}"
model=""; [ -r "$model_file" ] && model="$(tr -d '\0' < "$model_file")"
cpu="$(awk -F': ' '/model name|Model name/ {print $2; exit}' /proc/cpuinfo 2>/dev/null)"
[ -n "$cpu" ] || cpu="$(lscpu 2>/dev/null | awk -F': +' '/Model name/ {print $2; exit}')"
mem_mb="$(awk '/MemTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null)"
swap_mb="$(awk '/SwapTotal/ {printf "%d", $2/1024}' /proc/meminfo 2>/dev/null)"
os="$(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown}")"
root_src="$(findmnt -no SOURCE / 2>/dev/null || true)"
root_dev="$(lsblk -no PKNAME "$root_src" 2>/dev/null | head -1)"; root_dev="${root_dev:-$(basename "${root_src:-unknown}")}"
case "$root_dev" in
  mmcblk*) storage="SD card (wears out under database and video writes: an SSD is strongly advised)" ;;
  nvme*)   storage="NVMe SSD" ;;
  *) if [ "$(cat "/sys/block/$root_dev/queue/rotational" 2>/dev/null)" = 1 ]; then storage="hard disk (rotational)"; else storage="SSD or similar ($root_dev)"; fi ;;
esac

echo "# Machine card"
echo
echo "Generated $(date -u +%Y-%m-%dT%H:%MZ) by tools/machine-card.sh. Host name: $(hostname)."
echo
echo "## Hardware and system"
echo
echo "| | |"; echo "|---|---|"
row "Model" "${model:-n/a}"
row "Processor" "${cpu:-n/a} ($(uname -m), $(nproc) cores)"
row "Memory / swap" "${mem_mb:-?} MB / ${swap_mb:-0} MB"
row "Storage of /" "$storage"
row "System" "$os, kernel $(uname -r)"
if command -v vcgencmd >/dev/null 2>&1; then
  row "Temperature" "$(vcgencmd measure_temp 2>/dev/null | cut -d= -f2)"
  th="$(vcgencmd get_throttled 2>/dev/null | cut -d= -f2)"
  row "Throttling flags" "${th:-n/a} (0x0 = never throttled; anything else: power supply or cooling problem)"
fi
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  row "Docker" "$(docker version -f '{{.Server.Version}}' 2>/dev/null), Compose $(docker compose version --short 2>/dev/null)"
  tag="$(val DOMINET_TAG)"; tag="${tag:-local}"
  arch="$(docker image inspect "dominet:$tag" --format '{{.Architecture}}' 2>/dev/null)" || arch="not present"
  row "Application image" "dominet:$tag, architecture $arch"
fi
row "Hardware video decoding" "$([ -e /dev/dri/renderD128 ] && echo 'VAAPI node present' || echo 'none (software decoding)')"

echo
echo "## Stack at rest"
echo
if ! docker info >/dev/null 2>&1; then
  echo "Docker not reachable: stack measures skipped."
else
  mapfile -t ids < <(docker ps -q --filter "label=com.docker.compose.project=$PROJECT" 2>/dev/null)
  if [ "${#ids[@]}" -eq 0 ]; then
    echo "No running container for project \`$PROJECT\`: stack measures skipped."
  else
    echo "| Service | CPU | Memory |"; echo "|---|---|---|"
    docker stats --no-stream --format '{{.Name}}|{{.CPUPerc}}|{{.MemUsage}}' "${ids[@]}" 2>/dev/null | sed "s/^${PROJECT}-//; s/-1|/|/" | sort | awk -F'|' '{printf "| %s | %s | %s |\n", $1, $2, $3}'
    echo
    free -m | awk 'NR==2 {printf "Whole machine: %d MB used, %d MB available (of %d MB).\n", $3, $7, $2}'
    echo
    echo "## Interface latency"
    echo
    domain="$(val LAN_DOMAIN)"; domain="${domain:-localhost}"; port="$(val NGINX_HTTPS_PORT)"; port="${port:-443}"
    for path in /healthz /login; do
      times="$(for _ in $(seq 1 "$REQUESTS"); do curl -sk -o /dev/null -w '%{http_code} %{time_total}\n' --resolve "$domain:$port:127.0.0.1" "https://$domain:$port$path" 2>/dev/null; done | awk '$1 == 200 {print $2}' | sort -n)"
      n="$(printf '%s\n' "$times" | grep -c .)"
      if [ "$n" -eq 0 ]; then echo "- \`$path\`: no HTTP 200 answer."; else
        med="$(printf '%s\n' "$times" | sed -n "$(( (n + 1) / 2 ))p")"; p95="$(printf '%s\n' "$times" | sed -n "$(( (n * 95 + 99) / 100 ))p")"
        echo "- \`$path\`: median $(awk -v t="$med" 'BEGIN {printf "%.0f", t*1000}') ms, 95th percentile $(awk -v t="$p95" 'BEGIN {printf "%.0f", t*1000}') ms ($n requests, one at a time)."
      fi
    done
    if [ "$BACKUP" = 1 ]; then
      echo
      echo "## Backup"
      echo
      start=$(date +%s); if tools/prod.sh backup >/dev/null 2>&1; then echo "- \`make prod-backup\`: $(( $(date +%s) - start )) s."; else echo "- \`make prod-backup\`: FAILED."; fi
    fi
  fi
fi
echo
echo "## To add by hand"
echo
echo "- Time to start from cold: \`make prod-down\`, then \`time make prod-up\` (the stack is stopped meanwhile)."
echo "- Number of cameras held (see \`docs/measures.md\` section 2 for the method), and whether detection was enabled."
echo "- Duration of the first installation, and whether the image was downloaded or built."
