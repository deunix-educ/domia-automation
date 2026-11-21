# ADR 0057 — Authenticated MQTT: one listener, named accounts, rights per topic

Status: accepted; replaces the "anonymous LAN listener" part of ADR 0055.

## Context
ADR 0055 kept a second, **anonymous** listener (1884) for existing devices, switchable off with `MQTT_LAN_ANONYMOUS`, the final decision being deferred. The stack is installed fresh, with no existing MQTT client to spare (SolarMax is read by the `publishers` role of DomiNet, which publishes internally; Linky will publish later).

## Options
1. Keep the anonymous listener, disabled by default: a configuration slip reopens the broker to the LAN.
2. A single account shared by all external clients: simple, but no separation between clients.
3. **One listener, a mandatory password, one account per client and rights per topic** (chosen).

## Decision
- **A single listener, 1883**, `allow_anonymous false`, published on the host's `MQTT_LAN_PORT` (1883 by default). No more 1884, no more `MQTT_LAN_ANONYMOUS`. The stack's services (DomiNet and Zigbee2MQTT) use the internal account `MQTT_USER` / `MQTT_PASSWORD` (right `readwrite #`): same trust zone, the Docker network.
- **External accounts** in the `mosquitto-accounts` volume: `make prod-mqtt-user NAME=linky [TOPIC=linky/#] [ACCESS=read|write|readwrite]` creates the account (generated password shown **once**, pbkdf2-hashed in the volume), `prod-mqtt-del`, `prod-mqtt-users`. The default topic is `NAME/#`: a client only sees and writes its own topic.
- `deploy/mosquitto/accounts.sh` (mounted in the container) regenerates `passwd` and `acl` at every start and at every addition or removal, then sends `SIGHUP` to Mosquitto: **no restart**, connected clients are not cut.
- Names limited to `[a-z0-9_-]`, 32 characters, never the internal account's; topics limited to safe MQTT characters.
- The development stack keeps its local anonymous broker (`mosquitto.dev.conf`, loopback only): tests do not have to manage a secret.

## Consequences
- Every LAN client must receive an account **before** publishing; a refusal shows in Mosquitto's log (`Connection refused: not authorised`).
- Wi-Fi MQTT devices (Tasmota, ESP…) are configured with the user, password and port; sensors that cannot authenticate are not supported on this broker.
- No TLS on MQTT: the password travels in clear on the LAN; TLS (8883) is a possible improvement.
- Checked by `tools/check-stack.sh` (anonymous refused on 1883 from the Docker network and from the host, 1884 closed, external account: publish in its topic, refused elsewhere, wrong password, deletion) and `tools/check-install.sh` (the `make prod-mqtt-*` targets).
