# ADR 0064 — Authenticated camera viewing: MediaMTX asks DomiNet, per-account viewing tokens and personal stream credentials

Status: **implemented** (2026-11-24, lots 1 to 5; accepted the same day by the owner). The differences between this text and what was built are listed in *As built*. Closes the "per-viewer camera credentials" point of the roadmap and of [To improve](../to-improve.md).

## Context
Today anyone on the home network can watch every camera: MediaMTX allows `read` and `playback` to everybody (`authInternalUsers`, `user: any`, no password), over RTSP (8554) and WebRTC (8189, and WHEP through nginx on `/webrtc/`). Only publishing, the API and the metrics are restricted to the Docker network and the machine. The interface itself is protected by accounts (any signed-in account may watch), so the video is the one place where the account system does not apply. [Security](../security.md) documents the gap and tells people to keep guests on another network.

What the owner decided (2026-11-23): the authentication is **delegated to DomiNet**, **any signed-in account may watch every camera** (no per-camera rights), and each account can obtain **personal, revocable credentials** for external players (VLC, phone applications).

What the MediaMTX documentation says (read 2026-11-23; the page is not versioned and is probably newer than 1.21.3, so the points were checked on the real 1.21.3 binary first):
- `authMethod: http` with `authHTTPAddress`: MediaMTX POSTs `{user, password, token, ip, action, path, protocol, id, query}`; a `20x` status allows, anything else denies. An optional `{"expireAt": "<RFC 3339>"}` body closes the session at that time.
- RTSP clients first send empty credentials; the server must answer `401` so that the client sends them. Browsers and WHEP clients can send `Authorization: Basic …` or `Authorization: Bearer <token>`.
- `authHTTPExclude` skips the HTTP call for some actions (`api`, `metrics`, `pprof` by default). **Verified on 1.21.3 (lot 0):** `publish` can be excluded (`authHTTPExclude: [{action: publish}]`; the list accepts no `ips` field, so the exclusion holds for every address, which is safe because the publishing port is only reachable from the Docker network); `authInternalUsers` is **ignored** under `authMethod: http` (even `api` calls go through the HTTP call, and are refused if DomiNet is down unless excluded); a bare Bearer token arrives in `token` (no `user`, no `password`); with Basic the password arrives in `password` and is copied into `token`; a `?token=` in the WHEP URL is not read as a token (it only shows in `query`).
- No caching of the decisions is documented: one HTTP call per authentication, at least two for an RTSP session.

## Options
- **A. MediaMTX asks DomiNet (`authMethod: http`)** (chosen). One place decides, accounts and revocation are the application's own, browser and external players are covered.
- **B. nginx `auth_request` on `/webrtc/` only.** Simple, but RTSP (8554) and ICE (8189) stay open on the LAN: half of the gap remains.
- **C. One shared read password in the MediaMTX configuration.** Trivial, but not "per viewer": no way to cut one person without changing it for everybody.
- **D. One MediaMTX internal user per viewer, written to the configuration file.** Needs a rewrite and a reload at every change, and duplicates the account store.

## Decision
Option A.
- **MediaMTX** runs with `authMethod: http` and `authHTTPAddress: http://dominet:8000/internal/mediamtx/auth`. The route exists only on the Docker network side: nginx does not proxy it, and the handler refuses any caller outside the Docker network.
- **Excluded from the HTTP call:** `api`, `metrics` and `pprof` (`authHTTPExclude`), and only those. DomiNet calls the API while it starts, before it can answer, and Prometheus must keep scraping when DomiNet is down; their ports (9997, 9998) are not published on the host. `publish` is **not** excluded, although it can be: the exclusion has no address filter, so it would open publishing to the whole LAN (RTSP 8554 is published). The handler decides it, from a private address only (the cameras are pulled, nothing publishes except the demo's fake camera).
- **The decision** (`POST` handler) allows a `read` / `playback` on a camera path only for a valid credential of an active account; `publish` comes from a private address only; empty credentials get `401` (the RTSP challenge); everything else is denied. A denial is counted in a metric and logged without any secret. Failed Basic attempts count towards the same lock-out as the login (ADR 0004).
- **Browser:** when the live page opens, a server function (session required) creates a **viewing token**: 256 random bits, stored hashed (SHA-256), bound to the account, valid a few minutes. The WHEP client sends it as `Authorization: Bearer`; the handler reads it from `token` (never from `query`). It is renewed before it expires.
- **External players:** an account page ("My account" → "Camera access") creates a **personal stream credential**: the account's user name and a generated password shown once, stored hashed, revocable, with a last-use date. The page shows the RTSP address to paste into VLC. One account may hold several (one per device).
- **Same storage model as the MCP tokens** ([ADR 0059](0059-mcp-access-tokens.md)): table with hash, owner, kind, creation and expiry dates, revocation date. No signing key to manage (the `SECRET_KEY` setting was removed).
- **DomiNet's own reads** (the `video` role decodes MediaMTX's re-stream) use a **service credential** generated by `make prod-init` (`MEDIAMTX_READ_USER`, `MEDIAMTX_READ_PASSWORD`), not an IP exception. Trusting the Docker network range for `read` would be wrong: depending on the Docker proxy mode, LAN traffic can appear to come from the bridge address.
- **Revocation** takes effect at the next connection. For an open session DomiNet kicks it through the MediaMTX API (`/v3/*conns/kick`, `/v3/webrtcsessions/kick`), using the session `id` that MediaMTX sends in the authentication call (verified: it is the session id of the API, not the one in the WHEP `Location` header; for RTSP use `rtspsessions`, not `rtspconns`). `expireAt` (verified) also bounds a session; it is the fallback.

