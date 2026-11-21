#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Exploitation de la pile de production (R7 lot 6). Appele par les cibles `make prod-*` ; jamais a lancer sur une machine que l'on ne
# veut pas deployer. Variables : ENV_FILE (defaut .env), PROJECT (defaut dominet), CERTS_DIR (defaut ./certs).
# Sous-commandes : init pull-image compose doctor guard certs export-ca up superuser seed seed-energy health deploy update migrate passwd dump-context backup backups mqtt-user mqtt-del mqtt-users report video logs ps down
set -euo pipefail
cd "$(dirname "$0")/.."

ENV_FILE="${ENV_FILE:-.env}"
PROJECT="${PROJECT:-dominet}"
CERTS_DIR="${CERTS_DIR:-certs}"
DEPLOY=deploy

val() { grep -s "^$1=" "$ENV_FILE" | tail -1 | cut -d= -f2- | sed 's/[[:space:]]*#.*//' | tr -d ' "' || true; }
rand() { head -c 48 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c "${1:-24}"; }
say() { printf '%s\n' "$*"; }
die() { printf '\n  Erreur : %s\n\n' "$*" >&2; exit 1; }
setvar() { # setvar NOM VALEUR : remplace ou ajoute NOM=VALEUR dans le fichier d'environnement
  local k="$1" v="$2"
  if grep -q "^$k=" "$ENV_FILE"; then
    local esc; esc="$(printf '%s' "$v" | sed 's/[|&\\]/\\&/g')"
    sed -i "s|^$k=.*|$k=$esc|" "$ENV_FILE"
  else
    { [ -z "$(tail -c1 "$ENV_FILE")" ] || echo >> "$ENV_FILE"; echo "$k=$v" >> "$ENV_FILE"; }
  fi
}

profiles() {
  local p=()
  local zdev; zdev="$(val ZIGBEE_DEVICE)"
  [ -n "$zdev" ] && [ -e "$zdev" ] && p+=(--profile zigbee)
  [ -n "$(val WG_ADMIN_HASH)" ] && p+=(--profile vpn)
  [ "${ADMINER:-}" = 1 ] && p+=(--profile adminer)
  [ "${#p[@]}" -eq 0 ] || printf '%s\n' "${p[@]}"
}
tag() { local t; t="$(val DOMINET_TAG)"; echo "dominet:${t:-local}"; }
compose() {
  local pr=(); mapfile -t pr < <(profiles)
  CERTS_DIR="$(case "$CERTS_DIR" in /*) echo "$CERTS_DIR";; *) echo "$(pwd)/$CERTS_DIR";; esac)" docker compose -p "$PROJECT" --env-file "$ENV_FILE" \
    -f "$DEPLOY/docker-compose.yml" -f "$DEPLOY/docker-compose.prod.yml" "${pr[@]}" "$@"
}
dominet() { compose exec -T "$@" ; }

# Decodage video materiel : si l'hote a /dev/dri/renderD128, ecrit VIDEO_DEVICE et RENDER_GID dans .env (le compose passe le peripherique
# au conteneur). Idempotent ; sans GPU, ne fait rien.
enable_video() {
  local node=/dev/dri/renderD128 gid
  if [ -e "$node" ]; then
    gid="$(stat -c %g "$node")"
    setvar VIDEO_DEVICE /dev/dri
    setvar RENDER_GID "$gid"
    return 0
  fi
  return 1
}
cmd_video() {
  [ -f "$ENV_FILE" ] || die "$ENV_FILE absent : make prod-init d'abord."
  if enable_video; then
    say "  VIDEO_DEVICE=/dev/dri et RENDER_GID=$(val RENDER_GID) ecrits dans $ENV_FILE."
    say "  Il reste a recreer le service (l'image doit contenir le pilote : make prod-up reconstruit puis redemarre)."
  else
    say "  Pas de /dev/dri/renderD128 sur cette machine : decodage logiciel conserve."
  fi
}

