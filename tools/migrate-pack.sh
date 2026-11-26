#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Moves a whole installation to another machine: ONE archive with the database, the ReductStore data and, encrypted with a passphrase,
# everything that is not data but that you would otherwise lose (.env with DOMINET_MASTER_KEY, certificates and the mkcert authority key,
# Zigbee2MQTT folder with the pairing, MQTT accounts, VPN keys). See docs/migration.md.
#
#   tools/migrate-pack.sh export [--out DIR] [--no-backup] [--skip-reductstore] [--with-grafana]
#   tools/migrate-pack.sh verify FILE
#   tools/migrate-pack.sh import FILE [--dry-run] [--yes] [--skip-reductstore] [--no-zigbee]
#
# The passphrase is asked (twice when exporting) or taken from MIGRATE_PASSPHRASE. It is never written anywhere.
# `export` only READS the installation (and runs one normal `dominet backup`, unless --no-backup). `import` writes on THIS machine only
# and refuses to overwrite an existing installation without --yes. Variables as for tools/prod.sh: ENV_FILE, PROJECT, CERTS_DIR;
# ZIGBEE_DIR (default deploy/zigbee2mqtt), CAROOT (default `mkcert -CAROOT`).
set -euo pipefail
cd "$(dirname "$0")/.."
umask 077

ENV_FILE="${ENV_FILE:-.env}"
PROJECT="${PROJECT:-dominet}"
CERTS_DIR="${CERTS_DIR:-certs}"
ZIGBEE_DIR="${ZIGBEE_DIR:-deploy/zigbee2mqtt}"
PROD=(tools/prod.sh)
FORMAT=1

