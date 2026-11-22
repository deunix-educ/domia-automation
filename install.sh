#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Guided installer. Checks the machine, asks a few questions, writes .env, deploys the production stack.
# Messages are in English or French (French when the locale starts with "fr", or with --lang fr).
#
#   ./install.sh                 interactive
#   ./install.sh --dry-run       checks and questions, changes nothing
#   ./install.sh --yes           no questions: defaults and environment (see --help)
#
# Run it ON the machine that will host the application. It never touches another machine.
set -euo pipefail
cd "$(dirname "$0")"

ENV_FILE="${ENV_FILE:-.env}"
DRY=0 YES=0 LANGSEL="" DOMAIN_ARG="" EMAIL_ARG="" NOZIGBEE=0 REUSE=0 ADMIN_USER_ARG=""

# ---------------------------------------------------------------- messages
declare -A EN FR
EN[title]="DomiNet installer"
FR[title]="Assistant d'installation de DomiNet"
EN[usage]="Usage: ./install.sh [--dry-run] [--yes] [--lang en|fr] [--domain NAME] [--admin NAME] [--email ADDRESS] [--no-zigbee] [--reuse]

  --dry-run      run the checks and the questions, change nothing
  --yes          do not ask anything; take the options, then DOMINET_ADMIN_USER and DOMINET_ADMIN_PASSWORD
                 from the environment (the password is mandatory with --yes)
  --domain NAME  address the devices will use (default: <host name>.local)
  --admin NAME   administrator account name (default: admin)
  --email ADDR   address that receives the alerts (optional)
  --no-zigbee    do not enable Zigbee2MQTT even if a dongle is detected
  --reuse        a .env already exists: keep it and only deploy
  --lang en|fr   language of the messages"
FR[usage]="Usage : ./install.sh [--dry-run] [--yes] [--lang en|fr] [--domain NOM] [--admin NOM] [--email ADRESSE] [--no-zigbee] [--reuse]

  --dry-run      lance les vérifications et les questions, ne change rien
  --yes          ne pose aucune question ; prend les options, puis DOMINET_ADMIN_USER et DOMINET_ADMIN_PASSWORD
                 dans l'environnement (le mot de passe est obligatoire avec --yes)
  --domain NOM   adresse que les appareils utiliseront (défaut : <nom de la machine>.local)
  --admin NOM    nom du compte administrateur (défaut : admin)
  --email ADR    adresse qui reçoit les alertes (facultatif)
  --no-zigbee    n'active pas Zigbee2MQTT même si une clé est détectée
  --reuse        un .env existe déjà : le garder et seulement déployer
  --lang en|fr   langue des messages"
