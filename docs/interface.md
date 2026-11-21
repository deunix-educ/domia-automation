# Web interface

Server-side rendering (SSR) then WASM hydration, with Leptos 0.8. Decisions: [ADR 0007](adr/0007-leptos-ssr-frontend.md) and
[ADR 0012](adr/0012-interface-architecture.md) (in French).

## Code layout

| Crate | Role |
|---|---|
| `dominet-server` | library: configuration, state, authentication, settings, JSON API `/api`, CLI |
| `dominet-web` | interface: components, pages, Fluent translations, server functions (`/sf/…`), CSS |
| `dominet` | the `dominet` binary: assembles the server and the interface |
| `dominet-proto` | types shared between server and browser (DTOs) |

## Build and run

```bash
make dev-up && make db-migrate             # dev database
cargo run -p dominet -- migrate
DOMINET_PASSWORD='a solid password' cargo run -p dominet -- createsuperuser --username admin
make front                                 # cargo leptos build: WASM + JS + CSS in target/site, binary in target/debug
COOKIE_SECURE=false DATABASE_URL=postgres://dominet:dominet-dev@127.0.0.1:15432/dominet target/debug/dominet serve
# http://127.0.0.1:8000
```

**Pitfall**: the server and the WASM must be compiled with the same settings, otherwise the page renders but stays inert (disabled buttons).
`.cargo/config.toml` fixes them for every command (`--cfg erase_components`, `LEPTOS_OUTPUT_NAME`); do not remove them.

`SITE_ROOT` (default `target/site`) is the static files folder. No hot reload: re-run `make front` then restart the server
after a change.

## Check

- `make ci`: formatting, clippy (server **and** WASM), tests, `cargo deny`, `cargo audit`, `sqlx` metadata.
- `crates/dominet/tests/ui.rs`: full router (FR/EN rendered page, 404, security headers, server function with session, CSRF).
- Browser E2E (hydration, live translations, session):
  ```bash
  firefox --headless --marionette --profile <folder> -no-remote about:blank &
  python3 tests/e2e/home.py http://127.0.0.1:8000/ admin 'a solid password'
  python3 tests/e2e/layout.py http://127.0.0.1:8000/ admin 'a solid password'
  python3 tests/e2e/auth.py http://127.0.0.1:8000/ admin 'a solid password'
  python3 tests/e2e/locations.py http://127.0.0.1:8000/ admin 'a solid password'
  python3 tests/e2e/admin.py http://127.0.0.1:8000/ admin 'a solid password'
  python3 tests/e2e/users.py http://127.0.0.1:8000/ admin 'a solid password'
  python3 tests/e2e/realtime.py http://127.0.0.1:8000/ admin 'a solid password'
  SYSTEM_STATS_SECS=1 …   # server started like this for: python3 tests/e2e/system.py http://127.0.0.1:8000/ admin 'a solid password'
  ```

## Layout

`layout.rs` wraps every page (404 included). For a signed-in account the **sidebar** is part of the server-rendered HTML
(blocking `layout_data` resource), so nothing jumps at hydration. The content offset comes from CSS
(`.vc-sidebar + .vc-main-wrap`), not from a class set in JavaScript.

- **Dynamic navigation**: `/sf/layout` reads the `navbar` table, drops `admin_only` entries for a `user` account and
  splits top/bottom. An anonymous visitor gets an empty answer (no sidebar).
- **Lucide icons**: generated server-side (`crates/dominet-server/src/icons.rs`, shared with the MCP server) from `data/lucide.tsv`, built by `tools/gen-lucide.py` from the
  the official `lucide.min.js` (ISC, see `data/LUCIDE-LICENSE.txt`). Only whitelisted graphic tags are kept; an
  unknown or invalid name yields the `circle-help` icon, never free-form HTML.
- **Collapse** remembered in `localStorage` (`vc-sidebar-collapsed`), mobile menu, full screen, language switcher,
  sign-out (`/sf/logout`, which closes the session server-side).
