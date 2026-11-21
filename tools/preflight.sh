#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Verifications AVANT d'installer DomiNet sur une machine (R8 lot 4). LECTURE SEULE : rien n'est cree, modifie, arrete ni supprime ;
# les conteneurs et volumes existants (d'une autre installation, par exemple) sont seulement listes. Code de sortie = nombre de points BLOQUANTS.
#
#   tools/preflight.sh                  (ou : make preflight)
#   ENV_FILE=autre.env tools/preflight.sh    ports lus dans ce fichier (NGINX_HTTP_PORT, NGINX_HTTPS_PORT, MQTT_LAN_PORT, POSTGRES_HOST_PORT)
set -uo pipefail
cd "$(dirname "$0")/.."
ENV_FILE="${ENV_FILE:-.env}"
PROJECT="${PROJECT:-dominet}"
val() { grep -s "^$1=" "$ENV_FILE" | tail -1 | cut -d= -f2- | sed 's/[[:space:]]*#.*//' | tr -d ' "' || true; }

block=0; warn=0
ok()   { printf '  OK         %s\n' "$*"; }
att()  { printf '  ATTENTION  %s\n' "$*"; warn=$((warn + 1)); }
bad()  { printf '  BLOQUANT   %s\n' "$*"; block=$((block + 1)); }
info() { printf '             %s\n' "$*"; }
section() { printf '\n== %s\n' "$*"; }
version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; }

section "Machine"
ok "$(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-systeme inconnu}"), noyau $(uname -r), $(uname -m)"
case "$(uname -m)" in
  x86_64) ok "architecture x86_64" ;;
  aarch64|arm64) att "architecture $(uname -m) : prise en charge au mieux (image arm64 publiee, sinon construite ici, ce qui est long ; voir la page Installation rapide)" ;;
  *) att "architecture $(uname -m) non testee" ;;
esac
[ "$(uname -m)" != x86_64 ] || grep -q sse4_2 /proc/cpuinfo 2>/dev/null && ok "processeur x86-64-v2 (SSE4.2) : le binaire peut s'executer" || bad "processeur sans SSE4.2 : le binaire (x86-64-v2) ne s'executera pas"
mem_gb=$(awk '/MemTotal/ {printf "%d", $2/1048576}' /proc/meminfo)
[ "$mem_gb" -ge 4 ] && ok "memoire ${mem_gb} Go" || att "memoire ${mem_gb} Go : 4 Go ou plus recommandes (12 cameras en detection complete : ~2 Go pour l'application)"
nproc_n=$(nproc); ok "$nproc_n coeurs"

section "Docker"
if command -v docker >/dev/null 2>&1; then
  dv=$(docker version -f '{{.Server.Version}}' 2>/dev/null || true)
  if [ -n "$dv" ]; then ok "moteur Docker $dv"; else bad "Docker installe mais inaccessible (droits ? demon arrete ?) : docker version"; fi
  cv=$(docker compose version --short 2>/dev/null | sed 's/^v//' || true)
  if [ -z "$cv" ]; then bad "plugin « docker compose » absent"
  elif version_ge "$cv" 2.24; then ok "Docker Compose $cv (2.24 ou plus recent requis : !override, !reset)"
  else bad "Docker Compose $cv trop ancien (2.24 ou plus requis)"; fi
  root=$(docker info -f '{{.DockerRootDir}}' 2>/dev/null || echo /var/lib/docker)
  free_gb=$(df -P --output=avail -BG "$root" 2>/dev/null | tail -1 | tr -dc '0-9')
  if [ -n "$free_gb" ]; then
    if [ "$free_gb" -ge 20 ]; then ok "espace libre sous $root : ${free_gb} Go"
    elif [ "$free_gb" -ge 10 ]; then att "espace libre sous $root : ${free_gb} Go (image ~700 Mo + piles ; les enregistrements video en demandent beaucoup plus)"
    else bad "espace libre sous $root : ${free_gb} Go (moins de 10 Go)"; fi
  fi
  if curl -fsS -m 8 -o /dev/null https://registry-1.docker.io/v2/ 2>/dev/null || curl -sS -m 8 -o /dev/null -w '%{http_code}' https://registry-1.docker.io/v2/ 2>/dev/null | grep -q 401; then
    ok "Docker Hub joignable (telechargement des images postgres, mosquitto, grafana...)"
  else att "Docker Hub injoignable depuis cette machine : les images doivent deja etre presentes ou passer par un miroir"; fi