EN[step_check]="1/4  Checking this machine"
FR[step_check]="1/4  Vérification de la machine"
EN[step_ask]="2/4  A few questions"
FR[step_ask]="2/4  Quelques questions"
EN[step_conf]="3/4  Configuration"
FR[step_conf]="3/4  Configuration"
EN[step_deploy]="4/4  Deployment (certificates, build or download, start, administrator, first data)"
FR[step_deploy]="4/4  Déploiement (certificats, construction ou téléchargement, démarrage, administrateur, premières données)"
EN[ok]="OK"
FR[ok]="OK"
EN[warn]="WARNING"
FR[warn]="ATTENTION"
EN[block]="BLOCKING"
FR[block]="BLOQUANT"
EN[os_linux]="a Linux machine is required (found: %s)"
FR[os_linux]="une machine Linux est nécessaire (trouvé : %s)"
EN[bash_old]="bash 4 or newer is required (found: %s)"
FR[bash_old]="bash 4 ou plus récent est nécessaire (trouvé : %s)"
EN[missing_cmd]="%s is missing (install it with your package manager)"
FR[missing_cmd]="%s est absent (installez-le avec le gestionnaire de paquets)"
EN[docker_missing]="Docker is not installed: https://docs.docker.com/engine/install/"
FR[docker_missing]="Docker n'est pas installé : https://docs.docker.com/engine/install/"
EN[docker_down]="Docker is installed but not reachable (daemon stopped, or your user is not in the 'docker' group: sudo usermod -aG docker \$USER, then log in again)"
FR[docker_down]="Docker est installé mais inaccessible (démon arrêté, ou votre utilisateur n'est pas dans le groupe « docker » : sudo usermod -aG docker \$USER, puis reconnectez-vous)"
EN[compose_missing]="the 'docker compose' plugin is missing"
FR[compose_missing]="le plugin « docker compose » est absent"
EN[compose_old]="Docker Compose %s is too old (2.24 or newer is required)"
FR[compose_old]="Docker Compose %s est trop ancien (2.24 ou plus récent est nécessaire)"
EN[compose_ok]="Docker %s, Compose %s"
FR[compose_ok]="Docker %s, Compose %s"
EN[arch_x86]="architecture %s"
FR[arch_x86]="architecture %s"
EN[arch_nosse]="this processor lacks SSE4.2: the program (x86-64-v2) cannot run"
FR[arch_nosse]="ce processeur n'a pas SSE4.2 : le programme (x86-64-v2) ne pourra pas s'exécuter"
EN[arch_arm]="architecture %s: supported on a best-effort basis (the image is built on this machine, which is slow); see the documentation"
FR[arch_arm]="architecture %s : prise en charge au mieux (l'image est construite sur cette machine, ce qui est long) ; voir la documentation"
EN[arch_other]="architecture %s is not tested"
FR[arch_other]="l'architecture %s n'est pas testée"
EN[pi_found]="%s detected"
FR[pi_found]="%s détecté"
EN[sd_card]="the system is on an SD card: the database and the video recordings wear it out quickly. Use an SSD (USB 3 or NVMe), see the machines page of the documentation"
FR[sd_card]="le système est sur une carte SD : la base et les enregistrements vidéo l'usent vite. Utilisez un SSD (USB 3 ou NVMe), voir la page des machines dans la documentation"
EN[small_ram]="%s GB of memory on arm64: keep object detection off at first and add cameras one by one; measure with tools/machine-card.sh"
FR[small_ram]="%s Go de mémoire en arm64 : laissez la détection d'objets désactivée au début et ajoutez les caméras une par une ; mesurez avec tools/machine-card.sh"
EN[ram_low]="%s GB of memory: at least 2 GB are needed, 4 GB or more recommended"
FR[ram_low]="%s Go de mémoire : au moins 2 Go sont nécessaires, 4 Go ou plus recommandés"
EN[ram_ok]="%s GB of memory"
FR[ram_ok]="%s Go de mémoire"
EN[disk_low]="%s GB free for Docker: at least 10 GB are needed (video recordings need much more)"
FR[disk_low]="%s Go libres pour Docker : au moins 10 Go sont nécessaires (les enregistrements vidéo en demandent bien plus)"
EN[disk_warn]="%s GB free for Docker: 20 GB or more recommended"
FR[disk_warn]="%s Go libres pour Docker : 20 Go ou plus recommandés"
EN[disk_ok]="%s GB free for Docker"
FR[disk_ok]="%s Go libres pour Docker"
EN[port_busy]="port %s is already used on this machine (stop what uses it, or choose other ports in .env after the first step)"
FR[port_busy]="le port %s est déjà utilisé sur cette machine (arrêtez ce qui l'utilise, ou choisissez d'autres ports dans .env après la première étape)"
EN[ports_ok]="ports 80, 443, 1883 free"
FR[ports_ok]="ports 80, 443, 1883 libres"
EN[env_exists]="%s already exists: it will not be overwritten"
FR[env_exists]="%s existe déjà : il ne sera pas écrasé"
EN[env_reuse_hint]="rerun with --reuse to keep it and only deploy, or delete it by hand to start over"
FR[env_reuse_hint]="relancez avec --reuse pour le garder et seulement déployer, ou supprimez-le à la main pour recommencer"
EN[old_volume]="a database volume (%s) from an earlier installation exists, but there is no %s: a new password would be refused by the old database. To keep that data, put the old .env back and run ./install.sh --reuse. To start from scratch (this ERASES that data): make prod-down, then docker volume rm %s"
FR[old_volume]="un volume de base de données (%s) d'une installation précédente existe, mais il n'y a pas de %s : un nouveau mot de passe serait refusé par l'ancienne base. Pour garder ces données, remettez l'ancien .env et lancez ./install.sh --reuse. Pour repartir de zéro (cela EFFACE ces données) : make prod-down, puis docker volume rm %s"
EN[blocked]="%s blocking problem(s): fix them and run ./install.sh again."
FR[blocked]="%s problème(s) bloquant(s) : corrigez-les et relancez ./install.sh."
EN[q_domain]="Address the devices will use to reach the house"
FR[q_domain]="Adresse que les appareils utiliseront pour joindre la maison"
EN[q_admin]="Administrator account name"
FR[q_admin]="Nom du compte administrateur"
EN[q_pass]="Administrator password (at least 10 characters, hidden)"
FR[q_pass]="Mot de passe de l'administrateur (10 caractères au moins, masqué)"
EN[q_pass2]="Type it again"
FR[q_pass2]="Saisissez-le encore une fois"
EN[pass_short]="at least 10 characters are needed"
FR[pass_short]="10 caractères au moins sont nécessaires"
EN[pass_digits]="the password cannot be only digits"
FR[pass_digits]="le mot de passe ne peut pas être composé uniquement de chiffres"
EN[pass_same]="the password cannot be the same as the account name"
FR[pass_same]="le mot de passe ne peut pas être identique au nom du compte"
EN[pass_diff]="the two passwords differ"
FR[pass_diff]="les deux mots de passe diffèrent"
EN[pass_missing]="with --yes, set DOMINET_ADMIN_PASSWORD in the environment"
FR[pass_missing]="avec --yes, définissez DOMINET_ADMIN_PASSWORD dans l'environnement"
EN[q_email]="E-mail address for alerts (empty = none for now)"
FR[q_email]="Adresse e-mail des alertes (vide = aucune pour l'instant)"
EN[zigbee_found]="Zigbee dongle detected on %s: Zigbee2MQTT will be enabled"
FR[zigbee_found]="Clé Zigbee détectée sur %s : Zigbee2MQTT sera activé"
EN[zigbee_none]="no Zigbee dongle detected: Zigbee2MQTT stays off (you can add one later)"
FR[zigbee_none]="aucune clé Zigbee détectée : Zigbee2MQTT reste désactivé (vous pourrez en ajouter une plus tard)"
EN[zigbee_off]="Zigbee2MQTT disabled by --no-zigbee"
FR[zigbee_off]="Zigbee2MQTT désactivé par --no-zigbee"
EN[video_hw]="hardware video decoding available (/dev/dri)"
FR[video_hw]="décodage vidéo matériel disponible (/dev/dri)"
EN[video_sw]="no /dev/dri: video is decoded by the processor"
FR[video_sw]="pas de /dev/dri : la vidéo est décodée par le processeur"
EN[confirm]="Go ahead?"
FR[confirm]="On y va ?"
EN[yn]="[Y/n]"
FR[yn]="[O/n]"
EN[cancelled]="Cancelled: nothing was changed."
FR[cancelled]="Annulé : rien n'a été modifié."
EN[dry_end]="Dry run finished: nothing was changed."
FR[dry_end]="Essai terminé : rien n'a été modifié."
EN[would]="would run: %s"
FR[would]="lancerait : %s"
EN[running]="running: %s"
FR[running]="lance : %s"
EN[failed]="The installation stopped at: %s
  Look at the messages above, then:  make prod-logs S=dominet N=100
  You can run ./install.sh --reuse after fixing the cause."
