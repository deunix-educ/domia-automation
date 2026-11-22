# Troubleshooting

Start with the diagnosis, it is read-only and says what to fix:

```bash
make prod-doctor
```

It checks the machine (configuration file, disk space, certificate), the services (running, restarting, unhealthy), the web address, then asks the application itself (database, migrations, administrator account, master key, MQTT, ReductStore). Each finding is `OK`, `WARNING` (works, but you should know) or `BLOCKING` (something does not work), with the command that fixes it. Its exit code is non-zero when something blocks. The application part alone is `docker compose exec dominet dominet doctor`.

## Common cases

| Symptom | Likely cause | What to do |
|---|---|---|
| The installer stops with "port 443 is already used" | Another web server on the machine | Stop it, or move DomiNet's ports in `.env` (`NGINX_HTTPS_PORT`) |
| The browser warns about the certificate | The device does not know your local authority yet | Open `http://<address>/dominet-ca.crt` once and install it |
| `https://<name>.local` does not resolve | No mDNS on the client or on the machine | Install `avahi-daemon` on the machine, or use the IP address and set `LAN_DOMAIN` |
| Camera passwords are refused | `DOMINET_MASTER_KEY` is empty or was replaced | Restore the original key from your copy; a new key cannot read old secrets |
| The first start is very long | The image is being built locally | Normal on a small machine (15 to 40 minutes); published images avoid it |
| A service restarts in a loop | See its log | `make prod-logs S=<service> N=100` |
| Grafana restarts and the log says `invalid email address for SMTP from_address config` (nginx and the application then wait for it) | `DEFAULT_FROM_EMAIL` ends with an IP address (for example `dominet@192.168.1.104`) | Set `DEFAULT_FROM_EMAIL=dominet@dominet.local` in `.env`, then `make prod-up`. `make prod-doctor` reports it |
| `password authentication failed for user "dominet"` (the `migrate` service fails, nothing else starts); the installer says "a database volume from an earlier installation exists" | The PostgreSQL volume was created by an earlier installation with another password; a new `.env` cannot open it | Put the old `.env` back and run `./install.sh --reuse`. To start from scratch (**erases that data**): `make prod-down`, then `docker volume rm dominet_postgres-data` |
| No data from devices | MQTT or Zigbee2MQTT | `make prod-doctor`, then `make prod-logs S=mosquitto N=50` |

## Asking for help

Run `make prod-report` (read-only: services, resources, logs, automations, alerts, backups) and attach its output. It contains no password; read it before sharing anyway.
