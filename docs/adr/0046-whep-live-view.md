# 0046 — Camera live view: Rust/WASM WHEP player

- Status: accepted
- Date: 2026-11-08

## Context
ADR 0010: the browser negotiates a WebRTC session with MediaMTX through WHEP; this work makes it an application feature: live page, dashboard widget, reconnection, `camera:<uuid>` WebSocket topic. Decisions: browsers without H.264, rights.

## Decision
- **Player** (`dominet_web::live::WhepPlayer`, client in `whep.rs`, `web-sys`): `recvonly` offer, ICE gathering (2 s at most), `POST application/sdp`, track attached to the `<video>`; **on close, `DELETE` of the session resource** (`Location` header), so MediaMTX releases the connection at once. The decision part is pure and tested on the machine (`StallDetector`, `failure_for_status`).
- **Reconnection through `getStats`**: a read every 2 s; **no new decoded frame for 8 s** (15 s for the first, the time of a key frame) → session closed and restarted, with the WebSocket's growing delay (1 s doubling up to 30 s, plus 25 % random). The player says what it does: "connecting", "stream lost: reconnecting", "no stream".
- **Browser without H.264**: `RTCRtpReceiver.getCapabilities` is queried **before** any attempt; without H.264, an explicit message (and the names of suitable browsers) replaces the silent failure. If MediaMTX answers `400` to a browser without H.264, the same message. *Fallback through refreshed snapshots*: deferred (it needs snapshots).
- **WHEP address**: `<MEDIAMTX_WEBRTC_SCHEME>://<MEDIAMTX_WEBRTC_DOMAIN>/<uuid>/whep`: if the configured domain is a host of the instance itself (`ALLOWED_HOSTS` without `localhost`, `127.0.0.1`, wildcards), it is replaced by the host through which the browser arrived, scheme, port and path kept (`dominet_domain::public_url`, tested). A scheme other than `http` / `https` is refused.
- **CSP**: `connect-src` adds **MediaMTX's origin** when it differs from the page's; in production, behind nginx `/webrtc/` (same origin), nothing to add. The page opens no other origin.
- **Rights**: any signed-in account watches an **active** camera (`camera_live`, page `/surveillance/live/<uuid>`, `camera` widget). An inactive or unknown camera gets the same answer. The browser **never** receives the source stream's address or a credential: only MediaMTX's WHEP address.
- **`camera:<uuid>` WebSocket topic**: accepted for any signed-in account if the camera exists and is active (same refusal for an unknown and an inactive camera); events (motion, detection) arrive later. The topic stays written with the dashed UUID, the contract's canonical form, while the camera's public identifier is written as 32 hexadecimal characters.
- **Widget**: `WidgetDto.camera`; one `<video>` per display (`instance` tells two widgets of the same camera apart); an unknown or inactive camera shows "camera not found or inactive".

## Verification
- **End-to-end test with a real H.264 stream** (`tests/e2e/live.py`, in `make e2e`): `ffmpeg` publishes an H.264 test pattern into the development MediaMTX, the camera pulls it, and **headless Chromium** (DevTools protocol, `websockets`) opens the page: "live" state, 640×360 resolution, playing time that advances, statistics displayed; **the stream is cut** (the player reports it) **then restored** (it reconnects by itself); leaving the page closes the session at MediaMTX. Debian's Firefox ESR (no H.264) shows the explicit message. Without Chromium, `ffmpeg` or `websockets`, the script says so and only checks the Firefox part.
- Rust tests: WHEP address (own hosts), CSP, page and server function (rights, inactive camera, no secret), WebSocket topic, widget.

## Limits
- Headless Chromium validates the **network path and decoding**; the **look** and smoothness are judged in your browser.
- End-to-end latency with a real camera is not measured here (the test pattern is encoded locally); it depends on the camera's GOP (ADR 0010).
- The development WASM (48 MB) delays hydration: the player starts when the page is hydrated (`RELEASE=1 make dev-run`).