die() { printf '\n  Error: %s\n\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }
val() { grep -s "^$1=" "$ENV_FILE" | tail -1 | cut -d= -f2- | sed 's/[[:space:]]*#.*//' | tr -d ' "' || true; }
valof() { grep -s "^$2=" "$1" | tail -1 | cut -d= -f2- | sed 's/[[:space:]]*#.*//' | tr -d ' "' || true; }
setvar() { # setvar FILE NAME VALUE
  local f="$1" k="$2" v="$3" esc
  if grep -q "^$k=" "$f"; then esc="$(printf '%s' "$v" | sed 's/[|&\\]/\\&/g')"; sed -i "s|^$k=.*|$k=$esc|" "$f"
  else { [ -z "$(tail -c1 "$f")" ] || echo >> "$f"; echo "$k=$v" >> "$f"; }; fi
}
compose() { ENV_FILE="$ENV_FILE" PROJECT="$PROJECT" CERTS_DIR="$CERTS_DIR" "${PROD[@]}" compose "$@"; }
tag() { local t; t="$(val DOMINET_TAG)"; echo "dominet:${t:-local}"; }
ca_root() { if [ -n "${CAROOT:-}" ]; then echo "$CAROOT"; elif command -v mkcert >/dev/null 2>&1; then mkcert -CAROOT; fi; }
sha() { sha256sum "$1" | cut -d' ' -f1; }
arch() { uname -m; }

passphrase() { # sets PASS; $1 = confirm (1 when creating)
  if [ -n "${MIGRATE_PASSPHRASE:-}" ]; then PASS="$MIGRATE_PASSPHRASE"; return; fi
  [ -r /dev/tty ] || die "no terminal: set MIGRATE_PASSPHRASE"
  read -r -s -p "  Passphrase for the secrets: " PASS </dev/tty; echo >&2
  [ "${#PASS}" -ge 8 ] || die "the passphrase needs at least 8 characters"
  if [ "${1:-0}" = 1 ]; then
    local again; read -r -s -p "  Again: " again </dev/tty; echo >&2
    [ "$PASS" = "$again" ] || die "the two passphrases differ"
  fi
}
encrypt() { MIGRATE_PASS="$PASS" openssl enc -aes-256-cbc -pbkdf2 -iter 600000 -salt -pass env:MIGRATE_PASS; }
decrypt() { MIGRATE_PASS="$PASS" openssl enc -d -aes-256-cbc -pbkdf2 -iter 600000 -pass env:MIGRATE_PASS; }

# Archive of a Docker volume (or a host folder) read as root, written to stdout.
vol_tar() { docker run --rm --user 0 -v "$1:/v:ro" --entrypoint tar "$(tag)" czf - -C /v . ; }
vol_untar() { docker run --rm -i --user 0 -v "$1:/v" --entrypoint tar "$(tag)" xzf - -C /v ; }
vol_wipe() { docker run --rm --user 0 -v "$1:/v" --entrypoint sh "$(tag)" -c 'find /v -mindepth 1 -delete' ; }
vol_name() { echo "${PROJECT}_$1"; }

# ------------------------------------------------------------------------------------------------------------ export
cmd_export() {
  local out="." backup=1 skip_rs=0 grafana=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --out) shift; out="${1:?--out DIR}" ;;
      --no-backup) backup=0 ;;
      --skip-reductstore) skip_rs=1 ;;
      --with-grafana) grafana=1 ;;
      *) die "unknown option: $1" ;;
    esac; shift
  done
  [ -f "$ENV_FILE" ] || die "$ENV_FILE is missing: nothing to export here"
  docker info >/dev/null 2>&1 || die "Docker is not reachable"
  compose ps --status running --services 2>/dev/null | grep -qx dominet || die "the application is not running (make prod-up): the export asks it for a backup"
  mkdir -p "$out"; out="$(cd "$out" && pwd)"
  passphrase 1
  local ts W P S; ts="$(date -u +%Y%m%d-%H%M%S)"
  W="$(mktemp -d "$out/.migrate-work-XXXXXX")"; trap "rm -rf '$W'" EXIT
  P="$W/pack"; S="$W/secrets"; mkdir -p "$P/postgres" "$P/reductstore" "$S"

  if [ "$backup" = 1 ]; then
    say "  1/5  Backup (database$([ "$skip_rs" = 1 ] || echo " and ReductStore")) ..."
    compose exec -T dominet dominet backup >"$W/backup.log" 2>&1 || { cat "$W/backup.log" >&2; die "the backup failed"; }
  else
    say "  1/5  Using the latest backups already in the backup folder (--no-backup)"
  fi
  compose cp dominet:/backups/postgres/. "$P/postgres" >/dev/null 2>&1 || die "cannot read /backups/postgres of the application"
  compose cp dominet:/backups/reductstore/. "$P/reductstore" >/dev/null 2>&1 || true
  local pg rs
  pg="$(ls "$P"/postgres/*.sql.gz 2>/dev/null | sort | tail -1 || true)"; [ -n "$pg" ] || die "no database backup found"
  find "$P/postgres" -type f ! -name "$(basename "$pg")" ! -name "$(basename "$pg").sha256" -delete
  (cd "$P/postgres" && sha256sum -c "$(basename "$pg").sha256" >/dev/null) || die "the database backup does not match its checksum"
  rs="$(ls "$P"/reductstore/*.tar.gz 2>/dev/null | sort | tail -1 || true)"
  if [ "$skip_rs" = 1 ] || [ -z "$rs" ]; then
    rm -rf "$P/reductstore"; rs=""
    [ "$skip_rs" = 1 ] || say "       (no ReductStore backup found: history and video are not included)"
  else
    find "$P/reductstore" -type f ! -name "$(basename "$rs")" ! -name "$(basename "$rs").sha256" -delete
    (cd "$P/reductstore" && sha256sum -c "$(basename "$rs").sha256" >/dev/null) || die "the ReductStore backup does not match its checksum"
  fi

  say "  2/5  Collecting the secrets and configuration ..."
  cp "$ENV_FILE" "$S/env"
  [ -d "$CERTS_DIR" ] && { mkdir -p "$S/certs"; cp -a "$CERTS_DIR/." "$S/certs/"; }
  local has_ca=0 root; root="$(ca_root || true)"
  if [ -n "$root" ] && [ -f "$root/rootCA-key.pem" ] && [ -f "$root/rootCA.pem" ]; then
    mkdir -p "$S/mkcert-ca"; cp "$root/rootCA.pem" "$root/rootCA-key.pem" "$S/mkcert-ca/"; has_ca=1
  fi
  local has_z2m=0
  if [ -d "$ZIGBEE_DIR" ] && [ -f "$ZIGBEE_DIR/configuration.yaml" ]; then
    mkdir -p "$S/zigbee2mqtt"
    docker run --rm --user 0 -v "$(cd "$ZIGBEE_DIR" && pwd):/v:ro" --entrypoint tar "$(tag)" cf - -C /v --exclude=./log . | tar xf - -C "$S/zigbee2mqtt"
    has_z2m=1
  fi
  mkdir -p "$S/mosquitto-accounts"
  compose cp mosquitto:/mosquitto/accounts/. "$S/mosquitto-accounts" >/dev/null 2>&1 || say "       (no MQTT accounts folder found)"
  local has_vpn=0 has_graf=0
  # wg-easy 15: administrator, settings (firewall hooks, device), server keys and clients are in this volume. The wg-easy 14 volume
  # (wg-easy-data) has another format and is not exported.
  if docker volume inspect "$(vol_name wg-easy15-data)" >/dev/null 2>&1; then
    vol_tar "$(vol_name wg-easy15-data)" > "$S/wg-easy15-data.tgz"; has_vpn=1
  fi
  if [ "$grafana" = 1 ]; then vol_tar "$(vol_name grafana-data)" > "$S/grafana-data.tgz"; has_graf=1; fi

  say "  3/5  Encrypting the secrets ..."
  tar czf - -C "$S" . | encrypt > "$P/secrets.tgz.enc"

  say "  4/5  Writing the manifest ..."
  local du dn migrations pgmajor
  du="$(val DB_USER)"; dn="$(val DB_NAME)"
  migrations="$(compose exec -T postgres psql -U "${du:-dominet}" -d "${dn:-dominet}" -tAc 'SELECT count(*) FROM _sqlx_migrations' 2>/dev/null | tr -d ' \r' || true)"
  pgmajor="$(compose exec -T postgres psql -U "${du:-dominet}" -d "${dn:-dominet}" -tAc 'SHOW server_version_num' 2>/dev/null | tr -d ' \r' | cut -c1-2 || true)"
  {
    echo "format=$FORMAT"
    echo "created_utc=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "arch=$(arch)"
    echo "os=$(. /etc/os-release 2>/dev/null; echo "${PRETTY_NAME:-unknown}")"
    echo "image_tag=$(tag)"
    echo "migrations=${migrations:-0}"
    echo "pg_major=${pgmajor:-0}"
    echo "lan_domain=$(val LAN_DOMAIN)"
    echo "local_ip=$(val LOCAL_DOMAIN)"
    echo "zigbee_device=$(val ZIGBEE_DEVICE)"
    echo "has_reductstore=$([ -n "$rs" ] && echo 1 || echo 0)"
    echo "has_zigbee=$has_z2m"
    echo "has_mkcert_ca=$has_ca"
    echo "has_vpn=$has_vpn"
    echo "vpn_format=15"
    echo "has_grafana=$has_graf"
    (cd "$P" && find . -type f ! -name MANIFEST | sort | while read -r f; do echo "sha256 $(sha "$f") ${f#./}"; done)
  } > "$P/MANIFEST"

  say "  5/5  Writing the archive ..."
  local archive="$out/dominet-migration-$ts.tar"
  tar cf "$archive" -C "$P" .
  sha256sum "$archive" | sed "s|  .*|  $(basename "$archive")|" > "$archive.sha256"
  say ""
  say "  Archive: $archive ($(du -h "$archive" | cut -f1))"
  say "  It contains your data and, encrypted, your keys. Keep it and the passphrase TOGETHER NOWHERE:"
  say "  the passphrase must not be stored next to the archive."
  say "  On the new machine:  git clone ... && cd ... && tools/migrate-pack.sh import $(basename "$archive")"
  [ "$has_ca" = 1 ] || say "  Note: no mkcert authority key was found: devices will have to install the new certificate once."
}