cmd_init() {
  if [ -f "$ENV_FILE" ] && grep -qE '^DB_PASSWORD=.+' "$ENV_FILE"; then
    die "$ENV_FILE existe deja avec DB_PASSWORD renseigne. Supprimez-le a la main pour reinitialiser ($ENV_FILE n'est jamais ecrase)."
  fi
  [ -f "$ENV_FILE" ] || cp .env.example.prod "$ENV_FILE"
  chmod 600 "$ENV_FILE"
  local domain ip zdev
  domain="${LAN_DOMAIN:-$(hostname).local}"
  ip="$(hostname -I 2>/dev/null | awk '{print $1}')"; ip="${ip:-127.0.0.1}"
  zdev=""; for d in /dev/ttyUSB0 /dev/ttyACM0; do [ -e "$d" ] && { zdev="$d"; break; }; done
  local dbpass; dbpass="$(rand 24)"; local gpass; gpass="$(rand 18)"
  setvar LAN_DOMAIN "$domain"
  setvar LOCAL_DOMAIN "$ip"
  setvar ALLOWED_HOSTS "$domain,$ip,localhost"
  setvar CSRF_TRUSTED_ORIGINS "https://$domain,https://$ip"
  setvar DB_PASSWORD "$dbpass"
  setvar SECRET_KEY "$(rand 50)"
  setvar REDUCTSTORE_TOKEN "dominet-$(rand 32)"
  setvar MQTT_PASSWORD "$(rand 24)"
  setvar GRAFANA_PASSWORD "$gpass"
  setvar ZIGBEE_AUTH_TOKEN "$(rand 24)"
  if docker image inspect "$(tag)" >/dev/null 2>&1; then
    setvar DOMINET_MASTER_KEY "$(docker run --rm "$(tag)" gen-master-key | tail -1)"
  fi
  setvar GRAFANA_PUBLIC_URL "https://$domain"
  setvar MEDIAMTX_WEBRTC_DOMAIN "$domain/webrtc"
  setvar WEBRTC_ADDITIONAL_HOSTS "$domain,$ip"
  setvar WG_ALLOWED_IPS "10.20.0.0/24,${ip%.*}.0/24"
  [ -n "$zdev" ] && setvar ZIGBEE_DEVICE "$zdev"
  enable_video
  say ""
  say "  $ENV_FILE initialise (droits 600) :"
  say "    LAN_DOMAIN        = $domain"
  say "    LOCAL_DOMAIN      = $ip"
  say "    secrets           = generes (base, ReductStore, MQTT, Grafana, jeton Zigbee)"
  say "    GRAFANA_PASSWORD  = $gpass   <-- connexion Grafana (admin)"
  say "    ZIGBEE_DEVICE     = ${zdev:-aucun dongle detecte (Zigbee2MQTT desactive)}"
  say "    decodage video    = $(if [ "$(val VIDEO_DEVICE)" = /dev/dri ]; then echo "materiel VAAPI (groupe render $(val RENDER_GID))"; else echo "logiciel (pas de /dev/dri/renderD128)"; fi)"
  if [ -z "$(val DOMINET_MASTER_KEY)" ]; then
    say "    DOMINET_MASTER_KEY: vide, sera generee par « make prod-deploy » (image construite)"
  fi
  say ""
  say "  A relire dans $ENV_FILE : EMAIL_* et EMAIL_RECIPIENT (alertes), DYNDNS_HOST + WG_ADMIN_HASH (VPN)."
  say "  SAUVEGARDEZ DOMINET_MASTER_KEY hors de cette machine (sans elle, les secrets chiffres en base sont perdus)."
  say "  Puis : make prod-deploy"
}

cmd_guard() {
  [ -f "$ENV_FILE" ] || die "$ENV_FILE manquant : lancez d'abord « make prod-init »."
  local bad="" v x
  for v in DB_PASSWORD GRAFANA_PASSWORD REDUCTSTORE_TOKEN MQTT_PASSWORD; do
    x="$(val $v)"; case "$x" in ""|changeme*|TODO*) bad="$bad $v";; esac
  done
  [ -z "$bad" ] || die "valeur vide ou par defaut dans $ENV_FILE :$bad (« make prod-init » genere ces secrets)."
  for v in LAN_DOMAIN ALLOWED_HOSTS CSRF_TRUSTED_ORIGINS; do
    case "$(val $v)" in *TODO*|"") die "$v non renseigne dans $ENV_FILE";; esac
  done
  if [ -n "$(val WG_ADMIN_HASH)" ] && [ -z "$(val DYNDNS_HOST)" ]; then die "WG_ADMIN_HASH renseigne mais DYNDNS_HOST vide (VPN)."; fi
  local zdev; zdev="$(val ZIGBEE_DEVICE)"
  if [ -n "$zdev" ] && [ -e "$zdev" ] && [ -z "$(val ZIGBEE_AUTH_TOKEN)" ]; then die "dongle $zdev present mais ZIGBEE_AUTH_TOKEN vide."; fi
  if grep -qE '^[A-Z_0-9]+==' "$ENV_FILE" 2>/dev/null; then
    say "  Attention : « == » (double signe egal) dans $ENV_FILE, la valeur commencerait par « = » : $(grep -E '^[A-Z_0-9]+==' "$ENV_FILE" | cut -d= -f1 | tr '\n' ' ')"
  fi
  if [ "$(val VIDEO_DEVICE)" = /dev/dri ]; then
    case "$(val RENDER_GID)" in ''|*[!0-9]*) die "VIDEO_DEVICE=/dev/dri mais RENDER_GID absent ou non numerique (make prod-video).";; esac
  fi
  [ -n "$(val EMAIL_RECIPIENT)" ] || say "  Attention : EMAIL_RECIPIENT vide, les alertes Grafana n'iront nulle part (adresse de repli)."
  [ -n "$(val DOMINET_MASTER_KEY)" ] || say "  Attention : DOMINET_MASTER_KEY vide, les mots de passe de cameras seront refuses."
  say "  .env valide."
}

