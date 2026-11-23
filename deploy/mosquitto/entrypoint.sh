#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
# Mosquitto start-up (prod and demo, ADR 0055 then 0057): configuration, accounts and rights produced from the environment
# (MQTT_USER, MQTT_PASSWORD) and from the external accounts volume; never a secret in the repository.
# A single listener, 1883: mandatory password for everyone (Docker network and LAN), per-account rights (acl_file). No anonymous access.
set -eu
CONF=/mosquitto/config/mosquitto.conf
mkdir -p /mosquitto/config
sh /deploy/accounts.sh regen

{
  echo "per_listener_settings true"
  # Per-client queue (1000 by default): beyond it, Mosquitto drops the QoS 1 messages of a subscriber slower than the flow.
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
