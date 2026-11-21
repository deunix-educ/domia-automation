# Fresh install on the EliteDesk — step by step

Production install procedure. **You are at the keyboard**; each step says what it does and how to check it. Nothing is deleted: containers and volumes of another installation, if any, are left alone (Compose project `dominet`). Duration: about 1 h, 15 to 20 min of which is the image build.

## 0. Before starting
- A session on the EliteDesk (SSH or screen), with an account in the `docker` group.
- **Zigbee dongle plugged in** (detected at `make prod-init`). No other process may hold it.
- The original **mkcert authority**, if you want your devices to install nothing: copy `rootCA.pem` and `rootCA-key.pem` into the folder given by `mkcert -CAROOT` (created by `mkcert -install` if needed) **before** step 4. Otherwise a new authority is created and each device installs `/dominet-ca.crt` once.
- A sheet of paper or a password manager for: the administrator password, the Grafana password, `DOMINET_MASTER_KEY`.

## 1. Get the code
```bash
git clone git@github.com:deunix-educ/domia-automation.git && cd domia-automation
git checkout r8-termine 2>/dev/null || git checkout main      # the tag will exist at the end of the phase
```

## 2. Check the machine (read only)
```bash
make preflight
```
It checks architecture, Docker and Compose (2.24 or newer), disk space, **ports 80, 443, 1883, 8554, 8189 (TCP and UDP) and 5432 (loopback)**, the dongle, the GPU, mkcert and its authority, `.local` name resolution, and **lists** (without touching them) existing containers and volumes. No `BLOQUANT` (blocking) item may remain; `ATTENTION` items are read and judged. A port held by another service is dealt with first (stop that service, or change `NGINX_HTTP_PORT`, `NGINX_HTTPS_PORT`, `MQTT_LAN_PORT` in `.env` at step 3 — ICE port 8189 and RTSP 8554 are fixed).

## 3. Initialise and review the configuration
```bash
make prod-init       # creates .env (mode 600): generated secrets, detected name and address, detected dongle
$EDITOR .env         # review: EMAIL_* and EMAIL_RECIPIENT (alerts), LAN_DOMAIN, optionally DYNDNS_HOST + WG_ADMIN_HASH (VPN)
make prod-guard      # refuses an empty .env or one left at defaults
```
`make prod-init` prints the Grafana password: note it. **It refuses to overwrite an existing `.env`.** The detected host name must be the one your devices use (`LAN_DOMAIN`).

## 4. Deploy
```bash
make prod-deploy     # certificate, build, start, administrator (prompt), initial data, check
```
The step asks for the administrator's name and password. At the end: `DomiNet est prêt : https://<LAN_DOMAIN>`.
**Right after**: copy `DOMINET_MASTER_KEY` (in `.env`) off the machine. Without it, a restore loses the encrypted camera passwords.

Checks: `make prod-ps` (all healthy, `migrate` and `init` finished), open `https://<LAN_DOMAIN>` and log in, `https://<LAN_DOMAIN>/docs/` (documentation), `https://<LAN_DOMAIN>/grafana/` (anonymous viewer; `admin` + the step-3 password).

## 5. Devices: trust the certificate
On each device that does not know the authority yet, open `http://<LAN_DOMAIN>/dominet-ca.crt` and install it (Android: Settings → Security → Install a CA certificate). With the original authority reused (step 0), there is nothing to do.

## 6. Zigbee2MQTT
With the dongle detected, Zigbee2MQTT starts by itself (profile `zigbee`, interface on `http://<host>:8080`, `ZIGBEE_AUTH_TOKEN` from `.env`). If `deploy/zigbee2mqtt/` already held a pairing it is kept; otherwise pair your devices (permit join). They then appear under **Automation → Devices** (DomiNet's MQTT role).

## 7. LAN MQTT clients
MQTT accepts **no anonymous access**. Each external client gets its own account:
```bash
make prod-mqtt-user NAME=linky       # prints the password ONCE; topic linky/# by default
make prod-mqtt-users                 # lists accounts and their rights
```
In the client: host `LAN_DOMAIN`, port 1883 (or `MQTT_LAN_PORT`), the user name and password. SolarMax is read by DomiNet itself (`SOLARMAX_HOST` setting), it needs no account.

## 8. Cameras, scenes, automations, alerts
- **Cameras**: Administration → Cameras, or Surveillance → Discovery (ONVIF on the LAN); enter the cameras' credentials.
- **Scenes, automations, alert rule**: recreate them in the interface from your own configuration. `make prod-dump-context` produces the new installation's YAML snapshot to compare.
- **Android**: Android → add the device, token shown once, to enter in the app.
- **Settings**: e-mail (SMTP) and SMS (Free Mobile) in Settings; `dominet notify-test` (you only) for a real send.

## 9. Check the result
Open the application, sign in, and check the dashboard and the **Operations** pages (`make prod-health` does the same end to end).

## 10. If something goes wrong
| Symptom | What to do |
|---|---|
| `prod-deploy` stops | read the message; `make prod-logs S=<service>`; fix; rerun `make prod-deploy` (rerunnable) |
| `additional privileges requested: pass "--allow=network.host"` during the build | old checkout (Compose 5.6+ rejects `network: host` in `build`): `git pull`, then rerun `make prod-deploy` |
| A service restarts in a loop | `make prod-ps`, then `make prod-logs S=<service>` |
| Page unreachable | `curl -k https://127.0.0.1/healthz` on the machine; `make prod-logs S=nginx`; check `ALLOWED_HOSTS` and `CSRF_TRUSTED_ORIGINS` |
| The "Sign in" button does not react (the page shows, a private window or another browser works) | the interface did not start in that tab. In order: `Ctrl+Shift+R`; **Brave**: lion icon → turn Shields off for this site (harmless: the site loads no third-party content), sign in, Shields can then be turned back on; **Firefox**: clear the site cache (padlock → Clear cookies and site data) |
| Firefox refuses the certificate while another browser accepts it | Firefox has its own store: Settings → Privacy & Security → Certificates → View Certificates → Authorities → Import `dominet-ca.crt` (downloaded from `http://<LAN_DOMAIN>/dominet-ca.crt`) |
| Start over cleanly | `make prod-down` (volumes kept); to erase **the Rust stack only**: `docker compose -p dominet --env-file .env -f deploy/docker-compose.yml -f deploy/docker-compose.prod.yml down -v` — never `docker volume prune`, which would touch other stacks |

## 11. After the installation: 48 h of monitoring
Twice a day (morning and evening):
```bash
make prod-report                 # READ-ONLY report: services, resources, logs, devices, automations, alerts, backups
HOURS=24 make prod-report        # 24 h window instead of 12
make prod-logs S=dominet N=100   # last 100 lines of one service, without following (without N: continuous follow, Ctrl+C to quit)
make prod-logs S=dominet SINCE=3h | grep -i zigbee2mqtt   # everything since 3 h ago (better than N when looking for a specific event)
```
The report writes nothing and prints no secret; paste it as is so we can read it together. Look for: no service with growing `redemarrages` or `unhealthy`; stable memory and swap; few repeated errors; expected "no message" devices (batteries); explained automation errors; the 03:00 backup present. Grafana → **DomiNet — Vue générale** shows the curves. Reference values: `docs/R8-charge.md`.