FR[failed]="L'installation s'est arrêtée à : %s
  Regardez les messages ci-dessus, puis :  make prod-logs S=dominet N=100
  Vous pouvez relancer ./install.sh --reuse après avoir corrigé la cause."
EN[done]="DomiNet is installed."
FR[done]="DomiNet est installé."
EN[next]="Next steps:
  1. Open  https://%s  and sign in as %s.
  2. On each phone or computer, install the certificate once:  http://%s/dominet-ca.crt
  3. COPY the value of DOMINET_MASTER_KEY from %s to a safe place OFF this machine:
     without it, a restored backup loses the encrypted secrets (camera passwords).
  4. Add your devices and cameras (documentation: https://%s/docs/)."
FR[next]="Prochaines étapes :
  1. Ouvrez  https://%s  et connectez-vous avec %s.
  2. Sur chaque téléphone ou ordinateur, installez le certificat une fois :  http://%s/dominet-ca.crt
  3. COPIEZ la valeur de DOMINET_MASTER_KEY de %s dans un endroit sûr HORS de cette machine :
     sans elle, une sauvegarde restaurée perd les secrets chiffrés (mots de passe des caméras).
  4. Ajoutez vos appareils et vos caméras (documentation : https://%s/docs/)."

t() { # t KEY [printf args...]
  local key="$1"; shift
  local text
  if [ "$LANGSEL" = fr ]; then text="${FR[$key]:-${EN[$key]}}"; else text="${EN[$key]}"; fi
  # shellcheck disable=SC2059
  printf "$text" "$@"
}
say() { printf '%s\n' "$*"; }

# ---------------------------------------------------------------- arguments
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) DRY=1 ;;
    --yes|-y) YES=1 ;;
    --lang) shift; LANGSEL="${1:-}" ;;
    --domain) shift; DOMAIN_ARG="${1:-}" ;;
    --admin) shift; ADMIN_USER_ARG="${1:-}" ;;
    --email) shift; EMAIL_ARG="${1:-}" ;;
    --no-zigbee) NOZIGBEE=1 ;;
    --reuse) REUSE=1 ;;
    -h|--help) LANGSEL="${LANGSEL:-en}"; t usage; echo; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
  shift
