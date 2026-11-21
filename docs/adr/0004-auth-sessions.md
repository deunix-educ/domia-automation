# 0004 — Authentication and sessions

- Status: accepted
- Date: 2026-10-31

## Context
Already decided with the owner: **server-side** sessions (`tower-sessions`, PostgreSQL storage), **argon2id** passwords, `admin` / `user` roles, accounts created with `dominet createsuperuser`. What remains is the implementation detail and a version mismatch discovered during the spikes.

## Version mismatch (checked on crates.io on 2026-10-31)
- `tower-sessions` **0.15.0** (February 2026); `argon2` 0.6.0; `sqlx` **0.9.0**; `axum` 0.8.9.
- The official PostgreSQL store `tower-sessions-sqlx-store` 0.15.0 (January 2025) depends on `tower-sessions` **0.14** and `sqlx` **0.8**: incompatible with `tower-sessions` 0.15 + `sqlx` 0.9.
- `axum-login` 0.18 follows the same branch; not adopted (the need is simple: an extractor and a role check).

## Options
1. Stay on `tower-sessions` 0.14 + `sqlx` 0.8 + the official store — pins `sqlx` back and complicates `sqlx-cli`.
2. **Write our own `SessionStore`** (trait of `tower-sessions-core` 0.15) on `sqlx` 0.9 — about fifty lines, and we control the schema.
3. A fully home-made session layer — reinvents signed cookies, id rotation, expiry.

## Decision
- Option 2: `dominet-store::sessions` implements `SessionStore` (table `session(id, data jsonb, expiry_date, user_id)`, indexes on `expiry_date` and `user_id`). The `user_id` column lets us invalidate every session of an account (password change, deactivation).
- Expired sessions are purged by a scheduler task (`schedule_job`), not by a dedicated thread.
- Passwords: `argon2` (argon2id), random salt, transparent rehash at login when the parameters change.
- Cookie: `HttpOnly; Secure; SameSite=Lax`, identifier regenerated at login (anti-fixation), idle and absolute durations configurable (`AppSetting`).
- Anti brute-force: progressive lockout per (account, IP) pair stored in the database (`login_attempt`), plus per-IP rate limiting at the Axum level; identical error messages for unknown account / wrong password, with a dummy hash to equalise response times.
- CSRF: `Origin`/`Host` header check on every unsafe request, plus a token for forms; WebSockets require the session and check `Origin`.
- Roles: `admin` (full management) and `user` (viewing and commands); checked by an Axum `RequireRole` extractor reused by the Leptos server functions. Android tokens remain a separate mechanism (one UUID per device).
- CLI: `dominet createsuperuser`, `dominet passwd`.

## Consequences
- The home-made store is tested against a real PostgreSQL: creation, expiry, deletion, per-user invalidation.
- Watch `tower-sessions` upgrades; if the official store catches up with `sqlx` 0.9 we can go back to it.

## Implementation notes
- **Hashing**: argon2id `m = 64 MiB, t = 3, p = 1`, about 126 ms per hash on the target Core i5-6500 (release build). The OWASP minimum (19 MiB, 2 passes) costs only about 23 ms: too light for a personal server. At most 4 concurrent hashes (64 MiB each); the hash is transparently replaced at login when its parameters are old.
- **Lockout**: per (account, IP) pair 5 failures, per account 20, per IP 30, window and duration of 15 minutes. The per-pair threshold alone would let an attacker change IP without limit; the per-account threshold stops them at 20 tries per quarter of an hour.
- **Home-made session store** (`PgSessionStore`) on `sqlx` 0.9; `tower-sessions-core` 0.15 still uses `#[async_trait]`. The `user_id` column is derived from the `user_id` key of the data.
- **Pitfall found by a test**: `Session::flush()` destroys the session but **keeps its identifier in memory**, reused at the next save: session fixation stayed possible. `flush()` must be followed by `cycle_id()` (see `auth::session::open`). The test `login_rotates_the_session_id_against_fixation` locks this behaviour.
- **Lifetime**: idle time (`SESSION_IDLE_MINUTES`) is handled by the session layer; the absolute duration (`SESSION_MAX_DAYS`) is checked on every request from the login instant stored in the session.
- Usage details: `docs/authentication.md`.