cmd_certs() {
  mkdir -p "$CERTS_DIR"
  local domain ip extra
  domain="$(val LAN_DOMAIN)"; domain="${domain:-localhost}"
  ip="$(val LOCAL_DOMAIN)"; [ -n "$ip" ] || ip="$(hostname -I | awk '{print $1}')"
  extra="$(val CERT_EXTRA_SANS | tr ',' ' ')"
  if command -v mkcert >/dev/null 2>&1; then
    # Installe l'autorite dans le magasin de confiance de CETTE machine (sudo) : utile pour y tester, pas indispensable au serveur.
    mkcert -install || say "  (mkcert -install n'a pas pu modifier le magasin de confiance de cette machine : sans importance pour les autres appareils.)"
    # shellcheck disable=SC2086
    mkcert -cert-file "$CERTS_DIR/cert.pem" -key-file "$CERTS_DIR/key.pem" "$domain" localhost 127.0.0.1 $ip $extra
    cmd_export_ca
  else
    say "  mkcert absent : certificat AUTO-SIGNE (openssl). Les navigateurs afficheront un avertissement ;"
    say "  installez mkcert (sudo apt install mkcert) et relancez « make certs » pour une autorite locale a installer sur les appareils."
    local san="DNS:$domain,DNS:localhost,IP:127.0.0.1" h
    for h in $ip $extra; do
      if [[ "$h" =~ ^[0-9.]+$ ]]; then san="$san,IP:$h"; else san="$san,DNS:$h"; fi
    done
    openssl req -x509 -newkey rsa:2048 -nodes -keyout "$CERTS_DIR/key.pem" -out "$CERTS_DIR/cert.pem" -days 825 \
      -subj "/CN=$domain" -addext "subjectAltName=$san" 2>/dev/null
    cp "$CERTS_DIR/cert.pem" "$CERTS_DIR/dominet-ca.crt"
  fi
  chmod 600 "$CERTS_DIR/key.pem"
  say "  Certificat dans $CERTS_DIR/ (valable pour $domain, localhost, $ip $extra)"
}

cmd_export_ca() {
  mkdir -p "$CERTS_DIR"
  if command -v mkcert >/dev/null 2>&1; then
    cp "$(mkcert -CAROOT)/rootCA.pem" "$CERTS_DIR/dominet-ca.crt"
    say "  Autorite exportee : $CERTS_DIR/dominet-ca.crt (telechargeable sur https://$(val LAN_DOMAIN)/dominet-ca.crt)"
  else
    [ -f "$CERTS_DIR/dominet-ca.crt" ] || cp "$CERTS_DIR/cert.pem" "$CERTS_DIR/dominet-ca.crt"
  fi
}