done
if [ -z "$LANGSEL" ]; then
  case "${LC_ALL:-${LANG:-}}" in fr*) LANGSEL=fr ;; *) LANGSEL=en ;; esac
fi
[ "$LANGSEL" = fr ] || LANGSEL=en

# ---------------------------------------------------------------- helpers
BLOCKS=0
ok()    { printf '  %-9s %s\n' "$(t ok)" "$*"; }
warn()  { printf '  %-9s %s\n' "$(t warn)" "$*"; }
block() { printf '  %-9s %s\n' "$(t block)" "$*"; BLOCKS=$((BLOCKS + 1)); }
version_ge() { [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -1)" = "$2" ]; }
run() { # run CMD... : prints and runs, or only prints with --dry-run
  if [ "$DRY" = 1 ]; then printf '  %s\n' "$(t would "$*")"; return 0; fi
  printf '  %s\n' "$(t running "$*")"
  "$@"
}
ask() { # ask PROMPT DEFAULT -> answer on stdout
  local prompt="$1" default="${2:-}" answer
  if [ "$YES" = 1 ]; then printf '%s' "$default"; return; fi
  if [ -n "$default" ]; then read -r -p "  $prompt [$default]: " answer </dev/tty || answer=""; else read -r -p "  $prompt: " answer </dev/tty || answer=""; fi
  printf '%s' "${answer:-$default}"
}
pw_problem() { # pw_problem PASSWORD USER -> message key of the first problem, empty if fine
  if [ "${#1}" -lt 10 ]; then echo pass_short; return; fi
  case "$1" in *[!0-9]*) ;; *) echo pass_digits; return ;; esac
  [ "$1" != "$2" ] || echo pass_same
}
ask_secret() { local answer; read -r -s -p "  $1: " answer </dev/tty || answer=""; echo >&2; printf '%s' "$answer"; }
setvar() { # setvar NAME VALUE in the env file
  local k="$1" v="$2" esc
  if grep -q "^$k=" "$ENV_FILE"; then
    esc="$(printf '%s' "$v" | sed 's/[|&\\]/\\&/g')"; sed -i "s|^$k=.*|$k=$esc|" "$ENV_FILE"
  else
    { [ -z "$(tail -c1 "$ENV_FILE")" ] || echo >> "$ENV_FILE"; echo "$k=$v" >> "$ENV_FILE"; }
  fi
}
fail_step() { printf '\n'; t failed "$CURRENT_STEP"; printf '\n'; exit 1; }
CURRENT_STEP="start"
trap 'fail_step' ERR

say ""; say "== $(t title)"

# ---------------------------------------------------------------- 1. checks
say ""; say "== $(t step_check)"
[ "$(uname -s)" = Linux ] && ok "Linux $(uname -r)" || block "$(t os_linux "$(uname -s)")"
[ "${BASH_VERSINFO[0]}" -ge 4 ] || block "$(t bash_old "$BASH_VERSION")"

for c in git make openssl curl; do
  command -v "$c" >/dev/null 2>&1 && ok "$c" || block "$(t missing_cmd "$c")"
done

if ! command -v docker >/dev/null 2>&1; then
  block "$(t docker_missing)"
elif ! docker info >/dev/null 2>&1; then
  block "$(t docker_down)"
