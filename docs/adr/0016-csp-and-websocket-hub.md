# 0016 — Nonce-based CSP and the base WebSocket hub

- Status: accepted
- Date: 2026-11-01

## Context
Hardening the server integration of the interface (static files, content security policy) and laying the base of the real-time channel (`/ws/notifications/`) that later work (devices, cameras, Android) extends.

## Options
**CSP**
1. A `<meta http-equiv>` tag set by `leptos_meta` (the method documented by Leptos) — ignores `frame-ancestors`, `report-uri` and `sandbox`.
2. A header set by a middleware with its own nonce — impossible: Leptos generates its scripts' nonce during rendering and does not allow imposing one (the field is private).
3. **A header set during rendering** by the root component, which reads Leptos's nonce (`use_nonce`) and inserts the header into `ResponseOptions` — chosen.
4. `'unsafe-inline'` — refused: it cancels the point of the CSP.

**Real time**
1. SSE or long polling — no bidirectional channel, and the Android contract is WebSocket.
2. **Axum WebSocket + a `tokio::sync::broadcast` hub per topic** — chosen.
3. An external bus (Redis) — removed from the project.

## Decision
- **CSP**: `default-src 'self'; script-src 'self' 'nonce-…' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; form-action 'self'; frame-ancestors 'none'`. A different nonce on every response. No inline style nor `on…=` handler: the only `style=` (the sidebar reopening button) became a class. A test sweeps all pages (same nonce in header and scripts, no forbidden attribute); the end-to-end test checks in Firefox that an injected script without a nonce is blocked and that the WebSocket works despite the policy. The `frame-src` (Grafana) and `media-src`/`connect-src` (camera streams) directives are added with the functions concerned.
- **Static files**: `/pkg/*` with `Cache-Control: no-cache` (revalidate at every load, 304 answer) — file names do not change between versions, and a stale WASM with a fresh JS would break hydration; hash-based naming (`hash-files`) would allow a long cache, a possible improvement. Pages and server functions in `no-store` (private, no history). gzip compression (WASM loses ~70 %). `Cross-Origin-Opener-Policy` and `Cross-Origin-Resource-Policy`: `same-origin`.
- **Hub** (`hub.rs`): topics created on demand, a `broadcast` of 64 messages, a too-slow subscriber loses messages (logged) without blocking emitters; a topic is removed without subscribers; at most 8 simultaneous connections per account.
- **`/ws/notifications/`**: a session is mandatory (401 otherwise, **before** opening the WebSocket); the `Origin` header is checked as for unsafe requests (browsers do not apply the same-origin policy to WebSockets: without it a third-party page could open the connection with the user's cookie); browser messages limited to 1 KiB. **Heartbeat** (`WS_HEARTBEAT_SECS`, 25 s by default): a server `ping`, a `pong` expected before the next (otherwise close 4408); at every heartbeat the **session is revalidated** (it exists, the account is active, its password has not changed) otherwise close 4401: logout, deactivation, deletion or reset also cut real time, at most one heartbeat later.
- **WASM client** (`realtime.rs`): open as long as an account is signed in; reconnection with a doubling delay (1 s to 30 s, +25 % random); 4401 stops the attempts; a state indicator in the sidebar; notifications shown as **text only**.
- **Broadcast**: an administrator-only server function (`broadcast_notification`, Settings page): useful in itself and the only way to check the chain end to end.

## Consequences
- Message format (`WsServerMsg` / `WsClientMsg`, `type` tag) in `dominet-proto`: this is the contract to extend (`device_state`…). The `/ws/devices/`, `/ws/surveillance/…` and `/ws/android/…` routes are other routes on the same hub; the Android app's token authentication replaces the session for `/ws/android/device/{token}/`.
- The hub lives in the memory of a single process (consistent with single-server deployment); spreading over several instances would need an external bus.
- Per-heartbeat revalidation has a cost (two database reads per connection per heartbeat), acceptable at this scale; a revocation event pushed by the hub could replace it later.
- A strict policy means no more inline `style=`/`on…=` and no third-party resources without changing the CSP.