# Application image: tries the published multi-architecture image (GHCR) first, otherwise falls back to a local build.
# DOMINET_PULL=0 forces the build; DOMINET_REGISTRY_IMAGE / DOMINET_IMAGE_VERSION (.env) change the source (defaults below).
# Exit 0 if the image was downloaded and tagged under the name Compose expects, 1 otherwise (the caller then builds).
cmd_pull_image() {
  [ "$(val DOMINET_PULL)" != 0 ] || { say "  Image: local build requested (DOMINET_PULL=0)."; return 1; }
  local reg ver; reg="$(val DOMINET_REGISTRY_IMAGE)"; reg="${reg:-ghcr.io/deunix-educ/domia-automation}"
  ver="$(val DOMINET_IMAGE_VERSION)"; ver="${ver:-latest}"
  say "  Image: trying $reg:$ver ..."
  if docker pull -q "$reg:$ver" >/dev/null 2>&1 && docker tag "$reg:$ver" "$(tag)"; then
    say "  Image: downloaded (no build needed)."
    return 0
  fi
  say "  Image: not available, local build (15 to 40 minutes depending on the machine)."
  return 1
}

cmd_up() {
  [ -f "$DEPLOY/zigbee2mqtt/configuration.yaml" ] || { cp "$DEPLOY/zigbee2mqtt/configuration.example.yaml" "$DEPLOY/zigbee2mqtt/configuration.yaml"; say "  zigbee2mqtt/configuration.yaml cree"; }
  local zdev; zdev="$(val ZIGBEE_DEVICE)"
  say "  Zigbee2MQTT : $([ -n "$zdev" ] && [ -e "$zdev" ] && echo "active ($zdev)" || echo "desactive (pas de dongle)")"
  say "  VPN wg-easy : $([ -n "$(val WG_ADMIN_HASH)" ] && echo active || echo "desactive (WG_ADMIN_HASH vide)")"
  [ -f models/yolov8n.onnx ] || make fetch-model
  cmd_pull_image || compose build dominet
  if [ -z "$(val DOMINET_MASTER_KEY)" ]; then
    setvar DOMINET_MASTER_KEY "$(docker run --rm "$(tag)" gen-master-key | tail -1)"
    say "  DOMINET_MASTER_KEY generee dans $ENV_FILE : SAUVEGARDEZ-LA hors de cette machine."
  fi
  compose up -d --wait --wait-timeout 300
}

cmd_superuser() {
  local n du dn; du="$(val DB_USER)"; dn="$(val DB_NAME)"; n="$(compose exec -T postgres psql -U "${du:-dominet}" -d "${dn:-dominet}" -tAc "SELECT count(*) FROM users WHERE role = 'admin'" 2>/dev/null | tr -d ' \r')"
  if [ "${n:-0}" != 0 ]; then say "  Un administrateur existe deja : rien a creer."; return; fi
  if [ -n "${DOMINET_ADMIN_USER:-}" ] && [ -n "${DOMINET_ADMIN_PASSWORD:-}" ]; then
    compose exec -T -e DOMINET_PASSWORD="$DOMINET_ADMIN_PASSWORD" dominet dominet createsuperuser --username "$DOMINET_ADMIN_USER"
  else
    say "  Creation du compte administrateur :"
    compose exec dominet dominet createsuperuser
  fi
}

cmd_seed() { dominet dominet dominet seed; dominet dominet dominet sync-rules || true; }

cmd_health() {
  local domain code=000 i; domain="$(val LAN_DOMAIN)"; domain="${domain:-localhost}"
  local port; port="$(val NGINX_HTTPS_PORT)"; port="${port:-443}"
  say "  Verification https://$domain:$port/ (nginx -> dominet)"
  for i in $(seq 1 30); do
    code="$(curl -sk -o /dev/null -w '%{http_code}' --resolve "$domain:$port:127.0.0.1" "https://$domain:$port/healthz" 2>/dev/null || true)"
    [ "$code" = 200 ] && break; sleep 2
  done
  [ "$code" = 200 ] || die "pas de reponse valide (dernier code : $code) ; « make prod-logs s=dominet »"
  say "  OK  application (HTTP 200)"
  local bad; bad="$(compose ps --status restarting --services 2>/dev/null || true)"
  [ -z "$bad" ] || die "services en boucle de redemarrage : $bad"
  say "  OK  aucun service en boucle de redemarrage"
}

cmd_deploy() {
  cmd_guard
  [ -f "$CERTS_DIR/cert.pem" ] || cmd_certs
  cmd_export_ca
  cmd_up
  cmd_superuser
  cmd_seed
  cmd_health
  say ""
  say "  DomiNet est pret : https://$(val LAN_DOMAIN)"
}