else
  bad "Docker absent (https://docs.docker.com/engine/install/)"
fi
for c in git make openssl curl; do command -v "$c" >/dev/null 2>&1 && ok "$c present" || bad "$c absent"; done
command -v jq >/dev/null 2>&1 && ok "jq present" || att "jq absent (facultatif)"

section "Ports de l'hote (doivent etre libres)"
http=$(val NGINX_HTTP_PORT); https=$(val NGINX_HTTPS_PORT); mqtt=$(val MQTT_LAN_PORT); pg=$(val POSTGRES_HOST_PORT)
listening() { # listening PROTO PORT -> processus ou "?" si le port est pris
  local flag=-ltnH; [ "$1" = udp ] && flag=-lunH
  ss $flag 2>/dev/null | awk -v p=":$2" '$4 ~ p"$" {found=1} END {exit !found}'
}
holder() { ss -ltnpH 2>/dev/null | awk -v p=":$1" '$4 ~ p"$" && $NF ~ /^users:/ {print $NF; exit}' | sed 's/users:((//; s/))//' ; }
for spec in "tcp ${http:-80} HTTP (nginx)" "tcp ${https:-443} HTTPS (nginx)" "tcp ${mqtt:-1883} MQTT (Mosquitto, le LAN)" "tcp 8554 RTSP (MediaMTX)" "tcp 8189 WebRTC ICE (MediaMTX)" "udp 8189 WebRTC ICE (MediaMTX)" "tcp ${pg:-5432} PostgreSQL (boucle locale, decouverte ONVIF)"; do
  set -- $spec; proto=$1; port=$2; shift 2
  if listening "$proto" "$port"; then bad "$proto/$port ($*) deja utilise$( h=$(holder "$port"); [ -n "$h" ] && echo " par $h" )"; else ok "$proto/$port libre ($*)"; fi
done
for spec in "tcp 8080 interface Zigbee2MQTT (profil zigbee)" "udp 51820 WireGuard (profil vpn)"; do
  set -- $spec; proto=$1; port=$2; shift 2
  if listening "$proto" "$port"; then att "$proto/$port ($*) utilise : le profil correspondant ne pourra pas demarrer"; else ok "$proto/$port libre ($*)"; fi
done