else
  dv="$(docker version -f '{{.Server.Version}}' 2>/dev/null || echo '?')"
  cv="$(docker compose version --short 2>/dev/null | sed 's/^v//' || true)"
  if [ -z "$cv" ]; then block "$(t compose_missing)"
  elif version_ge "$cv" 2.24; then ok "$(t compose_ok "$dv" "$cv")"
  else block "$(t compose_old "$cv")"; fi
  root="$(docker info -f '{{.DockerRootDir}}' 2>/dev/null || echo /var/lib/docker)"
  free_gb="$(df -BG --output=avail "$root" 2>/dev/null | tail -1 | tr -dc '0-9' || true)"
  if [ -n "${free_gb:-}" ]; then
    if [ "$free_gb" -lt 10 ]; then block "$(t disk_low "$free_gb")"
    elif [ "$free_gb" -lt 20 ]; then warn "$(t disk_warn "$free_gb")"
    else ok "$(t disk_ok "$free_gb")"; fi
  fi
fi

arch="$(uname -m)"
case "$arch" in
  x86_64) ok "$(t arch_x86 "$arch")"; grep -q sse4_2 /proc/cpuinfo 2>/dev/null || block "$(t arch_nosse)" ;;
  aarch64|arm64) warn "$(t arch_arm "$arch")" ;;
  *) warn "$(t arch_other "$arch")" ;;
esac

model_file="${INSTALL_MODEL_FILE:-/proc/device-tree/model}"
if [ -r "$model_file" ]; then
  model="$(tr -d '\0' < "$model_file")"
  case "$model" in *Raspberry*) ok "$(t pi_found "$model")" ;; esac
fi
root_src="$(findmnt -no SOURCE / 2>/dev/null || true)"
root_dev="${INSTALL_ROOT_DEV:-$(lsblk -no PKNAME "$root_src" 2>/dev/null | head -1)}"
case "${root_dev:-$(basename "${root_src:-x}")}" in mmcblk*) warn "$(t sd_card)" ;; esac

mem_gb="$(awk '/MemTotal/ {printf "%d", $2/1048576}' /proc/meminfo 2>/dev/null || echo 0)"
if [ "$mem_gb" -lt 2 ]; then block "$(t ram_low "$mem_gb")"
elif [ "$mem_gb" -lt 4 ]; then warn "$(t ram_low "$mem_gb")"
else ok "$(t ram_ok "$mem_gb")"; fi

case "$arch" in aarch64|arm64) [ "$mem_gb" -ge 7 ] || warn "$(t small_ram "$mem_gb")" ;; esac

busy=0
if command -v ss >/dev/null 2>&1; then
  for p in ${INSTALL_PORTS:-80 443 1883}; do
    if ss -ltnH 2>/dev/null | awk -v p=":$p" '$4 ~ p"$" {f=1} END {exit !f}'; then block "$(t port_busy "$p")"; busy=1; fi
  done
  [ "$busy" = 0 ] && ok "$(t ports_ok)"
fi

if [ -f "$ENV_FILE" ] && grep -qE '^DB_PASSWORD=.+' "$ENV_FILE"; then
  if [ "$REUSE" = 1 ]; then ok "$(t env_exists "$ENV_FILE")"
  else block "$(t env_exists "$ENV_FILE"); $(t env_reuse_hint)"; fi
fi

# A PostgreSQL volume only learns its password when it is created: a new .env next to an old volume can never log in.
if [ ! -f "$ENV_FILE" ] || ! grep -qE '^DB_PASSWORD=.+' "$ENV_FILE"; then
  pg_volume="${PROJECT:-dominet}_postgres-data"
  if docker volume inspect "$pg_volume" >/dev/null 2>&1; then block "$(t old_volume "$pg_volume" "$ENV_FILE" "$pg_volume")"; fi
fi

if [ "$BLOCKS" -gt 0 ]; then say ""; t blocked "$BLOCKS"; say ""; exit 1; fi

# ---------------------------------------------------------------- 2. questions
say ""; say "== $(t step_ask)"
DOMAIN="${DOMAIN_ARG:-$(hostname).local}"
ADMIN_USER="${ADMIN_USER_ARG:-${DOMINET_ADMIN_USER:-admin}}"
ADMIN_PASSWORD="${DOMINET_ADMIN_PASSWORD:-}"
EMAIL="$EMAIL_ARG"

zdev=""
if [ "$NOZIGBEE" = 1 ]; then ok "$(t zigbee_off)"
else
  for d in /dev/ttyUSB0 /dev/ttyACM0; do [ -e "$d" ] && { zdev="$d"; break; }; done
  if [ -n "$zdev" ]; then ok "$(t zigbee_found "$zdev")"; else ok "$(t zigbee_none)"; fi
fi
if [ -e /dev/dri/renderD128 ]; then ok "$(t video_hw)"; else ok "$(t video_sw)"; fi