cmd_update() {
  cmd_guard
  if [ -z "${NOBACKUP:-}" ]; then
    if [ -n "$(compose ps --status running --services 2>/dev/null | grep -x dominet || true)" ]; then
      say "  Sauvegarde avant mise a jour"
      dominet dominet dominet backup || die "sauvegarde echouee : mise a jour annulee (NOBACKUP=1 pour passer outre)."
    else
      say "  Pile arretee : sauvegarde ignoree"
    fi
  fi
  [ -n "${NOPULL:-}" ] || git pull --ff-only
  cmd_up
  cmd_seed
  cmd_health
}

# Bilan de surveillance (R8 lot 6) : LECTURE SEULE, rien n'est modifie et aucun secret n'est affiche. HOURS (12) : fenetre des journaux
# et des compteurs. Chaque section est independante : une section en echec affiche « n/a » et le bilan continue.
# Lignes qui ne sont PAS des erreurs bien qu'elles contiennent le mot : niveau WARN/INFO/DEBUG (tracing, MediaMTX « WAR »,
# Zigbee2MQTT « info: »), ou le champ `error=` d'un avertissement. Sensible a la casse : le niveau est en majuscules ou suivi de « : ».
NOT_ERROR='(^|[^A-Za-z])(WARN|WAR|WRN|INFO|INF|DEBUG|DEB)([^A-Za-z]|$)|(info|debug|warn|warning):'
# Les niveaux colores (« ESC[31mERROR ») n'ont pas de frontiere de mot : on retire les couleurs avant de compter.
strip_ansi() { sed 's/\x1b\[[0-9;]*m//g'; }

# Diagnostic de la machine et de la pile, en lecture seule (rien n'est modifie, aucun secret affiche), puis diagnostic applicatif
# (`dominet doctor`) dans le conteneur. Code de sortie : nombre de points bloquants.
cmd_doctor() {
  local block=0 warn=0
  ok() { printf '  %-9s %s\n' OK "$*"; }
  att() { printf '  %-9s %s\n' WARNING "$*"; warn=$((warn + 1)); }
  bad() { printf '  %-9s %s\n' BLOCKING "$*"; block=$((block + 1)); }
  say ""; say "== Machine"
  if [ ! -f "$ENV_FILE" ]; then bad "$ENV_FILE is missing. Fix: ./install.sh (or make prod-init)."; else
    ok "$ENV_FILE present"
    [ "$(stat -c %a "$ENV_FILE")" = 600 ] && ok "$ENV_FILE is readable by you only" || att "$ENV_FILE permissions are $(stat -c %a "$ENV_FILE"): chmod 600 $ENV_FILE"
    [ -n "$(val DOMINET_MASTER_KEY)" ] && ok "DOMINET_MASTER_KEY set (is it copied off this machine?)" || att "DOMINET_MASTER_KEY is empty"
  fi
  if ! docker info >/dev/null 2>&1; then bad "Docker is not reachable (daemon stopped, or user not in the docker group)."; else
    local free_gb root; root="$(docker info -f '{{.DockerRootDir}}' 2>/dev/null || echo /var/lib/docker)"
    free_gb="$(df -BG --output=avail "$root" 2>/dev/null | tail -1 | tr -dc '0-9' || true)"
    if [ -n "$free_gb" ]; then
      if [ "$free_gb" -lt 5 ]; then bad "only ${free_gb} GB free for Docker: recordings and the database will stop. Fix: free space or lower VIDEO_QUOTA_GB."
      elif [ "$free_gb" -lt 15 ]; then att "${free_gb} GB free for Docker"; else ok "${free_gb} GB free for Docker"; fi
    fi
  fi
  if [ -f "$CERTS_DIR/cert.pem" ]; then
    if openssl x509 -checkend 0 -noout -in "$CERTS_DIR/cert.pem" >/dev/null 2>&1; then
      openssl x509 -checkend 2592000 -noout -in "$CERTS_DIR/cert.pem" >/dev/null 2>&1 && ok "certificate valid for more than 30 days" || att "certificate expires within 30 days. Fix: make certs, then restart nginx."
    else bad "certificate expired. Fix: make certs, then restart nginx."; fi
  else att "no certificate in $CERTS_DIR (make certs)"; fi
  say ""; say "== Services"
  if ! compose ps >/dev/null 2>&1; then att "the stack does not answer (is it started? make prod-up)"; else
    local services running restarting unhealthy
    services="$(compose config --services 2>/dev/null | grep -vxE 'init|migrate' || true)"
    running="$(compose ps --status running --services 2>/dev/null || true)"
    restarting="$(compose ps --status restarting --services 2>/dev/null || true)"
    unhealthy="$(compose ps --format '{{.Service}} {{.Health}}' 2>/dev/null | awk '$2=="unhealthy" {print $1}' || true)"
    local s
    for s in $services; do
      if printf '%s\n' "$restarting" | grep -qx "$s"; then bad "$s restarts in a loop. Fix: make prod-logs S=$s N=80"
      elif printf '%s\n' "$unhealthy" | grep -qx "$s"; then bad "$s is unhealthy. Fix: make prod-logs S=$s N=80"
      elif printf '%s\n' "$running" | grep -qx "$s"; then ok "$s running"
      else att "$s is not running (make prod-up)"; fi
    done
    local domain port code; domain="$(val LAN_DOMAIN)"; domain="${domain:-localhost}"; port="$(val NGINX_HTTPS_PORT)"; port="${port:-443}"
    code="$(curl -sk -m 8 -o /dev/null -w '%{http_code}' --resolve "$domain:$port:127.0.0.1" "https://$domain:$port/healthz" 2>/dev/null || true)"
    [ "$code" = 200 ] && ok "https://$domain:$port/ answers (HTTP 200)" || bad "https://$domain:$port/ does not answer (HTTP ${code:-000}). Fix: make prod-logs S=nginx N=50"
  fi
  say ""; say "== Application"
  local app_block=0
  if compose exec -T dominet true >/dev/null 2>&1; then
    dominet dominet dominet doctor || app_block=$?
  else att "the application container is not running: application checks skipped"; fi
  say ""
  say "machine and stack: $block blocking, $warn warning(s)."
  [ "$block" -eq 0 ] && [ "$app_block" -eq 0 ] || { say "Details of a service: make prod-logs S=<service> N=100   |   full report to share: make prod-report"; return 1; }
  say "Nothing blocking."
}