# ------------------------------------------------------------------------------------------------------------ verify / extract
mf() { grep -s "^$1=" "$2/MANIFEST" | head -1 | cut -d= -f2-; }

extract_and_verify() { # extract_and_verify FILE DEST -> unpacked pack in DEST, checksums verified
  local f="$1" d="$2"
  [ -f "$f" ] || die "no such file: $f"
  if [ -f "$f.sha256" ]; then
    (cd "$(dirname "$f")" && sha256sum -c "$(basename "$f").sha256" >/dev/null 2>&1) || die "the archive does not match its checksum (damaged or modified)"
  else
    say "  (no $(basename "$f").sha256 next to the archive: only the inner checksums are verified)"
  fi
  mkdir -p "$d"; tar xf "$f" -C "$d" 2>/dev/null || die "this is not a migration archive"
  [ -f "$d/MANIFEST" ] || die "MANIFEST missing: this is not a migration archive"
  [ "$(mf format "$d")" = "$FORMAT" ] || die "unsupported archive format ($(mf format "$d")); this tool reads format $FORMAT"
  local line h p bad=0
  while read -r line; do
    set -- $line; [ "$1" = sha256 ] || continue
    h="$2"; p="$3"
    [ -f "$d/$p" ] && [ "$(sha "$d/$p")" = "$h" ] || { say "  damaged or missing: $p"; bad=1; }
  done < <(grep '^sha256 ' "$d/MANIFEST")
  [ "$bad" = 0 ] || die "the archive content does not match its manifest"
}