- **`SsrMode::Async` rendering** on routes: everything is resolved server-side before sending. With out-of-order streaming
  (the default) the sidebar arrived in a `<template>` inserted by JS after `</html>` and the content slid from 0 to 220 px
  (seen in ~2 of 12 loads). `ui.rs` checks that `<aside>` precedes the content and that there is no `<template>`.
- The `ui.rs` tests run **one at a time** (static lock): several Leptos applications rendered in parallel inside one test
  process disturb each other (frozen render or `AppState` context not found, ~15% of runs, 0 of 25 when serial). Concurrency
  on a single application was checked on the real server: 3000 requests, 64 in parallel, all correct.
- The "Administration" link and the system gauges are deliberately absent: admin pages come in 7d-7f, statistics in R2.

## Sign-in, guards and account

- **`/login`**: form (username, password) calling the `login` server function. Expected refusals are *values* of the
  result (`InvalidCredentials`, `Locked { retry_after_s }`), translated in the UI; only an outage is an error (logged,
  never detailed to the browser). An unknown username and a wrong password give the same answer. Brute-force lockout (5
  failures per account+IP pair, 20 per account, 30 per IP over 15 min) applies. The `?next=` parameter is filtered by
  `nav::safe_next` (internal paths only): no open redirect.
- **Guards**: `RequireAuth` wraps `/` and `/account`. For a browser (`Accept: text/html`) an anonymous visitor gets a real
  **302** to `/login?next=...`; the protected page is never sent. An already signed-in account opening `/login` is sent
  to its target. These guards are for display: **every server function re-checks the session**.
- **`/account`**: identity (name, username, email, role, last sign-in) and password change. The current password is
  required; the policy (10 to 128 characters, not only digits, different from the username) is enforced server-side and
  each refusal reason is translated. The **same lockout as sign-in** applies (a stolen session cannot be used to guess the old password): checks are logged as sign-in attempts. A successful change **closes every session** of the account and sends to
  `/login?notice=password-changed`.
- **Before hydration** submit buttons are disabled (`use_hydrated`): otherwise a too-quick submit would go out as a plain
  HTML form. Forms also use `method="post"` so a password never ends up in the address.
- **Voluntary leave** (`leave_session`): sign-out and password change navigate to `/login` and declare the anonymous
  state before reloading; without this flag the guard of the page being left redirected to `/login?next=...` and the
  login page sent back to the home page.
- E2E: `python3 tests/e2e/auth.py url username password` (guard, failure then success, live translation, open redirect
  refused, round-trip password change, sign-out).

## Administration and generic CRUD

Decision: [ADR 0014](adr/0014-generic-crud.md) (in French). `crud.rs` provides `CrudTable`; a page describes its fields (`FieldSpec`),
its list and two actions (save, delete). Admin pages: **"Administration" drawer that can be hidden, on the right** (tab on the right edge of
the screen; administrators only; closes with the arrow, Escape, a click outside or by choosing a page). The links stay in the HTML even when closed (`#vc-nav-admin`), only hidden.
- **Logs**: `/admin/logs` (administrators) browses the application log since its last start (in-memory buffer of 20,000 lines, `dominet_server::logbuf`), newest first, with filters: minimum level, module, text, period, number of lines, refresh every 5 s. The other services stay in `make prod-logs`.
- **Tables**: a truncated cell (narrow column) carries its full value in its `title` (tooltip); "Edit" and "Add" bring you back to the top of the page,
  where the form opens, even from the bottom of a long list.

- **`/admin/locations`**: filterable list (case-insensitive search), add and edit in a form, delete with in-place confirmation.
  Validation errors come from the server **field by field** and are translated under the field. An item deleted in the meantime
  gives a message and a refreshed list, not an error.