## Consequences
- **Breaking change for viewers:** VLC and phone applications must be given a personal credential before they can watch. The release note and [Security](../security.md) say so, with the migration steps.
- **DomiNet becomes a dependency of viewing.** If it is down, MediaMTX refuses reads (fail closed). Recording is not affected: cameras are pulled by MediaMTX and written by the recorder inside the Docker network.
- One extra HTTP call per session (two for RTSP); negligible at this scale.
- The media plane (ICE, 8189) is not authenticated by itself: a WebRTC session is authorised when the WHEP offer is posted. A port forward of 8189 or 8554 to the Internet is still not recommended.
- Out of scope: per-camera rights, Digest authentication, HLS (disabled), end-to-end encryption of RTSP.

## Plan (one lot at a time, each validated before the next)
0. **Spike on MediaMTX 1.21.3**: done 2026-11-24 with a throw-away auth server, `ffmpeg` and `curl`. VLC and a real browser are still to try (lot 2 for VLC, lot 3 for the browser).
1. Table, migration and domain rules (tokens, credentials, expiry), with tests.
2. Authentication route (2a, done), then its wiring (2b): MediaMTX configuration (production and demo; the dev stack stays open until lot 3, which needs it to test the browser), service credential, installer and migration scripts, nginx.
3. Browser flow: viewing token and WHEP client, end-to-end test on the live page.
4. "Camera access" page, revocation and session kick.
5. Documentation (EN and FR), release note, removal of the roadmap and "To improve" entries.

## As built (2026-11-24)
- **Browser token**: the page asks for a new viewing token at **each connection** (including each reconnection) instead of renewing one in the background: MediaMTX checks it only when the WHEP offer is posted, and a running session is not cut when it expires. Closing a session (`DELETE`) is not authenticated by MediaMTX (checked), so the page does not keep the token.
- **Revocation**: the open sessions are found by the **user name** MediaMTX records for them (`GET /v3/{rtsp,webrtc}sessions/list`, then `kick/<id>`), not by the `id` of the authentication call. It is stateless and survives a DomiNet restart, but it cannot tell two credentials of the same account apart: revoking one closes the account's sessions, and a device with another password reconnects. A browser session opened with a viewing token has no user name and is not closed. If MediaMTX cannot be reached the revocation still holds (refusal at the next connection, `expireAt` for a session left open).
- **`expireAt`**: returned for a personal credential (the session ends with the credential) and never for a viewing token.
- **Lock-out**: only failed attempts with a user name count; the empty first request of an RTSP client does not. Successful attempts are not recorded (one line per RTSP session would fill the table).
- **Not excluded from the HTTP call**: `publish` (see Decision). **Excluded**: `api`, `metrics`, `pprof`.
- **Development stack**: its MediaMTX stays open (it runs in Docker, DomiNet on the host). The authenticated behaviour is covered by `tools/e2e.sh live_auth` (browser, throw-away MediaMTX with the shipped configuration) and by the ignored test `real_mediamtx_auth` (RTSP, WHEP, session kick on a real MediaMTX 1.21.3).
- **Not done**: closing the sessions of an account that is disabled or deleted; an audit trail of credential creations and revocations (they are logged, without the secret); the RTSP port shown by the page is the production one (8554), the demo's is 8555.
- **Not tried with a real player**: VLC and phone applications were not tried; `ffmpeg` and `curl` stand in for them.