cmd_report() {
  local h="${HOURS:-12}" u d tz
  u="$(val DB_USER)"; u="${u:-dominet}"; d="$(val DB_NAME)"; d="${d:-dominet}"; tz="$(val TZ)"; tz="${tz:-Europe/Paris}"
  sect() { printf '\n== %s\n' "$*"; }
  sql() { compose exec -T postgres psql -U "$u" -d "$d" -At -F ' | ' -c "$1" 2>/dev/null | sed 's/^/  /' || say "  n/a"; }

  sect "Machine ($(date '+%Y-%m-%d %H:%M %Z'), fenetre $h h)"
  say "  $(uptime -p 2>/dev/null || true) ; charge $(cut -d' ' -f1-3 /proc/loadavg 2>/dev/null)"
  free -m 2>/dev/null | awk 'NR==2 {printf "  memoire %d / %d Mo utilises, %d Mo disponibles\n", $3, $2, $7} NR==3 {printf "  swap %d / %d Mo\n", $3, $2}'
  df -h / 2>/dev/null | awk 'NR==2 {printf "  disque / : %s utilises sur %s (%s)\n", $3, $2, $5}'
  docker system df 2>/dev/null | sed 's/^/  /' || true

  sect "Services (etat, sante, redemarrages)"
  local id
  while read -r id; do
    [ -n "$id" ] || continue
    docker inspect -f '{{index .Config.Labels "com.docker.compose.service"}} {{.State.Status}} sante={{if .State.Health}}{{.State.Health.Status}}{{else}}-{{end}} redemarrages={{.RestartCount}} depuis={{.State.StartedAt}}' "$id" 2>/dev/null | sed 's/\.[0-9]*Z$/Z/; s/^/  /'
  done < <(compose ps -a -q 2>/dev/null || true) | sort

  sect "Ressources (instantane)"
  local ids; mapfile -t ids < <(compose ps -q 2>/dev/null || true)
  if [ "${#ids[@]}" -gt 0 ]; then
    docker stats --no-stream --format '  {{.Name}}  cpu={{.CPUPerc}}  memoire={{.MemUsage}}' "${ids[@]}" 2>/dev/null | sed "s/${PROJECT}-//" | sort || say "  n/a"
  else
    say "  aucun conteneur du projet $PROJECT"
  fi

  sect "Decodage video"
  local vend
  vend="$(dominet dominet sh -c 'ls /dev/dri/renderD128 >/dev/null 2>&1 && echo present || echo absent' 2>/dev/null || true)"
  say "  /dev/dri/renderD128 dans le conteneur dominet : ${vend:-n/a}"
  if [ "$vend" = present ]; then
    dominet dominet vainfo --display drm --device /dev/dri/renderD128 2>&1 | grep -E "vainfo: Driver|VAProfileH264Main.*VLD|VAProfileH264High.*VLD|error" | head -3 | sed 's/^/  /' || true
  else
    say "  decodage logiciel (activer : make prod-video, puis make prod-up)"
  fi

  sect "Application et certificat"
  local domain port code; domain="$(val LAN_DOMAIN)"; domain="${domain:-localhost}"; port="$(val NGINX_HTTPS_PORT)"; port="${port:-443}"
  code="$(curl -sk -o /dev/null -w '%{http_code}' --max-time 10 --resolve "$domain:$port:127.0.0.1" "https://$domain:$port/healthz" 2>/dev/null || true)"
  say "  https://$domain/healthz : HTTP ${code:-000}"
  if [ -f "$CERTS_DIR/cert.pem" ]; then say "  certificat $(openssl x509 -enddate -noout -in "$CERTS_DIR/cert.pem" 2>/dev/null || echo n/a)"; fi

  sect "Journaux des $h dernieres heures (ERROR / WARN, puis les messages les plus repetes)"
  local svc nerr nwarn
  for svc in $(compose ps --services 2>/dev/null | sort); do
    nerr="$(compose logs --no-log-prefix --since "${h}h" "$svc" 2>&1 | strip_ansi | grep -viE 'level=(info|debug|warn)' | grep -vE "$NOT_ERROR" | grep -ciE '\bERROR\b|\bERR\b|panic' || true)"
    nwarn="$(compose logs --no-log-prefix --since "${h}h" "$svc" 2>&1 | strip_ansi | grep -ciE '\bWARN(ING)?\b|\bWRN\b|\bWAR\b' || true)"
    printf '  %-20s erreurs=%-5s avertissements=%s\n' "$svc" "${nerr:-0}" "${nwarn:-0}"
  done
  say "  -- erreurs et avertissements les plus repetes (toutes les piles, chiffres masques) :"
  { for svc in $(compose ps --services 2>/dev/null); do compose logs --no-log-prefix --since "${h}h" "$svc" 2>&1 | strip_ansi | grep -viE 'level=(info|debug)' | grep -vE '(^|[^A-Za-z])(INFO|INF|DEBUG|DEB)([^A-Za-z]|$)|(info|debug):' | grep -iE '\bERROR\b|\bERR\b|panic|\bWARN(ING)?\b|\bWRN\b|\bWAR\b' || true; done; } \
    | sed -E 's/^[0-9T:. \/Z+-]+ +//; s/[0-9]+/N/g' | cut -c1-150 | sort | uniq -c | sort -rn | head -6 | sed 's/^/  /'

  sect "Appareils (actifs / disponibles / sans message depuis plus de 1 h)"
  sql "select count(*) filter (where active), count(*) filter (where active and available), count(*) filter (where active and (last_seen is null or last_seen < now() - interval '1 hour')) from device"
  sql "select '- ' || friendly_name || ' : ' || coalesce(to_char(last_seen at time zone '$tz', 'DD/MM HH24:MI'), 'jamais') from device where active and (last_seen is null or last_seen < now() - interval '1 hour') order by last_seen nulls first limit 25"

  sect "Automations ($h h : ok / erreur / ignore / limite ; actives seulement)"
  sql "select a.name || ' [' || a.trigger_type || '] ok=' || count(r.id) filter (where r.status='ok') || ' erreur=' || count(r.id) filter (where r.status='error') || ' ignore=' || count(r.id) filter (where r.status='skipped') || ' limite=' || count(r.id) filter (where r.status='limited') from automation a left join automation_run r on r.automation_id = a.id and r.at > now() - interval '$h hours' where a.enabled group by a.id, a.name, a.trigger_type order by a.name"
  sql "select 'automations desactivees : ' || count(*) from automation where not enabled"
  sql "select 'derniere erreur : ' || to_char(r.at at time zone '$tz', 'DD/MM HH24:MI') || ' ' || a.name || ' : ' || left(r.message, 120) from automation_run r join automation a on a.id = r.automation_id where r.status = 'error' and r.at > now() - interval '$h hours' order by r.at desc limit 3"

  sect "Alertes ($h h)"
  sql "select 'declenchees=' || count(*) filter (where fired_at > now() - interval '$h hours') || ' en cours=' || count(*) filter (where status = 'firing') from alert"
  sql "select '- ' || to_char(a.fired_at at time zone '$tz', 'DD/MM HH24:MI') || ' ' || coalesce(r.name, '?') || ' (' || a.severity || ', ' || a.status || ') valeur ' || a.value_triggered from alert a left join alert_rule r on r.id = a.rule_id where a.fired_at > now() - interval '$h hours' order by a.fired_at desc limit 5"

  sect "Historique (releves enregistres, $h h) et base"
  sql "select count(*) || ' releves, ' || count(distinct device_id) || ' appareils suivis' from device_state_log where ts > now() - interval '$h hours'"
  sql "select 'base ' || pg_size_pretty(pg_database_size(current_database()))"

  sect "Sauvegardes (les plus recentes)"
  local out; # Les 2 plus recentes de CHAQUE sorte (un simple `tail` ne montrait plus que ReductStore).
  out="$(dominet dominet dominet backups 2>/dev/null | awk '{ n[$1]++; l[$1, n[$1]] = $0 } END { for (k in n) for (i = (n[k] > 2 ? n[k] - 1 : 1); i <= n[k]; i++) print l[k, i] }' | sort || true)"
  if [ -n "$out" ]; then printf '%s\n' "$out" | sed 's/^/  /'; else say "  n/a"; fi

  sect "Comptes MQTT externes"
  out="$(dominet mosquitto sh /deploy/accounts.sh list 2>/dev/null || true)"
  if [ -n "$out" ]; then printf '%s\n' "$out" | sed 's/^/  /'; else say "  n/a"; fi
  printf '\n'
}