- **Permissions**: `RequireAdmin` shows an ordinary account an "Access denied" page (status 403, no data at all); an anonymous
  visitor is sent to sign-in. Every server function (`/sf/locations/{list,save,delete}`) re-checks the role.
- E2E: `python3 tests/e2e/locations.py url admin_user password` (deletes what it creates).
- **`/admin/navbar`**: same component, with dropdown (position, opens in) and number (order) field types and a help text under the
  fields. Server-side field-by-field validation (`NavbarDraft::problems`): icon in lowercase with dashes **and existing in Lucide**
  (otherwise the entry would show the fallback icon), internal URL or `http(s)://` without spaces (`javascript:` refused), target
  `_self`/`_blank`, order between 0 and 32767. The sidebar **updates immediately** (navigation reloaded after every write). An internal
  entry opened in a new tab is a plain HTML link (the router would ignore the target).
- **`/admin/settings`**: the 17 settings are fixed (registry): no CRUD table but one card per category and one editor per type (text,
  integer, decimal, checkbox). A value incompatible with the type is refused with a translated message. **A secret never reaches the
  browser**: it shows "Set / Not set", the field is a password input that is always empty, entering a value replaces it, "Clear" empties
  it; without `DOMINET_MASTER_KEY` saving a secret is refused (dedicated message). Descriptions come from the registry and stay in French
  in both languages. A change applies immediately (service cache and subscribers).
- **Guards and reloads**: `RequireAuth`/`RequireAdmin` use a `Gate` component that reads the resource *under* a `Transition` (otherwise
  server rendering does not wait for it) and reacts only to a **status** change (anonymous / account / administrator) through a memo.
  Without it, reloading the navigation (what the Navigation page does) re-created the displayed page, which then re-read the stale value
  serialised in the initial HTML instead of reloading. Every `Suspense` is a `Transition`: the old content stays visible during a reload.
- E2E: `python3 tests/e2e/admin.py url admin_user password` (navigation and settings; restores everything it changes).

## Users (`/admin/users`)

Same CRUD component, with three more field types: password (never prefilled or displayed), display-only column (full name, last
sign-in) and a field that is **read-only after creation** (username). Everything is validated server-side, field by field: username (1 to
150 characters, letters, digits and `@ . + - _`, case-insensitive uniqueness), email, mobile (normalised), role, password (same policy as
`My account`).

- **"Last administrator" rule**: enforced **in the repository** (`update_profile` and `delete`), not only in the screen. A transaction takes
  a PostgreSQL advisory lock on the administrator set, re-reads the target account and refuses (`RepoError::LastAdmin`) any operation that
  would leave zero active administrators: delete, deactivate, demote. Without the lock, two administrators demoting each other at the same
  time would each see "one remains" and both succeed (race test in `repos.rs` and `ui.rs`, verified by disabling the lock).
- **Protections on your own account** (deliberately stricter than the rule above): an administrator cannot delete, deactivate, demote
  themselves or change their password from this screen (`My account` is for that): their session would be cut mid-page. To lift it, remove
  the `Blocked::Yourself` / `Blocked::OwnPassword` checks in `save_user` and `delete_user`.
- **Immediate effect**: deactivating an account cuts its current session (the account is re-read on every request); resetting its password
  closes all its sessions; deleting an account deletes its sessions (cascade).
- Passwords are never returned or logged (`UserInput` and `NewAccount` hide the field in `Debug`).
- E2E: `python3 tests/e2e/users.py url admin_user password` (deletes the account it creates).

## Server integration, CSP and real time

Decisions: [ADR 0016](adr/0016-csp-and-websocket-hub.md) (in French).

- **Nonce-based CSP**, set **during rendering** (`csp.rs`, called by `App`): a different nonce per response, on Leptos's scripts and in the
  header. No inline script or style: write neither `style="…"` nor `on…="…"` in views (use classes and `on:`). If a feature needs a third-party
  resource (Grafana iframe, camera stream), add the matching directive in `csp::policy`.