if [ "$REUSE" = 0 ]; then
  if [ "$YES" = 0 ]; then
    DOMAIN="$(ask "$(t q_domain)" "$DOMAIN")"
    ADMIN_USER="$(ask "$(t q_admin)" "$ADMIN_USER")"
    [ -n "$EMAIL" ] || EMAIL="$(ask "$(t q_email)" "")"
  fi
  # administrator password: validated like the application does
  if [ -z "$ADMIN_PASSWORD" ] || [ "$YES" = 0 ]; then
    if [ "$YES" = 1 ]; then say "$(t pass_missing)" >&2; exit 2; fi
    if [ "$DRY" = 1 ] && [ ! -t 0 ]; then ADMIN_PASSWORD="dry-run-password"; else
      while :; do
        p1="$(ask_secret "$(t q_pass)")"
        problem="$(pw_problem "$p1" "$ADMIN_USER")"
        if [ -n "$problem" ]; then say "  $(t "$problem")"; continue; fi
        p2="$(ask_secret "$(t q_pass2)")"
        [ "$p1" = "$p2" ] || { say "  $(t pass_diff)"; continue; }
        ADMIN_PASSWORD="$p1"; break
      done
    fi
  else
    problem="$(pw_problem "$ADMIN_PASSWORD" "$ADMIN_USER")"
    [ -z "$problem" ] || { say "  $(t "$problem")" >&2; exit 2; }
  fi
fi

if [ "$YES" = 0 ]; then
  say ""
  say "  $DOMAIN  |  $ADMIN_USER  |  ${EMAIL:-(no e-mail)}"
  yn="$(ask "$(t confirm) $(t yn)" "")"
  case "${yn:-y}" in n|N|non|Non|no|No) say "$(t cancelled)"; exit 0 ;; esac
fi

# Sender address of the alert e-mails. Grafana refuses to start with an IP address as the domain part, so use a name then.
from_address() { case "$DOMAIN" in *[!0-9.]*) case "$DOMAIN" in *:*) echo "dominet@dominet.local" ;; *) echo "dominet@$DOMAIN" ;; esac ;; *) echo "dominet@dominet.local" ;; esac; }

# ---------------------------------------------------------------- 3. configuration
say ""; say "== $(t step_conf)"
if [ "$REUSE" = 0 ]; then
  CURRENT_STEP="make prod-init"
  if [ "$DRY" = 1 ]; then
    printf '  %s\n' "$(t would "LAN_DOMAIN=$DOMAIN make prod-init")"
    [ -z "$EMAIL" ] || printf '  %s\n' "$(t would "DEFAULT_FROM_EMAIL=$(from_address)")"
  else
    LAN_DOMAIN="$DOMAIN" make prod-init >/dev/null
    ok "$ENV_FILE"
    [ -z "$EMAIL" ] || { setvar EMAIL_RECIPIENT "$EMAIL"; setvar DEFAULT_FROM_EMAIL "$(from_address)"; }
    if [ "$NOZIGBEE" = 1 ]; then setvar ZIGBEE_DEVICE ""; fi
  fi
else
  ok "$(t env_exists "$ENV_FILE")"
  [ -f "$ENV_FILE" ] || { say "  $ENV_FILE missing" >&2; exit 1; }
  DOMAIN="$(grep -s '^LAN_DOMAIN=' "$ENV_FILE" | tail -1 | cut -d= -f2- | tr -d ' "' || true)"
  DOMAIN="${DOMAIN:-$(hostname).local}"
fi

# ---------------------------------------------------------------- 4. deployment
say ""; say "== $(t step_deploy)"
CURRENT_STEP="make prod-deploy"
if [ "$DRY" = 1 ]; then
  printf '  %s\n' "$(t would "DOMINET_ADMIN_USER=$ADMIN_USER DOMINET_ADMIN_PASSWORD=*** make prod-deploy")"
  say ""; say "$(t dry_end)"; exit 0
fi
if [ "$REUSE" = 0 ]; then
  DOMINET_ADMIN_USER="$ADMIN_USER" DOMINET_ADMIN_PASSWORD="$ADMIN_PASSWORD" make prod-deploy
else
  make prod-deploy
fi

say ""; say "== $(t done)"; say ""
t next "$DOMAIN" "$ADMIN_USER" "$DOMAIN" "$ENV_FILE" "$DOMAIN"; say ""
