#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Comptes MQTT de Mosquitto (prod et demo, R8 lot 2, ADR 0057). Un seul listener (1883, mot de passe obligatoire, jamais anonyme).
#  - le compte interne (MQTT_USER / MQTT_PASSWORD : DomiNet et Zigbee2MQTT) lit et ecrit tout ;
#  - chaque client externe (Linky, SolarMax...) a son compte nominatif et un droit limite a un sujet : volume mosquitto-accounts.
#
#   mqtt-accounts regen                                   regenere config/passwd et config/acl (demarrage)
#   mqtt-accounts add NOM MOT_DE_PASSE [SUJET] [ACCES]    cree ou remplace ; SUJET (defaut NOM/#), ACCES read|write|readwrite (defaut readwrite)
#   mqtt-accounts del NOM
#   mqtt-accounts list
# `add` et `del` demandent a Mosquitto de relire ses fichiers (SIGHUP) : aucun redemarrage, les clients connectes ne sont pas coupes.
set -eu
ACC=/mosquitto/accounts
CFG=/mosquitto/config
PASSWD=$CFG/passwd
ACL=$CFG/acl
mkdir -p "$ACC/acl.d" "$CFG"
[ -f "$ACC/passwd" ] || : > "$ACC/passwd"

regen() {
  : "${MQTT_USER:?MQTT_USER requis}"
  : "${MQTT_PASSWORD:?MQTT_PASSWORD requis}"
  : > "$PASSWD"
  mosquitto_passwd -b -c "$PASSWD" "$MQTT_USER" "$MQTT_PASSWORD" 2>/dev/null
  cat "$ACC/passwd" >> "$PASSWD"
  {
    echo "user $MQTT_USER"
    echo "topic readwrite #"
    echo "topic read \$SYS/#"   # sonde de sante ($SYS n'est pas couvert par « # »)
    for f in "$ACC"/acl.d/*; do [ -f "$f" ] && { echo; cat "$f"; }; done
  } > "$ACL"
  chown mosquitto:mosquitto "$PASSWD" "$ACL" 2>/dev/null || true
  chmod 0600 "$PASSWD" "$ACL"
}

reload() { kill -HUP 1 2>/dev/null || true; }

valid_name() { case "$1" in ""|*[!a-z0-9_-]*) return 1 ;; esac; [ ${#1} -le 32 ] && [ "$1" != "${MQTT_USER:-dominet}" ]; }

case "${1:-}" in
  regen) regen ;;
  add)
    name="${2:-}"; pw="${3:-}"; topic="${4:-$name/#}"; access="${5:-readwrite}"
    valid_name "$name" || { echo "nom invalide (a-z, 0-9, - et _, 32 au plus, different du compte interne) : $name" >&2; exit 2; }
    [ -n "$pw" ] || { echo "mot de passe vide" >&2; exit 2; }
    case "$access" in read|write|readwrite) ;; *) echo "acces invalide : $access (read, write ou readwrite)" >&2; exit 2 ;; esac
    case "$topic" in ""|*" "*|*[!A-Za-z0-9_/#+.-]*) echo "sujet invalide : $topic" >&2; exit 2 ;; esac
    mosquitto_passwd -b "$ACC/passwd" "$name" "$pw" 2>/dev/null
    printf 'user %s\ntopic %s %s\n' "$name" "$access" "$topic" > "$ACC/acl.d/$name"
    regen; reload
    echo "compte $name : $access sur $topic" ;;
  del)
    name="${2:-}"; valid_name "$name" || { echo "nom invalide : $name" >&2; exit 2; }
    mosquitto_passwd -D "$ACC/passwd" "$name" 2>/dev/null || true
    rm -f "$ACC/acl.d/$name"
    regen; reload
    echo "compte $name supprime" ;;
  list)
    echo "$MQTT_USER (interne) : readwrite #"
    for f in "$ACC"/acl.d/*; do [ -f "$f" ] && awk '/^user /{u=$2} /^topic /{print u " : " $2 " " $3}' "$f"; done ;;
  *) sed -n 4,12p "$0"; exit 1 ;;
esac