- **Page rendering**: a `GET` whose render does not answer within 5 s is abandoned and retried (3 attempts, then 503 with `Retry-After`), a workaround for a Leptos 0.8.23
  defect ([ADR 0021](adr/0021-page-render-guard.md), in French); `tools/stress-cold-render.sh` measures the defect on the image.
- **Static files**: `/pkg/*` revalidated on every load (`no-cache`), pages and server functions `no-store`, gzip, COOP/CORP `same-origin`.
- **WebSocket `/ws/notifications/`**: session required, `Origin` checked, `ping`/`pong` heartbeat (`WS_HEARTBEAT_SECS`), **session re-validated at every
  beat** (sign-out, deactivation, deletion or password change cut the connection), at most 8 connections per account.
- **Browser**: the "Real time" indicator in the sidebar shows the state (connecting, connected, offline); the connection comes back by itself with an
  increasing delay. Notifications are shown as plain text. **Settings → Broadcast a message** (administrators) sends a message to every signed-in user:
  it also checks the chain.
- **Per-topic subscriptions** ([ADR 0019](adr/0019-multiplexed-websocket-protocol.md), in French): on the same connection the client sends
  `{"type":"subscribe","topic":"device:7"}` (or `unsubscribe`); topics are `notifications` (automatic), `system`, `device:<id>`, `camera:<uuid>` (refused until R6).
  The server answers `subscribed`, then sends the topic's **last known state**, or `refused` (`invalid`, `forbidden`, `limit` at 128 subscriptions). In the
  browser: `Realtime::subscribe(&Topic::Device(7))` / `unsubscribe` (reference-counted, restored on every reconnection) and `Realtime::device(id)` to read the state.
- **System gauges** ([ADR 0020](adr/0020-system-statistics.md), in French): processor, memory, disk and temperature in the sidebar, fed by the `system`
  topic (measured every `SYSTEM_STATS_SECS` s). `GET /api/system-stats/` (signed-in account) returns the document described here (`cpu`, `memory`,
  `disk`, `updated_at`) plus `temperature`; `disk` and `temperature` are `null` when the machine does not expose them, and the API answers 503 until a first
  measure exists. Bars get their width through the CSSOM (`style:width`): the CSP forbids a rendered `style` attribute.
- **Settings → Device state simulator** (administrators): publishes a JSON state for a device number and shows live what the browser receives; it is the
  source until MQTT (R3) lands.
- Extending: add a variant to `WsServerMsg` (`dominet-proto`) and, for a new topic, a `Topic` variant and a rule in `authorize` (`web/ws.rs`); publish with
  `state.hub.publish(&topic, message)` (or `publish_retained` to keep the last state); handle the message in `realtime.rs`.
- Tests: `crates/dominet/tests/ws.rs` (real server and WebSocket client: authentication, foreign origin, limit, heartbeats, cut on sign-out/deactivation, subscriptions, last state, refusals, 128 limit),
  `ui.rs` (CSP and nonce on every page, cache, compression), E2E `python3 tests/e2e/realtime.py url admin_user password ["server restart command"]`
  (nonce-less script blocked, WebSocket connected, broadcast, sign-out, reconnection after restart, device state simulator, resubscription after restart).

## Dashboard (`/`)

Decisions: [ADR 0022](adr/0022-dashboard-read-view.md) (in French). Read-only (editing arrives in lot 5).

- **Data**: `dashboard_page(d)` (server function) returns **the account's own** dashboards (never another account's), the open one (`?d=<id>` if it belongs to the account, else the default,
  else the first) and its widgets ready to display: HTML already sanitized, iframe address already checked, last known device state. `create_dashboard(name)` creates a dashboard (the first one
  becomes the default). An account without a dashboard sees an invitation to create one.
- **Placement**: a widget carries the classes `vc-c-N`, `vc-r-N`, `vc-w-N`, `vc-h-N` (column 1–12, row 1–100, width and height 1–12). **Never a `style` attribute** (CSP). On medium and narrow
  screens widgets flow one after the other. Maximum row in the domain: 99.