case "${1:-}" in
  init) cmd_init ;;
  pull-image) cmd_pull_image ;;
  compose) shift; compose "$@" ;;
  doctor) cmd_doctor ;;
  guard) cmd_guard ;;
  certs) cmd_certs ;;
  export-ca) cmd_export_ca ;;
  up) cmd_guard; cmd_up ;;
  superuser) cmd_superuser ;;
  seed) cmd_seed ;;
  seed-energy) if [ -n "${DRY:-}" ]; then dominet dominet dominet seed-energy --dry-run; else dominet dominet dominet seed-energy; fi ;;
  health) cmd_health ;;
  deploy) cmd_deploy ;;
  update) cmd_update ;;
  migrate) dominet dominet dominet migrate ;;
  passwd) shift; if [ -n "${U:-}" ]; then compose exec dominet dominet passwd --username "$U"; else compose exec dominet dominet passwd; fi ;;
  dump-context) dominet dominet dominet dump-context ;;
  backup) dominet dominet dominet backup ;;
  backups) dominet dominet dominet backups ;;
  mqtt-user) # NAME=nom [TOPIC=sujet/#] [ACCESS=read|write|readwrite] [PASSWORD=...] : compte MQTT d'un client du LAN (mot de passe genere et affiche une fois)
    [ -n "${NAME:-}" ] || die "NAME requis : make prod-mqtt-user NAME=linky"
    pw="${PASSWORD:-$(rand 24)}"
    dominet mosquitto sh /deploy/accounts.sh add "$NAME" "$pw" "${TOPIC:-$NAME/#}" "${ACCESS:-readwrite}"
    say "  Hote : $(val LAN_DOMAIN)   port : $(val MQTT_LAN_PORT | grep . || echo 1883)   utilisateur : $NAME"
    [ -n "${PASSWORD:-}" ] || say "  Mot de passe (affiche UNE fois, a saisir dans le client) : $pw" ;;
  mqtt-del) [ -n "${NAME:-}" ] || die "NAME requis"; dominet mosquitto sh /deploy/accounts.sh del "$NAME" ;;
  mqtt-users) dominet mosquitto sh /deploy/accounts.sh list ;;
  report) cmd_report ;;
  video) cmd_video ;;
  logs) if [ -n "${SINCE:-}" ]; then compose logs --no-log-prefix --since "$SINCE" ${S:+"$S"}; elif [ -n "${N:-}" ]; then compose logs --no-log-prefix --tail "$N" ${S:+"$S"}; else compose logs -f --tail 100 ${S:+"$S"}; fi ;;
  ps) compose ps ;;
  down) compose down ;;
  *) echo "usage : tools/prod.sh init|guard|certs|export-ca|up|superuser|seed|health|deploy|update|migrate|passwd|dump-context|backup|report|video|logs|ps|down" >&2; exit 2 ;;
esac
