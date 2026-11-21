# 0010 — Camera live view: WHEP client in Rust/WASM towards MediaMTX

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go** (reservation lifted: a real 720p live view was validated in a real browser, see below)

## Context
Live view goes through WebRTC/WHEP: MediaMTX re-exposes the cameras' RTSP streams, and the front end (WASM) negotiates the session. Spike: about 140 lines of `web-sys` (`RTCPeerConnection` + `fetch` + `getStats`) integrated in the Leptos page of the first spike; MediaMTX 1.21.3 in development; a real camera (Thingino, H.264 High, RTSP/TCP ingest on two paths: 720p and 360p).

## Results
- **Server-side signalling**: MediaMTX ingests the two camera paths (`ready`, H264 track) and answers **201 + H.264 SDP** (`packetization-mode=1`) to a WHEP H.264 offer (tested by a direct `POST`) for both paths.
- **WASM client**: full negotiation (offer, ICE gathering, `POST`, answer, `ontrack` → `<video>`) in **51 ms**, 25 frames/s, 0 frames lost, average jitter buffer **11 ms**, jitter 0.6 ms, RTT 0 ms (loopback). Measured on a VP8 test source published in MediaMTX.
- **End-to-end latency** (VP8 test source encoding the wall-clock time in the image; measured in the browser through `canvas`, 120 samples): **median 40 ms, p95 59 ms**, min 19 ms, max 59 ms — generation + VP8 encoding + RTSP + MediaMTX + WebRTC + decoding + rendering, on loopback.
- **First frame**: 1.5 s after the click, dominated by the wait for a key frame (1 s GOP of the test source).
- **Test limit**: Debian's Firefox ESR (153) advertises **no H.264 in its WebRTC offer** (VP8, VP9, AV1 only); the `media.peerconnection.video.h264_enabled` and OpenH264 GMP preferences change nothing in headless mode. H.264 decoding of a real camera in a browser could therefore **not be checked automatically**; neither could the real-camera latency (camera encoder included).
- **Reproduced in a real browser** (Debian Firefox ESR in a VNC session): "WHEP HTTP 400", MediaMTX log `closed: codecs not supported by client`. Cause: Debian's `firefox-esr` package disables OpenH264 by default (`pref("media.gmp-gmpopenh264.enabled", false)`), so the WebRTC offer does not propose H.264. It is not a defect of the WHEP client.

## Decision
- `dominet-web::whep` takes over the spike module (`recvonly` offer, ICE gathering ≤ 2 s, `POST application/sdp`, `DELETE` of the resource on close); the WHEP URL is built from the settings `MEDIAMTX_WEBRTC_SCHEME` / `MEDIAMTX_WEBRTC_DOMAIN`, behind nginx `/webrtc/` in production (same origin, no CORS).
- `getStats` statistics (frames decoded/lost, fps, jitter buffer, RTT) are shown in the camera widget and used for automatic reconnection (no frame decoded for N s → new session).
- No transcoding in MediaMTX: the browser must decode the camera's H.264. For browsers without WebRTC H.264, planned fallback: MJPEG/refreshed snapshots through `FrameSource` (ADR 0003).
- **To validate on a browser that decodes WebRTC H.264** (Chrome, Edge, Safari, Mozilla's Firefox; or Debian Firefox ESR with `media.gmp-gmpopenh264.enabled = true` and the plugin installed). A Debian Firefox ESR without the setting will always show the 400 error: plan an explicit message in the UI ("this browser does not decode H.264") rather than a silent failure.

## Consequences / pitfalls
- **ICE port published identically**: MediaMTX announces its internal port (8189) to the browser; Compose must publish `8189:8189` in UDP **and** TCP without shifting (an `18189:8189` breaks the connection), and `webrtcAdditionalHosts` must list the address the browser reaches (127.0.0.1 in development; the LAN and VPN domain/IP in production).
- The display delay depends on the **camera's GOP**: MediaMTX cannot ask an RTSP source for a key frame. Measure on real cameras; prefer GOPs ≤ 2 s.
- Camera credentials appear in no repository file: the spike's MediaMTX paths were created live through the API (`/v3/config/paths/add`), the production config generates them from the database (`mediamtx_sync`).
- The `web-sys` API is verbose but stable; each `Rtc*` type needs its Cargo feature.