section "Dongle Zigbee"
zd=""; for d in /dev/ttyUSB0 /dev/ttyACM0; do [ -e "$d" ] && { zd="$d"; break; }; done
if [ -n "$zd" ]; then
  ok "dongle detecte : $zd"
  [ -L /dev/serial/by-id ] || ls /dev/serial/by-id/* >/dev/null 2>&1 && info "$(ls /dev/serial/by-id/ 2>/dev/null | head -2 | tr '\n' ' ')"
  if command -v fuser >/dev/null 2>&1 && fuser "$zd" >/dev/null 2>&1; then bad "$zd est deja tenu par un processus (un seul peut le tenir) : $(fuser "$zd" 2>&1 | tr -s ' ')"; else ok "$zd libre (aucun processus ne le tient)"; fi
  [ -r "$zd" ] && [ -w "$zd" ] && ok "$zd lisible et ecrivable par $(id -un)" || att "$zd non accessible par $(id -un) : Docker (root) le passera au conteneur, mais verifiez le groupe dialout pour les essais a la main"
else
  att "aucun dongle (/dev/ttyUSB0, /dev/ttyACM0) : Zigbee2MQTT sera desactive ; branchez-le avant « make prod-init » pour qu'il soit detecte"
fi
[ -f deploy/zigbee2mqtt/configuration.yaml ] && att "deploy/zigbee2mqtt/configuration.yaml existe deja : il sera conserve tel quel (appairage et cles reseau)" || ok "pas de configuration Zigbee2MQTT existante (l'exemple sera copie)"

section "Video"
[ -e /dev/dri/renderD128 ] && ok "GPU /dev/dri/renderD128 (decodage VAAPI : active par make prod-init, ou make prod-video sur une installation existante)" || att "pas de /dev/dri/renderD128 : decodage video logiciel (plus de CPU)"
[ -f models/yolov8n.onnx ] && ok "modele de detection present" || att "modele de detection absent : « make fetch-model » (fait automatiquement par « make prod-up »)"

section "Certificats et noms"
if command -v mkcert >/dev/null 2>&1; then
  ok "mkcert present"
  root=$(mkcert -CAROOT 2>/dev/null || true)
  if [ -n "$root" ] && [ -f "$root/rootCA.pem" ]; then ok "autorite locale existante : $root (sera reutilisee par « make certs » : les appareils qui la connaissent n'ont rien a reinstaller)"; else att "pas d'autorite mkcert existante : une nouvelle sera creee ; chaque appareil installera /dominet-ca.crt une fois"; fi
else
  att "mkcert absent : certificat auto-signe de repli (avertissement dans les navigateurs) ; « sudo apt install mkcert » pour une autorite locale"
fi
hn=$(hostname)
getent hosts "$hn.local" >/dev/null 2>&1 && ok "$hn.local se resout" || att "$hn.local ne se resout pas ici : installez avahi-daemon ou fixez LAN_DOMAIN dans .env"

section "Existant (liste seulement, JAMAIS touche)"
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  old=$(docker ps -a --format '{{.Names}}\t{{.Status}}' | grep -Ei 'dominet|zigbee|mosquitto|mediamtx|reduct|grafana|prometheus|postgres' || true)
  if [ -n "$old" ]; then
    att "conteneurs existants qui ressemblent a une pile domotique :"
    printf '%s\n' "$old" | sed 's/^/               /'
    echo "$old" | grep -Eq "^${PROJECT}-(nginx|dominet|dominet-discovery|postgres|reductstore|mosquitto|mediamtx|prometheus|grafana|migrate|init|zigbee2mqtt|wg-easy|adminer)-[0-9]+" && bad "des conteneurs du projet Compose « $PROJECT » existent deja : choisissez PROJECT=autre-nom ou arretez-les vous-meme"
  else ok "aucun conteneur d'une pile domotique"; fi
  vols=$(docker volume ls -q | grep -Ei 'dominet|postgres|reduct|mosquitto|grafana|prometheus' || true)
  if [ -n "$vols" ]; then att "volumes existants (ne seront pas supprimes ; la nouvelle pile utilise le prefixe « ${PROJECT}_ ») :"; printf '%s\n' "$vols" | head -12 | sed 's/^/               /'; else ok "aucun volume existant"; fi
fi
[ -f "$ENV_FILE" ] && att "$ENV_FILE existe deja : « make prod-init » refusera de l'ecraser (c'est voulu)" || ok "pas de $ENV_FILE : « make prod-init » le creera"

section "Depot"
if git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
  ok "branche $(git rev-parse --abbrev-ref HEAD), $(git rev-parse --short HEAD)$(git describe --tags --abbrev=0 2>/dev/null | sed 's/^/, dernier tag /')"
  [ -z "$(git status --porcelain 2>/dev/null | grep -v '^??')" ] && ok "arbre de travail propre" || att "modifications locales non commitees"
fi

printf '\n'
if [ "$block" -gt 0 ]; then printf 'RESULTAT : %d point(s) BLOQUANT(S), %d attention(s). N installez pas avant de les avoir traites.\n' "$block" "$warn"
else printf 'RESULTAT : aucun point bloquant, %d attention(s). Vous pouvez lancer « make prod-init ».\n' "$warn"; fi
exit "$block"
