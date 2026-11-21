# 0007 — Front end: Leptos SSR + hydration, WebSocket, Fluent i18n

- Status: accepted
- Date: 2026-10-31
- Spike verdict: **go**

## Context
The front end is a single Axum binary that renders Leptos on the server then hydrates in WASM (no separate SPA). The spike validates, end to end: SSR, hydration, WebSocket real time, a dark theme (CSS variables), FR/EN i18n with Fluent. Stack: Leptos 0.8.23, `leptos_axum` 0.8, `leptos-fluent` 0.3.1, `cargo-leptos` 0.3.11, Axum 0.8.

## Results
Browser end-to-end test (headless Firefox ESR, Marionette protocol): **16/16**.
- SSR: complete HTML on the server; `Accept-Language` picks the language; the `lang` cookie wins over the header.
- Hydration: interactive counter after the WASM loads.
- Real time: `tokio::sync::broadcast` hub → WebSocket; values and counter updated without reload; **automatic reconnection** on the WASM side (1 s retry, page "connected" again 3 s after the server restarts).
- i18n: hot FR/EN switch, `<html lang>` synchronised, page title, Fluent plurals (`[one]`/`*[other]`), language kept across reloads (cookie → SSR).
- Theme: dark by default, light through `data-theme` on `<html>` (CSS variables).
- Build: incremental dev ≈ 7 s; cold release ≈ 2 min 50 (4 cores). Release WASM `opt-level = "z"` + LTO + wasm-opt: **448 KB** (190 KB gzip), JS 17 KB, server binary 9.7 MB.

## Decision
- **Leptos 0.8.x** (0.8.23). 0.9.0-rc1 was published the same day and `leptos-fluent` 0.3.1 requires `< 0.9`: reassess when 0.9 final and `leptos-fluent` support are out.
- `cargo-leptos` for the build; a single `dominet-web` crate with `ssr`/`hydrate` features, shared types (WebSocket messages) in `dominet-proto`.
- i18n: `leptos-fluent` with `locales/{en,fr}/main.ftl`, `lang` cookie + `Accept-Language` on the server, **`customise: |b| b.set_use_isolating(false)`** mandatory.
- WebSocket client: direct `web-sys` (about 40 lines) with reconnection; application heartbeat and session authentication added afterwards.
- End-to-end tests: Marionette/Firefox with no dependency is enough; Playwright needs Node, absent from the host.

## Pitfalls found
- Fluent wraps variables in bidi marks U+2068/U+2069, visible in tests and copy-paste → disable them (see above).
- `leptos-fluent` has no `hydrate` feature (only `ssr`, `axum`, `actix`…); `<html lang>` must not be written by hand in the `shell` (duplicate of `leptos-fluent`'s synchronisation).
- `cargo leptos serve` does not rebuild on change (`cargo leptos watch` does); the `env` key of `[package.metadata.leptos]` is ignored by cargo-leptos 0.3.11.
- Not covered here: theme persistence (`localStorage`), widget drag-and-drop, `cargo leptos` in CI/Docker (dependency cache).
