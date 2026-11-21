# 0012 — Interface: Leptos server functions and a three-crate split

- Status: accepted
- Date: 2026-11-01

## Context
The front end is server-rendered then hydrated in WASM (ADR 0007). Two decisions are needed: (1) how pages talk to the server and (2) how to organise the crates, knowing that Leptos server functions must exist in the crate compiled for the browser (to provide the client stub) while running server code (state, database, session).

## Options
1. **The browser calls the JSON `/api/*` API with `fetch`.** One contract for all clients, but server-side rendering still has to fetch its data another way (calling the API from itself, or server functions).
2. **Leptos server functions** (`#[server]`): typed end to end, the same code serves server rendering and the browser, the session and application state are directly available. The JSON API stays for the Android app, scripts and tests.
3. Split: (a) everything in `dominet-server` (dependency cycle with the interface), (b) **`dominet-server` as a library, `dominet-web` for the interface (depends on the server under `ssr`), `dominet` for the binary** — chosen.

## Decision
- **Server functions** for the interface, prefix `/sf/…` (outside `/api`, so as not to confuse the two contracts). The JSON API `/api/*` is kept and tested.
- **Three crates**: `dominet-server` (library: config, state, auth, settings, API, CLI), `dominet-web` (components, pages, server functions, i18n; `ssr` feature on the server, `hydrate` on the WASM side) and `dominet` (thin binary: assembles both). There is thus no circular dependency, and `dominet-web` can use `CurrentUser`, `AppState`, etc.
- **Mounting**: `dominet-web::ssr::ui_service` produces an Axum service (pages, server functions, `/pkg` files) mounted as the main router's `fallback_service`. The middlewares (session, origin check, security headers, request id) wrap it **like the API**; a server function is protected against CSRF like any POST.
- **State in server functions**: `AppState` is provided through the Leptos context; authentication goes through the same extractors as the API (`CurrentUser`, `RequireAdmin`) via `leptos_axum::extract_with_state`.
- **Configuration**: `LeptosOptions` is built by the code (`SITE_ROOT`, listen address), without `LEPTOS_*` variables. No hot reload: `make front` then restart.
- **Build**: `cargo leptos build` from the root (`[[workspace.metadata.leptos]]`).

## Consequences
- A business rule used by both the interface and the API lives in the service layer (already the case: `AuthService`, `SettingsService`); JSON routes and server functions are thin wrappers.
- Server functions are not a public contract: they evolve with the interface. Any third-party client goes through `/api`.
- Integration tests of the whole router are in the `dominet` crate (`tests/ui.rs`); hydration is checked in a real browser (`tests/e2e/`).
- Licences/advisories: `BSL-1.0` (`xxhash-rust`) allowed; the "unmaintained" advisory on `paste` (a Leptos compile-time macro) is explicitly ignored in `deny.toml`, to reexamine at every Leptos upgrade.

## Pitfalls found
- `#[server]` needs explicit `prefix` and `endpoint` for stable URLs; accents in page texts are encoded as UTF-8 in the rendered HTML.
- The WASM file is loaded **before** being instantiated: a test that clicks right away loses the first click; wait until a click is taken into account before counting.
- An integration test rendering Leptos blocked once in the full suite without being reproducible (40 runs afterwards, 60 simultaneous requests on the real server without problem): tests now have a 20 s timeout per request.
