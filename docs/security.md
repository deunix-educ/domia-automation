# Security

DomiNet is designed for **a home network you control**. This page says what is protected, what is deliberately open on the LAN, and what must never be exposed to the Internet. To report a problem, see `SECURITY.md` at the root of the repository.

## What is exposed by the production stack

| Port | Service | Who can reach it | Protection |
|---|---|---|---|
| 443 (80 redirects) | nginx, the application, Grafana under `/grafana/` | the LAN | TLS; the application requires a login; the only unauthenticated routes are the login page, the health probe and the certificate download (`/dominet-ca.crt`, also on HTTP so a new device can get it) |
| 1883 | MQTT (Mosquitto) | the LAN | **password required for every client**, rights per account on a topic; no anonymous listener |
| 8554 | RTSP (MediaMTX) | the LAN | **an account is needed**: MediaMTX asks DomiNet before every read ([Camera access](camera-access.md)) |
| 8189 (udp, tcp) | WebRTC media (MediaMTX) | the LAN | authorised when the browser starts the view (token), not on the media itself: never forward it |
| 8080 | Zigbee2MQTT interface (only with a dongle) | the LAN | access token (`ZIGBEE_AUTH_TOKEN`) |
| 51820 (udp) and the wg-easy interface | optional VPN profile | the Internet if you forward it | WireGuard keys; the web interface is protected by `WG_ADMIN_PASSWORD` (plain HTTP: keep port 51821 on the home network) |
| 5432 | PostgreSQL | this machine only (`127.0.0.1`) | password |
| 8181 | Adminer (optional profile) | this machine only (`127.0.0.1`) | database password |
| 9997, 9998, 9100, Prometheus, Grafana's own port | internal APIs and metrics | the Docker network only | not published (MediaMTX's API and metrics skip its authentication for that reason) |

## Watching the cameras

Watching a camera over RTSP or WebRTC needs an **account**. In the browser nothing changes for you (a short-lived token tied to your session is sent for you); VLC and phone applications need your user name and a personal password per device, created on **My account → Camera access** and revocable at any time. Every account sees every camera. Details and what to do when a player is refused: [Camera access](camera-access.md).

What this does not do: the video itself is not encrypted end to end over RTSP, WebRTC media is authorised when a view starts rather than packet by packet, and a person who has an account can watch every camera. Keep guests on a separate network or Wi-Fi anyway, and **never forward ports 8554 or 8189 on your router**.

## What is protected

- **Accounts**: argon2id hashes, server-side sessions (opaque cookie, idle and absolute expiry), CSRF protection, lock-out after repeated failures (per account and address, per account, per address), password changes close every session. Passwords: 10 to 128 characters, not only digits, not the user name.
- **Secrets stored in the database** (camera passwords, SMS keys) are encrypted with `DOMINET_MASTER_KEY` (AES-256-GCM). Keep a copy of that key away from the machine: it is not in the backups.
- **Generated secrets**: `make prod-init` generates every password and token; there is **no default password** anywhere, and `make prod-guard` refuses an empty or placeholder value. `.env` is created with mode 600 and is never committed (only the `.env.example.*` templates are).
- **Containers**: the application runs as a non-root user, the images are pinned to a version, and the database has no network access from outside the machine.
- **MCP server** (optional, off by default): access tokens are hashed, scoped, revocable and must expire; every call is audited ([MCP audit](mcp-audit.md)).
- **Scripts of automations** run in a sandbox with an operation limit.
- **Dependencies**: `cargo deny` (licences, sources) and `cargo audit` (vulnerabilities) run in `make ci`. The advisories that are accepted, with the reason, are listed in `deny.toml`.

## Checklist for a safe installation

1. Install on a machine reachable only from your LAN; do not forward ports 80, 443, 1883, 8554, 8189 or 8080 on the router.
2. For remote access use the WireGuard profile (one UDP port) or your own VPN, with a strong `WG_ADMIN_PASSWORD` (and do not forward port 51821).
3. Keep `.env` and the migration archives private (they hold keys); keep `DOMINET_MASTER_KEY` in a password manager.
4. Install the certificate authority on your devices (`/dominet-ca.crt`) rather than clicking through browser warnings.
5. Update with `make prod-update` regularly; read the release notes (`CHANGELOG.md`).
6. Run `make prod-doctor` after any change.