decrypt_secrets() { # decrypt_secrets PACKDIR DEST
  mkdir -p "$2"
  if ! decrypt < "$1/secrets.tgz.enc" 2>/dev/null | tar xzf - -C "$2" 2>/dev/null; then
    rm -rf "$2"; die "cannot decrypt the secrets: wrong passphrase?"
  fi
  [ -f "$2/env" ] || die "the secrets do not contain the configuration file"
}

cmd_verify() {
  local f="${1:?usage: verify FILE}"; passphrase 0
  local W; W="$(mktemp -d)"; trap "rm -rf '$W'" EXIT
  extract_and_verify "$f" "$W/pack"
  decrypt_secrets "$W/pack" "$W/secrets"
  say "  OK  checksums and passphrase are good."
  summary "$W/pack" "$W/secrets"
}

summary() { # summary PACKDIR SECRETSDIR
  local p="$1" s="$2"
  say "  Created           $(mf created_utc "$p") on $(mf arch "$p"), $(mf os "$p")"
  say "  Address           $(mf lan_domain "$p") ($(mf local_ip "$p"))"
  say "  Schema            $(mf migrations "$p") migrations, PostgreSQL $(mf pg_major "$p")"
  say "  ReductStore       $([ "$(mf has_reductstore "$p")" = 1 ] && echo included || echo "not included")"
  say "  Zigbee2MQTT       $([ "$(mf has_zigbee "$p")" = 1 ] && echo "included (pairing kept), dongle was $(mf zigbee_device "$p")" || echo "not included")"
  say "  mkcert authority  $([ "$(mf has_mkcert_ca "$p")" = 1 ] && echo "included (devices keep trusting it)" || echo "not included")"
  say "  VPN (wg-easy)     $(if [ "$(mf has_vpn "$p")" != 1 ]; then echo "not included"; elif [ "$(mf vpn_format "$p")" = 15 ]; then echo "included (administrator, settings and keys)"; else echo "included in the wg-easy 14 format: will NOT be restored (wg-easy 15 starts empty)"; fi)"
  [ -n "$s" ] || true
}

# ------------------------------------------------------------------------------------------------------------ import
detect_dongle() { local d; for d in /dev/ttyUSB0 /dev/ttyACM0; do [ -e "$d" ] && { echo "$d"; return; }; done; }
add_host() { # add_host FILE VAR VALUE : appends VALUE to a comma list when missing
  local cur; cur="$(valof "$1" "$2")"
  case ",$cur," in *",$3,"*) ;; *) setvar "$1" "$2" "${cur:+$cur,}$3" ;; esac
}

