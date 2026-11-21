#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Demarrage de Mosquitto (prod et demo, ADR 0055 puis 0057) : configuration, comptes et droits produits a partir de l'environnement
# (MQTT_USER, MQTT_PASSWORD) et du volume des comptes externes ; jamais de secret dans le depot.
# Un seul listener, 1883 : mot de passe obligatoire pour tous (reseau Docker et LAN), droits par compte (acl_file). Pas d'acces anonyme.
set -eu
CONF=/mosquitto/config/mosquitto.conf
mkdir -p /mosquitto/config
sh /deploy/accounts.sh regen

{
  echo "per_listener_settings true"
  # File d'attente par client (1 000 par defaut) : au-dela, Mosquitto jette les messages QoS 1 d'un abonne plus lent que le flux.
  echo "max_queued_messages 100000"
  echo "persistence true"
  echo "persistence_location /mosquitto/data/"
  echo "log_dest stdout"
  echo "log_type error"
  echo "log_type warning"
  echo "log_type notice"
  echo ""
  echo "listener 1883"
  echo "protocol mqtt"
  echo "allow_anonymous false"
  echo "password_file /mosquitto/config/passwd"
  echo "acl_file /mosquitto/config/acl"
} > "$CONF"

exec /docker-entrypoint.sh mosquitto -c "$CONF"
