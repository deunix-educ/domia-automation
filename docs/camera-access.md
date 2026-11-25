# Camera access

Watching a camera needs an account. MediaMTX, which serves the video, asks DomiNet before **every** read (RTSP and WebRTC), and DomiNet only says yes to a signed-in account's proof. Anyone on your network who has no account sees nothing, and one person's access can be cut without touching the others. The design is in [ADR 0064](adr/0064-authenticated-camera-viewing.md).

Every account sees every camera: there are no per-camera rights.

## In the browser

Nothing to do. When you open a live view, the page asks DomiNet for a **viewing token** tied to your session (valid five minutes, only the browser holds it) and sends it to MediaMTX with the request. If you are signed out, the live view does not start. The token is needed only to start a view: a view that is running is not cut when the token expires.

## In VLC, a phone application or any RTSP player

1. Sign in, then open **My account → Manage my camera access** (`/account/cameras`).
2. Give the device a name (for example "Phone of Anna") and a validity (30, 90, 180 or 365 days), then **Create the password**.
3. **Copy the password now**: it is shown once and cannot be read back; only its fingerprint is stored.
4. In the player, open `rtsp://<host>:8554/<camera uuid>` (the page lists the address of each active camera) and give your **user name** (the account's) and the **password** of the device.

**Which player.** `ffplay`, `ffmpeg` and `mpv` work (`mpv "rtsp://<user>:<password>@<host>:8554/<camera uuid>"`). The **VLC package of Debian 13 cannot play RTSP at all**, with or without a password: Debian builds it without the `live555` module, so VLC falls back to an old RTSP dialect that MediaMTX refuses (`invalid SETUP path` in its log). This was reproduced against a MediaMTX with no authentication, so it has nothing to do with this feature. Use another player, or the Flatpak build of VLC (not tried here). Phone applications were not tried either.

Create one password per device. If a device is lost, **Revoke** its password: it is refused from the next connection, and the sessions your account has open are closed at once. Another device of yours using its own password reconnects by itself.

The *Last used* column tells you whether a password is still in use before you revoke it.

## What an administrator should know

- **Updating an installation** from a version without this feature: `make prod-update` (or `make prod-up`) adds `MEDIAMTX_READ_USER` and `MEDIAMTX_READ_PASSWORD` to `.env`. This is DomiNet's own account for decoding the video (snapshots, motion, detection); a new installation gets it from `make prod-init`. `make prod-doctor` blocks if they are missing. Nothing to type.
- **Breaking change for viewers**: a player that used to open `rtsp://<host>:8554/<uuid>` without a password is refused until the person creates a password on this page.
- **DomiNet is now needed to watch**: if it is stopped, MediaMTX refuses every read. Recording is not affected (MediaMTX pulls the cameras and the recorder runs inside the Docker network).
- **Lock-out**: failed attempts with a user name count towards the same lock-out as the login (five failures for a user name and address: refused for 15 minutes, even with the right password). A player that has the wrong password and retries in a loop locks itself out.
- **Disabling or deleting an account** refuses it from its next connection; a session already open is not cut. Revoke its passwords first if that matters.
- **Browser sessions** opened with a viewing token have no user name, so revoking a password does not close them; they end with the page.
- **Ports 8554 and 8189 are still not meant for the Internet** ([Security](security.md)). A player that connects from outside the network is the same risk as before, with a password in front of it.
- **Demo**: the demo stack publishes RTSP on 8555, not 8554; the address shown by the page then needs that port.
- **Development stack**: the dev MediaMTX stays open (`make dev-up`); the authenticated behaviour is tested by `tools/e2e.sh live_auth`, which starts a throw-away MediaMTX with the shipped configuration.

## If a player is refused

| Symptom | Likely cause |
|---|---|
| `401 Unauthorized` at once | no password given, wrong user name, or the password was revoked or has expired (the page shows the state) |
| It worked, now refused for a while | too many failures: wait 15 minutes, then check the password |
| Every read refused, browser included | DomiNet is stopped or unreachable from MediaMTX: `make prod-doctor` |
| The browser works but the video role shows no image | `MEDIAMTX_READ_USER` or `MEDIAMTX_READ_PASSWORD` is empty: `make prod-up` |