# Copies the archived Zigbee2MQTT folder into ZIGBEE_DIR. Files left there by the container belong to root, so when a plain copy is
# refused it is redone as root in a throw-away container (the folder keeps its owner).
restore_zigbee() { # restore_zigbee SRC
  mkdir -p "$ZIGBEE_DIR"
  cp -a "$1/." "$ZIGBEE_DIR/" 2>/dev/null && return 0
  local img owner dst; img="$(tag)"
  docker image inspect "$img" >/dev/null 2>&1 || img="$(grep -ho 'koenkk/zigbee2mqtt:[^"[:space:]]*' deploy/docker-compose*.yml | head -1)"
  owner="$(stat -c %u:%g "$ZIGBEE_DIR")"; dst="$(cd "$ZIGBEE_DIR" && pwd)"
  docker run --rm --user 0 -v "$1:/src:ro" -v "$dst:/dst" --entrypoint sh "$img" -c "cp -a /src/. /dst/ && chown -R $owner /dst" \
    || die "cannot write the Zigbee2MQTT folder $ZIGBEE_DIR (try: sudo chown -R \$USER $ZIGBEE_DIR)"
}

cmd_import() {
  local f="" dry=0 yes=0 skip_rs=0 no_zigbee=0
  while [ $# -gt 0 ]; do
    case "$1" in
      --dry-run) dry=1 ;;
      --yes) yes=1 ;;
      --skip-reductstore) skip_rs=1 ;;
      --no-zigbee) no_zigbee=1 ;;
      -*) die "unknown option: $1" ;;
      *) f="$1" ;;
    esac; shift
  done
  [ -n "$f" ] || die "usage: import FILE [--dry-run] [--yes] [--skip-reductstore] [--no-zigbee]"
  docker info >/dev/null 2>&1 || die "Docker is not reachable"
  passphrase 0
  local W; W="$(mktemp -d)"; trap "rm -rf '$W'" EXIT
  local P="$W/pack" S="$W/secrets"
  say "  Checking the archive ..."
  extract_and_verify "$f" "$P"
  decrypt_secrets "$P" "$S"
  summary "$P" "$S"

  # Compatibility
  local here_arch src_arch pgpin src_pg tree_migs src_migs
  here_arch="$(arch)"; src_arch="$(mf arch "$P")"
  [ "$here_arch" = "$src_arch" ] && say "  Architecture      same ($here_arch)" || say "  Architecture      $src_arch -> $here_arch (the data formats are portable; the image is for this machine)"
  pgpin="$(grep -o 'postgres:[0-9]*' deploy/docker-compose.yml | head -1 | cut -d: -f2)"; src_pg="$(mf pg_major "$P")"
  if [ -n "$pgpin" ] && [ "$src_pg" != 0 ] && [ "$src_pg" != "$pgpin" ]; then
    die "the archive comes from PostgreSQL $src_pg and this installation uses $pgpin: dump and restore across majors is not covered by this tool"
  fi
  src_migs="$(mf migrations "$P")"; tree_migs="$(ls crates/dominet-store/migrations/*.sql 2>/dev/null | wc -l | tr -d ' ')"
  if [ "${tree_migs:-0}" -gt 0 ] && [ "${src_migs:-0}" -gt "$tree_migs" ]; then
    die "the archive has $src_migs migrations and this code only $tree_migs: update this repository (git pull) first"
  fi
  say "  Schema            $src_migs applied there, $tree_migs here: $([ "$src_migs" -lt "$tree_migs" ] && echo "the missing ones will be applied after the restore" || echo "same")"

  # Existing installation. Running services are looked up by label, so a stack started from ANOTHER folder with the same project name
  # is seen too; a leftover PostgreSQL volume matters even with nothing running: it keeps the password of the earlier installation,
  # which the restored .env cannot open.
  local existing=0 running=0 oldvol=0 pgvol="${PROJECT}_postgres-data"
  { [ -f "$ENV_FILE" ] && grep -qE '^DB_PASSWORD=.+' "$ENV_FILE"; } && existing=1
  [ -n "$(docker ps -q --filter "label=com.docker.compose.project=$PROJECT" 2>/dev/null || true)" ] && running=1
  docker volume inspect "$pgvol" >/dev/null 2>&1 && oldvol=1
  if { [ "$existing" = 1 ] || [ "$running" = 1 ] || [ "$oldvol" = 1 ]; } && [ "$yes" = 0 ]; then
    say ""
    die "this machine already has an installation ($([ "$existing" = 1 ] && echo "$ENV_FILE")$([ "$running" = 1 ] && echo " running services of project $PROJECT")$([ "$oldvol" = 1 ] && echo " database volume $pgvol")). Importing REPLACES its database and configuration: its containers and its database volume are removed, the data is ERASED. Run again with --yes if that is what you want (the old $ENV_FILE is kept as $ENV_FILE.before-import-*)."
  fi

  local newip; newip="$(hostname -I 2>/dev/null | awk '{print $1}')"; newip="${newip:-127.0.0.1}"
  local dongle; dongle="$(detect_dongle || true)"
  say ""
  say "  This machine: address $newip, Zigbee dongle $([ -n "$dongle" ] && echo "$dongle" || echo none), hardware video $([ -e /dev/dri/renderD128 ] && echo yes || echo no)"
  if [ "$dry" = 1 ]; then
    say "  Dry run: nothing was changed. Would restore the configuration, certificates$([ "$(mf has_zigbee "$P")" = 1 ] && echo ", Zigbee2MQTT folder"), MQTT accounts, the database$([ "$skip_rs" = 0 ] && [ "$(mf has_reductstore "$P")" = 1 ] && echo " and ReductStore"), then start the stack."
    return 0
  fi

  say ""; say "  1/6  Configuration"
  local ts; ts="$(date -u +%Y%m%d-%H%M%S)"
  [ ! -f "$ENV_FILE" ] || cp -p "$ENV_FILE" "$ENV_FILE.before-import-$ts"
  if [ "$running" = 1 ] || [ "$oldvol" = 1 ]; then
    docker ps -aq --filter "label=com.docker.compose.project=$PROJECT" | xargs -r docker rm -f >/dev/null 2>&1 || true
  fi
  if [ "$oldvol" = 1 ]; then docker volume rm "$pgvol" >/dev/null 2>&1 || die "cannot remove the database volume $pgvol (still used?)"; say "       database volume of the previous installation removed"; fi
  cp "$S/env" "$ENV_FILE"; chmod 600 "$ENV_FILE"
  add_host "$ENV_FILE" ALLOWED_HOSTS "$newip"
  add_host "$ENV_FILE" CSRF_TRUSTED_ORIGINS "https://$newip"
  add_host "$ENV_FILE" WEBRTC_ADDITIONAL_HOSTS "$newip"
  setvar "$ENV_FILE" LOCAL_DOMAIN "$newip"
  if [ -n "$(valof "$ENV_FILE" ZIGBEE_DEVICE)" ]; then
    if [ "$no_zigbee" = 1 ]; then setvar "$ENV_FILE" ZIGBEE_DEVICE ""; say "       Zigbee2MQTT stays off here (--no-zigbee): the old machine keeps its Zigbee network to itself"
    elif [ -n "$dongle" ]; then setvar "$ENV_FILE" ZIGBEE_DEVICE "$dongle"; say "       Zigbee dongle: $dongle"
    else setvar "$ENV_FILE" ZIGBEE_DEVICE ""; say "       No Zigbee dongle here: Zigbee2MQTT stays off (plug it in and set ZIGBEE_DEVICE in $ENV_FILE)"; fi
  fi
  if [ -e /dev/dri/renderD128 ]; then setvar "$ENV_FILE" VIDEO_DEVICE /dev/dri; setvar "$ENV_FILE" RENDER_GID "$(stat -c %g /dev/dri/renderD128)"
  else setvar "$ENV_FILE" VIDEO_DEVICE ""; setvar "$ENV_FILE" RENDER_GID ""; fi

  say "  2/6  Certificates and authority"
  if [ -d "$S/certs" ]; then mkdir -p "$CERTS_DIR"; cp -a "$S/certs/." "$CERTS_DIR/"; chmod 600 "$CERTS_DIR/key.pem" 2>/dev/null || true
    # The archive is unpacked under umask 077: the certificate and the authority are public and nginx (another user) must read them.
    chmod 644 "$CERTS_DIR/cert.pem" "$CERTS_DIR/dominet-ca.crt" 2>/dev/null || true; fi
  local root; root="$(ca_root || true)"
  if [ -d "$S/mkcert-ca" ]; then
    if [ -n "$root" ]; then
      mkdir -p "$root"
      if [ -f "$root/rootCA-key.pem" ] && ! cmp -s "$root/rootCA.pem" "$S/mkcert-ca/rootCA.pem"; then
        say "       This machine already has another mkcert authority: kept (devices will have to trust it)."
      else
        # mkcert keeps its key read-only (0400): -f replaces it, as plain cp would fail when the same authority is already here.
        cp -f "$S/mkcert-ca/rootCA.pem" "$S/mkcert-ca/rootCA-key.pem" "$root/"; chmod 600 "$root/rootCA-key.pem"
        say "       mkcert authority restored in $root (devices keep trusting it)"
      fi
    else
      say "       The archive has the mkcert authority but mkcert is not installed here: install it (sudo apt install mkcert) and import again, or keep the certificate files as they are."
    fi
  fi
  if [ -f "$CERTS_DIR/cert.pem" ] && ! openssl x509 -noout -ext subjectAltName -in "$CERTS_DIR/cert.pem" 2>/dev/null | grep -q "$newip"; then
    if command -v mkcert >/dev/null 2>&1 && [ -n "$root" ] && [ -f "$root/rootCA-key.pem" ]; then
      say "       The certificate does not cover $newip: issuing a new one from the restored authority"
      local dom extra; dom="$(valof "$ENV_FILE" LAN_DOMAIN)"; extra="$(valof "$ENV_FILE" CERT_EXTRA_SANS | tr ',' ' ')"
      # Issued by the restored authority only (no `mkcert -install`: this machine's trust store is not touched).
      # shellcheck disable=SC2086
      CAROOT="$root" mkcert -cert-file "$CERTS_DIR/cert.pem" -key-file "$CERTS_DIR/key.pem" "${dom:-localhost}" localhost 127.0.0.1 "$newip" $extra >/dev/null 2>&1 \
        && chmod 600 "$CERTS_DIR/key.pem" || say "       (the new certificate could not be issued: run make certs by hand)"
    else
      say "       Note: the certificate does not cover $newip; use the host name ($(valof "$ENV_FILE" LAN_DOMAIN)) or run make certs."
    fi
  fi

  say "  3/6  Zigbee2MQTT folder"
  if [ -d "$S/zigbee2mqtt" ]; then
    restore_zigbee "$S/zigbee2mqtt"; say "       restored in $ZIGBEE_DIR"
    say "       This is the SAME Zigbee network as on the old machine: never run both coordinators at once (use --no-zigbee for a rehearsal)."
  else say "       none in the archive"; fi

  say "  4/6  Starting the stack (empty database, then replaced)"
  ENV_FILE="$ENV_FILE" PROJECT="$PROJECT" CERTS_DIR="$CERTS_DIR" "${PROD[@]}" up >"$W/up.log" 2>&1 || { tail -20 "$W/up.log" >&2; die "the stack did not start (see above)"; }
  if [ -d "$S/mosquitto-accounts" ] && [ -n "$(ls -A "$S/mosquitto-accounts" 2>/dev/null)" ]; then
    compose cp "$S/mosquitto-accounts/." mosquitto:/mosquitto/accounts >/dev/null
    compose restart mosquitto >/dev/null 2>&1; say "       MQTT accounts restored"
  fi
  if [ -f "$S/wg-easy15-data.tgz" ]; then
    # The stack just started wg-easy on an empty volume: stop it, empty the volume (a SQLite file next to leftovers of another would be
    # corrupt), restore. The restart in 6/6 brings it back; its first configuration is already in the restored settings.
    compose stop wg-easy >/dev/null 2>&1 || true
    vol_wipe "$(vol_name wg-easy15-data)"
    vol_untar "$(vol_name wg-easy15-data)" < "$S/wg-easy15-data.tgz"; say "       VPN restored (administrator, settings, keys, clients)"
  elif [ -f "$S/wg-easy-data.tgz" ]; then
    say "       The archive holds wg-easy 14 VPN data: not restored (wg-easy 15 starts empty; the clients must be created again)."
  fi
  [ ! -f "$S/grafana-data.tgz" ] || { vol_untar "$(vol_name grafana-data)" < "$S/grafana-data.tgz"; say "       Grafana data restored"; }

  say "  5/6  Database$([ "$skip_rs" = 0 ] && [ "$(mf has_reductstore "$P")" = 1 ] && echo " and ReductStore")"
  local pgf rsf; pgf="$(basename "$(ls "$P"/postgres/*.sql.gz | head -1)")"
  compose stop dominet dominet-discovery >/dev/null 2>&1 || true
  # The empty database was migrated to THIS version: a table the archive does not know (a newer migration) would block the dump's
  # `DROP ... IF EXISTS` through its foreign keys. Start from an empty schema; the dump recreates everything (and the migrations
  # missing from the archive are applied after it).
  local dbu dbn; dbu="$(val DB_USER)"; dbn="$(val DB_NAME)"
  compose exec -T postgres psql -X -q -v ON_ERROR_STOP=1 -U "${dbu:-dominet}" -d "${dbn:-dominet}" \
    -c 'DROP SCHEMA public CASCADE; CREATE SCHEMA public;' >/dev/null || die "cannot empty the database before the restore"
  # `docker cp` keeps the owner-only mode of the unpacked files; the application runs as another user and must read them.
  chmod -R a+rX "$P/postgres" "$P/reductstore" 2>/dev/null || true
  compose cp "$P/postgres/." dominet:/backups/postgres >/dev/null
  compose run --rm -T --no-deps dominet restore "/backups/postgres/$pgf" --yes >"$W/restore.log" 2>&1 || { cat "$W/restore.log" >&2; die "the database restore failed"; }
  compose run --rm -T --no-deps dominet migrate >"$W/migrate.log" 2>&1 || { cat "$W/migrate.log" >&2; die "migrations failed after the restore"; }
  say "       $(head -1 "$W/restore.log" | cut -c1-90)"
  if [ "$skip_rs" = 0 ] && [ "$(mf has_reductstore "$P")" = 1 ]; then
    rsf="$(basename "$(ls "$P"/reductstore/*.tar.gz | head -1)")"
    compose cp "$P/reductstore/." dominet:/backups/reductstore >/dev/null
    compose stop reductstore >/dev/null 2>&1 || true
    docker run --rm --user 10001:10001 -v "$(vol_name reductstore-data):/restore" -v "$(vol_name backups-data):/backups" \
      -e BACKUP_DIR=/backups -e DATABASE_URL=postgres://x@localhost/x -e METRICS_ADDR=off "$(tag)" \
      restore "/backups/reductstore/$rsf" --to /restore --force >"$W/rs.log" 2>&1 || { cat "$W/rs.log" >&2; die "the ReductStore restore failed"; }
    say "       $(head -1 "$W/rs.log" | cut -c1-90)"
  fi

  say "  6/6  Restart and check"
  ENV_FILE="$ENV_FILE" PROJECT="$PROJECT" CERTS_DIR="$CERTS_DIR" "${PROD[@]}" up >"$W/up2.log" 2>&1 || { tail -20 "$W/up2.log" >&2; die "the stack did not restart"; }
  compose exec -T dominet dominet sync-rules >/dev/null 2>&1 || true
  ENV_FILE="$ENV_FILE" PROJECT="$PROJECT" CERTS_DIR="$CERTS_DIR" "${PROD[@]}" health || say "       (health check failed: make prod-doctor)"
  say ""
  say "  Done. Sign in with the SAME accounts and passwords as on the old machine."
  say "  Check:  make prod-doctor"
  if [ "$(mf has_zigbee "$P")" = 1 ] && [ -z "$dongle" ]; then say "  Zigbee: plug the dongle in, set ZIGBEE_DEVICE in $ENV_FILE, then make prod-up (the pairing is already restored)."; fi
  [ "$(mf lan_domain "$P")" = "$(valof "$ENV_FILE" LAN_DOMAIN)" ] && say "  Devices use the name $(valof "$ENV_FILE" LAN_DOMAIN): point it at this machine ($newip) on your router/DNS, or set the host name here to match."
}

case "${1:-}" in
  export) shift; cmd_export "$@" ;;
  verify) shift; cmd_verify "$@" ;;
  import) shift; cmd_import "$@" ;;
  *) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 2 ;;
esac