- **Types**: `text` (HTML sanitized by `ammonia`), `grille`, `grafana` (iframe only towards the origin of `GRAFANA_PUBLIC_URL`, otherwise "address refused"; the CSP gets `frame-src` for that origin),
  `device` (below), `camera` (R6) announced and `scene` ([scenes](scenes.md): name and activation button).
- **Devices**: the widget subscribes to `device:<id>` (ADR 0019) and follows the state live; the frame is green when the device is on, dimmed when unavailable. The requested presentation (`device/light`…)
  wins; when automatic it is inferred from the state keys (`dominet_domain::widget_view`). Controls (ON/OFF, sliders, +/−) are present but **disabled until R3**. Before R3 the state comes from the
  **simulator** (Settings → Device state simulator).
- **Extending**: a new presentation = a `Presentation` variant (domain), a case in `resolve_presentation` if it must be inferred, a `*_view` function in `dashboard.rs`.
- **Editing** ([ADR 0023](adr/0023-dashboard-editing.md), in French) — **administrators only** (every server function refuses the others: 403):
  - *Move*: drag a widget's header (mouse, finger or stylus) on wide screens (≥ 1201 px). A dashed frame shows the target cell; on release `move_widget` saves, or refuses (outside the grid,
    position taken) and the widget snaps back with a message. Persistent (reload).
  - *Dashboards*: `/admin/dashboards` (all accounts; owner = user name, fixed at creation).
  - *Widgets*: `/admin/widgets?d=<id>`; the form only shows the fields of the chosen type (`field_visible` of the generic CRUD); overlap reported on "Column"; `text` widget HTML sanitized when
    saving; an "Edit widgets" button on the dashboard.
  - An ordinary account sees its dashboards without editing them: an administrator fills them for it.
- **Pitfalls**: no `StoredValue::new_local` in a server-rendered component (panic, stalled page); no resource whose source depends on another resource (out-of-order loads at hydration); no braces `{…}`
  in a `.ftl`; wait for hydration ("Real time: connected") before clicking in an E2E.
- Tests: `ui.rs` (invitation, creation, placement, sanitizing, Grafana and CSP, selector and account isolation, editing rights, per-field validation, refused moves), `widget_view` and `html` (unit), E2E `dashboard.py` (prepares its widgets in SQL
  through `E2E_DB_SHELL`, set by `tools/e2e.sh`).

## Translations

Fluent (`crates/dominet-web/locales/{fr,en}/main.ftl`). English by default; the language comes from the `lang` cookie, then
`Accept-Language`; the switcher changes it without reloading. `<html lang>` is kept in sync.

### Adding a language
French and English are the reference languages (documentation, E2E tests); another language is added without touching the code:
```bash
make i18n-new LANGUE=es          # creates crates/dominet-web/locales/es/main.ftl from English (FROM=fr to start from French)
make i18n-status                 # per language: messages still identical to English, missing or extra keys, differing variables
make i18n-status LIST=1          # ... and the list of keys to translate
make front                       # rebuilds the interface: the language shows up in the selector
```
Translate each value **without touching the keys or the variables** (`{ $minutes }`); texts that French shares with English ("OK", "Grafana"…) stay as they are. The name shown in the selector ("Español") comes from the code: it must be known to `leptos-fluent` (`es`, `de`, `it`, `pt-BR`…), otherwise the build stops and says so. The language is then chosen like the others: `lang` cookie, then `Accept-Language`; an unknown language falls back to French.

Translations are **compiled into the binary**: a new language requires rebuilding the image (`make prod-up`). The test `cargo test -p dominet-web --test i18n`, replayed by `make ci`, requires every language to define exactly the keys of English, with the same variables: an incomplete file or a renamed variable fails the push. The documentation (this site) stays in French and English.